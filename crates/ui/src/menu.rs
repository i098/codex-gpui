//! Focus rules for the recorded Radix menus: arrows stop at the ends, Escape returns
//! focus to the trigger, and Enter or Space activates the focused item.

use gpui::{App, BoxShadow, FocusHandle, StyleRefinement, Styled, Window, point, px, rgb};

pub(super) struct MenuFocus {
    pub content: FocusHandle,
    pub items: Vec<FocusHandle>,
    pub trigger: FocusHandle,
}

impl MenuFocus {
    pub fn new(cx: &mut App) -> Self {
        Self {
            content: cx.focus_handle(),
            items: Vec::new(),
            trigger: cx.focus_handle(),
        }
    }

    pub fn prepare(&mut self, count: usize, cx: &mut App) {
        self.items.resize_with(count, || cx.focus_handle());
    }

    pub fn focused_item(&self, window: &Window) -> Option<usize> {
        self.items.iter().position(|item| item.is_focused(window))
    }

    pub fn move_focus(&self, forward: bool, window: &mut Window, cx: &mut App) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        let index = match self.focused_item(window) {
            Some(i) if forward => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
            None if forward => 0,
            None => last,
        };
        self.items[index].focus(window, cx);
    }

    pub fn restore(&self, window: &mut Window, cx: &mut App) {
        self.trigger.focus(window, cx);
    }
}

pub(super) fn trigger_focus(style: StyleRefinement) -> StyleRefinement {
    style.shadow(vec![BoxShadow {
        color: rgb(0x3a83f7).into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }])
}
