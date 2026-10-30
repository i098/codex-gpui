//! App shell: the window chrome every screen shares (titlebar, navigation rail, sidebar
//! frame, main panel), reproduced from `refs/home/dark@1280x832.spec.json`. All numbers are
//! the spec's CSS px.
//!
//! Tab groups follow the original DOM order, not the paint order: rail, sidebar,
//! titlebar, then main content. `refs/interactions/f1c.json` records both directions.

use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Bounds, ClickEvent, ContentMask, Context,
    Corners, CursorStyle, DispatchPhase, Div, FontWeight, Hsla, Image, ImageFormat,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, RenderImage, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Svg, Window, actions, canvas, div, fill, img, point,
    prelude::FluentBuilder as _, px, size, svg,
};
use std::cell::RefCell;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use theme::{ActiveTheme as _, Theme};
use tooltip::{Side, tooltip};

pub mod browser;
pub mod composer;
pub mod confirmation;
pub mod dialog;
pub mod fixture;
pub mod focus;
pub mod icon;
mod image_context;
pub mod motion;
pub mod native_text;
pub mod screens;
pub mod settings_kit;
pub mod toast;
pub mod tooltip;
pub mod updater;
pub mod workspace;
pub mod zoom;

actions!(
    shell,
    [
        /// Show or hide the sidebar panel.
        ToggleSidebar,
        ToggleActivity,
        /// `switchToMode1`, `⌃1`: ChatGPT mode.
        SwitchMode1,
        /// `switchToMode2`, `⌃2`: Work mode (the recorded command table).
        SwitchMode2,
        /// `switchToMode3`, `⌃3`: Codex mode.
        SwitchMode3,
        /// Go to Home with an empty composer.
        NewChat,
        GoBack,
        GoForward,
        OpenSettings,
        LogOut,
        ShowKeyboardShortcuts,
        OpenMini,
        /// Rail `Space`: the Spaces list (`/space`).
        OpenSpace,
        /// Rail `Scheduled`: the Automations page.
        OpenScheduled,
        OpenPullRequests,
        OpenPlugins,
        OpenSecurity,
        /// Rail `Sites` (`/sites`).
        OpenSites,
        /// Home beacon `Start Voice` and the composer's `Start new voice chat`.
        StartVoiceChat,
        /// Profile menu ▸ Help ▸ `Set up mobile`.
        OpenMobileSetup,
        /// `⌥⌘B`: show or hide the Changes side panel.
        ToggleChangesPanel,
        /// The thread summary's `Changes` row and generic Changes navigation.
        OpenChangesPanel,
        /// Thread summary ▸ `Background processes` row: the command's terminal tab.
        OpenBackgroundTerminal,
        /// Thread summary ▸ `Plan`: the latest plan in its editable side panel tab.
        OpenPlanPanel,
        ShowWorkspaceTabView,
        StepWorkspaceLayout,
        /// `⌘K`, `⌘⇧P`, sidebar `Search`: the command menu.
        OpenCommandMenu,
    ]
);

/// Settings ▸ Appearance ▸ Theme: `"system"`, `"light"` or `"dark"`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct SetTheme(pub &'static str);

/// A change card opens review with an override for this exact requested path.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct ViewChanges(pub String);

// Edit menu commands; text inputs handle them.
actions!(
    edit,
    [
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        PasteAndMatchStyle,
        DeleteSelection,
        SelectAll,
        StartSpeaking,
        StopSpeaking,
    ]
);

/// `--app-shell-navigation-rail-width`
pub const RAIL_WIDTH: Pixels = px(52.);
/// `--app-shell-page-inline-end-inset`, also the layout's bottom padding.
const PAGE_INSET: Pixels = px(4.);
/// `--border-width-hairline` of `_MainContentLeftBorder`.
const MAIN_HAIRLINE: Pixels = px(0.5);

/// An authored length `v` in Blink's LayoutUnits (1/64 device px, truncated) at scale `s`.
fn layout_unit(v: Pixels, s: f32) -> Pixels {
    px((f32::from(v) * s * 64.).trunc() / 64. / s)
}

/// `_MainContentSurface`'s box in `root` at `left`, unsnapped: Blink resolves its insets in
/// LayoutUnits.
fn main_box(root: Bounds<Pixels>, left: Pixels, s: f32) -> Bounds<Pixels> {
    let inset = layout_unit(PAGE_INSET, s);
    Bounds::from_corners(
        root.origin + point(layout_unit(left, s), layout_unit(Theme::TITLEBAR_HEIGHT, s)),
        root.bottom_right() - point(inset, inset),
    )
}

/// `--button-radius` of the 36 px (`xl`) and 28 px `_Button` sizes.
const RADIUS_BUTTON_XL: Pixels = px(12.5);
const RADIUS_BUTTON_MD: Pixels = px(9.375);
/// Rail buttons are `data-squircle`: 12.5 px `corner-shape: superellipse(1.5)`.
const RADIUS_RAIL: Pixels = px(12.5);
/// `-apple-system`, as the shell sets it.
pub const SANS: &str = ".SystemUIFont";
/// `ui-monospace`, which Chromium on macOS resolves to Menlo.
pub const MONO: &str = "Menlo";

static CODE_FONT: std::sync::RwLock<&'static str> = std::sync::RwLock::new(MONO);

/// The code font of the appearance settings (`fonts.code`); `MONO` until one is set.
pub fn code_font() -> &'static str {
    *CODE_FONT.read().unwrap_or_else(|error| error.into_inner())
}

/// Follows the appearance setting. A changed name is leaked once so callers keep `&'static str`.
pub fn set_code_font(name: &str) {
    if name != code_font() {
        *CODE_FONT.write().unwrap_or_else(|error| error.into_inner()) =
            Box::leak(name.to_owned().into_boxed_str());
    }
}

/// A native destination shown after the rail divider.
pub struct RailDestination {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub mcp_entrypoints: &'static [(&'static str, &'static str)],
}

pub const RAIL_DESTINATIONS: &[RailDestination] = &[
    RailDestination {
        id: "builtin:pull-requests",
        icon: "rail-code-review",
        label: "Code Review",
        mcp_entrypoints: &[
            ("code-review", "pull_requests.open"),
            ("connector_openai_code_review", "pull_requests.open"),
        ],
    },
    RailDestination {
        id: "builtin:sites",
        icon: "sites",
        label: "Sites",
        mcp_entrypoints: &[],
    },
];

/// The pinned destinations' rail order, as indices into [`RAIL_DESTINATIONS`].
// shortcut: session-only; persist it with the account's sidebar customization
// (`sidebar-customization-snapshots-v2` `order`) once the port writes that state.
#[derive(Clone, Copy)]
struct RailOrder([usize; RAIL_DESTINATIONS.len()]);

impl gpui::Global for RailOrder {}

impl RailOrder {
    fn get(cx: &App) -> Self {
        cx.try_global()
            .copied()
            .unwrap_or(Self(std::array::from_fn(|i| i)))
    }

