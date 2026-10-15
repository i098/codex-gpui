//! App shell: the window chrome every screen shares (titlebar, navigation rail, sidebar
//! frame, main panel), reproduced from `refs/home/dark@1280x832.spec.json`. All numbers are
//! the spec's CSS px.

use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Bounds, ClickEvent, ContentMask, Context,
    Corners, Div, FontWeight, Hsla, Image, ImageFormat, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Svg, Window, actions, canvas, div, fill, img, point,
    prelude::FluentBuilder as _, px, size, svg,
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
        /// Rail `Scheduled`: the Automations page.
        OpenScheduled,
        /// Profile menu ▸ Help ▸ `Set up mobile`.
        OpenMobileSetup,
        /// `⌥⌘B`: show or hide the Changes side panel.
        ToggleChangesPanel,
        /// A file change card's `View changes`, the thread summary's `Changes` row.
        OpenChangesPanel,
        /// Thread summary ▸ `Background processes` row: the command's terminal tab.
        OpenBackgroundTerminal,
        /// Side panel tab strip `Hide tabs`: close the side panel.
        CloseSidePanel,
        /// `⌘K`, `⌘⇧P`, sidebar `Search`: the command menu.
        OpenCommandMenu,
    ]
);

/// Settings ▸ Appearance ▸ Theme: `"system"`, `"light"` or `"dark"`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct SetTheme(pub &'static str);

// Edit menu commands; text inputs handle them.
actions!(edit, [Undo, Redo, Cut, Copy, Paste, SelectAll]);

/// `--app-shell-navigation-rail-width`
pub const RAIL_WIDTH: Pixels = px(52.);
/// `.sidebar-navigation` panel width (left panel minus the rail).
pub const SIDEBAR_PANEL_WIDTH: Pixels = px(288.);
/// `--app-shell-page-inline-end-inset`, also the layout's bottom padding.
const PAGE_INSET: Pixels = px(4.);
/// `--border-width-hairline` of `_MainContentLeftBorder`.
const MAIN_HAIRLINE: Pixels = px(0.5);
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
    /// Opaque background boxes the page paints under the main surface's clip that reach its
    /// rounded corners (Settings' `electron:bg-surface`): each blends the clip's anti-aliased
    /// edge once more. With any, the shell paints those corners itself, over what the main
    /// panel's content painted there.
    pub main_backgrounds: u8,
    /// The main surface is a composited layer (thread routes): Chromium clips each layer in
    /// it, so its rounded corners show its own fill and then the content (the thread's top fade
    /// or composer band, or the side panel) at the clip's coverage.
    pub main_composited: bool,
    /// With `main_composited`: the main surface itself (fill and hairline) is one more
    /// composited layer (Chromium's layer tree of refs/diff), not part of the page's; the shell
    /// draws its right corners' px of partial clip coverage.
    pub main_layer: bool,
    /// With `main_composited`: the main view draws its right corners' px of partial clip
    /// coverage itself, after the shell (a lone thread: `shapes::paint_clipped_tile`).
    pub main_clips_right: bool,
    /// A chat needs attention (unread): the rail's Home item carries the unread dot.
    pub attention: bool,
    /// The profile avatar's menu is open.
    pub profile_menu: bool,
    /// Pointer over the avatar: its own click toggles the menu, so a press there must not
    /// also close it as a click outside.
    avatar_hovered: bool,
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
            main_backgrounds: 0,
            main_composited: false,
            main_layer: false,
            main_clips_right: false,
            attention: false,
            profile_menu: false,
            avatar_hovered: false,
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

/// `child` inside its own `overflow: hidden` rounded clip (`shapes::with_rounded_clip`): Skia
/// draws the shapes and icons in it under that anti-aliased clip, whose padding box is the
/// child's bounds, with `radius` and `corner`.
pub fn rounded_clip(
    radius: Pixels,
    corner: shapes::Corner,
    child: impl IntoElement,
) -> RoundedClip {
    RoundedClip {
        radius,
        corner,
        child: child.into_any_element(),
        layers: true,
    }
}

pub struct RoundedClip {
    radius: Pixels,
    corner: shapes::Corner,
    child: AnyElement,
    /// See `shapes::with_rounded_clip`.
    layers: bool,
}

impl gpui::Element for RoundedClip {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        shapes::with_rounded_clip(
            Corners::all(self.radius),
            self.corner,
            bounds,
            self.layers,
            window,
            |window| child.paint(window, cx),
        );
    }
}

