use super::*;
use crate::composer::Composer;
use crate::tooltip::{Side, tooltip};
use gpui::{Entity, FocusHandle, MouseButton, PathBuilder, Stateful, canvas, point};

#[derive(IntoElement)]
pub(super) struct UserMessage {
    pub id: SharedString,
    pub message: SharedString,
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

    fn edit(
        &mut self,
        message: &str,
        thread_id: String,
        turn_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
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
            composer.on_submit(move |submission, window, cx| {
                if let Some(editor) = editor.upgrade() {
                    window.dispatch_action(
                        Box::new(EditMessage {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            message: submission.text.clone(),
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
        let (edit_state, edit_message) = (state.clone(), self.message.clone());
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
        let bubble = match &self.find {
            None => user_bubble(&self.message, t),
            Some(find) => div().flex().flex_col().items_end().pb(px(30.)).child(
                div()
                    .relative()
                    .max_w(relative(0.7))
                    .py(px(10.))
                    .px(px(16.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(22.)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.user_message_background),
                        ..Default::default()
                    }))
                    .child(
                        div()
                            .text_size(px(14.))
                            .line_height(px(22.75))
                            .font_weight(FontWeight(430.))
                            .text_color(white())
                            .child(find.text(&self.message, 0)),
                    ),
            ),
        };
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

/// A file change card's `Undo`: revert the thread to before that turn (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = thread, no_json)]
pub struct RevertTurn(pub String);

/// Copy, Read aloud, Rate response, Fork chat from here: shown under the latest turn; earlier
/// turns (group `group`) show them on hover.
/// ponytail: Read aloud (ChatGPT voice playback) and Rate response (feedback menu) have no
/// handler; wire them with a voice backend and the feedback menu.
pub(super) fn action_row(
    last: bool,
    group: &SharedString,
    turn_id: &str,
    answer: &str,
    t: &Theme,
) -> Div {
    let action = |name: &'static str| {
        let color = t.text_tertiary;
        // `Icon` folds its own opacity into the color, so the hover sets the color.
        let icon = if last {
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
    let (answer, turn_id) = (answer.to_owned(), turn_id.to_owned());
    div()
        .mt(px(6.))
        .ml(px(-4.))
        .h(px(26.))
        .flex()
        .gap(px(2.))
        .child(tooltip(
            action("thread-copy")
                .id("copy-response")
                .aria_label("Copy")
                .role(gpui::Role::Button)
                .focusable()
                .tab_index(0)
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
            action("thread-read-aloud")
                .id("read-response")
                .aria_label("Read aloud"),
            "Read aloud",
            None,
            Side::Top,
        ))
        .child(tooltip(
            action("thread-rate")
                .id("rate-response")
                .aria_label("Rate response"),
            "Rate response",
            None,
            Side::Top,
        ))
        .child(tooltip(
            action("thread-fork")
                .id("fork-thread")
                .aria_label("Fork chat from here")
                .role(gpui::Role::Button)
                .focusable()
                .tab_index(0)
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(ForkThread(turn_id.clone())), cx)
                }),
            "Branch in new chat",
            None,
            Side::Top,
        ))
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
