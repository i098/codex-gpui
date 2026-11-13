//! The Recents header's `Chat sidebar options` menu (`refs/chat-options*`): `Organize sidebar`
//! (`By project`, `In one list`), `Sort chats by` (`Last updated`, `Manual order`), `Show` ▸
//! `Projects`, and `New section`. The app persists [`ChatOptions`] and sorts again on
//! [`ChatOrderChanged`].
use super::*;
use crate::composer::popover;

/// The original's `chatSortMode`. 26.1007 maps a stored `created_at` to `updated_at`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatSort {
    #[default]
    UpdatedAt,
    Manual,
}

/// The original's sidebar `mode`: projects with their chats, or every chat in one list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Organize {
    #[default]
    Project,
    List,
}

/// The menu's saved choices.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChatOptions {
    pub sort: ChatSort,
    pub organize: Organize,
    /// `sidebarShowProjectsInRecents`: Recents also lists the chats of a project.
    pub show_projects: bool,
    /// `Manual order`: chat ids top to bottom, kept from when it was picked.
    pub manual_order: Vec<String>,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            sort: ChatSort::default(),
            organize: Organize::default(),
            show_projects: true,
            manual_order: Vec::new(),
        }
    }
}

/// The user changed a choice in the menu.
pub struct ChatOrderChanged;
/// `New section`: the app opens the `New section` dialog.
pub struct NewSectionRequested;

impl EventEmitter<ChatOrderChanged> for HomeSidebar {}
impl EventEmitter<NewSectionRequested> for HomeSidebar {}

const ROW: f32 = 28.5714;
const WIDTH: f32 = 220.;
const SUB_WIDTH: f32 = 180.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Organize,
    Sort,
    Projects,
    NewSection,
}

impl Entry {
    fn sub(self) -> &'static [(&'static str, Option<&'static str>)] {
        match self {
            Entry::Organize => &[
                ("By project", Some("sidebar-organize-project")),
                ("In one list", Some("sidebar-organize-list")),
            ],
            Entry::Sort => &[("Last updated", None), ("Manual order", None)],
            Entry::Projects | Entry::NewSection => &[],
        }
    }
}

