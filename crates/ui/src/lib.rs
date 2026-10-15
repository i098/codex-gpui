//! App shell: the window chrome every screen shares (titlebar, navigation rail, sidebar
//! frame, main panel), reproduced from `refs/home/dark@1280x832.spec.json`. All numbers are
//! the spec's CSS px.

use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Bounds, ClickEvent, Context, Corners, Div,
    FontWeight, Hsla, Image, ImageFormat, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, Render, Rgba, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Svg, Window, actions, canvas, div, img, point, prelude::FluentBuilder as _, px, svg,
};
use std::sync::{Arc, LazyLock};
use theme::{ActiveTheme as _, Theme};

pub mod composer;
pub mod fixture;
pub mod icon;
pub mod screens;

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
const RADIUS_RAIL: Pixels = px(12.5);
/// `-apple-system`, as the shell sets it.
pub const SANS: &str = ".SystemUIFont";
/// `ui-monospace`, which Chromium on macOS resolves to Menlo.
pub const MONO: &str = "Menlo";

pub struct Shell {
    pub sidebar_open: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Rail item shown as selected (`"home"`, `"scheduled"`, ...); its icon switches to
    /// `<name>-filled`. `None` selects nothing.
    pub selected_rail: Option<&'static str>,
    /// Titlebar `Hide sidebar` button (chat routes; Settings hides it).
    pub chat_chrome: bool,
    /// Titlebar `New tab` button (chat routes; Settings and Automations hide it).
    pub new_tab: bool,
    sidebar: AnyView,
    main: AnyView,
}

/// The fixture account's 24 px avatar with its circular clip baked into alpha. The two
/// straight-alpha layers are solved so GPUI's sprite blend lands on the refs' pixels over
/// both layout colors; `avatar-fix.png` only covers edge pixels one layer cannot reach.
fn avatar() -> impl IntoElement {
    static LAYERS: LazyLock<[Arc<Image>; 2]> = LazyLock::new(|| {
        [
            &include_bytes!("../assets/avatar.png")[..],
            include_bytes!("../assets/avatar-fix.png"),
        ]
        .map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png.to_vec())))
    });
    let layer = |image: &Arc<Image>| img(image.clone()).absolute().size(px(24.));
    div()
        .relative()
        .size(px(24.))
        .child(layer(&LAYERS[0]))
        .child(layer(&LAYERS[1]))
}

