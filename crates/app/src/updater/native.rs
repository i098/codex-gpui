//! Sparkle 2.9 user-driver messages stay on the main thread.
//! Sparkle owns archive verification, privileged installation, and relaunch.
use anyhow::{Context as _, Result, ensure};
use async_channel::{Receiver, Sender};
use block2::{DynBlock, RcBlock};
use objc2::{
    DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject},
};
use objc2_foundation::{
    MainThreadMarker, NSBundle, NSError, NSObject, NSObjectProtocol, NSProcessInfo, NSString,
};
use std::cell::{Cell, RefCell};
use ui::updater::Status;

const SKIP: isize = 0;
const INSTALL: isize = 1;
const DISMISS: isize = 2;
const STAGE_INSTALLING: isize = 2;
/// Sparkle checks in the background on this interval when automatic checks are on.
const CHECK_INTERVAL_SECONDS: f64 = 3600.;

struct Installation {
    reply: RcBlock<dyn Fn(isize)>,
    cancel_choice: isize,
}

struct DriverState {
    events: Sender<Status>,
    standard: Retained<AnyObject>,
    install: RefCell<Option<Installation>>,
    cancel: RefCell<Option<RcBlock<dyn Fn()>>>,
    blocked: Cell<bool>,
    expected: Cell<u64>,
    received: Cell<u64>,
    status: Cell<Status>,
    feed: RefCell<Option<Retained<NSString>>>,
}

impl DriverState {
    fn emit(&self, status: Status) {
        if self.status.replace(status) != status {
            let _ = self.events.try_send(status);
        }
    }

    fn install(&self) {
        if self.blocked.get() {
            return;
        }
        let reply = self.install.borrow_mut().take();
        if let Some(reply) = reply {
            self.emit(Status::Installing);
            reply.reply.call((INSTALL,));
        }
    }

    fn set_managed(&self, managed: bool) {
        self.blocked.set(managed);
        if !managed {
            return;
        }
        let cancel = self.cancel.borrow_mut().take();
        if let Some(cancel) = cancel {
            cancel.call(());
        }
        let install = self.install.borrow_mut().take();
        if let Some(install) = install {
            install.reply.call((install.cancel_choice,));
        }
        self.emit(Status::Idle);
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = DriverState]
    #[name = "CGXUpdaterUserDriver"]
    struct UserDriver;

    unsafe impl NSObjectProtocol for UserDriver {}

    impl UserDriver {
        #[unsafe(method(updater:mayPerformUpdateCheck:error:))]
        fn may_check(&self, _: &AnyObject, _: isize, _: *mut *mut NSError) -> bool {
            !self.ivars().blocked.get()
        }

        #[unsafe(method_id(feedURLStringForUpdater:))]
        fn feed_url(&self, _: &AnyObject) -> Option<Retained<NSString>> {
            self.ivars().feed.borrow().clone()
        }

