use super::*;
use crate::composer::Composer;
use crate::tooltip::{Side, tooltip};
use gpui::{
    Anchor, AnyElement, Edges, Entity, FocusHandle, MouseButton, PathBuilder, Stateful, anchored,
    canvas, deferred, point,
};

#[derive(IntoElement)]
pub(super) struct UserMessage {
    pub id: SharedString,
    pub message: SharedString,
    /// The message's non-text inputs: images, file mentions, skills.
    pub attachments: Vec<UserInput>,
    pub sent_at: Option<String>,
    pub find: Option<FindPaint>,
    pub thread_id: String,
    pub editable_turn: Option<String>,
}

struct MessageActions {
    copied: bool,
    focus: FocusHandle,
    copy_focus: FocusHandle,
    reset: Option<Task<()>>,
    editor: Option<Entity<Composer>>,
}

impl MessageActions {
    fn copy(&mut self, message: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(message.to_owned()));
        #[cfg(target_os = "macos")]
        if !cx.is_test()
            && !write_html(
                &objc2_app_kit::NSPasteboard::generalPasteboard(),
                &message_html(message),
            )
        {
            return;
        }
        self.copied = true;
        self.reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.copied = false;
                this.reset = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Opens the editor on the message: its text and local images in the composer, the other
    /// attachments (`extra`) resent unchanged.
    fn edit(
        &mut self,
        message: &str,
        attachments: &[UserInput],
        thread_id: String,
        turn_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        let (images, extra): (Vec<_>, Vec<_>) = attachments
            .iter()
            .partition(|input| matches!(input, UserInput::LocalImage { .. }));
        let images: Vec<std::path::PathBuf> = images
            .into_iter()
            .filter_map(|input| match input {
                UserInput::LocalImage { path } => Some(path.into()),
                _ => None,
            })
            .collect();
        let extra: Vec<serde_json::Value> = extra
            .into_iter()
            .filter_map(|input| serde_json::to_value(input).ok())
            .collect();
        self.editor = Some(cx.new(|cx| {
            let editor = cx.weak_entity();
            let mut composer = Composer::message_edit(
                message,
                move |window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.editor = None;
                            window.blur(cx);
                            cx.notify();
                        })
                        .ok();
                },
                window,
                cx,
            );
            composer.load_queued("", &images, window, cx);
            composer.on_submit(move |submission, window, cx| {
                if let Some(editor) = editor.upgrade() {
                    window.dispatch_action(
                        Box::new(EditMessage {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            message: submission.text.clone(),
                            attachments: submission.attachments.clone(),
                            extra: extra.clone(),
                            editor,
                        }),
                        cx,
                    );
                }
            });
            composer
        }));
        cx.notify();
    }
}

/// `e` in the visible controls color while the pointer is over the message (`group-hover` in
/// the original). A style, not state: a message that scrolls under the pointer paints its
/// controls in the same frame, without a second one.
fn reveal<E: InteractiveElement>(e: E, group: &SharedString, color: Hsla) -> E {
    e.group_hover(group.clone(), move |s| s.text_color(color))
}

