use super::*;
use gpui::{Action, KeyDownEvent, Task};
use std::time::Duration;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct PreviewSidebar(pub &'static str);

#[derive(Default)]
pub(super) struct WindowState {
    pub peek: Option<&'static str>,
    pub sidebar: Option<AnyView>,
    hover_task: Option<Task<()>>,
    menu_task: Option<Task<()>>,
    pub menu_open: bool,
    pub menu_focus: Option<usize>,
    pub menu_hovered: bool,
}

pub(crate) fn resized(key: &str, current: Pixels, minimum: Pixels, maximum: Pixels, reverse: bool) -> Option<Pixels> {
    let step = px(if reverse { -10. } else { 10. });
    Some(match key {
        "left" => current - step,
        "right" => current + step,
        "home" => minimum,
        "end" => maximum,
        _ => return None,
    }.clamp(minimum, maximum))
}

impl Shell {
    pub fn set_peek_sidebar(&mut self, sidebar: AnyView, cx: &mut Context<Self>) {
        self.window_state.sidebar = Some(sidebar);
        cx.notify();
    }

    pub fn close_peek(&mut self, cx: &mut Context<Self>) {
        self.window_state.hover_task = None;
        self.window_state.peek = None;
        self.window_state.sidebar = None;
        cx.notify();
    }

    pub fn is_peeking(&self) -> bool {
        self.window_state.peek.is_some()
    }

    pub(super) fn hover_rail(&mut self, name: &'static str, entered: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_open || window.viewport_size().width < px(768.) || !matches!(name, "home" | "scheduled") {
            return;
        }
        self.window_state.hover_task = None;
        let delay = if entered && self.window_state.peek.is_some_and(|current| current != name) { 300 } else { 100 };
        self.window_state.hover_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if entered {
                    this.window_state.peek = Some(name);
                    window.dispatch_action(Box::new(PreviewSidebar(name)), cx);
                    cx.notify();
                } else {
                    this.close_peek(cx);
                }
            });
        }));
    }

    fn hover_peek(&mut self, entered: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.window_state.hover_task = None;
        if !entered && let Some(name) = self.window_state.peek {
            self.hover_rail(name, false, window, cx);
        }
    }

    pub(super) fn dismiss_window_overlay(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && (self.is_peeking() || self.window_state.menu_open) {
            self.close_peek(cx);
            self.window_state.menu_open = false;
            self.window_state.menu_task = None;
            cx.stop_propagation();
        }
    }

    pub(super) fn floating_sidebar(&self, narrow: bool, progress: f32, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let sidebar = if narrow { &self.sidebar } else { self.window_state.sidebar.as_ref().unwrap_or(&self.sidebar) };
        div()
            .id("floating-sidebar")
            .absolute()
            .left(if narrow { px(-8.) * (1. - progress) } else { RAIL_WIDTH })
            .top(if narrow { px(0.) } else { Theme::TITLEBAR_HEIGHT })
            .bottom(if narrow { px(0.) } else { PAGE_INSET })
            .w(if narrow { self.sidebar_width } else { self.sidebar_width - RAIL_WIDTH })
            .opacity(if narrow { progress } else { 1. })
            .occlude()
            .on_hover(cx.listener(|this, entered, window, cx| this.hover_peek(*entered, window, cx)))
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if event.position.x >= this.sidebar_width || event.position.y < Theme::TITLEBAR_HEIGHT {
                    if narrow {
                        if this.sidebar_open { this.toggle_sidebar(cx); }
                    } else {
                        this.close_peek(cx);
                    }
                }
            }))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(8.)),
                fill: Some(t.surface),
                shadows: vec![shapes::Shadow {
                    color: t.shadow_card,
                    offset: point(px(0.), px(8.)),
                    blur: px(32.),
                    spread: px(0.),
                    inset: false,
                }],
                ..Default::default()
            }))
            .child(rounded_clip(px(8.), shapes::Corner::Round,
                div().size_full().flex().flex_col()
                    .when(narrow, |d| d.child(div().relative().h(Theme::TITLEBAR_HEIGHT).flex_none().child(
                        window.with_element_namespace("floating-sidebar", |window| self.titlebar(t, window, cx, false))
                    )))
                    .child(div().flex_1().min_h_0().child(sidebar.clone()))))
    }


    pub(super) fn hover_new_tab(&mut self, entered: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.window_state.menu_hovered = entered;
        self.window_state.menu_task = None;
        if entered {
            self.window_state.menu_open = true;
            cx.notify();
        } else {
            self.window_state.menu_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let _ = this.update(cx, |this, cx| {
                    this.window_state.menu_open = false;
                    cx.notify();
                });
            }));
        }
    }

    pub(super) fn new_tab_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down" | "up") {
            self.window_state.menu_task = None;
            self.window_state.menu_open = true;
            self.window_state.menu_focus = Some(usize::from(event.keystroke.key == "up"));
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn new_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let t = cx.theme().clone();
        let focus_request = self.window_state.menu_focus.take();
        let mut menu = div().id("new-tab-menu").role(gpui::Role::Menu)
            .absolute().right(px(8.)).top(px(40.))
            .on_hover(cx.listener(|this, entered, window, cx| this.hover_new_tab(*entered, window, cx)))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.window_state.menu_hovered {
                    this.window_state.menu_open = false;
                    this.window_state.menu_task = None;
                    cx.notify();
                }
            }));
        let mut body = div().flex().flex_col();
        let hover = t.menu_item_hover;
        for (index, label) in ["New tab", "New tab in full view"].into_iter().enumerate() {
            let handle = focus::handle(("new-tab-option", index), index as isize, window, cx);
            if focus_request == Some(index) { handle.read(cx).handle.clone().focus(window, cx); }
            let item = focus::ring(div().id(("new-tab-option", index)).h(px(32.)).px(px(8.)).flex().items_center().gap(px(8.))
                .role(gpui::Role::MenuItem).aria_label(label).hover(move |s| s.bg(hover))
                .child(text(label, 14., 20., 430., t.text))
                .when(index == 1, |d| d.child(text("⇧⌘F", 12., 16., 430., t.text_tertiary))),
                &handle, px(8.), px(0.), cx)
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "up" | "down" | "home" | "end") {
                        this.window_state.menu_focus = Some(if event.keystroke.key == "home" { 0 } else if event.keystroke.key == "end" { 1 } else { 1 - index });
                    } else if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.window_state.menu_open = false;
                    } else { return; }
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    // The original creates its first content tab only with browser.in-app capability.
                    // Browser pages remain gated out of the native product.
                    this.window_state.menu_open = false;
                    cx.notify();
                }));
            body = body.child(item);
        }
        menu = menu.child(composer::popover(&t, 256., |d| d.child(body)));
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::resized;
    use gpui::px;

    #[test]
    fn separators_obey_their_edge_direction_and_bounds() {
        assert_eq!(resized("left", px(400.), px(300.), px(500.), false), Some(px(390.)));
        assert_eq!(resized("left", px(400.), px(300.), px(500.), true), Some(px(410.)));
        assert_eq!(resized("right", px(500.), px(300.), px(500.), false), Some(px(500.)));
        assert_eq!(resized("home", px(400.), px(300.), px(500.), true), Some(px(300.)));
        assert_eq!(resized("end", px(400.), px(300.), px(500.), false), Some(px(500.)));
        assert_eq!(resized("up", px(400.), px(300.), px(500.), false), None);
    }
}
