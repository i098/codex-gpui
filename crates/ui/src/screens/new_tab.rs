//! The side panel's `New tab` page (the original's `thread-side-panel-new-tab`): `Tools`
//! that open a tab, and `Recents`, the workspace's recently opened files.

use crate::{
    icon::icon,
    icon_button,
    screens::{
        settings::keyboard_shortcuts::shortcut_label,
        thread::view::menu::{ContextMenu, Item},
    },
    text,
};
use gpui::{
    Bounds, Context, EventEmitter, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

/// What a row of the page opens; the app replaces the `New tab` tab with it.
#[derive(Clone, Debug, PartialEq)]
pub enum NewTabPick {
    Changes,
    /// The interactive terminal as a side tab.
    Terminal,
    /// `Terminal options` ▸ `Open bottom terminal`.
    BottomTerminal,
    Browser,
    /// `Side chat`: the current chat forked into a side chat.
    SideChat,
    /// A recent file, relative to the workspace.
    File(String),
}

pub struct NewTabPage {
    /// `Changes` shows only when the thread has a diff.
    changes: bool,
    /// The original hides `Terminal options` while the bottom terminal is open.
    bottom_open: bool,
    recents: Vec<String>,
    /// The page's content box: the original's `@container`, two columns from `@md`.
    width: Pixels,
    options: Bounds<Pixels>,
    menu: Option<ContextMenu>,
}

impl EventEmitter<NewTabPick> for NewTabPage {}

/// `px-panel`: the Electron window's `--padding-panel-base`, `calc(var(--spacing) * 5)`.
const PADDING: f32 = 20.;
/// Tailwind's `@md` container width (28 rem).
const TWO_COLUMNS: f32 = 448.;

impl NewTabPage {
    pub fn new(changes: bool, bottom_open: bool, recents: Vec<String>) -> Self {
        Self {
            changes,
            bottom_open,
            recents,
            width: px(0.),
            options: Bounds::default(),
            menu: None,
        }
    }

    /// `text-sm font-medium text-default`.
    fn heading(label: &'static str, cx: &Context<Self>) -> impl IntoElement {
        text(label, 13., 18.5714, 500., cx.theme().text)
    }

    /// The original's `X` row: a `min-h-10` button on a `bg-primary-soft-alpha` pill, with
    /// its shortcut as a `kbd` chip and an optional trailing action beside the button.
    fn row(
        &self,
        id: SharedString,
        glyph: &'static str,
        label: SharedString,
        shortcut: Option<String>,
        pick: NewTabPick,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme();
        let chip = shortcut.map(|keys| {
            div().ml_auto().flex_none().pl(px(8.)).child(
                text(keys, 12., 12., 400., t.text_secondary)
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(Theme::RADIUS_MD)
                    .bg(t.text_secondary.opacity(0.1)),
            )
        });
        div()
            .id(id.clone())
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .rounded(Theme::RADIUS_MD)
            .bg(t.primary_soft_alpha)
            .hover(|s| s.bg(t.ghost_hover))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-button")))
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .min_h(px(40.))
                    .flex_1()
                    .min_w_0()
                    .px(px(10.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(pick.clone())))
                    .child(icon(glyph, 16., t.text_secondary).flex_none())
                    .child(text(label, 13., 18.5714, 400., t.text).flex_1().truncate())
                    .children(chip),
            )
    }

    /// `Terminal options`: a ghost toolbar `…` button whose menu holds `Open bottom terminal`.
    fn terminal_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let entity = cx.entity();
        icon_button(
            "new-tab-terminal-options",
            icon("thread-chat-actions", 16., t.text_tertiary),
            &t,
        )
        .relative()
        .role(gpui::Role::Button)
        .aria_label("Terminal options")
        .when(self.menu.is_some(), |d| d.bg(t.ghost_hover))
        .on_click(cx.listener(|this, _, window, cx| {
            cx.stop_propagation();
            if this.menu.take().is_none() {
                let items = vec![Item {
                    icon: Some(include_bytes!(
                        "../../../../assets/icons/thread-terminal.svg"
                    )),
                    accelerator: shortcut_label("toggleBottomPanel", cx).map(Into::into),
                    ..Item::new("open-bottom-terminal", "Open bottom terminal", true)
                }];
                this.menu = Some(ContextMenu::app_dropdown(
                    items,
                    String::new(),
                    this.options,
                    window,
                    cx,
                ));
            }
            cx.notify();
        }))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), _, cx| entity.update(cx, |page, _| page.options = bounds),
            )
            .absolute()
            .inset_0(),
        )
    }
}