impl RenderOnce for UserMessage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bubble = user_bubble(
            &self.id,
            &self.message,
            &self.attachments,
            self.find.as_ref(),
            window,
            cx,
        );
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| MessageActions {
            copied: false,
            focus: cx.focus_handle(),
            copy_focus: cx.focus_handle(),
            reset: None,
            editor: None,
        });
        let s = state.read(cx);
        if let Some(editor) = &s.editor {
            return div().child(editor.clone()).into_any_element();
        }
        let visible = s.focus.contains_focused(window, cx);
        let copied = s.copied;
        let focus = s.focus.clone();
        let t = cx.theme();
        let (edit_state, edit_message, edit_attachments) =
            (state.clone(), self.message.clone(), self.attachments);
        let group = SharedString::from(format!("{}-group", self.id));
        let shown = t.text_tertiary;
        let color = shown.opacity(if visible { 1. } else { 0. });
        let button = |id: &'static str| {
            div()
                .id(SharedString::from(format!("{}-{id}", self.id)))
                .role(gpui::Role::Button)
                .relative()
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(Theme::RADIUS_MD)
                .hover(|s| s.bg(t.ghost_hover))
                .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
        };
        let copy = copy_button(button("copy-message"), state.clone(), self.message.clone())
            .aria_label(if copied { "Copied" } else { "Copy message" })
            .track_focus(&s.copy_focus)
            // The checkmark takes the button's text color.
            .text_color(color)
            .child(if copied {
                checkmark().into_any_element()
            } else {
                reveal(icon("thread-copy", 16., color), &group, shown).into_any_element()
            });
        let copy = reveal(copy, &group, shown);
        let controls = div()
            .absolute()
            .bottom_0()
            .right(px(4.))
            .h(px(26.))
            .flex()
            .items_center()
            .gap(px(8.))
            .children(
                self.sent_at
                    .map(|sent| reveal(text(sent, 12., 16., 400., color), &group, shown)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .child(tooltip(
                        copy,
                        if copied { "Copied" } else { "Copy message" },
                        None,
                        Side::Bottom,
                    ))
                    .when_some(self.editable_turn, |d, turn_id| {
                        d.child(tooltip(
                            button("edit-message")
                                .aria_label("Edit message")
                                .focusable()
                                .tab_index(0)
                                .on_click(move |_, window, cx| {
                                    edit_state.update(cx, |s, cx| {
                                        s.edit(
                                            &edit_message,
                                            &edit_attachments,
                                            self.thread_id.clone(),
                                            turn_id.clone(),
                                            window,
                                            cx,
                                        );
                                    })
                                })
                                .child(reveal(icon("pencil", 16., color), &group, shown)),
                            "Edit message",
                            None,
                            Side::Bottom,
                        ))
                    }),
            );
        // Keep the existing 30 px reserve: revealing controls must not move the transcript.
        bubble
            .relative()
            .id(self.id)
            .group(group)
            .track_focus(&focus)
            .on_mouse_down_out(move |_, window, cx| {
                if focus.contains_focused(window, cx) {
                    window.blur(cx);
                }
            })
            .child(controls)
            .into_any_element()
    }
}

fn copy_button(
    button: Stateful<Div>,
    state: Entity<MessageActions>,
    message: SharedString,
) -> Stateful<Div> {
    let (key_state, key_message, mouse_state) = (state.clone(), message.clone(), state.clone());
    button
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            let focus = mouse_state.read(cx).copy_focus.clone();
            window.focus(&focus, cx);
            cx.stop_propagation();
        })
        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.copy(&message, cx)))
        .on_key_down(move |event, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key_state.update(cx, |s, cx| s.copy(&key_message, cx));
                cx.stop_propagation();
            }
        })
}

/// The copied check, in the text color where it paints (the button's, hover included).
fn checkmark() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            let mut path = PathBuilder::stroke(px(1.5));
            let p = |x, y| bounds.origin + point(px(x), px(y));
            path.move_to(p(3., 8.));
            path.line_to(p(6.5, 11.5));
            path.line_to(p(13., 4.5));
            if let Ok(path) = path.build() {
                window.paint_path(path, window.text_style().color);
            }
        },
    )
    .size(px(16.))
}

#[cfg(target_os = "macos")]
fn message_html(message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<div><p><span>{}</span></p></div>",
        escaped.replace('\n', "<br>")
    )
}

#[cfg(target_os = "macos")]
fn write_html(pasteboard: &objc2_app_kit::NSPasteboard, html: &str) -> bool {
    use objc2_app_kit::NSPasteboardTypeHTML;
    use objc2_foundation::{NSArray, NSString};
    // No owner callback: both clipboard representations are available immediately.
    unsafe {
        pasteboard.addTypes_owner(&NSArray::from_slice(&[NSPasteboardTypeHTML]), None);
        pasteboard.setString_forType(&NSString::from_str(html), NSPasteboardTypeHTML)
    }
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct EditMessage {
    pub thread_id: String,
    pub turn_id: String,
    pub message: String,
    /// The editor's images, sent as `localImage` inputs.
    pub attachments: Vec<std::path::PathBuf>,
    /// The message's other inputs (mentions, skills, remote images), resent unchanged.
    pub extra: Vec<serde_json::Value>,
    pub editor: Entity<Composer>,
}

#[cfg(target_os = "macos")]
fn response_html(answer: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    use std::fmt::Write as _;
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut html = String::new();
    let mut events = Parser::new_ext(
        answer,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    )
    .into_offset_iter();
    while let Some((event, range)) = events.next() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let mut label = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::End(TagEnd::Link) => break,
                        Event::Text(s) | Event::Code(s) => label.push_str(&s),
                        _ => {}
                    }
                }
                let _ = write!(
                    html,
                    "<span data-prompt-link-label=\"{}\" data-prompt-link-href=\"{}\">{}</span>",
                    escape(&label),
                    escape(&dest_url),
                    escape(&answer[range])
                );
            }
            Event::Start(tag) => {
                if let Some(tag) = response_html_tag(tag.to_end()) {
                    let _ = write!(html, "<{tag}>");
                }
            }
            Event::End(tag) => {
                if let Some(tag) = response_html_tag(tag) {
                    let _ = write!(html, "</{tag}>");
                }
            }
            Event::Code(s) => {
                let _ = write!(html, "<code dir=\"ltr\">{}</code>", escape(&s));
            }
            Event::Text(s) | Event::Html(s) | Event::InlineHtml(s) => {
                let _ = write!(html, "<span>{}</span>", escape(&s));
            }
            Event::SoftBreak | Event::HardBreak => html.push_str("<br>"),
            Event::Rule => html.push_str("<hr>"),
            Event::TaskListMarker(checked) => {
                html.push_str(if checked {
                    "<input type=\"checkbox\" disabled checked>"
                } else {
                    "<input type=\"checkbox\" disabled>"
                });
            }
            _ => {}
        }
    }
    html
}