        #[unsafe(method(showUpdatePermissionRequest:reply:))]
        fn permission(&self, request: &AnyObject, reply: &DynBlock<dyn Fn(*mut AnyObject)>) {
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdatePermissionRequest: request, reply: reply]; }
        }

        #[unsafe(method(showUserInitiatedUpdateCheckWithCancellation:))]
        fn checking(&self, cancellation: &DynBlock<dyn Fn()>) {
            *self.ivars().cancel.borrow_mut() = Some(cancellation.copy());
            self.emit(Status::Checking);
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUserInitiatedUpdateCheckWithCancellation: cancellation]; }
        }

        #[unsafe(method(showUpdateFoundWithAppcastItem:state:reply:))]
        fn found(&self, item: &AnyObject, state: &AnyObject, reply: &DynBlock<dyn Fn(isize)>) {
            let stage: isize = unsafe { msg_send![state, stage] };
            if self.ivars().blocked.get() {
                reply.call((if stage == STAGE_INSTALLING { SKIP } else { DISMISS },));
                return;
            }
            unsafe {
                let information_only: bool = msg_send![item, isInformationOnlyUpdate];
                if information_only {
                    let _: () = msg_send![&*self.ivars().standard, showUpdateFoundWithAppcastItem: item, state: state, reply: reply];
                    return;
                }
                if stage == STAGE_INSTALLING {
                    *self.ivars().install.borrow_mut() = Some(Installation { reply: reply.copy(), cancel_choice: SKIP });
                    self.emit(Status::Ready);
                } else {
                    self.dismiss_standard();
                    reply.call((INSTALL,));
                }
            }
        }

        #[unsafe(method(showUpdateReleaseNotesWithDownloadData:))]
        fn release_notes(&self, data: &AnyObject) {
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdateReleaseNotesWithDownloadData: data]; }
        }

        #[unsafe(method(showUpdateReleaseNotesFailedToDownloadWithError:))]
        fn release_notes_error(&self, error: &NSError) {
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdateReleaseNotesFailedToDownloadWithError: error]; }
        }

        #[unsafe(method(showUpdateNotFoundWithError:acknowledgement:))]
        fn not_found(&self, error: &NSError, reply: &DynBlock<dyn Fn()>) {
            self.emit(Status::Idle);
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdateNotFoundWithError: error, acknowledgement: reply]; }
        }

        #[unsafe(method(showUpdaterError:acknowledgement:))]
        fn error(&self, error: &NSError, reply: &DynBlock<dyn Fn()>) {
            eprintln!("codex-gpui: update failed: {error:?}");
            self.ivars().install.borrow_mut().take();
            self.emit(Status::Idle);
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdaterError: error, acknowledgement: reply]; }
        }

        #[unsafe(method(showDownloadInitiatedWithCancellation:))]
        fn downloading(&self, cancellation: &DynBlock<dyn Fn()>) {
            *self.ivars().cancel.borrow_mut() = Some(cancellation.copy());
            self.ivars().expected.set(0);
            self.ivars().received.set(0);
            self.dismiss_standard();
            self.emit(Status::Downloading(None));
        }

        #[unsafe(method(showDownloadDidReceiveExpectedContentLength:))]
        fn expected(&self, length: u64) {
            self.ivars().expected.set(length);
            self.progress();
        }

        #[unsafe(method(showDownloadDidReceiveDataOfLength:))]
        fn received(&self, length: u64) {
            self.ivars().received.set(self.ivars().received.get().saturating_add(length));
            self.progress();
        }

        #[unsafe(method(showDownloadDidStartExtractingUpdate))]
        fn extracting(&self) {
            self.ivars().cancel.borrow_mut().take();
            self.emit(Status::Installing);
        }

        #[unsafe(method(showExtractionReceivedProgress:))]
        fn extraction_progress(&self, _: f64) {}

        #[unsafe(method(showReadyToInstallAndRelaunch:))]
        fn ready(&self, reply: &DynBlock<dyn Fn(isize)>) {
            if self.ivars().blocked.get() {
                reply.call((SKIP,));
                return;
            }
            *self.ivars().install.borrow_mut() = Some(Installation { reply: reply.copy(), cancel_choice: SKIP });
            self.emit(Status::Ready);
        }

        #[unsafe(method(showInstallingUpdateWithApplicationTerminated:retryTerminatingApplication:))]
        fn installing(&self, terminated: bool, retry: &DynBlock<dyn Fn()>) {
            self.emit(Status::Installing);
            unsafe { let _: () = msg_send![&*self.ivars().standard, showInstallingUpdateWithApplicationTerminated: terminated, retryTerminatingApplication: retry]; }
        }

        #[unsafe(method(showUpdateInstalledAndRelaunched:acknowledgement:))]
        fn installed(&self, relaunched: bool, reply: &DynBlock<dyn Fn()>) {
            self.emit(Status::Idle);
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdateInstalledAndRelaunched: relaunched, acknowledgement: reply]; }
        }

        #[unsafe(method(dismissUpdateInstallation))]
        fn dismiss(&self) {
            self.ivars().install.borrow_mut().take();
            self.ivars().cancel.borrow_mut().take();
            self.dismiss_standard();
            self.emit(Status::Idle);
        }

        #[unsafe(method(showUpdateInFocus))]
        fn focus(&self) {
            unsafe { let _: () = msg_send![&*self.ivars().standard, showUpdateInFocus]; }
        }
    }
);

impl UserDriver {
    fn emit(&self, status: Status) {
        self.ivars().emit(status);
    }

    fn progress(&self) {
        let expected = self.ivars().expected.get();
        let percent = (expected > 0).then(|| {
            ((self.ivars().received.get() as f64 / expected as f64 * 100.)
                .round()
                .clamp(0., 100.)) as u8
        });
        self.emit(Status::Downloading(percent));
    }

    fn dismiss_standard(&self) {
        unsafe {
            let _: () = msg_send![&*self.ivars().standard, dismissUpdateInstallation];
        }
    }
}

pub(super) struct Backend {
    updater: Retained<AnyObject>,
    driver: Retained<UserDriver>,
    base: String,
}

impl Backend {
    pub(super) fn check(&self) {
        if self.driver.ivars().blocked.get() {
            return;
        }
        unsafe {
            let allowed: bool = msg_send![&*self.updater, canCheckForUpdates];
            if allowed {
                let _: () = msg_send![&*self.updater, checkForUpdates];
            }
        }
    }

    pub(super) fn automatic(&self) -> bool {
        unsafe { msg_send![&*self.updater, automaticallyChecksForUpdates] }
    }

    pub(super) fn set_automatic(&self, on: bool) {
        unsafe {
            let _: () = msg_send![&*self.updater, setAutomaticallyChecksForUpdates: on];
        }
    }

    pub(super) fn install(&self) {
        self.driver.ivars().install();
    }

    pub(super) fn set_managed(&self, managed: bool) {
        self.driver.ivars().set_managed(managed);
    }

