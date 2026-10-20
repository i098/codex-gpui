//! System notifications through the `UNUserNotificationCenter` API.
//!
//! Everything here runs lazily: nothing touches the notification center (and
//! so nothing can trigger the authorization prompt or the framework's
//! not-in-a-bundle abort) until the application posts a notification or
//! registers a response callback.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use block2::RcBlock;
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{
    ForegroundExecutor, SharedString, SystemNotification, SystemNotificationAction,
    SystemNotificationReply, SystemNotificationResponse, Task,
};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNTextInputNotificationAction,
    UNTextInputNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

type NotificationCallback<T> = Rc<RefCell<Option<Box<dyn FnMut(T)>>>>;

enum NotificationEvent {
    Response(SystemNotificationResponse),
    Reply(SystemNotificationReply),
}

fn dispatch<T>(callback: &NotificationCallback<T>, event: T) {
    // Release the borrow before callbacks replace the handler or re-enter the app.
    let handler = callback.borrow_mut().take();
    if let Some(mut handler) = handler {
        handler(event);
        callback.borrow_mut().get_or_insert(handler);
    }
}

pub(crate) struct SystemNotificationState {
    initialized: bool,
    center: Option<NotificationCenter>,
    callback: NotificationCallback<SystemNotificationResponse>,
    reply_callback: NotificationCallback<SystemNotificationReply>,
    _response_task: Option<Task<()>>,
}

impl SystemNotificationState {
    pub(crate) fn new() -> Self {
        Self {
            initialized: false,
            center: None,
            callback: Rc::new(RefCell::new(None)),
            reply_callback: Rc::new(RefCell::new(None)),
            _response_task: None,
        }
    }

    pub(crate) fn show(&mut self, executor: &ForegroundExecutor, notification: SystemNotification) {
        self.initialize(executor);
        let notification = self.prepare_notification(notification);
        if let Some(center) = &self.center {
            center.show(notification);
        }
    }

    pub(crate) fn dismiss(&mut self, executor: &ForegroundExecutor, tag: &str) {
        self.initialize(executor);
        if let Some(center) = &self.center {
            center.dismiss(tag);
        }
    }

    pub(crate) fn on_response(
        &mut self,
        executor: &ForegroundExecutor,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        self.initialize(executor);
        *self.callback.borrow_mut() = Some(callback);
    }

    pub(crate) fn on_reply(
        &mut self,
        executor: &ForegroundExecutor,
        callback: Box<dyn FnMut(SystemNotificationReply)>,
    ) {
        self.initialize(executor);
        *self.reply_callback.borrow_mut() = Some(callback);
    }

    fn prepare_notification(&self, mut notification: SystemNotification) -> SystemNotification {
        if self.reply_callback.borrow().is_none() {
            notification.reply_placeholder = None;
        }
        notification
    }

    fn initialize(&mut self, executor: &ForegroundExecutor) {
        if self.initialized {
            return;
        }
        self.initialized = true;

        let (sender, mut receiver) = mpsc::unbounded();
        self.center = NotificationCenter::new(sender);
        if self.center.is_none() {
            return;
        }

        // Responses arrive from the delegate on an arbitrary thread; this task
        // hands them to the registered callback on the main thread.
        let callback = self.callback.clone();
        let reply_callback = self.reply_callback.clone();
        self._response_task = Some(executor.spawn(async move {
            while let Some(event) = receiver.next().await {
                match event {
                    NotificationEvent::Response(response) => dispatch(&callback, response),
                    NotificationEvent::Reply(reply) => dispatch(&reply_callback, reply),
                }
            }
        }));
    }
}

#[derive(PartialEq, Eq, Hash)]
struct CategoryDefinition {
    actions: Vec<SystemNotificationAction>,
    reply_placeholder: Option<SharedString>,
}

struct NotificationCenter {
    center: Retained<UNUserNotificationCenter>,
    /// The center's `delegate` property is weak; keeping the delegate retained
    /// here keeps response handling alive for the app's lifetime.
    _delegate: Retained<NotificationResponseDelegate>,
    /// Keep all categories registered because macOS replaces the entire set
    /// whenever a category is added.
    categories:
        RefCell<HashMap<CategoryDefinition, (SharedString, Retained<UNNotificationCategory>)>>,
    authorization_requested: Cell<bool>,
}

