//! The ChatGPT and Work modes' sidebar: Projects and Recents stay, and Recents also lists the
//! account's ChatGPT conversations (`GET /conversations`, loaded by the owner).

use super::{HomeSidebar, RADIUS_ROW};
use crate::{focus, text};
use gpui::{
    AnyElement, Context, Corners, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use theme::ActiveTheme as _;

/// One ChatGPT conversation row; `url` is its canonical `https://chatgpt.com/c/<id>` page.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatGptChat {
    pub id: String,
    pub title: SharedString,
    pub url: String,
}

/// Emitted when a pick or a shortcut changes the mode: the new [`super::MODES`] index.
pub struct ModeChanged(pub usize);

impl EventEmitter<ModeChanged> for HomeSidebar {}

impl HomeSidebar {
    /// ChatGPT (`⌃1`) and Work (`⌃2`) share the ChatGPT surface; Codex keeps its own.
    pub fn chatgpt_surface(&self) -> bool {
        self.mode != super::DEFAULT_MODE
    }

    pub fn chatgpt_chats(&self) -> &[ChatGptChat] {
        &self.chatgpt_chats
    }

    /// The original's `chatgpt-conversations` query drops a failed page without a message,
    /// so only a loaded page replaces the rows.
    pub fn set_chatgpt_chats(&mut self, chats: Vec<ChatGptChat>, cx: &mut Context<Self>) {
        self.chatgpt_chats = chats;
        cx.notify();
    }

    /// The ChatGPT rows under the local Recents rows, or nothing in Codex mode.
    pub(super) fn chatgpt_rows(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<_> = if self.chatgpt_surface() {
            self.chatgpt_chats
                .iter()
                .map(|chat| chat_row(chat, window, cx))
                .collect()
        } else {
            Vec::new()
        };
        div().flex().flex_col().children(rows).into_any_element()
    }
}

/// A 30 px row like Recents: hover fill, the title clipped with the row's right fade. The
/// port has no ChatGPT conversation view, so a click opens the canonical page, like the
/// ChatGPT results of the sidebar search.
fn chat_row(
    chat: &ChatGptChat,
    window: &mut Window,
    cx: &mut Context<HomeSidebar>,
) -> gpui::Stateful<gpui::Div> {
    let id = SharedString::from(format!("chatgpt-chat-{}", chat.id));
    let handle = focus::handle(id.clone(), 0, window, cx);
    let t = cx.theme();
    let url = chat.url.clone();
    let hover = t.ghost_hover;
    let row = div()
        .id(id.clone())
        .group(id.clone())
        .role(gpui::Role::Link)
        .aria_label(chat.title.clone())
        .relative()
        .h(px(30.))
        .child(
            div()
                .absolute()
                .inset_0()
                .opacity(0.)
                .group_hover(id.clone(), |s| s.opacity(1.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(RADIUS_ROW),
                    corner: shapes::Corner::Squircle,
                    fill: Some(hover),
                    ..Default::default()
                })),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(5.))
                .h(px(20.))
                .pl(px(8.))
                .overflow_hidden()
                .layer()
                .child(text(chat.title.clone(), 14., 20., 430., t.text_default_nav))
                .child(shapes::x_fade(px(8.), px(16.))),
        );
    focus::ring(row, &handle, RADIUS_ROW, px(2.), cx).on_click(move |_, _, cx| cx.open_url(&url))
}
