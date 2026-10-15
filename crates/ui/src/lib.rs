//! App shell: the window chrome every screen shares (titlebar, navigation rail, sidebar
//! frame, main panel), reproduced from `refs/home/dark@1280x832.spec.json`. All numbers are
//! the spec's CSS px.

use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, BoxShadow, ClickEvent, Context, Div,
    EdgesRefinement, FontWeight, Hsla, Image, ImageFormat, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Svg, Transformation, Window, actions, div, img, point,
    prelude::FluentBuilder as _, px, size, svg,
};
use std::sync::{Arc, LazyLock};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    shell,
    [
        /// Show or hide the sidebar panel.
        ToggleSidebar,
        /// Go to Home with an empty composer.
        NewChat,
        GoBack,
        GoForward,
        OpenSettings,
    ]
);

// Edit menu commands; text inputs handle them.
actions!(edit, [Undo, Redo, Cut, Copy, Paste, SelectAll]);

/// `--app-shell-navigation-rail-width`
pub const RAIL_WIDTH: Pixels = px(52.);
/// `.sidebar-navigation` panel width (left panel minus the rail).
pub const SIDEBAR_PANEL_WIDTH: Pixels = px(288.);
/// `--app-shell-page-inline-end-inset`, also the layout's bottom padding.
const PAGE_INSET: Pixels = px(4.);
/// `--button-radius` of the 36 px (`xl`) and 28 px `_Button` sizes.
const RADIUS_BUTTON_XL: Pixels = px(12.5);
const RADIUS_BUTTON_MD: Pixels = px(9.375);
/// Rail buttons are `data-squircle`: 12.5 px `corner-shape: superellipse(1.5)`.
/// ponytail: a circular radius fitted to the refs/home pixels; a true superellipse needs a
/// path element.
const RADIUS_RAIL: Pixels = px(9.5);

pub struct Shell {
    pub sidebar_open: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Rail `Home` shows as selected (route is Home).
    pub home_selected: bool,
    /// Titlebar `Hide sidebar` and `New tab` buttons (chat routes; Settings hides them).
    pub chat_chrome: bool,
    sidebar: AnyView,
    main: AnyView,
    avatar: Arc<Image>,
}

impl Shell {
    /// `sidebar` fills the sidebar panel, `main` the main panel.
    pub fn new(sidebar: AnyView, main: AnyView) -> Self {
        Self {
            sidebar_open: true,
            can_go_back: false,
            can_go_forward: false,
            home_selected: true,
            chat_chrome: true,
            sidebar,
            main,
            // The fixture account's avatar, pre-scaled to its 24 px slot at 2x.
            avatar: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../assets/avatar.png").to_vec(),
            )),
        }
    }

    pub fn set_main(&mut self, main: AnyView, cx: &mut Context<Self>) {
        self.main = main;
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        cx.notify();
    }
}

/// `top` over opaque `bottom` the way Chromium's Skia composites it: the source is
/// premultiplied to 8 bits before blending. The shell paints its translucent CSS layers as
/// these opaque results, which also keeps the page surface's drop shadow hidden under them
/// (CSS never paints a box shadow under its box).
fn over(top: Hsla, bottom: Hsla) -> Hsla {
    let (t, b) = (Rgba::from(top), Rgba::from(bottom));
    let a = (t.a * 255.).round() / 255.;
    let blend = |t: f32, b: f32| ((t * a * 255.).round() + b * 255. * (1. - a)).round() / 255.;
    Rgba {
        r: blend(t.r, b.r),
        g: blend(t.g, b.g),
        b: blend(t.b, b.b),
        a: 1.,
    }
    .into()
}

/// Window color behind the rail and titlebar: `_PageSurfaceLayout` over the body.
fn layout_color(t: &Theme) -> Hsla {
    over(t.page_surface_layout, t.background_surface_under)
}

fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// One line of UI text. `textsys` lands glyphs on Chromium's baseline (ascent and descent
/// rounded in device px), so no offset is needed here.
fn text(s: impl Into<SharedString>, size: f32, line_height: f32, weight: f32, color: Hsla) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(FontWeight(weight))
        .text_color(color)
        .whitespace_nowrap()
        .child(s.into())
}

fn dispatch<A: Action + Default>(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(A::default()), cx);
}

/// 28 px secondary ghost `_Button`: tertiary icon; on hover a `ghost_hover` fill and white icon.
fn icon_button(id: &'static str, glyph: Svg, t: &Theme) -> Stateful<Div> {
    let (hover_bg, hover_fg) = (t.ghost_hover, t.text);
    div()
        .id(id)
        .group(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_BUTTON_MD)
        .hover(move |s| s.bg(hover_bg))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(glyph.group_hover(id, move |s| s.text_color(hover_fg)))
}

/// A sidebar `group/nav-section`: 14 px medium title (`Projects`, `Recents`) over its list.
pub fn sidebar_section(title: impl Into<SharedString>, body: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(8.))
        .child(
            div()
                .h(px(30.))
                .flex()
                .items_center()
                .pl(px(8.))
                .child(text(title, 14., 21., 500., cx.theme().text_tertiary)),
        )
        .child(div().pt(px(4.)).child(body))
}

/// Empty state line of a sidebar section (`No projects`, `No chats`).
pub fn sidebar_empty(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .p(px(8.))
        .opacity(0.5)
        .child(text(label, 14., 21., 430., cx.theme().text_tertiary))
}

/// Home sidebar of the fixture state: header, then empty Projects and Recents.
pub struct EmptySidebar;

impl Render for EmptySidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(home_sidebar_header(cx))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pt(px(4.))
                    .child(sidebar_section(
                        "Projects",
                        sidebar_empty("No projects", cx),
                        cx,
                    ))
                    .child(sidebar_section(
                        "Recents",
                        sidebar_empty("No chats", cx),
                        cx,
                    )),
            )
    }
}

/// Main panel with nothing in it.
pub struct EmptyMain;