impl Shell {
    /// `sidebar` fills the sidebar panel, `main` the main panel.
    pub fn new(sidebar: AnyView, main: AnyView) -> Self {
        Self {
            sidebar_open: true,
            can_go_back: false,
            can_go_forward: false,
            selected_rail: Some("home"),
            chat_chrome: true,
            new_tab: true,
            sidebar,
            main,
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
pub fn over(top: Hsla, bottom: Hsla) -> Hsla {
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

/// `top` over opaque `bottom` through Skia's anti-aliased blitter (`blitAntiH` at full
/// coverage): `C + ((D * (256 - A)) >> 8)` with premultiplied 8-bit `C`. Chromium paints AA
/// borders and hairlines this way; the result can be one level below [`over`].
pub fn blit(top: Hsla, bottom: Hsla) -> Hsla {
    let (t, b) = (Rgba::from(top), Rgba::from(bottom));
    let a = (t.a * 255.).round() as u32;
    let blend = |t: f32, b: f32| {
        let c = (t * a as f32).round() as u32;
        (c + (((b * 255.).round() as u32 * (256 - a)) >> 8)) as f32 / 255.
    };
    Rgba {
        r: blend(t.r, b.r),
        g: blend(t.g, b.g),
        b: blend(t.b, b.b),
        a: 1.,
    }
    .into()
}

/// Window color behind the rail and titlebar: `_PageSurfaceLayout` over the body.
pub fn layout_color(t: &Theme) -> Hsla {
    over(t.page_surface_layout, t.background_surface_under)
}

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// One line of UI text. `textsys` lands glyphs on Chromium's baseline (ascent and descent
/// rounded in device px), so no offset is needed here.
pub fn text(
    s: impl Into<SharedString>,
    size: f32,
    line_height: f32,
    weight: f32,
    color: Hsla,
) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(FontWeight(weight))
        .text_color(color)
        .whitespace_nowrap()
        .child(s.into())
}

pub fn dispatch<A: Action + Default>(_: &ClickEvent, window: &mut Window, cx: &mut App) {
    window.dispatch_action(Box::new(A::default()), cx);
}

/// [`shapes::decoration`] for a box Chromium paints into the composited layer whose origin
/// (window px) `at` gives at paint.
pub(crate) fn in_layer(
    d: shapes::Decoration,
    at: impl Fn() -> Point<Pixels> + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, (), window, cx| shapes::paint(&d, Some(at()), b, window, cx),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// 28 px secondary ghost `_Button`: tertiary icon; on hover a `ghost_hover` fill and white icon.
/// `glyph` is a [`icon::icon`] (Chromium-exact) or a plain [`icon`] svg.
pub fn icon_button(
    id: &'static str,
    glyph: impl InteractiveElement + IntoElement + 'static,
    t: &Theme,
) -> Stateful<Div> {
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
pub static SCREENS: &[(&str, ScreenBuilder)] = &[
    ("home", screens::home::build),
    ("sidebar", screens::sidebar::build),
    ("thread", screens::thread::build),
    ("composer", screens::composer::build),
    ("diff", screens::diff::build),
    ("settings", screens::settings::build),
    ("automations", screens::automations::build),
    ("terminal", screens::terminal::build),
    ("mobile", screens::mobile::build),
];

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
                        .child(icon::icon_sized(
                            "codex-wordmark",
                            53.22,
                            18.,
                            t.text_default_nav,
                        ))
                        .child(icon::icon("chevron-down-medium", 14., t.text_tertiary)),
                )
                .child(div().flex_1())
                .child(icon_button(
                    "activity",
                    icon::icon("bell", 16., t.text_tertiary),
                    t,
                ))
                .child(div().w(px(4.)))
                .child(icon_button(
                    "search",
                    icon::icon("search", 16., t.text_tertiary),
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
                        icon::icon("new-chat", 16., t.text_default_nav).into_any_element(),
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
        let selected = self.selected_rail;
        // The list (`[6, 50, 40, 732]`) is a composited scroller: its items paint into the
        // layer's transparent tiles, which `vertical-scroll-fade-mask` fades over the top 16 px
        // (the scroll timeline is inactive, so `--top-fade` is 1rem) before the compositor
        // blends the layer onto the page.
        let list = point(px(6.), Theme::TITLEBAR_HEIGHT + px(6.));
        let fill = move |color: Hsla| {
            in_layer(
                shapes::Decoration {
                    radius: Corners::all(RADIUS_RAIL),
                    corner: shapes::Corner::Squircle,
                    fill: Some(color),
                    ..Default::default()
                },
                move || list,
            )
        };
        let item = move |name: &'static str| {
            let on = selected == Some(name);
            let fg = if on { t.text } else { t.text_tertiary };
            let id = SharedString::from(format!("rail-{name}"));
            let glyph = if on {
                format!("{name}-filled")
            } else {
                name.to_string()
            };
            div()
                .id(id.clone())
                .group(id.clone())
                .relative()
                .flex_none()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_RAIL)
                .when(on, |d| d.child(fill(hover_bg)))
                .when(!on, |d| d.hover(move |s| s.bg(hover_bg)))
                .child(
                    icon::icon(&glyph, 20., fg)
                        .composited(Some(list))
                        .group_hover(id, move |s| s.text_color(hover_fg)),
                )
        };
        div()
            .absolute()
            .left_0()
            .top(Theme::TITLEBAR_HEIGHT)
            .bottom(PAGE_INSET)
            .w(RAIL_WIDTH)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(6.))
            .pb(px(4.))
            .child(
                div()
                    .flex_1()
                    .w(px(40.))
                    .pt(px(2.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .layer()
                    .child(item("home").on_click(dispatch::<NewChat>))
                    .child(item("space"))
                    .child(item("scheduled"))
                    .child(item("plugins"))
                    // Explore with its unread dot (`bg-info-solid`, 8 px, top-right).
                    .child(
                        item("explore").child(
                            div()
                                .absolute()
                                .top(px(4.))
                                .right(px(4.))
                                .size(px(8.))
                                .child(in_layer(
                                    shapes::Decoration {
                                        radius: Corners::all(Theme::RADIUS_FULL),
                                        fill: Some(t.info_solid),
                                        ..Default::default()
                                    },
                                    move || list,
                                )),
                        ),
                    )
                    .child(div().w(px(24.)).h(px(1.)).bg(t.border))
                    .child(item("rail-code-review"))
                    .child(item("sites"))
                    .child(shapes::top_fade(px(16.))),
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
                    .child(avatar()),
            )
    }

    /// `.sidebar-navigation`: its left corners come from `clip-path: inset(... round 12px)`.
    /// It is a composited cc layer: its tile holds the translucent background and the content
    /// is blitted onto it before the layer is composited (`.layer()`).
    fn sidebar_panel(&self, t: &Theme) -> Div {
        let r = Theme::RADIUS_XL_BASE;
        div()
            .absolute()
            .left(RAIL_WIDTH)
            .top(Theme::TITLEBAR_HEIGHT)
            .bottom(PAGE_INSET)
            .w(SIDEBAR_PANEL_WIDTH)
            .flex()
            .flex_col()
            .overflow_hidden()
            .layer()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners {
                    top_left: r,
                    bottom_left: r,
                    ..Default::default()
                },
                fill: Some(t.sidebar_background),
                ..Default::default()
            }))
            .child(self.sidebar.clone())
    }