impl Render for NewTabPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        // The original's action order (side chat, browser, review, terminal), rows with a
        // shortcut first. `Files` and `New page` need a file tree tab and Pages, which the app
        // does not have.
        let mut tools = vec![
            (
                "new-tab-side-chat",
                "new-tab-side-chat",
                "Side chat",
                shortcut_label("openSideChat", cx),
                NewTabPick::SideChat,
            ),
            (
                "new-tab-browser",
                "project-marker-globe",
                "Browser",
                shortcut_label("openBrowserTab", cx),
                NewTabPick::Browser,
            ),
            (
                "new-tab-changes",
                "diff-word-changes",
                "Changes",
                shortcut_label("openReviewTab", cx),
                NewTabPick::Changes,
            ),
            (
                "new-tab-terminal",
                "thread-terminal",
                "Terminal",
                shortcut_label("toggleTerminal", cx),
                NewTabPick::Terminal,
            ),
        ];
        tools.retain(|tool| self.changes || tool.4 != NewTabPick::Changes);
        tools.sort_by_key(|tool| tool.3.is_none());
        let tools: Vec<_> = tools
            .into_iter()
            .map(|(id, glyph, label, shortcut, pick)| {
                let terminal = pick == NewTabPick::Terminal && !self.bottom_open;
                self.row(id.into(), glyph, label.into(), shortcut, pick, cx)
                    .when(self.menu.is_some() && terminal, |d| d.bg(t.ghost_hover))
                    .when(terminal, |d| d.child(self.terminal_options(cx)))
            })
            .collect();
        let recents: Vec<_> = self
            .recents
            .clone()
            .into_iter()
            .enumerate()
            .map(|(ix, path)| {
                self.row(
                    format!("new-tab-recent-{ix}").into(),
                    "navigation-files",
                    path.clone().into(),
                    None,
                    NewTabPick::File(path),
                    cx,
                )
            })
            .collect();
        let columns = if self.width >= px(TWO_COLUMNS) { 2 } else { 1 };
        let section = |heading, rows: Vec<_>| {
            div().flex().flex_col().gap(px(16.)).child(heading).child(
                div()
                    .grid()
                    .grid_cols(columns)
                    .gap_x(px(16.))
                    .gap_y(px(4.))
                    .children(rows),
            )
        };
        let entity = cx.entity();
        // `bg-surface-canvas`: `color-mix(in srgb, var(--color-surface) 98%, black)`, which
        // scales every sRGB channel and so the HSL lightness alone.
        let canvas_bg = Hsla {
            l: t.surface.l * 0.98,
            ..t.surface
        };
        div()
            .id("new-tab-page")
            .relative()
            .size_full()
            .overflow_y_scroll()
            .bg(canvas_bg)
            .px(px(PADDING))
            .py(px(32.))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), _, cx| {
                        let width = bounds.size.width - px(2. * PADDING);
                        entity.update(cx, |page, cx| {
                            if page.width != width {
                                page.width = width;
                                cx.notify();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(768.))
                    .flex()
                    .flex_col()
                    .gap(px(40.))
                    .child(section(Self::heading("Tools", cx), tools))
                    .when(!recents.is_empty(), |d| {
                        d.child(section(Self::heading("Recents", cx), recents))
                    }),
            )
            .children(self.menu.as_ref().map(|menu| {
                menu.overlay(
                    "new-tab-terminal-menu",
                    "Terminal options",
                    |page: &mut Self| &mut page.menu,
                    |page, _, _, _, cx| {
                        page.bottom_open = true;
                        cx.emit(NewTabPick::BottomTerminal)
                    },
                    window,
                    cx,
                )
            }))
    }
}