    /// `from` takes `to`'s place and the items between shift, as the original's `arrayMove`
    /// on drop.
    fn moved(mut self, from: usize, to: usize) -> Self {
        let at = |id| self.0.iter().position(|&i| i == id).expect("a permutation");
        let (from, to) = (at(from), at(to));
        if from < to {
            self.0[from..=to].rotate_left(1);
        } else {
            self.0[to..=from].rotate_right(1);
        }
        self
    }
}

/// A dragged pinned rail destination (its [`RAIL_DESTINATIONS`] index); it also draws the
/// item under the pointer.
struct RailDrag(usize);

impl Render for RailDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .size(px(36.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_RAIL)
            .bg(t.ghost_hover)
            .child(icon::icon(RAIL_DESTINATIONS[self.0].icon, 20., t.text))
    }
}

pub struct Shell {
    pub sidebar_open: bool,
    /// Below the sidebar breakpoint the panel floats over the main surface instead of docking.
    pub overlay: bool,
    /// The toggle's spring: its start and the progress it left from
    /// ([`Shell::sidebar_progress`]). `None` when set without motion.
    sidebar_motion: Option<(Instant, f32)>,
    pub sidebar_width: Pixels,
    sidebar_drag_offset: Option<Pixels>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Rail item shown as selected (`"home"`, `"scheduled"`, ...); its icon switches to
    /// `<name>-filled`. `None` selects nothing.
    pub selected_rail: Option<&'static str>,
    /// Titlebar `Hide sidebar` button (chat routes; Settings hides it).
    pub chat_chrome: bool,
    /// Titlebar `New tab` button (chat routes; Settings and Automations hide it).
    pub new_tab: bool,
    pub header_actions: Option<AnyView>,
    /// Opaque background boxes the page paints under the main surface's clip that reach its
    /// rounded corners (Settings' `electron:bg-surface`): each blends the clip's anti-aliased
    /// edge once more. With any, the shell paints those corners itself, over what the main
    /// panel's content painted there.
    pub main_backgrounds: u8,
    /// Per-corner `main_backgrounds`, when the boxes differ by corner (Code Review).
    pub main_corner_backgrounds: Option<Corners<u8>>,
    /// The main surface is a composited layer (thread routes): Chromium clips each layer in
    /// it, so its rounded corners show its own fill and then the content (the thread's top fade
    /// or composer band, or the side panel) at the clip's coverage.
    pub main_composited: bool,
    /// Corners where only the content is a composited layer (Code Review's empty detail
    /// pane): the page's raster draws the rounded F, then viz draws the content through the
    /// clip's coverage, as at `main_composited`'s corners.
    pub main_composited_corners: Corners<bool>,
    /// With `main_composited`: the main surface itself (fill and hairline) is one more
    /// composited layer (Chromium's layer tree of refs/diff), not part of the page's; the shell
    /// draws its right corners' px of partial clip coverage.
    pub main_layer: bool,
    /// The opaque accelerated scroller that can occlude the page frame.
    pub main_opaque_scroll: Option<gpui::ScrollHandle>,
    /// With `main_composited`: the main view draws its right corners' px of partial clip
    /// coverage itself, after the shell (a lone thread: `shapes::paint_clipped_tile`).
    pub main_clips_right: bool,
    /// A chat needs attention (unread): the rail's Home item carries the unread dot.
    pub attention: bool,
    /// Unseen, unpinned MCP destinations; `None` while their source is unavailable.
    pub unread_destinations: Option<Arc<[SharedString]>>,
    pub security_available: bool,
    pub scheduled_attention: bool,
    /// The profile avatar's menu is open.
    pub profile_menu: bool,
    /// Pointer over the avatar: its own click toggles the menu, so a press there must not
    /// also close it as a click outside.
    avatar_hovered: bool,
    /// The Explore `…` menu is open.
    pub explore_menu: bool,
    explore_hovered: bool,
    sidebar: AnyView,
    main: AnyView,
    main_content_inline_inset: Pixels,
    pub bottom: Option<AnyView>,
}

/// The fixture account's 24 px avatar (`rounded-full object-cover` `<img>`), rasterized the way
/// Chromium draws it in the profile button's layer: the JPEG decoded to scale, drawn bilinear
/// into the snapped box under its anti-aliased circular clip, and composited over `under`.
///
/// `avatar-decoded-64.png` is the capture's 256 px JPEG data URL (sha256 6e76408f…dd2375)
/// decoded by libjpeg-turbo at scale 2/8, as Blink decodes a JPEG to the smallest supported
/// size at least the drawn size (Pillow: `Image.draft("RGB", (64, 64))`).
// ponytail: one decode size, valid while the avatar is at most 64 device px (scale 2.66);
// add the 3/8 and larger decodes when a larger zoom matters.
fn avatar(under: Hsla) -> AnyElement {
    const PNG: &[u8] = include_bytes!("../assets/avatar-decoded-64.png");
    if !cfg!(target_os = "macos") {
        let image = Arc::new(Image::from_bytes(ImageFormat::Png, PNG.to_vec()));
        return img(image).size(px(24.)).rounded_full().into_any_element();
    }
    static DECODED: LazyLock<image::RgbaImage> =
        LazyLock::new(|| image::load_from_memory(PNG).expect("avatar").to_rgba8());
    type Cached = (Bounds<Pixels>, Hsla, Arc<RenderImage>);
    thread_local! {
        static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
    }
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let s = window.scale_factor();
            let dev = |v: Pixels| (f32::from(v) * s).round();
            let (x, y) = (dev(bounds.left()), dev(bounds.top()));
            let (w, h) = (dev(bounds.right()) - x, dev(bounds.bottom()) - y);
            let at = Bounds::new(point(px(x / s), px(y / s)), size(px(w / s), px(h / s)));
            let cached = CACHE.with_borrow(|c| {
                c.as_ref()
                    .filter(|(b, u, _)| *b == at && *u == under)
                    .map(|(_, _, image)| image.clone())
            });
            let Some(image) = cached.or_else(|| {
                // The replaced raster (another box or hover color) leaves the sprite atlas.
                if let Some((_, _, old)) = CACHE.take() {
                    window.drop_image(old).ok();
                }
                let image = Arc::new(shapes::raster_images(
                    &[(&DECODED, at)],
                    at,
                    Corners::all(px(w.min(h) / 2. / s)),
                    shapes::Corner::Round,
                    shapes::ImageComposition::Composited(under),
                    window,
                )?);
                CACHE.set(Some((at, under, image.clone())));
                Some(image)
            }) else {
                return;
            };
            window
                .paint_image(at, at, Corners::default(), image, 0, false)
                .ok();
        },
    )
    .size(px(24.))
    .into_any_element()
}