#[cfg(target_os = "macos")]
fn response_html_tag(tag: pulldown_cmark::TagEnd) -> Option<&'static str> {
    use pulldown_cmark::{HeadingLevel, TagEnd};
    Some(match tag {
        TagEnd::Paragraph => "p",
        TagEnd::Heading(HeadingLevel::H1) => "h1",
        TagEnd::Heading(HeadingLevel::H2) => "h2",
        TagEnd::Heading(HeadingLevel::H3) => "h3",
        TagEnd::Heading(HeadingLevel::H4) => "h4",
        TagEnd::Heading(HeadingLevel::H5) => "h5",
        TagEnd::Heading(HeadingLevel::H6) => "h6",
        TagEnd::BlockQuote(_) => "blockquote",
        TagEnd::CodeBlock => "pre",
        TagEnd::List(true) => "ol",
        TagEnd::List(false) => "ul",
        TagEnd::Item => "li",
        TagEnd::Emphasis => "em",
        TagEnd::Strong => "strong",
        TagEnd::Strikethrough => "s",
        _ => return None,
    })
}

/// `Fork chat from here` under a turn: a new chat with the thread up to that turn (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct ForkThread(pub String);

/// Revert the conversation to before a turn.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct RevertTurn(pub String);

#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct PatchTurn {
    pub thread_id: String,
    pub turn_id: String,
}

/// A press inside the conversation of thread `id`: the original marks the open thread read.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct ThreadPressed(pub String);

/// A rating picked in the `Rate response` menu: `feedback/upload` with `classification` and the
/// option label.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct SubmitRating {
    pub turn_id: String,
    pub classification: &'static str,
    pub reason: &'static str,
}

/// The rating menu: `(id, label, classification)`; the original's `turnRatingControls` options
/// (`thumbs_up` then `thumbs_down`). An empty classification is a heading.
pub(super) const RATINGS: [(&str, &str, &str); 13] = [
    ("good-heading", "Good response", ""),
    ("solved_my_task", "Solved my task", "good_result"),
    (
        "followed_my_instructions",
        "Followed my instructions",
        "good_result",
    ),
    (
        "good_code_or_output_quality",
        "Good code / output quality",
        "good_result",
    ),
    ("fast_and_efficient", "Fast and efficient", "good_result"),
    ("useful_autonomy", "Useful autonomy", "good_result"),
    ("bad-heading", "Bad response", ""),
    (
        "incorrect_or_incomplete",
        "Incorrect or incomplete",
        "bad_result",
    ),
    (
        "didnt_follow_my_instructions",
        "Didn't follow my instructions",
        "bad_result",
    ),
    (
        "off_track_or_wrong_scope",
        "Off track / wrong scope",
        "bad_result",
    ),
    ("lost_context", "Lost context", "bad_result"),
    ("slow_or_buggy", "Slow or buggy", "bad_result"),
    (
        "safety_or_legal_concern",
        "Safety or legal concern",
        "safety_check",
    ),
];

/// Focus handles of a turn's buttons. The original's `group-focus-within` shows a turn's hover
/// UI (action row time, diff card subtitle) while any of them has focus.
#[derive(Clone)]
pub(super) struct TurnFocus {
    pub thread_id: String,
    /// The diff card's buttons, which draw the shared keyboard ring.
    pub undo: Entity<crate::focus::ControlFocus>,
    pub view: Entity<crate::focus::ControlFocus>,
    pub copy: FocusHandle,
    pub more: FocusHandle,
    /// One of them has focus.
    pub engaged: bool,
    pub reverted: bool,
    pub patch_pending: bool,
}