impl IntoElement for RoundedClip {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
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
            .child(home_sidebar_header(false, cx))
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
/// `New chat` and `Your dot` rows. With `unread` chats the bell carries the blue dot.
pub fn home_sidebar_header(unread: bool, cx: &App) -> Div {
    let t = cx.theme();
    // Chromium squashes the search button's `_ButtonInner` and the rows below into one
    // transparent overlap layer, from the rows' left edge and the button's top.
    let overlap = point(RAIL_WIDTH + px(8.), Theme::TITLEBAR_HEIGHT + px(12.));
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
            // `.text-fade-truncate`: a CSS mask layer inside the squashing cc layer Chromium
            // makes for the rows (they overlap the header buttons' layers), composited over
            // the sidebar's layer.
            .child(
                text(label, 14., 21., 430., t.text_default_nav)
                    .flex_1()
                    .top_layer(),
            )
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
                    // The unread bell's dot is a path in its own color (`--blue-400`).
                    icon::icon(
                        if unread { "bell-unread" } else { "bell" },
                        16.,
                        t.text_tertiary,
                    ),
                    t,
                ))
                .child(div().w(px(4.)))
                .child(
                    icon_button(
                        "search",
                        icon::icon("search", 16., t.text_tertiary).own_layer(Some(overlap)),
                        t,
                    )
                    .on_click(dispatch::<OpenCommandMenu>),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .px(px(8.))
                .mb(px(8.))
                // The rows are `overflow: hidden`: Skia draws the Your dot `<img>` under the
                // row's AA clip, whose run blitter snaps its edge coverage.
                .child(rounded_clip(
                    RADIUS_BUTTON_XL,
                    shapes::Corner::Squircle,
                    row(
                        "new-chat",
                        icon::icon("new-chat", 16., t.text_default_nav).into_any_element(),
                        "New chat",
                    )
                    .on_click(dispatch::<NewChat>),
                ))
                .child(rounded_clip(
                    RADIUS_BUTTON_XL,
                    shapes::Corner::Squircle,
                    row(
                        "your-dot",
                        // An `<img>`: its svg (with its own fill) draws into the overlap layer.
                        icon::icon("your-dot", 16., gpui::rgb(0xbac1d3).into())
                            .own_layer(Some(overlap))
                            .into_any_element(),
                        "Your dot",
                    ),
                )),
        )
}