impl Shell {
    /// `sidebar` fills the sidebar panel, `main` the main panel.
    pub fn new(sidebar: AnyView, main: AnyView) -> Self {
        Self {
            sidebar_open: true,
            overlay: false,
            sidebar_motion: None,
            sidebar_width: Theme::SIDEBAR_WIDTH,
            sidebar_drag_offset: None,
            can_go_back: false,
            can_go_forward: false,
            selected_rail: Some("home"),
            chat_chrome: true,
            new_tab: true,
            header_actions: None,
            main_backgrounds: 0,
            main_corner_backgrounds: None,
            main_composited: false,
            main_composited_corners: Corners::default(),
            main_layer: false,
            main_opaque_scroll: None,
            main_clips_right: false,
            attention: false,
            unread_destinations: None,
            security_available: false,
            scheduled_attention: false,
            profile_menu: false,
            avatar_hovered: false,
            explore_menu: false,
            explore_hovered: false,
            sidebar,
            main,
            main_content_inline_inset: px(0.),
            bottom: None,
        }
    }

    pub fn main_content_inline_inset(&self) -> Pixels {
        self.main_content_inline_inset
    }

    pub fn set_main(&mut self, main: AnyView, cx: &mut Context<Self>) {
        self.main = main;
        cx.notify();
    }

    pub fn set_bottom(&mut self, bottom: Option<AnyView>, cx: &mut Context<Self>) {
        if self.bottom.as_ref().map(|view| view.entity_id())
            != bottom.as_ref().map(|view| view.entity_id())
        {
            self.bottom = bottom;
            cx.notify();
        }
    }

    pub fn main_width(&self, window: &Window, cx: &App) -> Pixels {
        let progress = self.layout_progress(cx.background_executor().now());
        let left = RAIL_WIDTH + (self.sidebar_width - RAIL_WIDTH) * progress;
        let border = if progress > 0.1 {
            MAIN_HAIRLINE
        } else {
            px(0.)
        };
        (window.viewport_size().width - left - PAGE_INSET - border).max(px(0.))
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_drag_offset = None;
        let now = cx.background_executor().now();
        let from = self.sidebar_progress(now);
        self.sidebar_open = !self.sidebar_open;
        self.sidebar_motion = (!motion::reduced(cx)).then_some((now, from));
        cx.notify();
    }

    /// How far the sidebar pushes the main surface: an overlay pushes nothing.
    fn layout_progress(&self, now: Instant) -> f32 {
        if self.overlay {
            0.
        } else {
            self.sidebar_progress(now)
        }
    }

    /// How far the sidebar is open at `now`, 0 to 1: the toggle's `MO-01` spring
    /// ([`motion::spring_300`]) between the states, else the state itself.
    fn sidebar_progress(&self, now: Instant) -> f32 {
        let target = if self.sidebar_open { 1. } else { 0. };
        match self.sidebar_motion {
            Some((started, from)) => {
                motion::spring_300(from, target, now.saturating_duration_since(started))
            }
            None => target,
        }
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

/// A measured inline-run advance, kept on the LayoutUnit (1/64 device px) grid by rounding:
/// Blink positions inline runs from their advances, so authored-size truncation must not
/// apply to it.
pub(crate) fn run_width(width: Pixels, window: &Window) -> Pixels {
    let unit = window.scale_factor() * 64.;
    px((f32::from(width) * unit).round() / unit)
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
        unsnapped: None,
    }
}

pub struct RoundedClip {
    radius: Pixels,
    corner: shapes::Corner,
    child: AnyElement,
    /// A `clip-path: inset(.. round radius)` box at its unsnapped geometry, in place of the
    /// child's bounds (`shapes::with_unsnapped_clip`): nested layers raster without it.
    unsnapped: Option<Bounds<Pixels>>,
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
        let radius = Corners::all(self.radius);
        let paint = |window: &mut Window| child.paint(window, cx);
        match self.unsnapped {
            Some(clip) => shapes::with_unsnapped_clip(clip, radius, window, paint),
            None => shapes::with_rounded_clip(radius, self.corner, bounds, true, window, paint),
        }
    }
}

impl IntoElement for RoundedClip {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Tracks decoration bounds for an existing compositor layer without adding a layer.
pub fn recorded_bounds(child: impl IntoElement) -> impl IntoElement {
    RecordedBounds {
        child: child.into_any_element(),
        dense: false,
    }
}

/// Tracks the tight bounds of a compositor layer, including shadow overflow.
pub fn composited_bounds(child: impl IntoElement) -> impl IntoElement {
    RecordedBounds {
        child: child.into_any_element(),
        dense: true,
    }
}

struct RecordedBounds {
    child: AnyElement,
    dense: bool,
}

impl gpui::Element for RecordedBounds {
    type RequestLayoutState = ();
    type PrepaintState = Option<[i32; 4]>;

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
    ) -> Self::PrepaintState {
        shapes::recorded_bounds(|| {
            self.child.prepaint(window, cx);
        })
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        recorded: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dense = self.dense;
        let paint = || self.child.paint(window, cx);
        if dense {
            shapes::with_composited_bounds(*recorded, paint);
        } else {
            shapes::with_recorded_bounds(*recorded, paint);
        }
    }
}