impl TurnFocus {
    pub(super) fn new(
        thread_id: String,
        undo: Entity<crate::focus::ControlFocus>,
        view: Entity<crate::focus::ControlFocus>,
        copy: Entity<crate::focus::ControlFocus>,
        more: Entity<crate::focus::ControlFocus>,
        window: &Window,
        cx: &App,
    ) -> Self {
        let [copy, more] = [copy, more].map(|focus| focus.read(cx).handle.clone());
        let engaged = [&undo, &view]
            .map(|focus| &focus.read(cx).handle)
            .into_iter()
            .chain([&copy, &more])
            .any(|handle| handle.is_focused(window));
        Self {
            thread_id,
            undo,
            view,
            copy,
            more,
            engaged,
            reverted: false,
            patch_pending: false,
        }
    }
}

/// `sent_at` is the turn's `Wednesday 9:54 PM`, shown beside the actions on hover or focus.
/// `rate` opens the `Rate response` menu. `menu` is the open More actions menu; `toggle_menu`
/// opens or closes it and `hover_menu` reports the pointer over its trigger.
#[allow(clippy::too_many_arguments)]
pub(super) fn action_row(
    last: bool,
    group: &SharedString,
    answer: &str,
    sent_at: Option<String>,
    menu: Option<AnyElement>,
    rate: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    toggle_menu: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    hover_menu: impl Fn(&bool, &mut Window, &mut App) + 'static,
    focus: &TurnFocus,
    t: &Theme,
) -> Div {
    let engaged = focus.engaged || menu.is_some();
    let action = |name: &'static str| {
        let color = t.text_tertiary;
        // `Icon` folds its own opacity into the color, so the hover sets the color.
        let icon = if last || engaged {
            icon(name, 16., color)
        } else {
            icon(name, 16., color.opacity(0.))
                .group_hover(group.clone(), move |s| s.text_color(color))
        };
        div()
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(Theme::RADIUS_MD)
            .child(icon)
    };
    let (answer, spoken) = (answer.to_owned(), answer.to_owned());
    div()
        .mt(px(6.))
        .ml(px(-4.))
        .min_h(px(26.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(2.))
        .child(tooltip(
            action("thread-copy")
                .id("copy-response")
                .aria_label("Copy")
                .role(gpui::Role::Button)
                .track_focus(&focus.copy)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(answer.clone()));
                    #[cfg(target_os = "macos")]
                    if !cx.is_test() {
                        write_html(
                            &objc2_app_kit::NSPasteboard::generalPasteboard(),
                            &response_html(&answer),
                        );
                    }
                }),
            "Copy response",
            None,
            Side::Top,
        ))
        .child(tooltip(
            action("thread-rate")
                .id("rate-response")
                .aria_label("Rate response")
                .role(gpui::Role::Button)
                .on_click(rate),
            "Rate response",
            None,
            Side::Top,
        ))
        .child(tooltip(
            action("thread-read-aloud")
                .id("read-response")
                .aria_label("Read aloud")
                .role(gpui::Role::Button)
                .on_click(move |_, _, cx| {
                    // shortcut: native system speech, not the original's ChatGPT `/synthesize`
                    // voice (needs a ChatGPT session); a second press stops it.
                    if crate::native_text::is_speaking(cx) {
                        crate::native_text::stop_speaking(cx);
                    } else if let Err(error) = crate::native_text::speak(&spoken, cx) {
                        eprintln!("codex-gpui: cannot read the response aloud: {error:#}");
                    }
                }),
            "Read aloud",
            None,
            Side::Top,
        ))
        .child(
            div()
                .id("more-actions-trigger")
                .relative()
                .on_hover(hover_menu)
                .children(menu)
                .child(tooltip(
                    action("diff-more")
                        .id("more-actions")
                        .aria_label("More actions")
                        .role(gpui::Role::Button)
                        .track_focus(&focus.more)
                        .on_click(toggle_menu),
                    "More actions",
                    None,
                    Side::Top,
                )),
        )
        .children(sent_at.map(|time| {
            let mut stamp = div()
                .ml(px(6.))
                .h(px(20.))
                .flex()
                .items_center()
                .child(text(time, 12., 16., 430., t.text_tertiary));
            stamp = if engaged {
                stamp
            } else {
                stamp
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
            };
            stamp
        }))
}

impl ThreadView {
    pub(super) fn toggle_turn_menu(
        &mut self,
        turn_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.turn_menu.as_ref().is_some_and(|(id, _)| id == turn_id) {
            self.turn_menu = None;
        } else {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            self.turn_menu = Some((turn_id.to_owned(), focus));
        }
        cx.notify();
    }