impl NotificationCenter {
    fn new(sender: mpsc::UnboundedSender<NotificationEvent>) -> Option<Self> {
        // `UNUserNotificationCenter` raises `NSInternalInconsistencyException`
        // ("bundleProxyForCurrentProcess is nil"), aborting the process, when
        // the binary isn't part of an app bundle — which is how dev builds run
        // via `cargo run`. A bundle identifier is only present when launched
        // from a real `.app`, so use it as the guard.
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            log::info!("system notifications disabled: not running from an app bundle");
            return None;
        }

        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = NotificationResponseDelegate::new(sender);
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        Some(Self {
            center,
            _delegate: delegate,
            categories: RefCell::new(HashMap::new()),
            authorization_requested: Cell::new(false),
        })
    }

    fn request_authorization(&self) {
        if self.authorization_requested.replace(true) {
            return;
        }
        let completion = RcBlock::new(|granted: Bool, error: *mut NSError| {
            // SAFETY: when non-null, `error` is a valid `NSError` for the
            // duration of the callback.
            if let Some(error) = unsafe { error.as_ref() } {
                log::warn!(
                    "system notification authorization failed: {}",
                    error.localizedDescription()
                );
            } else if !granted.as_bool() {
                log::info!("system notification authorization denied");
            }
        });
        self.center
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &completion,
            );
    }

    fn show(&self, mut notification: SystemNotification) {
        self.request_authorization();
        let category = self.register_category(CategoryDefinition {
            actions: std::mem::take(&mut notification.actions),
            reply_placeholder: notification.reply_placeholder.take(),
        });
        let request = notification_request(&notification, category.as_deref());
        let completion = RcBlock::new(|error: *mut NSError| {
            // SAFETY: when non-null, `error` is a valid `NSError` for the
            // duration of the callback.
            if let Some(error) = unsafe { error.as_ref() } {
                log::warn!(
                    "failed to deliver system notification: {}",
                    error.localizedDescription()
                );
            }
        });
        self.center
            .addNotificationRequest_withCompletionHandler(&request, Some(&completion));
    }

    fn register_category(&self, definition: CategoryDefinition) -> Option<SharedString> {
        if definition.actions.is_empty() && definition.reply_placeholder.is_none() {
            return None;
        }
        let mut categories = self.categories.borrow_mut();
        if let Some((identifier, _)) = categories.get(&definition) {
            return Some(identifier.clone());
        }
        let identifier =
            SharedString::from(format!("gpui-system-notification-{}", categories.len()));
        let category = notification_category(&identifier, &definition);
        categories.insert(definition, (identifier.clone(), category));
        let all: Vec<_> = categories
            .values()
            .map(|(_, category)| category.clone())
            .collect();
        self.center
            .setNotificationCategories(&NSSet::from_retained_slice(&all));
        Some(identifier)
    }

    fn dismiss(&self, tag: &str) {
        let identifiers = NSArray::from_retained_slice(&[NSString::from_str(tag)]);
        self.center
            .removePendingNotificationRequestsWithIdentifiers(&identifiers);
        self.center
            .removeDeliveredNotificationsWithIdentifiers(&identifiers);
    }
}

fn notification_request(
    notification: &SystemNotification,
    category_identifier: Option<&str>,
) -> Retained<UNNotificationRequest> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notification.title));
    content.setBody(&NSString::from_str(&notification.body));
    if let Some(identifier) = category_identifier {
        content.setCategoryIdentifier(&NSString::from_str(identifier));
    }
    UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&notification.tag),
        &content,
        None,
    )
}

fn notification_category(
    identifier: &str,
    definition: &CategoryDefinition,
) -> Retained<UNNotificationCategory> {
    let mut actions: Vec<_> = definition
        .actions
        .iter()
        .map(|action| {
            UNNotificationAction::actionWithIdentifier_title_options(
                &NSString::from_str(&action.id),
                &NSString::from_str(&action.label),
                UNNotificationActionOptions::empty(),
            )
        })
        .collect();
    if let Some(placeholder) = &definition.reply_placeholder {
        // ponytail: English reply button; accept a localized title when callers need another locale.
        actions.push(
            UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                &NSString::from_str("gpui-inline-reply"),
                &NSString::from_str("Reply"),
                UNNotificationActionOptions::empty(),
                &NSString::from_str("Reply"),
                &NSString::from_str(placeholder),
            )
            .into_super(),
        );
    }
    UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &NSString::from_str(identifier),
        &NSArray::from_retained_slice(&actions),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    )
}

struct DelegateIvars {
    sender: mpsc::UnboundedSender<NotificationEvent>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and
    // `NotificationResponseDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = DelegateIvars]
    struct NotificationResponseDelegate;

