//! The side panel's `New tab` page (the original's `thread-side-panel-new-tab`): `Tools`
//! that open a tab, and `Recents`, the workspace's recently opened files.

use crate::{icon::icon, text};
use gpui::{
    Context, EventEmitter, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use theme::ActiveTheme as _;

/// What a row of the page opens; the app replaces the `New tab` tab with it.
#[derive(Clone, Debug, PartialEq)]
pub enum NewTabPick {
    Changes,
    /// The interactive terminal as a side tab.
    Terminal,
    /// `Terminal options` ▸ `Open bottom terminal`.
    BottomTerminal,
    Browser,
    /// A recent file, relative to the workspace.
    File(String),
}

pub struct NewTabPage {
    /// `Changes` shows only when the thread has a diff.
    changes: bool,
    recents: Vec<String>,
}

impl EventEmitter<NewTabPick> for NewTabPage {}

impl NewTabPage {
    pub fn new(changes: bool, recents: Vec<String>) -> Self {
        Self { changes, recents }
    }

    fn heading(label: &'static str, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div().px(px(10.)).pt(px(12.)).pb(px(4.)).child(text(
            label,
            13.,
            18.,
            500.,
            t.text_secondary,
        ))
    }

    fn row(
        &self,
        id: SharedString,
        glyph: &'static str,
        label: SharedString,
        pick: NewTabPick,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme();
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(t.list_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(pick.clone())))
            .child(icon(glyph, 16., t.text_secondary))
            .child(text(label, 13., 18., 430., t.text).flex_1().truncate())
    }
}

impl Render for NewTabPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let bottom = div()
            .id("new-tab-bottom-terminal")
            .role(gpui::Role::Button)
            .aria_label("Open bottom terminal")
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .hover(|s| s.bg(t.list_hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(NewTabPick::BottomTerminal);
            }))
            .child(icon("chevron-down-small", 14., t.text_tertiary));
        let tools = div()
            .flex()
            .flex_col()
            .child(Self::heading("Tools", cx))
            .when(self.changes, |d| {
                d.child(self.row(
                    "new-tab-changes".into(),
                    "diff-tab-changes",
                    "Changes".into(),
                    NewTabPick::Changes,
                    cx,
                ))
            })
            .child(
                self.row(
                    "new-tab-terminal".into(),
                    "terminal-interactive",
                    "Terminal".into(),
                    NewTabPick::Terminal,
                    cx,
                )
                .child(bottom),
            )
            .child(self.row(
                "new-tab-browser".into(),
                "sidebar-cmd-browser",
                "Browser".into(),
                NewTabPick::Browser,
                cx,
            ));
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
                    NewTabPick::File(path),
                    cx,
                )
            })
            .collect();
        div()
            .id("new-tab-page")
            .size_full()
            .overflow_y_scroll()
            .bg(t.surface)
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(576.))
                    .px(px(16.))
                    .py(px(16.))
                    .flex()
                    .flex_col()
                    .child(tools)
                    .when(!recents.is_empty(), |d| {
                        d.child(Self::heading("Recents", cx)).children(recents)
                    }),
            )
    }
}