    /// Closes the menu and returns focus to its trigger.
    fn close_turn_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.turn_menu.take()
            && let Some(trigger) = self.turn_focus.borrow().get(&format!("{id}-more")).cloned()
        {
            let handle = trigger.read(cx).handle.clone();
            handle.focus(window, cx);
        }
        cx.notify();
    }

    fn branch_from_turn_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, _)) = &self.turn_menu {
            let action = ForkThread(id.clone());
            self.close_turn_menu(window, cx);
            window.dispatch_action(Box::new(action), cx);
        }
    }

    /// The original's More actions dropdown (`side: top`): the response time as a label, then
    /// `Branch in new chat`.
    pub(super) fn turn_menu_popup(
        &self,
        started_at: Option<i64>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let hover = t.menu_item_hover;
        let content = crate::composer::popover(t, 220., |menu| {
            menu.children(started_at.map(|at| {
                let (day, time) = day_and_time(at, self.clock.now(), self.clock.utc_offset);
                let label = format!("{day}, {time}");
                div()
                    .id("turn-menu-time")
                    .px(px(8.))
                    .py(px(5.))
                    .role(gpui::Role::MenuItem)
                    .aria_label(label.clone())
                    .aria_disabled(true)
                    .child(text(label, 13., 18.5714, 430., t.text_tertiary))
            }))
            .child(
                div()
                    .id("turn-menu-branch")
                    .h(px(28.5625))
                    .px(px(8.))
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .role(gpui::Role::MenuItem)
                    .aria_label("Branch in new chat")
                    .hover(move |s| s.bg(hover))
                    .child(icon("thread-branch", 16., t.text))
                    .child(text("Branch in new chat", 13., 18.5714, 430., t.text))
                    .on_click(
                        cx.listener(|view, _, window, cx| view.branch_from_turn_menu(window, cx)),
                    ),
            )
        });
        let menu = div()
            .id("turn-menu")
            .role(gpui::Role::Menu)
            .aria_label("More actions")
            .occlude()
            .when_some(self.turn_menu.as_ref(), |d, (_, focus)| {
                d.track_focus(focus)
            })
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                if !view.turn_menu_trigger_hovered {
                    view.turn_menu = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => view.close_turn_menu(window, cx),
                    "enter" | "space" => view.branch_from_turn_menu(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(content);
        deferred(
            div().absolute().bottom(px(30.)).left_0().child(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .snap_to_window_with_margin(Edges::all(px(6.)))
                    .child(menu),
            ),
        )
        .with_priority(3)
        .into_any_element()
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
    use objc2_foundation::{NSArray, NSString};

    #[test]
    fn response_copy_matches_recorded_html() {
        let recording: serde_json::Value =
            serde_json::from_str(include_str!("../../../../refs/interactions/s006.json")).unwrap();
        let plain = recording["result"]["clipboard"][0]["text/plain"]
            .as_str()
            .unwrap();
        let html = recording["result"]["clipboard"][0]["text/html"]
            .as_str()
            .unwrap();
        let board = NSPasteboard::pasteboardWithUniqueName();
        unsafe {
            board.declareTypes_owner(&NSArray::from_slice(&[NSPasteboardTypeString]), None);
            assert!(board.setString_forType(&NSString::from_str(plain), NSPasteboardTypeString));
            assert!(write_html(&board, &response_html(plain)));
            assert_eq!(
                board
                    .stringForType(NSPasteboardTypeString)
                    .unwrap()
                    .to_string(),
                plain
            );
            assert_eq!(
                board
                    .stringForType(NSPasteboardTypeHTML)
                    .unwrap()
                    .to_string(),
                html
            );
            board.clearContents();
        }
    }

    #[test]
    fn rich_copy_preserves_plain_text_and_escapes_html() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        let message = "<script>&\nsecond line";
        unsafe {
            board.declareTypes_owner(&NSArray::from_slice(&[NSPasteboardTypeString]), None);
            assert!(board.setString_forType(&NSString::from_str(message), NSPasteboardTypeString));
            assert!(write_html(&board, &message_html(message)));
            assert_eq!(
                board
                    .stringForType(NSPasteboardTypeString)
                    .unwrap()
                    .to_string(),
                message
            );
            assert_eq!(
                board
                    .stringForType(NSPasteboardTypeHTML)
                    .unwrap()
                    .to_string(),
                "<div><p><span>&lt;script&gt;&amp;<br>second line</span></p></div>"
            );
            board.clearContents();
        }
    }
}