impl IntoElement for RecordedBounds {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// A 28 px secondary ghost button with a 150 ms hover fill and icon color transition.
/// `glyph` is a [`icon::icon`] (Chromium-exact) or a plain [`icon`] svg.
pub fn icon_button(
    id: &'static str,
    glyph: impl Styled + IntoElement + 'static,
    t: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_BUTTON_MD)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(motion::ButtonVisual {
            glyph,
            radius: RADIUS_BUTTON_MD,
            hover: t.ghost_hover,
            foreground: t.text.opacity(0.85),
            selected: false,
        })
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
            .child(home_sidebar_header(
                false,
                false,
                true,
                ModeSwitch {
                    current: "Codex",
                    wordmark: ("codex-wordmark", 53.22),
                    on_click: Box::new(|_, _, _| {}),
                    menu: None,
                },
                cx,
            ))
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

/// Click handler for the Home mode switch.
pub type ModeSwitchClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The `Codex ⌄` trigger's current mode, click handler and open menu.
pub struct ModeSwitch {
    pub current: &'static str,
    /// The `h-[1em] w-auto` svg's name and viewBox width (its height is 18).
    pub wordmark: (&'static str, f32),
    pub on_click: ModeSwitchClick,
    pub menu: Option<AnyElement>,
}

/// Top of the Home sidebar: `Codex ⌄` mode switch, activity bell and search, then the
/// `New chat` and `Your dot` rows. With `unread` chats the bell carries the blue dot.
pub fn home_sidebar_header(
    unread: bool,
    activity: bool,
    available: bool,
    mode: ModeSwitch,
    cx: &App,
) -> Div {
    let t = cx.theme();
    // The Oklab round-trip leaves blue above one; retain it for Skia's float layer blend.
    let activity_background = gpui::Rgba {
        r: 0.007_846_47,
        g: 0.521_568_5,
        b: 1.000_000_1,
        a: 0.13,
    }
    .into();
    let activity_label = if activity {
        "Turn off activity view"
    } else if unread {
        "View activity, needs attention"
    } else {
        "View activity"
    };
    let keyboard_ring = |inset| {
        move |style: gpui::StyleRefinement| {
            style.shadow(vec![gpui::BoxShadow {
                color: gpui::rgb(0x3a83f7).into(),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset,
            }])
        }
    };
    // Chromium squashes the search button's `_ButtonInner` and the rows below into one
    // transparent overlap layer, from the rows' left edge and the button's top; the mode
    // switch's wordmark and chevron go into another, from the wordmark's left edge.
    let overlap = point(RAIL_WIDTH + px(8.), Theme::TITLEBAR_HEIGHT + px(12.));
    let mode_switch = point(RAIL_WIDTH + px(16.), Theme::TITLEBAR_HEIGHT + px(10.));
    let row = |id: &'static str, leading: AnyElement, label: &'static str| {
        let hover_bg = t.list_hover;
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(keyboard_ring(true))
            .h(px(30.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            // `py-[5px]`: the 21 px line overflows the content box and centers with negative
            // free space.
            .py(px(5.))
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
                        .role(gpui::Role::Button)
                        .aria_label(format!("Switch mode, current mode: {}", mode.current))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(mode.on_click)
                        .relative()
                        .tab_index(0)
                        .focus_visible(keyboard_ring(true))
                        .h(px(32.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(RADIUS_BUTTON_XL)
                        .child(
                            icon::icon_sized(
                                mode.wordmark.0,
                                mode.wordmark.1,
                                18.,
                                t.text_default_nav,
                            )
                            // `h-[1em] w-auto` on the svg's viewBox (width x 18).
                            .auto_width(mode.wordmark.1 / 18.)
                            .own_layer(Some(mode_switch)),
                        )
                        .child(
                            icon::icon("chevron-down-medium", 14., t.text_tertiary)
                                .own_layer(Some(mode_switch)),
                        )
                        .children(mode.menu),
                )
                .child(div().flex_1())
                .when(available, |d| {
                    d.child(tooltip(
                        div()
                            .id("activity")
                            .relative()
                            .flex_none()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_BUTTON_MD)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .when(activity, |d| {
                                // The original button pseudo-element has will-change: transform.
                                d.child(div().absolute().inset_0().top_layer().child(
                                    shapes::decoration(shapes::Decoration {
                                        radius: Corners::all(RADIUS_BUTTON_MD),
                                        corner: shapes::Corner::Squircle,
                                        fill: Some(activity_background),
                                        ..Default::default()
                                    }),
                                ))
                            })
                            .child(motion::ButtonVisual {
                                glyph: icon::icon(
                                    if unread { "bell-unread" } else { "bell" },
                                    16.,
                                    if activity {
                                        t.info_solid
                                    } else {
                                        t.text_tertiary
                                    },
                                )
                                .own_layer(None),
                                radius: RADIUS_BUTTON_MD,
                                hover: if activity {
                                    activity_background
                                } else {
                                    t.ghost_hover
                                },
                                foreground: if activity {
                                    t.info_solid
                                } else {
                                    t.text.opacity(0.85)
                                },
                                selected: activity,
                            })
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .aria_label(activity_label)
                            .focus_visible(keyboard_ring(false))
                            .on_click(dispatch::<ToggleActivity>),
                        activity_label,
                        Some("⌥⌘U"),
                        Side::Bottom,
                    ))
                    .child(div().w(px(4.)))
                })
                .child(tooltip(
                    icon_button(
                        "search",
                        icon::icon("search", 16., t.text_tertiary).own_layer(Some(overlap)),
                        t,
                    )
                    .tab_index(0)
                    .focus_visible(keyboard_ring(false))
                    .on_click(dispatch::<OpenCommandMenu>),
                    "Search",
                    Some("⌘K"),
                    Side::Bottom,
                )),
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
                        icon::icon("new-chat", 16., t.text_default_nav)
                            .own_layer(Some(overlap))
                            .into_any_element(),
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
                            .image()
                            .own_layer(Some(overlap))
                            .into_any_element(),
                        "Your dot",
                    )
                    // The original navigates in-app to its `/dots/home` route; the port has no
                    // dot screen, so it opens the same ChatGPT page, like dot search results.
                    .on_click(|_, _, cx| cx.open_url("https://chatgpt.com/dots/home")),
                )),
        )
}

impl Shell {
    fn rail(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (hover_bg, hover_fg) = (t.ghost_hover, t.text);
        let selected = self.selected_rail;
        let home = self.main.entity_type() == std::any::TypeId::of::<screens::home::HomeMain>();
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
        let profile_focus = focus::handle("rail-profile", 0, window, cx);
        let info_solid = t.info_solid;
        // The avatar composites over the rail, or over the profile button's hover fill.
        // ponytail: the hover fill's end color, not its 150 ms fade; pass the fade's color if
        // the avatar's corners ever show during it.
        let under = over(t.page_surface_layout, t.background_surface_under);
        let avatar_under = if self.avatar_hovered {
            over(hover_bg, under)
        } else {
            under
        };
        let scale = window.scale_factor();
        let rail_order = RailOrder::get(cx);
        let shell = cx.entity().downgrade();
        let explore_hover =
            cx.listener(|this, hovered: &bool, _, _| this.explore_hovered = *hovered);
        let explore_click = cx.listener(|this, _: &ClickEvent, _, cx| {
            this.explore_menu ^= true;
            cx.notify();
        });
        let mut item = |name: &'static str| {
            let on = selected == Some(name);
            let fg = if on { t.text } else { t.text_tertiary };
            let id = SharedString::from(format!("rail-{name}"));
            let focus = focus::handle(id.clone(), 0, window, cx);
            let glyph = if on && !matches!(name, "explore" | "rail-code-review" | "sites") {
                format!("{name}-filled")
            } else {
                name.to_string()
            };
            focus::ring(
                div()
                    .id(id)
                    .relative()
                    .flex_none()
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_RAIL)
                    .when(on, |d| d.child(fill(hover_bg)))
                    .child(motion::ButtonVisual {
                        glyph: icon::icon(&glyph, 20., fg).composited(Some(list)),
                        radius: RADIUS_RAIL,
                        hover: hover_bg,
                        foreground: hover_fg,
                        selected: on,
                    }),
                &focus,
                RADIUS_RAIL,
                px(1.),
                cx,
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
        let rail_tip =
            |item: Stateful<Div>, label: &'static str| tooltip(item, label, None, Side::Right);
        div()
            .tab_group()
            .tab_index(0)
            .tab_stop(false)
            .absolute()
            .left_0()
            .top(Theme::TITLEBAR_HEIGHT)
            .bottom(PAGE_INSET)
            .w(RAIL_WIDTH)
            // `#app-shell-sidebar` (`[contain:layout_paint]`) starts at the rail's origin.
            .snap_origin()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(8.))
            .pb(px(4.))
            .child(
                // `-m-0.5 p-0.5`: the list keeps 2 px for its items' focus rings. Blink
                // truncates the padding and the margin to LayoutUnits on their own.
                div()
                    .flex_1()
                    .w(px(40.))
                    .m(px(-2.))
                    .p(px(2.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .layer()
                    .child(rail_tip(
                        item("home")
                            .when(self.attention, |d| d.child(dot(6.)))
                            .on_click(move |_, window, cx| {
                                let focus = window.focused(cx).filter(|_| home);
                                window.dispatch_action(Box::new(NewChat), cx);
                                window.defer(cx, move |window, cx| {
                                    if let Some(focus) = focus {
                                        focus.focus(window, cx);
                                    }
                                });
                            }),
                        "Home",
                    ))
                    .child(rail_tip(
                        item("space").on_click(dispatch::<OpenSpace>),
                        "Space",
                    ))
                    .child(rail_tip(
                        item("scheduled")
                            .when(self.scheduled_attention, |d| d.child(dot(6.)))
                            .on_click(dispatch::<OpenScheduled>),
                        "Scheduled",
                    ))
                    .child(rail_tip(
                        item("plugins").on_click(dispatch::<OpenPlugins>),
                        "Plugins",
                    ))
                    // Explore has no tooltip; its marker follows the bound destination state.
                    // `absolute -end-0.5 -top-0.5` around a `size-5` box centering an `icon-xs`
                    // dot at `scale-50`.
                    .child(
                        item("explore")
                            .on_hover(explore_hover)
                            .on_click(explore_click)
                            .when(
                                self.unread_destinations
                                    .as_ref()
                                    .is_some_and(|ids| !ids.is_empty()),
                                |item| {
                                    item.child(
                                        div()
                                            .absolute()
                                            .top(px(-2.))
                                            .right(px(-2.))
                                            .size(px(20.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                canvas(
                                                    |_, _, _| {},
                                                    move |bounds, (), window, cx| {
                                                        shapes::paint_scaled(
                                                            &shapes::Decoration {
                                                                radius: Corners::all(
                                                                    Theme::RADIUS_FULL,
                                                                ),
                                                                fill: Some(info_solid),
                                                                ..Default::default()
                                                            },
                                                            Some(list),
                                                            bounds,
                                                            size(px(16.), px(16.)),
                                                            0.5,
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                )
                                                .size(px(16.)),
                                            ),
                                    )
                                },
                            ),
                    )
                    // A 1 px `border-t` on an empty box: Blink paints it rounded down to whole
                    // device px (at least 1), and the box is that tall.
                    .child(
                        div()
                            .w(px(24.))
                            .h(px(scale.floor().max(1.) / scale))
                            .bg(t.border),
                    )
                    // The pinned destinations reorder by drag (`sidebarElectron.rail.reorder`).
                    .children(rail_order.0.map(|index| {
                        let destination = &RAIL_DESTINATIONS[index];
                        let shell = shell.clone();
                        rail_tip(
                            item(destination.icon)
                                .when(destination.id == "builtin:pull-requests", |item| {
                                    item.on_click(dispatch::<OpenPullRequests>)
                                })
                                .when(destination.id == "builtin:sites", |item| {
                                    item.on_click(dispatch::<OpenSites>)
                                })
                                .on_drag(RailDrag(index), |drag, _, _, cx| {
                                    cx.new(|_| RailDrag(drag.0))
                                })
                                .on_drop(move |drag: &RailDrag, _, cx| {
                                    cx.set_global(RailOrder::get(cx).moved(drag.0, index));
                                    shell.update(cx, |_, cx| cx.notify()).ok();
                                }),
                            destination.label,
                        )
                    }))
                    .child(shapes::top_fade(px(16.))),
            )
            .children(updater::rail(t, window, cx))
            .child(
                focus::ring(
                    div().id("rail-profile"),
                    &profile_focus,
                    RADIUS_RAIL,
                    px(1.),
                    cx,
                )
                .size(px(36.))
                // The original composites the profile button: the avatar rasterizes in its layer.
                .layer()
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS_RAIL)
                .on_hover(cx.listener(|this, hovered: &bool, _, _| this.avatar_hovered = *hovered))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.profile_menu ^= true;
                    cx.notify();
                }))
                .child(motion::ButtonVisual {
                    glyph: div().size(px(24.)).child(avatar(avatar_under)),
                    radius: RADIUS_RAIL,
                    hover: hover_bg,
                    foreground: hover_fg,
                    selected: false,
                }),
            )
    }