    /// `_MainContentSurface`. With the sidebar open its left edge is square, its right corners
    /// come from `clip-path: inset(0 -12px 0 0 round 12px)`, and `_MainContentLeftBorder` puts
    /// a 0.5 px hairline left of the fill (`background-clip: padding-box`, so the hairline sits
    /// on the layout color). With it closed it is a plain 12 px rounded box.
    fn main_panel(&self, t: &Theme) -> Div {
        let r = Theme::RADIUS_XL_BASE;
        let open = self.sidebar_open;
        let radius = if open {
            Corners {
                top_right: r,
                bottom_right: r,
                ..Default::default()
            }
        } else {
            Corners::all(r)
        };
        let hairline = px(if open { 0.5 } else { 0. });
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
            .when(open, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(hairline)
                        .bg(blit(t.sidebar_divider, layout_color(t))),
                )
            })
            .child(
                div()
                    .absolute()
                    .left(hairline)
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .overflow_hidden()
                    .child(shapes::decoration(shapes::Decoration {
                        radius,
                        fill: Some(t.surface),
                        ..Default::default()
                    }))
                    .child(self.main.clone()),
            )
    }

    fn titlebar(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        // Disabled (`opacity: .4`, no hover) when there is no history in that direction. The
        // opacity goes on the icon itself so `icon::icon` finds its fitted image.
        let nav = |id: &'static str, glyph: icon::Icon, enabled: bool| {
            if enabled {
                icon_button(id, glyph, t)
            } else {
                div()
                    .id(id)
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph.opacity(0.4))
            }
            .absolute()
            .top(px(8.))
        };
        let back = icon::icon("arrow-left", 16., t.text_tertiary);
        // Forward is the Back glyph mirrored (`-scale-x-100`).
        let forward = icon::icon("arrow-left", 16., t.text_tertiary).flip_x();
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
                    icon_button(
                        "toggle-sidebar",
                        icon::icon("sidebar", 16., t.text_tertiary),
                        t,
                    )
                    .absolute()
                    .left(px(156.))
                    .top(px(8.))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
            })
            .when(self.new_tab, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon::icon("new-tab", 16., t.text_tertiary)),
                )
            })
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        // Chromium paints the page (everything over the body background) into one composited
        // layer: its tile holds `_PageSurfaceLayout`'s translucent color, the page surface's
        // drop shadow (`--shadow-card`) and 0.5 px ring (`--elevation-stroke-subtle`) blend onto
        // that translucent pixel with the blits Skia picks for them (not `.layer()`'s
        // transparent-tile blits), and the compositor blends the tile onto the body.
        let layout = shapes::Decoration {
            fill: Some(t.page_surface_layout),
            ..Default::default()
        };
        let surface = shapes::Decoration {
            radius: Corners::all(Theme::RADIUS_XL_BASE),
            shadows: vec![
                shapes::Shadow {
                    color: t.page_surface_stroke,
                    offset: point(px(0.), px(0.)),
                    blur: px(0.),
                    spread: px(0.5),
                    inset: false,
                },
                shapes::Shadow {
                    color: t.shadow_card,
                    offset: point(px(0.), px(4.)),
                    blur: px(16.),
                    spread: px(0.),
                    inset: false,
                },
            ],
            ..Default::default()
        };
        let page = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let page_surface = Bounds::from_corners(
                    bounds.origin + point(RAIL_WIDTH, Theme::TITLEBAR_HEIGHT),
                    bounds.bottom_right() - point(PAGE_INSET, PAGE_INSET),
                );
                window.with_skia_layer(255, |window| {
                    shapes::paint(&layout, None, bounds, window, cx);
                    shapes::paint(&surface, None, page_surface, window, cx);
                });
            },
        )
        .absolute()
        .size_full();
        // The rail and titlebar do not overlap the panels, so they paint after the frame.
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .font_family(SANS)
            .bg(t.background_surface_under)
            .child(page)
            .when(self.sidebar_open, |d| d.child(self.sidebar_panel(&t)))
            .child(self.main_panel(&t))
            .child(self.rail(&t))
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