    unsafe impl NSObjectProtocol for NotificationResponseDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationResponseDelegate {
        // Called when the user activates a delivered notification, possibly on
        // a non-main thread; the channel hop hands the response to the
        // foreground task holding the receiver.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_notification_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let tag =
                SharedString::from(response.notification().request().identifier().to_string());
            let event = if let Some(reply) =
                response.downcast_ref::<UNTextInputNotificationResponse>()
            {
                NotificationEvent::Reply(SystemNotificationReply {
                    tag,
                    text: SharedString::from(reply.userText().to_string()),
                })
            } else {
                let action = response.actionIdentifier();
                let action_id = if &*action == unsafe { UNNotificationDefaultActionIdentifier } {
                    None
                } else {
                    Some(SharedString::from(action.to_string()))
                };
                NotificationEvent::Response(SystemNotificationResponse { tag, action_id })
            };
            self.ivars().sender.unbounded_send(event).ok();
            completion_handler.call(());
        }

        // Without this, macOS suppresses banners while the app is frontmost.
        // Posting is the application's decision; whether the app is focused is
        // its criterion to apply, not the platform's.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present_notification(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);

impl NotificationResponseDelegate {
    fn new(sender: mpsc::UnboundedSender<NotificationEvent>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { sender });
        // SAFETY: `NSObject`'s `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn without_crash_dialogs(check: impl FnOnce()) {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        check();
        std::panic::set_hook(previous);
    }

    fn notification() -> SystemNotification {
        SystemNotification {
            tag: "turn-owned-initial".into(),
            title: "Review".into(),
            body: "Owned task completed.".into(),
            actions: vec![SystemNotificationAction {
                id: "details".into(),
                label: "Details".into(),
            }],
            reply_placeholder: Some("Reply to ChatGPT".into()),
        }
    }

    #[test]
    fn inline_reply_preserves_native_notification_fields() {
        without_crash_dialogs(|| {
            let notification = notification();
            let category = notification_category(
                "owned-reply-category",
                &CategoryDefinition {
                    actions: notification.actions.clone(),
                    reply_placeholder: notification.reply_placeholder.clone(),
                },
            );
            let request = notification_request(&notification, Some("owned-reply-category"));
            let content = request.content();
            assert_eq!(request.identifier().to_string(), "turn-owned-initial");
            assert_eq!(content.title().to_string(), "Review");
            assert_eq!(content.body().to_string(), "Owned task completed.");
            assert_eq!(
                content.categoryIdentifier().to_string(),
                "owned-reply-category"
            );
            assert!(request.trigger().is_none());

            let actions = category.actions();
            assert_eq!(actions.len(), 2);
            assert_eq!(actions.objectAtIndex(0).identifier().to_string(), "details");
            let reply = actions.objectAtIndex(1);
            let reply = reply
                .downcast_ref::<UNTextInputNotificationAction>()
                .unwrap();
            assert_eq!(reply.textInputPlaceholder().to_string(), "Reply to ChatGPT");
            assert_eq!(reply.textInputButtonTitle().to_string(), "Reply");
            assert_eq!(reply.title().to_string(), "Reply");
        });
    }

    #[test]
    fn a_reply_field_requires_a_registered_handler() {
        without_crash_dialogs(|| {
            let state = SystemNotificationState::new();
            let unavailable = state.prepare_notification(notification());
            assert!(unavailable.reply_placeholder.is_none());
            assert_eq!(unavailable.title.as_ref(), "Review");
            assert_eq!(unavailable.actions[0].id.as_ref(), "details");

            *state.reply_callback.borrow_mut() = Some(Box::new(|_| {}));
            let available = state.prepare_notification(notification());
            assert_eq!(
                available.reply_placeholder.as_deref(),
                Some("Reply to ChatGPT")
            );
        });
    }

    #[test]
    fn a_reply_handler_can_replace_itself() {
        without_crash_dialogs(|| {
            let callback: NotificationCallback<SystemNotificationReply> =
                Rc::new(RefCell::new(None));
            let replacement = Rc::downgrade(&callback);
            let seen = Rc::new(RefCell::new(Vec::new()));
            let first_seen = seen.clone();
            *callback.borrow_mut() = Some(Box::new(move |reply| {
                first_seen.borrow_mut().push(("first", reply.text));
                let next_seen = first_seen.clone();
                *replacement.upgrade().unwrap().borrow_mut() = Some(Box::new(move |reply| {
                    next_seen.borrow_mut().push(("next", reply.text))
                }));
            }));
            dispatch(
                &callback,
                SystemNotificationReply {
                    tag: "turn-owned-initial".into(),
                    text: "one".into(),
                },
            );
            dispatch(
                &callback,
                SystemNotificationReply {
                    tag: "turn-owned-initial".into(),
                    text: "two".into(),
                },
            );
            assert_eq!(
                *seen.borrow(),
                vec![
                    ("first", SharedString::from("one")),
                    ("next", SharedString::from("two"))
                ]
            );
        });
    }
}
