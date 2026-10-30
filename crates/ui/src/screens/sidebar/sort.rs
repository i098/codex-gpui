//! The Recents header's `Chat sidebar options` menu (the original's `sortMenu`): `Sort
//! chats by` Last updated or Created and, beside ChatGPT chats, the All, Chat and Work
//! filter. The app persists both and sorts again on [`ChatOrderChanged`].
use super::*;
use crate::screens::thread::view::menu::{ContextMenu, Item};

const CHECK: &[u8] = include_bytes!("../../../../../assets/icons/menu-check.svg");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatSort {
    #[default]
    UpdatedAt,
    CreatedAt,
}

/// The original's `flatProjectSidebar.filter`: `All`, `Chat` (ChatGPT chats only) or `Work`
/// (Codex chats only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatFilter {
    #[default]
    All,
    Chats,
    Work,
}

/// The user picked another sort or filter.
pub struct ChatOrderChanged;

impl EventEmitter<ChatOrderChanged> for HomeSidebar {}

const SORTS: [(&str, &str, ChatSort); 2] = [
    ("sort-updated_at", "Last updated", ChatSort::UpdatedAt),
    ("sort-created_at", "Created", ChatSort::CreatedAt),
];
const FILTERS: [(&str, &str, ChatFilter); 3] = [
    ("filter-all", "All", ChatFilter::All),
    ("filter-chats", "Chat", ChatFilter::Chats),
    ("filter-tasks", "Work", ChatFilter::Work),
];

impl HomeSidebar {
    /// `threads` in the chosen order: as `thread/list` returns them (last updated first),
    /// or newest created first.
    /// shortcut: later pages still arrive in update order, so `Created` sorts only the loaded
    /// pages; ask `thread/list` for `sortKey: created_at` when paging must follow it.
    pub(super) fn sorted<'a>(&self, threads: &'a [Thread]) -> Vec<&'a Thread> {
        let mut order: Vec<_> = threads.iter().collect();
        if self.chat_sort == ChatSort::CreatedAt {
            order.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        }
        order
    }

    /// The loaded chats' ids in sidebar order.
    pub fn chat_ids(&self) -> impl Iterator<Item = &str> {
        self.threads.iter().map(|(id, ..)| id.as_str())
    }

    /// The filter in effect: it applies only where ChatGPT chats show.
    pub(super) fn filter_in_effect(&self) -> ChatFilter {
        if self.chatgpt_surface() {
            self.chat_filter
        } else {
            ChatFilter::All
        }
    }

    fn chat_options_items(&self) -> Vec<Item> {
        let item = |id: &'static str, label: &'static str, on: bool| Item {
            icon: on.then_some(CHECK),
            ..Item::new(id, label, true)
        };
        let mut items: Vec<Item> = SORTS
            .iter()
            .map(|(id, label, sort)| item(*id, *label, self.chat_sort == *sort))
            .collect();
        if self.chatgpt_surface() {
            items.push(Item::separator("filter-separator"));
            items.extend(
                FILTERS
                    .iter()
                    .map(|(id, label, filter)| item(*id, *label, self.chat_filter == *filter)),
            );
        }
        items
    }

    fn chat_option(&mut self, id: &'static str, _: String, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((.., sort)) = SORTS.iter().find(|(key, ..)| *key == id) {
            self.chat_sort = *sort;
        } else if let Some((.., filter)) = FILTERS.iter().find(|(key, ..)| *key == id) {
            self.chat_filter = *filter;
        }
        cx.emit(ChatOrderChanged);
        cx.notify();
    }

    /// The `...` button at the right of the `Recents` header, shown on hover or while open.
    pub(super) fn chat_options_button(
        &self,
        group: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handle = focus::handle("chat-options", -1, window, cx);
        let shown = self.chat_options.is_some() || handle.read(cx).handle.is_focused(window);
        let bounds = window.use_keyed_state("chat-options-bounds", cx, |_, _| {
            Bounds::<Pixels>::default()
        });
        let measured = bounds.clone();
        div()
            .absolute()
            .right(px(6.))
            .top(px(3.))
            .when(!shown, |d| d.invisible())
            .group_hover(group, |s| s.visible())
            .child(
                focus::ring(
                    div()
                        .id("chat-options")
                        .debug_selector(|| "chat-options".into())
                        .role(gpui::Role::Button)
                        .aria_label("Chat sidebar options")
                        .relative()
                        .size(px(24.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(cx.theme().ghost_hover))
                        .child(icon("diff-more", 16., cx.theme().text_tertiary))
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |rect, (), _, cx| measured.update(cx, |b, _| *b = rect),
                            )
                            .absolute()
                            .inset_0(),
                        ),
                    &handle,
                    px(8.),
                    px(0.),
                    cx,
                )
                .on_click(cx.listener(
                    move |this, _: &gpui::ClickEvent, window, cx| {
                        let at = *bounds.read(cx);
                        this.chat_options = Some(ContextMenu::open(
                            this.chat_options_items(),
                            String::new(),
                            point(at.left(), at.bottom()),
                            window,
                            cx,
                        ));
                        cx.notify();
                    },
                )),
            )
            .into_any_element()
    }

    pub(super) fn chat_options_overlay(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.chat_options.as_ref()?;
        Some(
            menu.overlay(
                "chat-options-menu",
                "Chat sidebar options",
                |this| &mut this.chat_options,
                Self::chat_option,
                window,
                cx,
            )
            .into_any_element(),
        )
    }
}