impl Render for EmptyMain {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Builds one screen in its reference fixture state.
pub type ScreenBuilder = fn(&mut Window, &mut App) -> AnyView;

/// Every screen the parity harness renders: `(refs/<screen> name, builder)`. The harness
/// sets the `Theme` global before calling a builder.
pub static SCREENS: &[(&str, ScreenBuilder)] = &[("home", home_shell)];

/// The shell as refs/home shows it: sidebar open, Home selected, empty lists and main panel.
pub fn home_shell(_: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = cx.new(|_| EmptySidebar).into();
    let main = cx.new(|_| EmptyMain).into();
    cx.new(|_| Shell::new(sidebar, main)).into()
}

/// Top of the Home sidebar: `Codex ⌄` mode switch, activity bell and search, then the
/// `New chat` and `Your dot` rows.
pub fn home_sidebar_header(cx: &App) -> Div {
    static YOUR_DOT: LazyLock<Arc<Image>> = LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("../assets/your-dot.svg").to_vec(),
        ))
    });
    let t = cx.theme();
    let row = |id: &'static str, leading: AnyElement, label: &'static str| {
        let hover_bg = t.list_hover;
        div()
            .id(id)
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(RADIUS_BUTTON_XL)
            .hover(move |s| s.bg(hover_bg))
            .child(leading)
            .child(text(label, 14., 21., 430., t.text_default_nav))
    };
    div()
        .flex_none()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(52.))
                .flex()
                .items_center()
                .pl(px(10.))
                .pr(px(12.))
                .child(
                    div()
                        .id("mode-switch")
                        .h(px(32.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(RADIUS_BUTTON_XL)
                        .child(
                            svg()
                                .path("icons/codex-wordmark.svg")
                                .w(px(53.22))
                                .h(px(18.))
                                .text_color(t.text_default_nav),
                        )
                        .child(icon("chevron-down-medium", 14., t.text_tertiary)),
                )
                .child(div().flex_1())
                .child(icon_button(
                    "activity",
                    icon("bell", 16., t.text_tertiary),
                    t,
                ))
                .child(div().w(px(4.)))
                .child(icon_button(
                    "search",
                    icon("search", 16., t.text_tertiary),
                    t,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .px(px(8.))
                .mb(px(8.))
                .child(
                    row(
                        "new-chat",
                        icon("new-chat", 16., t.text_default_nav).into_any_element(),
                        "New chat",
                    )
                    .on_click(dispatch::<NewChat>),
                )
                .child(row(
                    "your-dot",
                    img(YOUR_DOT.clone()).size(px(16.)).into_any_element(),
                    "Your dot",
                )),
        )
}

impl Shell {
    fn rail(&self, t: &Theme) -> Div {
        let (hover_bg, hover_fg) = (t.ghost_hover, t.text);
        let item = |id: &'static str, name: &'static str, selected: bool| {
            let fg = if selected { t.text } else { t.text_tertiary };
            div()
                .id(id)
                .group(id)
                .relative()
                .flex_none()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_RAIL)
                .when(selected, |d| d.bg(hover_bg))
                .hover(move |s| s.bg(hover_bg))
                .child(icon(name, 20., fg).group_hover(id, move |s| s.text_color(hover_fg)))
        };
        let home = self.home_selected;
        div()
            .absolute()
            .left_0()
            .top(Theme::TITLEBAR_HEIGHT)
            .bottom(PAGE_INSET)
            .w(RAIL_WIDTH)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(8.))
            .pb(px(4.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        item("rail-home", if home { "home-filled" } else { "home" }, home)
                            .on_click(dispatch::<NewChat>),
                    )
                    .child(item("rail-space", "space", false))
                    .child(item("rail-scheduled", "scheduled", false))
                    .child(item("rail-plugins", "plugins", false))
                    // Explore with its unread dot (`bg-info-solid`, 8 px, top-right).
                    .child(
                        item("rail-explore", "explore", false).child(
                            div()
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .size(px(8.))
                                .rounded_full()
                                .bg(t.info_solid),
                        ),
                    )
                    .child(div().w(px(24.)).h(px(1.)).bg(t.border))
                    .child(item("rail-code-review", "code-review", false))
                    .child(item("rail-sites", "sites", false)),
            )
            .child(
                div()
                    .id("rail-profile")
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_RAIL)
                    .hover(move |s| s.bg(hover_bg))
                    .child(img(self.avatar.clone()).size(px(24.)).rounded_full()),
            )
    }

    fn sidebar_panel(&self, t: &Theme) -> Div {
        div()
            .absolute()
            .left(RAIL_WIDTH)
            .top(Theme::TITLEBAR_HEIGHT)
            .bottom(PAGE_INSET)
            .w(SIDEBAR_PANEL_WIDTH)
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(over(t.sidebar_background, layout_color(t)))
            .rounded_tl(Theme::RADIUS_XL_BASE)
            .rounded_bl(Theme::RADIUS_XL_BASE)
            .child(self.sidebar.clone())
    }

    fn main_panel(&self, t: &Theme) -> Div {
        let r = Theme::RADIUS_XL_BASE;
        let open = self.sidebar_open;
        let surface = div()
            .size_full()
            .bg(t.surface)
            .overflow_hidden()
            .map(|d| {
                if open {
                    d.rounded_tr(r).rounded_br(r)
                } else {
                    d.rounded(r)
                }
            })
            .child(self.main.clone());
        div()
            .absolute()
            .left(if open {
                Theme::SIDEBAR_WIDTH
            } else {
                RAIL_WIDTH
            })
            .top(Theme::TITLEBAR_HEIGHT)
            .right(PAGE_INSET)
            .bottom(PAGE_INSET)
            .map(|d| {
                if open {
                    // `_MainContentLeftBorder`: a 0.5 px hairline. The surface is
                    // `background-clip: padding-box`, so the hairline sits on the layout color.
                    d.rounded_tr(r)
                        .rounded_br(r)
                        .pl(px(0.5))
                        .bg(over(t.sidebar_divider, layout_color(t)))
                } else {
                    d.rounded(r)
                }
            })
            .child(surface)
    }

    fn titlebar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        // Disabled (`opacity: .4`, no hover) when there is no history in that direction.
        let nav = |id: &'static str, glyph: Svg, enabled: bool| {
            if enabled {
                icon_button(id, glyph, t)
            } else {
                div()
                    .id(id)
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.4)
                    .child(glyph)
            }
            .absolute()
            .top(px(8.))
        };
        let back = icon("arrow-left", 16., t.text_tertiary);
        // Forward is the Back glyph mirrored (`-scale-x-100`).
        let forward = icon("arrow-left", 16., t.text_tertiary)
            .with_transformation(Transformation::scale(size(-1., 1.)));
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(Theme::TITLEBAR_HEIGHT)
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .child(
                nav("back", back, self.can_go_back)
                    .left(px(88.))
                    .on_click(dispatch::<GoBack>),
            )
            .child(
                nav("forward", forward, self.can_go_forward)
                    .left(px(122.))
                    .on_click(dispatch::<GoForward>),
            )
            .when(self.chat_chrome, |d| {
                d.child(
                    icon_button("toggle-sidebar", icon("sidebar", 16., t.text_tertiary), t)
                        .absolute()
                        .left(px(156.))
                        .top(px(8.))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("new-tab", 16., t.text_tertiary)),
                )
            })
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        // `_PageSurface` 0.5 px ring (`--elevation-stroke-subtle`), drawn as a border on a box
        // 0.5 px larger than the surface.
        let mut ring = div()
            .absolute()
            .left(RAIL_WIDTH - px(0.5))
            .top(Theme::TITLEBAR_HEIGHT - px(0.5))
            .right(PAGE_INSET - px(0.5))
            .bottom(PAGE_INSET - px(0.5))
            .rounded(Theme::RADIUS_XL_BASE + px(0.5))
            .border_color(t.page_surface_stroke);
        let hairline = Some(px(0.5).into());
        ring.style().border_widths = EdgesRefinement {
            top: hairline,
            right: hairline,
            bottom: hairline,
            left: hairline,
        };
        // `_PageSurface` drop shadow (`--shadow-card`: 0 4px 16px). Chromium blends it in 8-bit
        // integer steps while GPUI blends in float, so a CSS-faithful shadow lands one level
        // off over most of its tail. (radius, blur, spread, y offset) are instead fitted to
        // the pixels of refs/home/dark and refs/settings/light.
        let (radius, blur, spread, offset) = match t.appearance {
            Appearance::Dark => (6., 12., 4., 4.),
            Appearance::Light => (12., 6.5, 0., 3.5),
        };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .font_family(".SystemUIFont")
            .bg(layout_color(&t))
            .child(
                div()
                    .absolute()
                    .left(RAIL_WIDTH)
                    .top(Theme::TITLEBAR_HEIGHT)
                    .right(PAGE_INSET)
                    .bottom(PAGE_INSET)
                    .rounded(px(radius))
                    .shadow(vec![BoxShadow {
                        color: t.shadow_card,
                        offset: point(px(0.), px(offset)),
                        blur_radius: px(blur),
                        spread_radius: px(spread),
                        inset: false,
                    }]),
            )
            .child(ring)
            .child(self.rail(&t))
            .when(self.sidebar_open, |d| d.child(self.sidebar_panel(&t)))
            .child(self.main_panel(&t))
            .child(self.titlebar(&t, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_matches_chromium() {
        let t = Theme::dark();
        let red = |c: Hsla| (Rgba::from(c).r * 255.).round() as u8;
        // refs/home/dark@1280x832.png: titlebar and rail 36, sidebar 29.
        assert_eq!(red(layout_color(&t)), 36);
        assert_eq!(red(over(t.sidebar_background, layout_color(&t))), 29);
    }
}