impl Shell {
    fn rail(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
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
        // An unread dot (`bg-info-solid`, 8 px) `inset` px inside the item's top-right corner.
        let dot = move |inset: f32| {
            div()
                .absolute()
                .top(px(inset))
                .right(px(inset))
                .size(px(8.))
                .child(in_layer(
                    shapes::Decoration {
                        radius: Corners::all(Theme::RADIUS_FULL),
                        fill: Some(t.info_solid),
                        ..Default::default()
                    },
                    move || list,
                ))
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
                    .child(
                        item("home")
                            .when(self.attention, |d| d.child(dot(6.)))
                            .on_click(dispatch::<NewChat>),
                    )
                    .child(item("space"))
                    .child(item("scheduled").on_click(dispatch::<OpenScheduled>))
                    .child(item("plugins"))
                    // Explore with its unread dot.
                    .child(item("explore").child(dot(4.)))
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
                    .on_hover(
                        cx.listener(|this, hovered: &bool, _, _| this.avatar_hovered = *hovered),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.profile_menu ^= true;
                        cx.notify();
                    }))
                    .child(avatar()),
            )
    }

    /// Profile avatar menu (`codex.profileDropdown.*`) above the avatar: `Settings`, then the
    /// `Help` submenu's `Set up mobile`, flattened under a `Help` heading.
    /// ponytail: Profile, Usage, Log out and Show Mini open pages the port does not have; add
    /// them with those pages.
    fn profile_menu(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = t.menu_item_hover;
        let item = |id: &'static str,
                    label: &'static str,
                    action: fn(&ClickEvent, &mut Window, &mut App)| {
            div()
                .id(id)
                .px(px(8.))
                .py(px(6.))
                .rounded(px(12.))
                .hover(move |s| s.bg(hover))
                .child(text(label, 14., 20., 430., t.text))
                .on_click(cx.listener(move |this, e, window, cx| {
                    this.profile_menu = false;
                    cx.notify();
                    action(e, window, cx);
                }))
        };
        let settings = item("profile-settings", "Settings", dispatch::<OpenSettings>);
        let mobile = item(
            "profile-set-up-mobile",
            "Set up mobile",
            dispatch::<OpenMobileSetup>,
        );
        div()
            .absolute()
            .left(px(8.))
            .bottom(px(48.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.avatar_hovered {
                    this.profile_menu = false;
                    cx.notify();
                }
            }))
            .child(composer::popover(t, 220., |d| {
                d.child(settings)
                    .child(div().px(px(8.)).pt(px(8.)).pb(px(4.)).child(text(
                        "Help",
                        12.,
                        16.,
                        400.,
                        t.text_tertiary,
                    )))
                    .child(mobile)
            }))
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

    /// `_MainContentSurface`'s content box. Its fill and `_MainContentLeftBorder` hairline
    /// (`background-clip: padding-box`) paint with the page in `render`: with the sidebar open
    /// the left edge is square and the right corners are rounded by its `clip-path`; with it
    /// closed it is a plain 12 px rounded box.
    fn main_panel(&self) -> Div {
        let open = self.sidebar_open;
        let r = Theme::RADIUS_XL_BASE;
        let panel = div()
            .absolute()
            .top(Theme::TITLEBAR_HEIGHT)
            .right(PAGE_INSET)
            .bottom(PAGE_INSET)
            .overflow_hidden()
            .child(self.main.clone());
        if self.main_composited {
            return panel.left(if open {
                Theme::SIDEBAR_WIDTH + MAIN_HAIRLINE
            } else {
                RAIL_WIDTH
            });
        }
        // The `clip-path` box (its left corners under the open sidebar): Skia rasters main's
        // draws under that anti-aliased clip, but not its composited layers.
        let (left, inset) = if open {
            (Theme::SIDEBAR_WIDTH - r, r + MAIN_HAIRLINE)
        } else {
            (RAIL_WIDTH, px(0.))
        };
        div()
            .absolute()
            .left(left)
            .top(Theme::TITLEBAR_HEIGHT)
            .right(PAGE_INSET)
            .bottom(PAGE_INSET)
            .child(RoundedClip {
                radius: r,
                corner: shapes::Corner::Round,
                child: div()
                    .size_full()
                    .relative()
                    .child(panel.left(inset).top_0().right_0().bottom_0())
                    .into_any_element(),
                layers: false,
            })
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        // `_MainContentSurface` paints into the same layer: its fill and hairline blend onto
        // the layer's translucent pixels too.
        let open = self.sidebar_open;
        let composited = self.main_composited;
        let main_layer = composited && self.main_layer;
        let clip_twice = main_layer && screens::diff::panel::panel_clamped(window);
        let clips_right = composited && self.main_clips_right;
        let fills = if composited {
            1
        } else {
            1 + self.main_backgrounds
        };
        let r = Theme::RADIUS_XL_BASE;
        let radius = if open {
            Corners {
                top_right: r,
                bottom_right: r,
                ..Default::default()
            }
        } else {
            Corners::all(r)
        };
        let main_fill = shapes::Decoration {
            radius,
            fill: Some(t.surface),
            ..Default::default()
        };
        // A composited main surface's own fill, in its layer: the compositor rounds it.
        let square_fill = shapes::Decoration {
            radius: Corners::default(),
            ..main_fill.clone()
        };
        let hairline = shapes::Decoration {
            fill: Some(t.sidebar_divider),
            ..Default::default()
        };
        let under = t.background_surface_under;
        let surface_px = {
            let c = gpui::Rgba::from(t.surface);
            [c.r, c.g, c.b, 1.].map(|v| (v * 255.).round() as u8)
        };
        // The page frame over the root `bounds`.
        let frame = move |bounds: Bounds<Pixels>,
                          fills: u8,
                          square: bool,
                          window: &mut Window,
                          cx: &mut App| {
            let fill = if square { &square_fill } else { &main_fill };
            let page_surface = Bounds::from_corners(
                bounds.origin + point(RAIL_WIDTH, Theme::TITLEBAR_HEIGHT),
                bounds.bottom_right() - point(PAGE_INSET, PAGE_INSET),
            );
            let main_left = if open {
                Theme::SIDEBAR_WIDTH
            } else {
                RAIL_WIDTH
            };
            let mut main = Bounds::from_corners(
                bounds.origin + point(main_left, Theme::TITLEBAR_HEIGHT),
                page_surface.bottom_right(),
            );
            let line = open.then(|| {
                let line = Bounds::new(main.origin, size(MAIN_HAIRLINE, main.size.height));
                main.origin.x += MAIN_HAIRLINE;
                main.size.width -= MAIN_HAIRLINE;
                line
            });
            let main_surface = |window: &mut Window, cx: &mut App| {
                if let Some(line) = line {
                    shapes::paint(&hairline, None, line, window, cx);
                }
                for _ in 0..fills {
                    shapes::paint(fill, None, main, window, cx);
                }
            };
            window.with_skia_layer(255, |window| {
                shapes::paint(&layout, None, bounds, window, cx);
                shapes::paint(&surface, None, page_surface, window, cx);
                if !main_layer {
                    main_surface(window, cx);
                }
            });
            // Its own layer: the hairline blends onto transparent, then onto the page.
            if main_layer {
                window.with_skia_layer(255, |window| main_surface(window, cx));
            }
            main
        };
        let page = canvas(|_, _, _| {}, {
            let frame = frame.clone();
            move |bounds, (), window, cx| {
                frame(bounds, fills, composited, window, cx);
            }
        })
        .absolute()
        .size_full();
        // `_MainContentSurface` clips its content with a rounded `clip-path`.
        let corners = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let s = window.scale_factor();
                let dev = |v: Pixels| f32::from(v) * s;
                let rd = dev(r).round();
                let (left, top) = (dev(RAIL_WIDTH), dev(Theme::TITLEBAR_HEIGHT));
                let right = dev(bounds.size.width - PAGE_INSET);
                let bottom = dev(bounds.size.height - PAGE_INSET);
                // Each corner: its point and which way the main panel lies.
                let mut corners = vec![(right, top, -1., 1.), (right, bottom, -1., -1.)];
                if !open {
                    corners.extend([(left, top, 1., 1.), (left, bottom, 1., -1.)]);
                }
                let mut main = bounds;
                for (x, y, dx, dy) in corners {
                    if composited {
                        // viz draws each layer of the clipped surface through the clip's
                        // coverage `m`, partial px with `SkBlendARGB32`: its own fill F onto the
                        // page, then the content C (painted over the square F). In a layer of
                        // the page with the rounded F (the frame), C is drawn that way onto it.
                        // At `main_clips_right` and `main_layer` the layer's pixel stays: the
                        // view (or the shell, below) draws the content onto it.
                        let x0 = if dx < 0. { x - rd } else { x };
                        let y0 = if dy > 0. { y } else { y - rd };
                        let square = Bounds::new(
                            point(px(x0 / s), px(y0 / s)),
                            size(px(rd / s), px(rd / s)),
                        );
                        let mask = ContentMask {
                            bounds: square + bounds.origin,
                        };
                        window.with_content_mask(Some(mask), |window| {
                            window.with_skia_layer(255, |window| {
                                let fill = shapes::Decoration {
                                    fill: Some(under),
                                    ..Default::default()
                                };
                                shapes::paint(&fill, None, bounds, window, cx);
                                main = frame(bounds, u8::from(!main_layer), false, window, cx);
                                let own = clips_right || main_layer;
                                let own = Corners {
                                    top_right: own,
                                    bottom_right: own,
                                    ..Default::default()
                                };
                                shapes::paint_rounded_clip(radius, own, main, window, cx);
                            })
                        });
                        continue;
                    }
                    // Each opaque box under a raster clip blends its edge once: the frame is
                    // painted again over each rounded corner's device px that the arc does not
                    // fully cover, row by row; the content inside the arc stays.
                    // ponytail: the arc's edge px take the main fill as the content.
                    for i in 0..rd as u32 {
                        let i = i as f32;
                        let w = rd - (rd * rd - (rd - i).powi(2)).sqrt().floor();
                        let x0 = if dx < 0. { x - w } else { x };
                        let y0 = if dy > 0. { y + i } else { y - i - 1. };
                        let row =
                            Bounds::new(point(px(x0 / s), px(y0 / s)), size(px(w / s), px(1. / s)));
                        let mask = ContentMask {
                            bounds: row + bounds.origin,
                        };
                        window.with_content_mask(Some(mask), |window| {
                            window.paint_quad(fill(bounds, under));
                            frame(bounds, fills, false, window, cx);
                        });
                    }
                }
                if main_layer {
                    // cc's frame (`disabled-by-default-viz.quads`) clips the surface's fill F
                    // and the side panel's (F too) in a render surface. With a clamped panel
                    // (`panel_clamped`) it is the clip's own: F reaches it at `m` (onto
                    // transparent), and it is drawn onto the page at `m` again. Else it is an
                    // ancestor's: F is drawn into it twice at `m`, and it is composited onto
                    // the page.
                    // ponytail: takes the panel's px at the corners as F.
                    let right = Corners {
                        top_right: radius.top_right,
                        bottom_right: radius.bottom_right,
                        ..Default::default()
                    };
                    if clip_twice {
                        let once = |_, _, m: u8| {
                            surface_px.map(|c| ((u32::from(c) * (u32::from(m) + 1)) >> 8) as u8)
                        };
                        shapes::paint_clipped_tile(&once, false, right, main, window, cx);
                    } else {
                        window.with_skia_layer(255, |window| {
                            for _ in 0..2 {
                                shapes::paint_clipped_tile(
                                    &|_, _, _| surface_px,
                                    false,
                                    right,
                                    main,
                                    window,
                                    cx,
                                );
                            }
                        });
                    }
                }
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
            .bg(under)
            .child(page)
            .when(self.sidebar_open, |d| d.child(self.sidebar_panel(&t)))
            .child(self.main_panel())
            .when(composited || self.main_backgrounds > 0, |d| {
                d.child(corners)
            })
            .child(self.rail(&t, cx))
            .child(self.titlebar(&t, cx))
            .when(self.profile_menu, |d| d.child(self.profile_menu(&t, cx)))
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