    /// The Explore `…` menu (`sidebarCustomization.moreMenu`), right of the rail item: the
    /// secondary destinations the port has. `Customize sidebar` and MCP destinations need the
    /// sidebar-customization store, which the port lacks.
    fn explore_menu(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = t.menu_item_hover;
        div()
            .absolute()
            .left(RAIL_WIDTH)
            .top(Theme::TITLEBAR_HEIGHT + px(180.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.explore_hovered {
                    this.explore_menu = false;
                    cx.notify();
                }
            }))
            .child(composer::popover(t, 220., |d| {
                if self.security_available {
                    d.child(
                        div()
                            .id("explore-security")
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(12.))
                            .hover(move |s| s.bg(hover))
                            .child(text("Security", 14., 20., 430., t.text))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.explore_menu = false;
                                cx.notify();
                                window.dispatch_action(Box::new(OpenSecurity), cx);
                            })),
                    )
                } else {
                    d.child(div().px(px(8.)).py(px(6.)).child(text(
                        "No more destinations",
                        14.,
                        20.,
                        400.,
                        t.text_tertiary,
                    )))
                }
            }))
    }

    /// Profile avatar menu (`codex.profileDropdown.*`) above the avatar: `Settings`, then the
    /// `Help` submenu's `Set up mobile`, flattened under a `Help` heading.
    /// Profile and Usage open pages that this menu does not yet expose.
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
        let mini = item("profile-mini", "Show Mini", dispatch::<OpenMini>);
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
                    .child(mini)
                    .child(item("profile-log-out", "Log out", dispatch::<LogOut>))
                    .child(div().px(px(8.)).pt(px(8.)).pb(px(4.)).child(text(
                        "Help",
                        12.,
                        16.,
                        400.,
                        t.text_tertiary,
                    )))
                    .child(mobile)
                    .child(item(
                        "profile-keyboard-shortcuts",
                        "Keyboard Shortcuts",
                        dispatch::<ShowKeyboardShortcuts>,
                    ))
            }))
    }

    /// `.sidebar-navigation`: its left corners come from `clip-path: inset(... round 12px)`.
    /// It is a composited cc layer: its tile holds the translucent background and the content
    /// is blitted onto it before the layer is composited (`.layer()`).
    /// While the toggle springs, the content fades with `progress` and slides from -16 px.
    fn sidebar_panel(&self, t: &Theme, progress: f32) -> Div {
        let r = Theme::RADIUS_XL_BASE;
        // Square: the `clip-path` rounds the corners at its fractional edges.
        let background = shapes::Decoration {
            fill: Some(t.sidebar_background),
            ..Default::default()
        };
        div()
            .tab_group()
            .tab_index(1)
            .tab_stop(false)
            .absolute()
            .left(RAIL_WIDTH)
            .top_0()
            .bottom_0()
            .w(self.sidebar_width - RAIL_WIDTH)
            .flex()
            .flex_col()
            .overflow_hidden()
            .layer()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, cx| {
                        // The `clip-path` box keeps a zoomed edge's fraction, in
                        // `#app-shell-sidebar`'s space: its left is the content-sized rail
                        // (`px-2` around 36 px items, each in LayoutUnits), its bottom an inset
                        // in LayoutUnits. Blink rounds that paint-containment box's LayoutUnit
                        // top to device px and drops the fraction, so the whole box moves with it.
                        let s = window.scale_factor();
                        let left = layout_unit(px(8.), s) * 2. + layout_unit(px(36.), s);
                        let top = layout_unit(Theme::TITLEBAR_HEIGHT, s);
                        let shift = px((f32::from(top) * s).round() / s) - top;
                        let bottom =
                            window.viewport_size().height - layout_unit(PAGE_INSET, s) + shift;
                        let clip = Bounds::from_corners(
                            point(left, top + shift),
                            point(bounds.right() + r, bottom),
                        );
                        // Blink clips to `#app-shell-sidebar`'s paint-containment box (snapped,
                        // anti-aliased) before the `clip-path`: Skia rasters the rounded clip
                        // within that clip, which changes the arc's last rows above the box's
                        // bottom edge. The fill rasters in that box's composited layer (layer 46
                        // at (0, 97) at 110%), so neither the box nor a tile edge cuts the arc's
                        // left: at 90% the panel's snapped left (94) is right of the clip's.
                        let shell = Bounds::from_corners(
                            point(px(0.), bounds.top()),
                            bounds.bottom_right(),
                        );
                        shapes::with_unsnapped_clip(shell, Corners::default(), window, |window| {
                            shapes::with_unsnapped_clip(clip, Corners::all(r), window, |window| {
                                shapes::paint(&background, Some(shell.origin), bounds, window, cx);
                            });
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .left(px(-16.) * (1. - progress))
                    .opacity(progress)
                    .child(self.sidebar.clone()),
            )
    }

    /// `_MainContentSurface`'s content box. Its fill and `_MainContentLeftBorder` hairline
    /// (`background-clip: padding-box`) paint with the page in `render`: with the sidebar open
    /// the left edge is square and the right corners are rounded by its `clip-path`; with it
    /// closed it is a plain 12 px rounded box. `left` is its edge, `open` its shape.
    #[allow(clippy::too_many_arguments)]
    fn main_panel(
        &self,
        left: Pixels,
        open: bool,
        focus: &gpui::FocusHandle,
        window: &Window,
        theme: &Theme,
        hairline_width: Pixels,
        cx: &Context<Self>,
    ) -> Div {
        let r = Theme::RADIUS_XL_BASE;
        let shell = cx.entity().downgrade();
        let measured = canvas(
            move |bounds, window, cx| {
                let outer_width = window.viewport_size().width - PAGE_INSET - left;
                shell
                    .update(cx, |shell, _| {
                        shell.main_content_inline_inset =
                            (outer_width - bounds.size.width).max(px(0.));
                    })
                    .ok();
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        let panel = div()
            .tab_group()
            .tab_index(3)
            .tab_stop(false)
            .track_focus(focus)
            .absolute()
            .top(Theme::TITLEBAR_HEIGHT)
            .right(PAGE_INSET)
            .bottom(PAGE_INSET)
            .overflow_hidden()
            .child(if let Some(bottom) = &self.bottom {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(self.main.clone()))
                    .child(
                        div()
                            .h(px(280.))
                            .min_h(px(160.))
                            .max_h(gpui::relative(0.5))
                            .flex_none()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(bottom.clone()),
                    )
                    .into_any_element()
            } else {
                self.main.clone().into_any_element()
            })
            .child(measured);
        if self.main_composited {
            return panel.left(if open { left + hairline_width } else { left });
        }
        // The `clip-path` box (its left corners under the open sidebar), unsnapped as for the
        // fill in `render`: Skia rasters main's draws under that anti-aliased clip, but not its
        // composited layers. The panel lays out at `left` as in Blink; the clip reaches r under
        // the sidebar at paint time.
        let (outset, inset) = if open {
            (r, hairline_width)
        } else {
            (px(0.), px(0.))
        };
        let root = Bounds::new(Point::default(), window.viewport_size());
        let main = main_box(root, left, window.scale_factor());
        let clip =
            Bounds::from_corners(point(main.left() - outset, main.top()), main.bottom_right());
        div()
            .absolute()
            .left(left)
            .top(Theme::TITLEBAR_HEIGHT)
            .right(PAGE_INSET)
            .bottom(PAGE_INSET)
            .child(RoundedClip {
                radius: r,
                corner: shapes::Corner::Round,
                unsnapped: Some(clip),
                child: div()
                    .size_full()
                    .relative()
                    .child(panel.left(inset).top_0().right_0().bottom_0())
                    .into_any_element(),
            })
    }

    fn titlebar(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let native_inset = if window.is_fullscreen() {
            px(2.)
        } else {
            px(82.)
        };
        // The native traffic-light inset does not scale with the UI.
        let nav_left = native_inset / window.ui_scale() + px(6.);
        // The header is the window drag region (`-webkit-app-region: drag`, macOS hiddenInset);
        // its buttons are `no-drag`: each stops the press before the move handler below.
        // Disabled history controls do not participate in keyboard navigation.
        let mut nav = |id: &'static str, glyph: icon::Icon, enabled: bool| {
            if enabled {
                let focus = focus::handle(id, 0, window, cx);
                focus::ring(
                    icon_button(id, glyph, t),
                    &focus,
                    RADIUS_BUTTON_MD,
                    px(2.),
                    cx,
                )
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    focus.read(cx).handle.clone().focus(window, cx);
                    cx.stop_propagation();
                })
            } else {
                div()
                    .id(id)
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(glyph.opacity(0.4))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            }
            .absolute()
            .top(px(8.))
        };
        let back = icon::icon("arrow-left", 16., t.text_tertiary);
        // Forward is the Back glyph mirrored (`-scale-x-100`).
        let forward = icon::icon("arrow-left", 16., t.text_tertiary).flip_x();
        let back = nav("back", back, self.can_go_back);
        let forward = nav("forward", forward, self.can_go_forward);
        let sidebar_focus = focus::handle("toggle-sidebar", 0, window, cx);
        let new_tab_focus = focus::handle("new-tab", 0, window, cx);
        div()
            .tab_group()
            .tab_index(2)
            .tab_stop(false)
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
            .child(tooltip(
                back.left(nav_left).on_click(dispatch::<GoBack>),
                "Back",
                Some("⌘["),
                Side::Bottom,
            ))
            .child(tooltip(
                forward
                    .left(nav_left + px(34.))
                    .on_click(dispatch::<GoForward>),
                "Forward",
                Some("⌘]"),
                Side::Bottom,
            ))
            .when(self.chat_chrome, |d| {
                d.child(tooltip(
                    focus::ring(
                        icon_button(
                            "toggle-sidebar",
                            if self.sidebar_open {
                                icon::icon("sidebar", 16., t.text_tertiary)
                            } else {
                                // Closed, Chromium composites the titlebar's Show sidebar and
                                // New chat icons: they blit into a layer of their own.
                                icon::icon("sidebar-hidden", 16., t.text_tertiary).own_layer(None)
                            },
                            t,
                        ),
                        &sidebar_focus,
                        RADIUS_BUTTON_MD,
                        px(2.),
                        cx,
                    )
                    .role(gpui::Role::Button)
                    .aria_label(if self.sidebar_open {
                        "Hide sidebar"
                    } else {
                        "Show sidebar"
                    })
                    .absolute()
                    .left(nav_left + px(68.))
                    .top(px(8.))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        sidebar_focus.read(cx).handle.clone().focus(window, cx);
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    "Toggle sidebar",
                    Some("⇧⌘S"),
                    Side::Bottom,
                ))
            })
            // Closed, the titlebar also carries New chat after Show sidebar, except on Home,
            // which already is a new chat.
            .when(
                self.chat_chrome
                    && !self.sidebar_open
                    && self.main.entity_type() != std::any::TypeId::of::<screens::home::HomeMain>(),
                |d| {
                    d.child(tooltip(
                        icon_button(
                            "titlebar-new-chat",
                            icon::icon("sidebar-cmd-new-chat", 16., t.text_tertiary)
                                .own_layer(None),
                            t,
                        )
                        .role(gpui::Role::Button)
                        .aria_label("New chat")
                        .absolute()
                        .left(nav_left + px(102.))
                        .top(px(8.))
                        .on_click(dispatch::<NewChat>),
                        "New chat",
                        Some("⌘N"),
                        Side::Bottom,
                    ))
                },
            )
            .when(self.new_tab, |d| {
                d.child(
                    focus::ring(div().id("new-tab"), &new_tab_focus, px(8.), px(2.), cx)
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            new_tab_focus.read(cx).handle.clone().focus(window, cx);
                            cx.stop_propagation();
                        })
                        // Chromium squashes it into the titlebar's overlap layer, at whole
                        // device px (layer 44 at (380, 0) at 2x, layer 97 at (0, 0) at 110%).
                        .child(
                            icon::icon("new-tab", 16., t.text_tertiary)
                                .own_layer(Some(Point::default())),
                        ),
                )
            })
            .when_some(self.header_actions.clone(), |bar, actions| {
                bar.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(44.))
                        .h(px(28.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(actions),
                )
            })
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let main_focus = focus::handle("main-content", -1, window, cx);
        let main_focus = main_focus.read(cx).handle.clone().tab_stop(false);
        let skip_focus = focus::handle("skip-to-content", -1, window, cx);
        let skip_link = focus::ring(
            div()
                .id("skip-to-content")
                .role(gpui::Role::Link)
                .aria_label("Skip to content")
                .absolute()
                .left(px(88.))
                .top(px(8.))
                .size(px(1.))
                .overflow_hidden()
                .opacity(0.)
                .focus(|mut s| {
                    s.overflow.x = Some(gpui::Overflow::Visible);
                    s.overflow.y = Some(gpui::Overflow::Visible);
                    // Include the original 1 px border in the padding; the decoration paints it.
                    s.w_auto().h_auto().px(px(17.)).py(px(7.)).opacity(1.)
                })
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(RADIUS_BUTTON_XL),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.button_primary_solid),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
                .child(text(
                    "Skip to content",
                    14.,
                    18.,
                    430.,
                    t.surface_elevated_secondary,
                )),
            &skip_focus,
            RADIUS_BUTTON_XL,
            px(2.),
            cx,
        )
        .on_click({
            let main_focus = main_focus.clone();
            move |_, window, cx| main_focus.focus(window, cx)
        })
        .on_key_down({
            let main_focus = main_focus.clone();
            move |event, window, cx| {
                if event.keystroke.key == "enter" {
                    main_focus.focus(window, cx);
                    cx.stop_propagation();
                }
            }
        });
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
        let now = cx.background_executor().now();
        if let Some((started, _)) = self.sidebar_motion
            && now.saturating_duration_since(started) < motion::RELAXED
        {
            window.request_animation_frame();
        }
        let progress = self.sidebar_progress(now);
        // The original keeps the page-sidebar shape (`data-app-shell-page-sidebar`) above 0.1
        // of progress; its hairline fades out over 0.2 to 0.05 while closing.
        let docked = self.layout_progress(now);
        let open = docked > 0.1;
        let main_left = RAIL_WIDTH + (self.sidebar_width - RAIL_WIDTH) * docked;
        let scale = window.scale_factor();
        let hairline_width = px((f32::from(MAIN_HAIRLINE) * scale).floor().max(1.) / scale);
        let hairline_alpha = if self.sidebar_open && !self.overlay {
            1.
        } else {
            ((docked - 0.05) / 0.15).clamp(0., 1.)
        };
        let composited = self.main_composited;
        let main_layer = composited && self.main_layer;
        let clip_twice = main_layer && screens::diff::panel::panel_clamped(window, cx);
        let clips_right = composited && self.main_clips_right;
        let n = self.main_backgrounds;
        let backgrounds = self.main_corner_backgrounds.unwrap_or(Corners {
            top_left: n,
            top_right: n,
            bottom_right: n,
            bottom_left: n,
        });
        let viz_corners = self.main_composited_corners;
        let viz_any = viz_corners != Corners::default();
        let fills = if composited {
            1
        } else {
            let b = &backgrounds;
            1 + b
                .top_left
                .max(b.top_right)
                .max(b.bottom_right)
                .max(b.bottom_left)
        };
        let r = Theme::RADIUS_XL_BASE;
        // The corners a composited main surface's compositor clip rounds.
        let radius = if open {
            Corners {
                top_right: r,
                bottom_right: r,
                ..Default::default()
            }
        } else {
            Corners::all(r)
        };
        // Square (`border-radius: 0`): its `clip-path`, or for a composited main surface the
        // compositor, rounds the corners.
        let main_fill = shapes::Decoration {
            fill: Some(t.surface),
            ..Default::default()
        };
        let hairline = shapes::Decoration {
            fill: Some(t.sidebar_divider.opacity(hairline_alpha)),
            ..Default::default()
        };
        let under = t.background_surface_under;
        let surface_px = {
            let c = gpui::Rgba::from(t.surface);
            [c.r, c.g, c.b, 1.].map(|v| (v * 255.).round() as u8)
        };
        #[cfg(target_os = "macos")]
        let opaque_scroll = self.main_opaque_scroll.clone();
        // The page frame over the root `bounds`.
        let frame = move |bounds: Bounds<Pixels>,
                          fills: u8,
                          square: bool,
                          window: &mut Window,
                          cx: &mut App| {
            // The compositor rounds a composited (`square`) main surface; otherwise the clip does.
            // Under an open sidebar the clip's left corners lie under it, so they stay square.
            let clip_radius = if square { Corners::default() } else { radius };
            // Blink resolves the insets in LayoutUnits: a zoomed clip edge keeps that fraction.
            let s = window.scale_factor();
            let (top, inset) = (
                layout_unit(Theme::TITLEBAR_HEIGHT, s),
                layout_unit(PAGE_INSET, s),
            );
            let page_surface = Bounds::from_corners(
                bounds.origin + point(layout_unit(RAIL_WIDTH, s), top),
                bounds.bottom_right() - point(inset, inset),
            );
            let mut main = main_box(bounds, main_left, s);
            // Its `clip-path` box (`inset(0 0 0 -r)` under an open sidebar) keeps the fraction
            // of a zoomed edge.
            let clip_left = if open { main.left() - r } else { main.left() };
            let clip = Bounds::from_corners(point(clip_left, main.top()), main.bottom_right());
            let line = open.then(|| {
                let line = Bounds::new(main.origin, size(hairline_width, main.size.height));
                main.origin.x += hairline_width;
                main.size.width -= hairline_width;
                line
            });
            // `clip-path: inset(0 -r 0 0 round r)` rounds every corner at its fractional edges.
            let main_surface = |window: &mut Window, cx: &mut App| {
                shapes::with_unsnapped_clip(clip, clip_radius, window, |window| {
                    if let Some(line) = line {
                        shapes::paint(&hairline, None, line, window, cx);
                    }
                    for _ in 0..fills {
                        shapes::paint(&main_fill, None, main, window, cx);
                    }
                });
            };
            window.with_skia_layer(255, |window| {
                shapes::paint(&layout, None, bounds, window, cx);
                shapes::paint(&surface, None, page_surface, window, cx);
                #[cfg(target_os = "macos")]
                if main_layer
                    && let Some(scroll) = &opaque_scroll
                    && scroll.max_offset().y > px(0.)
                {
                    shapes::paint_page_occlusion(bounds, scroll.bounds(), under, window, cx);
                }
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
        let docked = self.bottom.is_some();
        let corners = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let s = window.scale_factor();
                let dev = |v: Pixels| f32::from(v) * s;
                let rd = dev(r).round();
                let (left, top) = (dev(main_left), dev(Theme::TITLEBAR_HEIGHT));
                let right = dev(bounds.size.width - PAGE_INSET);
                let bottom = dev(bounds.size.height - PAGE_INSET);
                // Each corner: its point, which way the main panel lies, its background boxes
                // and whether viz draws its content.
                let (b, viz) = (&backgrounds, &viz_corners);
                let mut corners = vec![
                    (right, top, -1., 1., b.top_right, viz.top_right),
                    (right, bottom, -1., -1., b.bottom_right, viz.bottom_right),
                ];
                if !open {
                    corners.extend([
                        (left, top, 1., 1., b.top_left, viz.top_left),
                        (left, bottom, 1., -1., b.bottom_left, viz.bottom_left),
                    ]);
                }
                let mut main = bounds;
                for (x, y, dx, dy, backgrounds, viz) in corners {
                    if composited || viz {
                        // viz draws each layer of the clipped surface through the clip's
                        // coverage `m`, partial px with `SkBlendARGB32`: its own fill F onto the
                        // page, then the content C (painted over the square F). In a layer of
                        // the page with the rounded F (the frame), C is drawn that way onto it.
                        // At `main_clips_right` and `main_layer` the layer's pixel stays: the
                        // view (or the shell, below) draws the content onto it. A docked bottom
                        // panel holds the bottom-right corner instead of the view: C is drawn here.
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
                                // A lone thread draws its right corners, and its top-left one
                                // when the sidebar is closed (`ThreadView::clip_top_left`).
                                let own = clips_right || main_layer;
                                let own = Corners {
                                    top_right: own,
                                    bottom_right: main_layer || (clips_right && !docked),
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
                            frame(bounds, 1 + backgrounds, false, window, cx);
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
        let sidebar = (progress > 0.).then(|| {
            let panel = div()
                .absolute()
                .left_0()
                .top(Theme::TITLEBAR_HEIGHT)
                .bottom(PAGE_INSET)
                .w(self.sidebar_width)
                .snap_origin()
                .child(self.sidebar_panel(&t, progress));
            // An overlay floats over the main surface, so it casts a shadow.
            if self.overlay {
                panel.shadow_lg()
            } else {
                panel
            }
        });
        let (sidebar_below, sidebar_above) = if self.overlay {
            (None, sidebar)
        } else {
            (sidebar, None)
        };
        // The rail and titlebar do not overlap the panels, so they paint after the frame.
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .on_key_down(focus::tab)
            .font_family(SANS)
            .bg(under)
            .child(page)
            // `#app-shell-sidebar`, around the rail too, is `[contain:layout_paint]`: its
            // content drops a fractional (zoomed) top.
            .children(sidebar_below)
            .child(self.main_panel(main_left, open, &main_focus, window, &t, hairline_width, cx))
            .when(composited || fills > 1 || viz_any, |d| d.child(corners))
            .children(sidebar_above)
            .when(self.sidebar_open, |d| {
                d.child(
                    div()
                        .id("sidebar-resize")
                        .role(gpui::Role::Splitter)
                        .absolute()
                        .left(self.sidebar_width - px(8.))
                        .top(Theme::TITLEBAR_HEIGHT)
                        .bottom(PAGE_INSET)
                        .w(px(16.))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, e: &MouseDownEvent, _, cx| {
                                this.sidebar_drag_offset = Some(e.position.x - this.sidebar_width);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
            })
            .when_some(self.sidebar_drag_offset, |d, offset| {
                let moved = cx.entity().downgrade();
                let released = moved.clone();
                d.child(drag_layer(
                    move |x, _, cx| {
                        moved
                            .update(cx, |this, _| {
                                let width = x - offset;
                                this.sidebar_open = width >= px(145.);
                                if this.sidebar_open {
                                    this.sidebar_width = width.clamp(px(290.), px(520.));
                                }
                            })
                            .ok();
                    },
                    move |cx| {
                        released
                            .update(cx, |this, cx| {
                                this.sidebar_drag_offset = None;
                                cx.notify();
                            })
                            .ok();
                    },
                ))
            })
            .child(self.rail(&t, window, cx))
            .child(self.titlebar(&t, window, cx))
            .child(skip_link)
            .when(self.profile_menu, |d| d.child(self.profile_menu(&t, cx)))
            .when(self.explore_menu, |d| d.child(self.explore_menu(&t, cx)))
    }
}

/// The window-wide capture layer of a live drag: `on_move` with every pointer x (then a
/// refresh) until the left button's release, then `on_release`.
pub(crate) fn drag_layer(
    on_move: impl Fn(Pixels, &mut Window, &mut App) + 'static,
    on_release: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, (), window, _| {
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_move(e.position.x, window, cx);
                    window.refresh();
                }
            });
            window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && e.button == MouseButton::Left {
                    on_release(cx);
                }
            });
        },
    )
    .absolute()
    .size_full()
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

    #[test]
    fn rail_order_moves_the_dragged_item_to_the_drop_target() {
        let start = RailOrder([0, 1]);
        assert_eq!(start.moved(0, 1).0, [1, 0]);
        assert_eq!(start.moved(1, 0).0, [1, 0]);
        assert_eq!(start.moved(1, 0).moved(0, 1).0, [0, 1]);
        assert_eq!(start.moved(1, 1).0, [0, 1]);
    }
}