    /// The original checks once in the background at start when automatic checks are on.
    pub(super) fn check_at_start(&self) {
        if self.automatic() {
            unsafe {
                let _: () = msg_send![&*self.updater, checkForUpdatesInBackground];
            }
        }
    }

    /// Sets the appcast URL that Sparkle reads on its next check.
    pub(super) fn configure(&self, beta: bool, plan: &str, installation: &str) {
        let version = bundle_string("CFBundleShortVersionString")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        let os = NSProcessInfo::processInfo().operatingSystemVersion();
        let os = format!(
            "{}.{}.{}",
            os.majorVersion, os.minorVersion, os.patchVersion
        );
        let feed = super::feed_url(
            &self.base,
            installation,
            std::env::consts::ARCH,
            &version,
            beta,
            &os,
            plan,
        );
        *self.driver.ivars().feed.borrow_mut() = feed.as_deref().map(NSString::from_str);
    }
}

fn bundle_string(key: &str) -> Option<String> {
    let bundle = NSBundle::mainBundle();
    let value = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?;
    value
        .downcast_ref::<NSString>()
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

pub(super) fn start(
    beta: bool,
    plan: &str,
    installation: &str,
) -> Result<Option<(Backend, Receiver<Status>)>> {
    let (Some(feed), Some(key)) = (bundle_string("SUFeedURL"), bundle_string("SUPublicEDKey"))
    else {
        return Ok(None);
    };
    super::validate(&feed, &key)?;
    let mtm = MainThreadMarker::new().context("updater requires the main thread")?;
    let bundle = NSBundle::mainBundle();
    let frameworks = bundle
        .privateFrameworksPath()
        .context("application has no Frameworks directory")?;
    let path = NSString::from_str(&format!("{frameworks}/Sparkle.framework"));
    let framework = NSBundle::bundleWithPath(&path).context("Sparkle.framework is missing")?;
    ensure!(unsafe { framework.load() }, "cannot load Sparkle.framework");
    let class =
        AnyClass::get(c"SPUStandardUserDriver").context("Sparkle user driver is unavailable")?;
    let standard: Retained<AnyObject> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![class, alloc];
        msg_send![allocated, initWithHostBundle: &*bundle, delegate: std::ptr::null::<AnyObject>()]
    };
    let (events, receiver) = async_channel::unbounded();
    let driver = UserDriver::alloc(mtm).set_ivars(DriverState {
        events,
        standard,
        install: RefCell::new(None),
        cancel: RefCell::new(None),
        blocked: Cell::new(false),
        expected: Cell::new(0),
        received: Cell::new(0),
        status: Cell::new(Status::Idle),
        feed: RefCell::new(None),
    });
    let driver: Retained<UserDriver> = unsafe { msg_send![super(driver), init] };
    let class = AnyClass::get(c"SPUUpdater").context("Sparkle updater is unavailable")?;
    let updater: Retained<AnyObject> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![class, alloc];
        msg_send![allocated, initWithHostBundle: &*bundle, applicationBundle: &*bundle, userDriver: &*driver, delegate: &*driver]
    };
    let backend = Backend {
        updater,
        driver,
        base: feed,
    };
    backend.configure(beta, plan, installation);
    unsafe {
        let _: () = msg_send![&*backend.updater, setUpdateCheckInterval: CHECK_INTERVAL_SECONDS];
    }
    let mut error: Option<Retained<NSError>> = None;
    let started: bool = unsafe { msg_send![&*backend.updater, startUpdater: &mut error] };
    ensure!(
        started,
        "{}",
        error
            .map(|error| error.localizedDescription().to_string())
            .unwrap_or_else(|| "Sparkle could not start".into())
    );
    Ok(Some((backend, receiver)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn managed_policy_cancels_a_staged_update_before_install() {
        let (events, receiver) = async_channel::unbounded();
        let choice = Rc::new(Cell::new(None));
        let reply_choice = choice.clone();
        let canceled = Rc::new(Cell::new(false));
        let cancel_called = canceled.clone();
        let standard = unsafe { msg_send![objc2::class!(NSObject), new] };
        let state = DriverState {
            events,
            standard,
            install: RefCell::new(Some(Installation {
                reply: RcBlock::new(move |value| reply_choice.set(Some(value))),
                cancel_choice: SKIP,
            })),
            cancel: RefCell::new(Some(RcBlock::new(move || cancel_called.set(true)))),
            blocked: Cell::new(false),
            automatic: Cell::new(true),
            expected: Cell::new(0),
            received: Cell::new(0),
            status: Cell::new(Status::Ready),
            feed: RefCell::new(None),
        };

        state.set_managed(true);
        state.install();

        assert!(canceled.get());
        assert_eq!(choice.get(), Some(SKIP));
        assert_eq!(receiver.try_recv().unwrap(), Status::Idle);
        assert!(receiver.try_recv().is_err());
        assert!(state.install.borrow().is_none());
    }
}