/// The open menu: the highlighted row and, with a submenu open, its highlighted row.
pub(super) struct OptionsMenu {
    /// The trigger's bounds; the box opens 1 px right of its left edge and 2 px below it.
    at: Bounds<Pixels>,
    active: Option<usize>,
    sub: Option<(Entry, Option<usize>)>,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

impl HomeSidebar {
    /// `threads` in the chosen order: as `thread/list` returns them (last updated first), or
    /// the saved manual order with chats it does not know yet on top.
    pub(super) fn sorted<'a>(&self, threads: &'a [Thread]) -> Vec<&'a Thread> {
        let mut order: Vec<_> = threads.iter().collect();
        if self.chat_options.sort == ChatSort::Manual {
            let manual = &self.chat_options.manual_order;
            order.sort_by_key(|t| {
                manual
                    .iter()
                    .position(|id| *id == t.id)
                    .map_or(0, |i| i + 1)
            });
        }
        order
    }

    /// The loaded chats' ids in sidebar order.
    pub fn chat_ids(&self) -> impl Iterator<Item = &str> {
        self.threads.iter().map(|(id, ..)| id.as_str())
    }

    /// Whether Recents lists `id`: without `Show` ▸ `Projects`, a project's chats show only
    /// under their project.
    pub(super) fn in_recents(&self, id: &str) -> bool {
        let cwd = self.cwds.get(id);
        // ChatGPT mode's chats are threads of folders under `Documents/ChatGPT`; Codex mode lists
        // only its own.
        let chatgpt_chat = cwd.is_some_and(|cwd| {
            let parts: Vec<_> = std::path::Path::new(cwd.as_ref()).components().collect();
            parts
                .windows(3)
                .any(|w| w[0].as_os_str() == "Documents" && w[1].as_os_str() == "ChatGPT")
        });
        (self.chatgpt_surface() || !chatgpt_chat)
            && (self.chat_options.show_projects
                || self.chat_options.organize == Organize::List
                || !self
                    .projects
                    .iter()
                    .any(|p| p.root.as_ref().is_some_and(|root| cwd == Some(root))))
    }

    /// The rows top to bottom; `Show` ▸ `Projects` only while organized by project.
    fn entries(&self) -> Vec<Entry> {
        let mut rows = vec![Entry::Organize, Entry::Sort];
        if self.chat_options.organize == Organize::Project {
            rows.push(Entry::Projects);
        }
        rows.push(Entry::NewSection);
        rows
    }

    fn sub_checked(&self, entry: Entry, index: usize) -> bool {
        match entry {
            Entry::Organize => {
                self.chat_options.organize == [Organize::Project, Organize::List][index]
            }
            Entry::Sort => self.chat_options.sort == [ChatSort::UpdatedAt, ChatSort::Manual][index],
            Entry::Projects | Entry::NewSection => false,
        }
    }

    fn close_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.options_menu.take()
            && let Some(focus) = menu.return_focus
        {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn pick_entry(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        match entry {
            Entry::Organize | Entry::Sort => {
                if let Some(menu) = &mut self.options_menu {
                    menu.sub = Some((entry, None));
                }
                cx.notify();
            }
            Entry::Projects => {
                self.chat_options.show_projects ^= true;
                self.close_options(window, cx);
                cx.emit(ChatOrderChanged);
            }
            Entry::NewSection => {
                self.close_options(window, cx);
                cx.emit(NewSectionRequested);
            }
        }
    }

    fn pick_sub(
        &mut self,
        entry: Entry,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match entry {
            Entry::Organize => {
                self.chat_options.organize = [Organize::Project, Organize::List][index];
            }
            Entry::Sort => {
                let sort = [ChatSort::UpdatedAt, ChatSort::Manual][index];
                if sort == ChatSort::Manual && self.chat_options.sort != ChatSort::Manual {
                    self.chat_options.manual_order = self.chat_ids().map(str::to_owned).collect();
                }
                self.chat_options.sort = sort;
            }
            Entry::Projects | Entry::NewSection => return,
        }
        self.close_options(window, cx);
        cx.emit(ChatOrderChanged);
    }

    /// Arrows move and open or close the submenu, Enter and Space pick, Escape closes.
    fn options_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.entries();
        let Some(menu) = &mut self.options_menu else {
            return;
        };
        let step = |active: Option<usize>, n: usize, forward: bool| match (active, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        match (event.keystroke.key.as_str(), menu.sub) {
            ("escape", _) => self.close_options(window, cx),
            (key @ ("down" | "up"), Some((entry, active))) => {
                menu.sub = Some((entry, Some(step(active, entry.sub().len(), key == "down"))));
            }
            (key @ ("down" | "up"), None) => {
                menu.active = Some(step(menu.active, entries.len(), key == "down"));
            }
            ("left", Some(_)) => menu.sub = None,
            ("right", None) => {
                if let Some(entry) = menu.active.map(|i| entries[i])
                    && !entry.sub().is_empty()
                {
                    menu.sub = Some((entry, Some(0)));
                }
            }
            ("enter" | "space", Some((entry, Some(index)))) => {
                self.pick_sub(entry, index, window, cx)
            }
            ("enter" | "space", None) => {
                if let Some(index) = menu.active {
                    self.pick_entry(entries[index], window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The `...` and `New chat` buttons at the right of the `Recents` header, shown on hover
    /// or while the menu is open.
    pub(super) fn chat_options_button(
        &self,
        group: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = focus::handle("chat-options", -1, window, cx);
        let new_chat = focus::handle("recents-new-chat", -1, window, cx);
        let shown = self.options_menu.is_some()
            || [&options, &new_chat]
                .iter()
                .any(|f| f.read(cx).handle.is_focused(window));
        let bounds = window.use_keyed_state("chat-options-bounds", cx, |_, _| {
            Bounds::<Pixels>::default()
        });
        let measured = bounds.clone();
        let button = |id: &'static str,
                      label: &'static str,
                      name: &'static str,
                      color: Hsla,
                      handle: &gpui::Entity<focus::ControlFocus>| {
            focus::ring(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .relative()
                    .size(px(24.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(cx.theme().ghost_hover))
                    .child(icon(name, 16., color)),
                handle,
                px(8.),
                px(0.),
                cx,
            )
        };
        div()
            .absolute()
            .right(px(6.))
            .top(px(3.))
            .flex()
            .gap(px(4.))
            .when(!shown, |d| d.invisible())
            .group_hover(group, |s| s.visible())
            .child(
                button(
                    "chat-options",
                    "Chat sidebar options",
                    "diff-more",
                    // The open trigger (`data-state=open`) is the text at 85 % (217 of 255;
                    // `assets/icons/px` holds its fitted pixels).
                    if self.options_menu.is_some() {
                        cx.theme().text.opacity(217. / 255.)
                    } else {
                        cx.theme().text_tertiary
                    },
                    &options,
                )
                .child(
                    canvas(
                        |_, _, _| {},
                        move |rect, (), _, cx| measured.update(cx, |b, _| *b = rect),
                    )
                    .absolute()
                    .inset_0(),
                )
                .on_click(cx.listener(
                    move |this, _: &gpui::ClickEvent, window, cx| {
                        let return_focus = window.focused(cx);
                        let focus = cx.focus_handle();
                        window.focus(&focus, cx);
                        this.options_menu = Some(OptionsMenu {
                            at: *bounds.read(cx),
                            active: None,
                            sub: None,
                            focus,
                            return_focus,
                        });
                        cx.notify();
                    },
                )),
            )
            .child(
                button(
                    "recents-new-chat",
                    "New chat",
                    "new-chat",
                    cx.theme().text_tertiary,
                    &new_chat,
                )
                .on_click(|_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(crate::NewChat), cx);
                }),
            )
            .into_any_element()
    }
}

/// One `px-2 py-[5px]` row: leading icon, label, then the chevron or the check.
fn options_row(
    t: &Theme,
    id: SharedString,
    lead: Option<&'static str>,
    label: &'static str,
    trail: Option<(&'static str, Hsla)>,
    highlighted: bool,
) -> Stateful<Div> {
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .role(gpui::Role::MenuItem)
        .aria_label(label)
        .h(px(ROW))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(6.))
        .relative()
        // Skia paints both in the menu's layer: its 8-bit blend, not the plain one.
        .when(highlighted, |d| {
            d.child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(15.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.menu_item_hover),
                ..Default::default()
            }))
        })
        .children(lead.map(|name| icon(name, 16., t.text)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(text(label, 13., 18.5714, 430., t.text)),
        )
        .children(trail.map(|(name, color)| icon(name, 16., color)))
}

impl HomeSidebar {
    pub(super) fn chat_options_overlay(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.options_menu.as_ref()?;
        let t = cx.theme();
        let entries = self.entries();
        let separator =
            || {
                div()
                    .py(px(4.))
                    .px(px(8.))
                    .child(div().relative().h(px(1.)).child(shapes::decoration(
                        shapes::Decoration {
                            fill: Some(t.border),
                            ..Default::default()
                        },
                    )))
            };
        let open_sub = menu.sub.map(|(entry, _)| entry);
        let mut top = px(4.);
        let mut sub_top = px(0.);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, entry) in entries.iter().copied().enumerate() {
            let highlighted = menu.active == Some(index) || open_sub == Some(entry);
            let chevron = Some(("chevron-right", t.text_tertiary));
            let row = match entry {
                Entry::Organize => options_row(
                    t,
                    "chat-options-organize".into(),
                    Some("sidebar"),
                    "Organize sidebar",
                    chevron,
                    highlighted,
                ),
                Entry::Sort => options_row(
                    t,
                    "chat-options-sort".into(),
                    Some("sidebar-sort"),
                    "Sort chats by",
                    chevron,
                    highlighted,
                ),
                Entry::Projects => {
                    rows.push(separator().into_any_element());
                    rows.push(
                        div()
                            .px(px(8.))
                            .py(px(5.))
                            .child(text("Show", 13., 16., 430., t.text_tertiary))
                            .into_any_element(),
                    );
                    top += px(9. + 26.);
                    options_row(
                        t,
                        "chat-options-projects".into(),
                        None,
                        "Projects",
                        self.chat_options
                            .show_projects
                            .then_some(("menu-check", t.text)),
                        highlighted,
                    )
                }
                Entry::NewSection => {
                    rows.push(separator().into_any_element());
                    top += px(9.);
                    options_row(
                        t,
                        "chat-options-new-section".into(),
                        Some("diff-new-tab"),
                        "New section",
                        None,
                        highlighted,
                    )
                }
            };
            if open_sub == Some(entry) {
                sub_top = top;
            }
            top += px(ROW);
            rows.push(
                row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered && let Some(menu) = &mut this.options_menu {
                        menu.active = Some(index);
                        menu.sub = (!entry.sub().is_empty()).then_some((entry, None));
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.pick_entry(entry, window, cx);
                    cx.stop_propagation();
                }))
                .into_any_element(),
            );
        }
        let focus = menu.focus.clone();
        let main = popover(t, WIDTH, |d| d.p(px(4.)).children(rows))
            .id("chat-options-menu")
            .role(gpui::Role::Menu)
            .aria_label("Chat sidebar options")
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.options_key(event, window, cx)
            }));
        let sub = menu.sub.map(|(entry, active)| {
            let rows = entry
                .sub()
                .iter()
                .enumerate()
                .map(|(index, &(label, lead))| {
                    options_row(
                        t,
                        format!("chat-options-sub-{index}").into(),
                        lead,
                        label,
                        self.sub_checked(entry, index)
                            .then_some(("menu-check", t.text)),
                        active == Some(index),
                    )
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && let Some(menu) = &mut this.options_menu {
                            menu.sub = Some((entry, Some(index)));
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_sub(entry, index, window, cx);
                        cx.stop_propagation();
                    }))
                });
            // `alignOffset -4`: its first row lines up with the trigger row, on the device px.
            // `anchored` lands on whole px; the half px goes in a margin.
            let y = ((menu.at.bottom() + px(2.) + sub_top - px(4.)) * 2.).round() / 2.;
            anchored()
                .position(point(menu.at.left() + px(1. + WIDTH + 5.), y.floor()))
                .child(
                    popover(t, SUB_WIDTH, |d| d.p(px(4.)).children(rows))
                        .mt(y - y.floor() - px(12.))
                        .id("chat-options-submenu"),
                )
        });
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .w(window.viewport_size().width)
                        .h(window.viewport_size().height)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.close_options(window, cx)),
                        )
                        .child(
                            anchored()
                                .position(menu.at.bottom_left() + point(px(1.), px(2.)))
                                .child(main.on_mouse_down(MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })),
                        )
                        .children(sub),
                ),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }
}
