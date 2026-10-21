//! `mobile` parity screen: `/codex-mobile` (`CodexMobilePage`), from `refs/mobile/*.spec.json`
//! (initial step) and `codex-mobile-setup-dialog` (the allow-host and MFA steps). Sidebar
//! closed, no rail item current.

use super::connections::{self, Action, ConnectionsState};
use crate::{EmptySidebar, Shell, icon, text};
use gpui::{
    AnyView, App, AppContext as _, Bounds, Context, Corners, Div, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement, Pixels, Render, RenderImage, Stateful,
    StatefulInteractiveElement as _, Styled, Transformation, Window, actions, canvas, div, point,
    prelude::FluentBuilder as _, px, radians, rgb, size,
};
use image::{Frame, Rgba, RgbaImage};
use shapes::Corner::Squircle;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, LazyLock},
};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(
    codex_mobile,
    [
        /// `Get started`: checks the account's MFA requirement, then moves to the allow-host
        /// or MFA step.
        GetStarted,
        ManageConnections,
        FinishSetup,
    ]
);

/// `Continue on chatgpt.com` on the MFA step.
const MFA_SETTINGS: &str = "https://chatgpt.com/#settings/Security?mfa_source=codex_remote";

/// The setup step on screen (`codex-mobile-setup-flow` `Pe`): `Pending` and `Error` are the
/// initial step while the MFA requirement read runs (`setupInProgress`) and after it failed
/// (`showStartSetupError`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SetupStep {
    #[default]
    Initial,
    Pending,
    Error,
    AllowHost,
    MfaRequired,
    Enabling,
    AllowError,
    Waiting,
    Connected,
    Ready,
}

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    let (shell, _) = shell(cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    shell.into()
}

/// `/codex-mobile`: sidebar closed, no rail item current.
pub fn shell(cx: &mut App) -> (Entity<Shell>, Entity<MobileSetupPage>) {
    let sidebar = cx.new(|_| EmptySidebar).into();
    let page = cx.new(|cx| {
        cx.observe_global::<ConnectionsState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::computer_use::ComputerUseState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::computer_use_native::NativeSettingsState>(|_, cx| cx.notify())
            .detach();
        MobileSetupPage::default()
    });
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.clone().into());
        shell.sidebar_open = false;
        shell.selected_rail = None;
        shell.chat_chrome = false;
        shell.new_tab = false;
        // The page's own `bg-token-main-surface-primary` under `<main>`'s rounded clip.
        shell.main_backgrounds = 1;
        shell
    });
    (shell, page)
}

/// One `codexMobile.setupDialog.initial` feature row: 32 px icon slot, title and description.
fn feature(glyph: &'static str, title: &'static str, body: &'static str, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .px(px(14.))
        .py(px(8.))
        .child(
            div()
                .size(px(32.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(icon::icon(glyph, 20., t.text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(text(title, 14., 21., 430., t.text))
                .child(text(body, 13., 19.5, 430., t.text_tertiary).whitespace_normal()),
        )
}

#[derive(Default)]
pub struct MobileSetupPage {
    /// The artwork as last drawn: its box (the tile grid matters) and page color.
    artwork: ArtworkCache,
    /// Pointer over the step's primary button.
    hovered: bool,
    step: SetupStep,
    /// When `Pending` began: the loading spinner's `animate-spin` phase.
    pending_since: Option<std::time::Instant>,
    qr: Option<(String, Arc<qrcodegen::QrCode>)>,
    dialog_mode: bool,
    computer_device: bool,
    dialog_focus: Option<gpui::FocusHandle>,
}

impl MobileSetupPage {
    pub fn pairing_dialog(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<ConnectionsState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::computer_use::ComputerUseState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<super::computer_use_native::NativeSettingsState>(|_, cx| cx.notify())
            .detach();
        Self {
            dialog_mode: true,
            step: SetupStep::Waiting,
            dialog_focus: Some(cx.focus_handle().tab_stop(true)),
            ..Default::default()
        }
    }

    pub(super) fn dialog_focus(&self) -> Option<gpui::FocusHandle> {
        self.dialog_focus.clone()
    }

    fn trap_tab(&self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "tab" {
            return;
        }
        let Some(root) = &self.dialog_focus else {
            return;
        };
        if event.keystroke.modifiers.shift {
            window.focus_prev(cx);
            if !root.contains_focused(window, cx) {
                root.focus(window, cx);
                let mut last = root.clone();
                loop {
                    window.focus_next(cx);
                    if root.is_focused(window) || !root.contains_focused(window, cx) {
                        break;
                    }
                    if let Some(focused) = window.focused(cx) {
                        last = focused;
                    } else {
                        break;
                    }
                }
                last.focus(window, cx);
            }
        } else {
            window.focus_next(cx);
            if !root.contains_focused(window, cx) {
                root.focus(window, cx);
                window.focus_next(cx);
                if !root.contains_focused(window, cx) {
                    root.focus(window, cx);
                }
            }
        }
        cx.stop_propagation();
    }

    pub fn step(&self) -> SetupStep {
        self.step
    }

    pub fn set_step(&mut self, step: SetupStep, cx: &mut Context<Self>) {
        if matches!(step, SetupStep::Pending | SetupStep::Enabling) && self.step != step {
            self.pending_since = Some(cx.background_executor().now());
        }
        if artwork_kind(self.step) != artwork_kind(step) {
            self.artwork.borrow_mut().take();
        }
        self.step = step;
        self.hovered = false;
        cx.notify();
    }

    fn pairing_qr(&mut self, state: &ConnectionsState) -> Option<Arc<qrcodegen::QrCode>> {
        let code = &state.pairing.as_ref()?.pairing_code;
        if let Some((current, qr)) = &self.qr
            && current == code
        {
            return Some(qr.clone());
        }
        let mut url = String::from("https://chatgpt.com/codex/pair?pairing_code=");
        use std::fmt::Write as _;
        for byte in code.bytes() {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                url.push(char::from(byte));
            } else {
                write!(&mut url, "%{byte:02X}").ok()?;
            }
        }
        let qr = Arc::new(qrcodegen::QrCode::encode_text(&url, qrcodegen::QrCodeEcc::Medium).ok()?);
        self.qr = Some((code.clone(), qr.clone()));
        Some(qr)
    }
}

type ArtworkCache = Rc<RefCell<Option<(Bounds<Pixels>, Hsla, Arc<RenderImage>, bool)>>>;

/// The two artwork PNGs as RGBA; the overlay premultiplied (Skia's N32 premul,
/// `round(c * a / 255)`).
static ARTWORK: LazyLock<(RgbaImage, [RgbaImage; 4])> = LazyLock::new(|| {
    let decode = |bytes: &[u8]| {
        image::load_from_memory(bytes)
            .expect("artwork PNG")
            .to_rgba8()
    };
    let background = decode(include_bytes!(
        "../../../../assets/ui/mobile/page-artwork-background.png"
    ));
    let overlays: [&[u8]; 4] = [
        include_bytes!("../../../../assets/ui/mobile/page-artwork-shared.png"),
        include_bytes!("../../../../assets/ui/mobile/page-artwork-allow-host.png"),
        include_bytes!("../../../../assets/ui/mobile/page-artwork-mfa.png"),
        include_bytes!("../../../../assets/ui/mobile/page-artwork-waiting.png"),
    ];
    let overlays = overlays.map(|bytes| {
        let mut image = decode(bytes);
        shapes::premultiply(&mut image);
        image
    });
    (background, overlays)
});

static DIALOG_ARTWORK: LazyLock<[Arc<gpui::Image>; 3]> = LazyLock::new(|| {
    [
        include_bytes!("../../../../assets/ui/mobile/dialog-artwork-waiting-phone.png").as_slice(),
        include_bytes!("../../../../assets/ui/mobile/dialog-artwork-waiting-computer.png")
            .as_slice(),
        include_bytes!("../../../../assets/ui/mobile/dialog-artwork-connected.png").as_slice(),
    ]
    .map(|bytes| {
        Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            bytes.to_vec(),
        ))
    })
});

fn artwork_kind(step: SetupStep) -> usize {
    match step {
        SetupStep::AllowHost | SetupStep::AllowError | SetupStep::Enabling => 1,
        SetupStep::MfaRequired => 2,
        SetupStep::Waiting => 3,
        _ => 0,
    }
}

/// The `rounded-xl` artwork box: `page-artwork-background` (cover) under `page-artwork-shared`
/// (contain). On macOS Skia rasterizes both inside the anti-aliased clip over `page` like
/// Chrome's software raster, drawn 1:1 since GPU texture filtering rounds differently;
/// elsewhere [`compose`] resamples them on the CPU and GPUI rounds the corners.
fn artwork(cache: ArtworkCache, page: Hsla, kind: usize) -> impl IntoElement {
    let radius = Corners::all(Theme::RADIUS_XL);
    canvas(
        move |bounds, window, _| {
            let mut cache = cache.borrow_mut();
            if let Some((b, p, image, rounded)) = &*cache
                && *b == bounds
                && *p == page
            {
                return (image.clone(), *rounded);
            }
            let scale = window.scale_factor();
            let dev = |v: Pixels| (f32::from(v) * scale).round() as u32;
            let (w, h) = (dev(bounds.size.width), dev(bounds.size.height));
            let (background, overlays) = &*ARTWORK;
            let overlay = &overlays[kind];
            let dst = |src: &RgbaImage, cover: bool| {
                aligned_bounds(bounds, scale, w, h, src, cover, kind)
            };
            let skia = shapes::raster_images(
                &[
                    (background, dst(background, true)),
                    (overlay, dst(overlay, false)),
                ],
                bounds,
                radius,
                Squircle,
                shapes::ImageComposition::Direct(page),
                window,
            );
            let rounded = skia.is_none();
            let image = Arc::new(skia.unwrap_or_else(|| compose(w, h, kind)));
            *cache = Some((bounds, page, image.clone(), rounded));
            (image, rounded)
        },
        move |bounds, (image, rounded), window, _| {
            let radius = if rounded { radius } else { Corners::default() };
            window
                .paint_image(bounds, bounds, radius, image, 0, false)
                .ok();
        },
    )
    .size_full()
}

/// `object-fit` / `background-size` rect of a `src` image in a `w` x `h` box, edges snapped
/// half up to device px like Blink: `(x, len)` per axis.
fn fit(w: u32, h: u32, src: &RgbaImage, cover: bool) -> [(f32, f32); 2] {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let (sx, sy) = (w as f32 / sw, h as f32 / sh);
    let s = if cover { sx.max(sy) } else { sx.min(sy) };
    let axis = |boxed: f32, len: f32| {
        let start = ((boxed - len) / 2. + 0.5).floor();
        (start, ((boxed + len) / 2. + 0.5).floor() - start)
    };
    [axis(w as f32, sw * s), axis(h as f32, sh * s)]
}

fn aligned_fit(w: u32, h: u32, src: &RgbaImage, cover: bool, kind: usize) -> [(f32, f32); 2] {
    let mut result = fit(w, h, src, cover);
    if !cover && matches!(kind, 1 | 2) {
        result[0].0 = w as f32 - result[0].1;
        if kind == 1 {
            result[1].0 = h as f32 - result[1].1;
        }
    }
    result
}

fn aligned_bounds(
    bounds: Bounds<Pixels>,
    scale: f32,
    w: u32,
    h: u32,
    src: &RgbaImage,
    cover: bool,
    kind: usize,
) -> Bounds<Pixels> {
    let [(x, iw), (y, ih)] = aligned_fit(w, h, src, cover, kind);
    let at = |v: f32| px(v / scale);
    Bounds::new(bounds.origin + point(at(x), at(y)), size(at(iw), at(ih)))
}

/// Skia's legacy bilinear sampling (`SkBitmapProcState`) along one axis for device pixels
/// `from..from + n` (relative to the box): the pixel center goes through the float inverse
/// of the dst-rect mapping, then steps in 32.32 fixed point; each result is the clamped
/// texel pair and a 4-bit weight.
fn skia_axis(
    from: i32,
    n: i32,
    (start, len): (f32, f32),
    src_len: u32,
    step: bool,
) -> Vec<(u32, u32, u32)> {
    let inv = 1. / (len / src_len as f32);
    let offset = -start * inv;
    let fixed =
        |x: i32| ((f64::from((x as f32 + 0.5) * inv + offset) * 4_294_967_296.) as i64) - (1 << 31);
    let (first, dx) = (fixed(from), (f64::from(inv) * 4_294_967_296.) as i64);
    let max = i64::from(src_len) - 1;
    (0..n)
        .map(|i| {
            // Columns step from the span start; rows map each center anew.
            let f = if step {
                first + i64::from(i) * dx
            } else {
                fixed(from + i)
            } >> 16;
            let i0 = f >> 16;
            (
                i0.clamp(0, max) as u32,
                (i0 + 1).clamp(0, max) as u32,
                ((f >> 12) & 0xF) as u32,
            )
        })
        .collect()
}

fn bilerp(
    img: &RgbaImage,
    (x0, x1, qx): (u32, u32, u32),
    (y0, y1, qy): (u32, u32, u32),
) -> [u32; 4] {
    let (a, b, c, d) = (
        img.get_pixel(x0, y0),
        img.get_pixel(x1, y0),
        img.get_pixel(x0, y1),
        img.get_pixel(x1, y1),
    );
    std::array::from_fn(|k| {
        let top = u32::from(a[k]) * (16 - qx) + u32::from(b[k]) * qx;
        let bottom = u32::from(c[k]) * (16 - qx) + u32::from(d[k]) * qx;
        (top * (16 - qy) + bottom * qy) >> 8
    })
}

/// Background (cover) with the premultiplied overlay (contain) composited src-over, as BGRA:
/// Chrome's software raster on the CPU, without its span restarts (the fallback off macOS).
fn compose(w: u32, h: u32, kind: usize) -> RenderImage {
    let (background, overlays) = &*ARTWORK;
    let overlay = &overlays[kind];
    let span = |(start, len): (f32, f32)| (start as i32).max(0)..((start + len) as i32);
    let [bx, by] = fit(w, h, background, true);
    let (bcols, brows) = (
        skia_axis(0, w as i32, bx, background.width(), true),
        skia_axis(0, h as i32, by, background.height(), false),
    );
    let [ox, oy] = aligned_fit(w, h, overlay, false, kind);
    let (oxs, oys) = (span(ox), span(oy));
    let (ocols, orows) = (
        skia_axis(oxs.start, oxs.len() as i32, ox, overlay.width(), true),
        skia_axis(oys.start, oys.len() as i32, oy, overlay.height(), false),
    );
    let mut out = RgbaImage::new(w, h);
    for (y, row) in out.rows_mut().enumerate() {
        for (x, px) in row.enumerate() {
            let mut c = bilerp(background, bcols[x], brows[y]);
            let (xi, yi) = (x as i32, y as i32);
            if oxs.contains(&xi) && oys.contains(&yi) {
                let s = bilerp(
                    overlay,
                    ocols[(xi - oxs.start) as usize],
                    orows[(yi - oys.start) as usize],
                );
                for k in 0..3 {
                    c[k] = s[k] + (c[k] * (255 - s[3]) + 127) / 255;
                }
            }
            *px = Rgba([c[2] as u8, c[1] as u8, c[0] as u8, 255]);
        }
    }
    RenderImage::new([Frame::new(out)])
}

fn setup_toggle(
    control: (&'static str, &'static str),
    body: &'static str,
    enabled: bool,
    disabled: bool,
    change: super::computer_use_native::LockedUseDispatch,
    t: &Theme,
) -> Div {
    let (id, label) = control;
    let key_change = change.clone();
    let control = super::settings::switch(id, enabled, disabled, t)
        .role(gpui::Role::Switch)
        .aria_label(label)
        .aria_disabled(disabled)
        .aria_toggled(if enabled {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .focusable()
        .tab_index(0)
        .on_click(move |_, _, cx| {
            if !disabled {
                change(!enabled, cx);
            }
        })
        .on_key_down(move |event: &gpui::KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key_change(!enabled, cx);
                cx.stop_propagation();
            }
        });
    div()
        .w_full()
        .max_w(px(400.))
        .flex()
        .items_center()
        .gap(px(16.))
        .child(
            div()
                .flex_1()
                .text_left()
                .child(text(label, 14., 21., 430., t.text))
                .child(text(body, 13., 19.5, 430., t.text_secondary).whitespace_normal()),
        )
        .child(control)
}

fn connected_rows(state: &ConnectionsState, t: &Theme, cx: &App) -> Vec<Div> {
    let mut rows = vec![setup_toggle(
        ("mobile-keep-awake", "Keep this Mac awake"),
        "Prevent sleep when this computer is plugged in and remote access is enabled",
        state.keep_awake,
        state.pending,
        Arc::new(|enabled, cx| connections::dispatch(Action::KeepAwake(enabled), cx)),
        t,
    )];
    if let Some(computer) = cx.try_global::<super::computer_use::ComputerUseState>()
        && computer.available
        && let super::computer_use::Load::Ready(Some(plugin)) = &computer.plugin
    {
        let dispatch = computer.dispatch.clone();
        let installed = plugin.installed;
        rows.push(setup_toggle(
            ("mobile-computer-use", "Enable computer use"),
            "Let ChatGPT control apps on your Mac",
            plugin.enabled,
            computer.pending || dispatch.is_none(),
            Arc::new(move |enabled, cx| {
                if let Some(dispatch) = &dispatch {
                    dispatch(
                        if !installed && enabled {
                            super::computer_use::ComputerUseAction::Install
                        } else {
                            super::computer_use::ComputerUseAction::SetEnabled(enabled)
                        },
                        cx,
                    );
                }
            }),
            t,
        ));
    }
    if let Some(native) = cx.try_global::<super::computer_use_native::NativeSettingsState>()
        && native.locked_use.visible(&native.policy)
        && let Some(dispatch) = native.dispatch.clone()
    {
        rows.push(setup_toggle(
            ("mobile-locked-use", "Use your Mac apps while locked"),
            "Control Mac apps from your phone.",
            native.locked_use.enabled == Some(true),
            native.pending,
            dispatch,
            t,
        ));
    }
    rows
}

fn qr_image(qr: Arc<qrcodegen::QrCode>, foreground: Hsla, background: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let width = qr.size();
            let cell = f32::from(bounds.size.width) / (width + 8) as f32;
            let surface = shapes::Decoration {
                fill: Some(background),
                ..Default::default()
            };
            shapes::paint(&surface, None, bounds, window, cx);
            let module = shapes::Decoration {
                fill: Some(foreground),
                ..Default::default()
            };
            for y in 0..width {
                for x in 0..width {
                    if qr.get_module(x, y) {
                        let at = bounds.origin
                            + point(px((x + 4) as f32 * cell), px((y + 4) as f32 * cell));
                        shapes::paint(
                            &module,
                            None,
                            Bounds::new(at, size(px(cell), px(cell))),
                            window,
                            cx,
                        );
                    }
                }
            }
        },
    )
    .size(px(330.))
}

impl Render for MobileSetupPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx
            .try_global::<ConnectionsState>()
            .cloned()
            .unwrap_or_default();
        let qr = self.pairing_qr(&state);
        let t = cx.theme();
        let step = if self.dialog_mode && state.pairing_connected {
            SetupStep::Connected
        } else {
            self.step
        };
        let pending = matches!(step, SetupStep::Pending | SetupStep::Enabling);
        // `animate-spin`: one turn per second, linear, while the read runs.
        let spin = pending.then(|| {
            window.request_animation_frame();
            let now = cx.background_executor().now();
            let turn = now - self.pending_since.unwrap_or(now);
            (turn.as_secs_f32() % 1.) * std::f32::consts::TAU
        });
        // Primary solid button; dark mode text is `#2d2d2d`, not `--color-text-inverse`.
        let button_fg = if t.appearance == Appearance::Dark {
            rgb(0x2d2d2d).into()
        } else {
            t.text_inverse
        };
        let disabled = pending;
        let button_bg = if self.hovered && !disabled {
            t.button_primary_solid_hover
        } else {
            t.button_primary_solid
        };
        // A `size="large"` primary button: 36 px pill, 14/18 label after the loading spinner.
        let button = |id: &'static str, label: &'static str| -> Stateful<Div> {
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .focusable()
                .tab_index(0)
                .h(px(36.))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(9999.)),
                    fill: Some(button_bg),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.hovered = *hovered;
                    cx.notify();
                }))
                .on_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                    if disabled || !matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        return;
                    }
                    match id {
                        "allow-host" => connections::dispatch(Action::SetEnabled(true), cx),
                        "get-started" => window.dispatch_action(Box::new(GetStarted), cx),
                        "finish-setup" | "close-device-pairing" => {
                            window.dispatch_action(Box::new(FinishSetup), cx)
                        }
                        "manage-connections" => {
                            window.dispatch_action(Box::new(ManageConnections), cx)
                        }
                        "refresh-pairing" => connections::dispatch(Action::RefreshPairing, cx),
                        "continue-on-chatgpt" => cx.open_url(MFA_SETTINGS),
                        _ => {}
                    }
                    cx.stop_propagation();
                })
                .when_some(spin, |b, angle| {
                    b.child(
                        crate::icon("sidebar-cmd-spinner", 12., button_fg)
                            .with_transformation(Transformation::rotate(radians(angle))),
                    )
                })
                .child(text(label, 14., 18., 430., button_fg))
                .when(disabled, |b| b.opacity(0.4))
        };
        let (heading, description) = match step {
            SetupStep::AllowHost | SetupStep::AllowError | SetupStep::Enabling => (
                "Allow devices to control this computer?",
                "This lets authorized devices, such as your phone, connect to ChatGPT and control this computer remotely",
            ),
            SetupStep::MfaRequired => (
                "Turn on Multi-Factor Authentication",
                "To enable this feature, you’ll need to turn on Multi-Factor Authentication for your ChatGPT account",
            ),
            SetupStep::Waiting => (
                "Approve on your device to control this Mac remotely",
                "Scan to continue setup in ChatGPT",
            ),
            SetupStep::Connected => (
                "You’re connected",
                "Make the most out of your new connection. You can change these later in Settings.",
            ),
            SetupStep::Ready => (
                "Remote",
                "Scan QR code to set up a new phone or manage existing connections",
            ),
            SetupStep::Initial | SetupStep::Pending | SetupStep::Error => (
                "Connect your phone to this Mac",
                "Continue chats from the ChatGPT desktop app on your phone or another device",
            ),
        };
        // `divide-y-[0.5px] divide-border`
        let divider = |row: Div| row.border_b(px(0.5)).border_color(t.border);
        let column = div()
            .w_full()
            .max_w(px(640.))
            .px(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .text_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(400.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text(heading, 28., 35., 400., t.text).whitespace_normal())
                    .child(text(description, 16., 24., 430., t.text_tertiary).whitespace_normal()),
            );
        let column = match step {
            SetupStep::AllowHost | SetupStep::AllowError | SetupStep::Enabling => column
                .child(button("allow-host", "Allow").px(px(20.)).on_click(|_, _, cx| connections::dispatch(Action::SetEnabled(true), cx)))
                .when(step == SetupStep::AllowError, |column| column.child(text(
                    "Couldn’t enable remote control. Try again", 13., 18.571_428, 430., t.text_error,
                ))),
            SetupStep::Waiting | SetupStep::Ready => {
                let mut column = column;
                if self.dialog_mode {
                    column = column.child(div().flex().gap(px(8.)).children([(false, "Phone"), (true, "Computer")].into_iter().map(|(computer, label)| {
                        super::settings::button(label, t.text_alpha_5, None, t).id(label).on_click(cx.listener(move |this, _, _, cx| {
                            this.computer_device = computer;
                            cx.notify();
                        }))
                    })));
                }
                let column = if self.computer_device {
                    column.children(state.pairing.as_ref().and_then(|p|p.manual_pairing_code.clone()).map(|code| {
                        text(code, 32., 40., 500., t.text)
                    })).child(text("Click Add in the Settings > Connections > Control other devices tab on your other computer and enter this code", 14., 21., 430., t.text_secondary).whitespace_normal())
                } else {
                    column.children(qr.map(|qr| qr_image(qr, t.text, t.surface)))
                }
                    .when(state.pairing.is_none() && state.pairing_error.is_none(), |column| column.child(text(
                        "Loading pairing code", 13., 20., 430., t.text_secondary,
                    )))
                    .children(state.pairing_error.as_ref().map(|error| text(
                        format!("Pairing code unavailable: {error}"), 13., 20., 430., t.text_secondary,
                    )))
                    .child(button("refresh-pairing", "Refresh pairing code").px(px(20.))
                        .on_click(|_, _, cx| connections::dispatch(Action::RefreshPairing, cx)));
                if step == SetupStep::Ready {
                    column.child(button("manage-connections", "Manage connections").px(px(20.)).on_click(crate::dispatch::<ManageConnections>))
                } else {
                    column
                }
            }
            SetupStep::Connected => column.children(connected_rows(&state, t, cx))
                .child(button("finish-setup", "Done").w(px(256.)).on_click(crate::dispatch::<FinishSetup>))
                .when(!self.dialog_mode, |column| column.child(button("manage-connections", "Manage connections").w(px(256.)).on_click(crate::dispatch::<ManageConnections>))),
            SetupStep::MfaRequired => column.child(
                button("continue-on-chatgpt", "Continue on chatgpt.com")
                    .px(px(20.))
                    .on_click(|_, _, cx| cx.open_url(MFA_SETTINGS)),
            ),
            SetupStep::Initial | SetupStep::Pending | SetupStep::Error => column
                .child(
                    div()
                        .w_full()
                        .max_w(px(400.))
                        .relative()
                        .flex()
                        .flex_col()
                        .child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(px(20.)),
                            corner: Squircle,
                            border: Some((px(1.), t.border)),
                            ..Default::default()
                        }))
                        .p(px(1.))
                        .text_left()
                        .child(divider(feature(
                            "mobile-phone",
                            "Pick up where you left off",
                            "Continue any chat or project from the ChatGPT desktop app",
                            t,
                        )))
                        .child(divider(feature(
                            "mobile-chat",
                            "Stay in the loop",
                            "Get notified when ChatGPT finishes a task or needs your attention",
                            t,
                        )))
                        .child(feature(
                            "mobile-start",
                            "Start something new",
                            "Just send a message to start a task on your desktop",
                            t,
                        )),
                )
                .child(
                    button("get-started", "Get started")
                        .w_full()
                        .max_w(px(400.))
                        .when(!pending, |b| b.on_click(crate::dispatch::<GetStarted>)),
                )
                .child(
                    div().w_full().max_w(px(400.)).child(
                        text(
                            "ChatGPT can access your desktop—including files, apps, and your browser—to complete tasks you send from your phone. This can introduce security risks. Only connect devices that you own and trust.",
                            12.,
                            18.,
                            430.,
                            t.text_tertiary,
                        )
                        .whitespace_normal(),
                    ),
                )
                .when(step == SetupStep::Error, |c| {
                    c.child(text(
                        "Couldn’t check security requirements. Try again",
                        13.,
                        18.571_428,
                        430.,
                        t.text_error,
                    ))
                }),
        };
        if self.dialog_mode {
            let artwork = if step == SetupStep::Connected {
                2
            } else {
                usize::from(self.computer_device)
            };
            return div()
                .id("mobile-pairing-dialog")
                .size_full()
                .overflow_y_scroll()
                .bg(t.surface)
                .flex()
                .flex_col()
                .when_some(self.dialog_focus.as_ref(), |page, focus| {
                    page.track_focus(focus)
                })
                .capture_key_down(
                    cx.listener(|this, event, window, cx| this.trap_tab(event, window, cx)),
                )
                .child(
                    gpui::img(DIALOG_ARTWORK[artwork].clone())
                        .h(px(214.))
                        .w_full()
                        .flex_none(),
                )
                .child(
                    div()
                        .py(px(24.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(24.))
                        .child(column)
                        .when(step != SetupStep::Connected, |column| {
                            column.child(
                                button("close-device-pairing", "Cancel")
                                    .px(px(20.))
                                    .on_click(crate::dispatch::<FinishSetup>),
                            )
                        }),
                );
        }
        let page = div()
            .size_full()
            .relative()
            .flex()
            // `bg-token-main-surface-primary`; the shell paints its corners
            // (`Shell::main_backgrounds`).
            .bg(t.surface)
            .child(
                div().flex_1().min_w_0().flex().child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(24.))
                        .py(px(48.))
                        .child(column),
                ),
            )
            .when(step != SetupStep::Ready, |page| {
                page.child(div().flex_1().min_w_0().p(px(12.)).child(artwork(
                    self.artwork.clone(),
                    t.surface,
                    artwork_kind(step),
                )))
            });
        div()
            .id("mobile-page")
            .size_full()
            .flex()
            .flex_col()
            .when(
                matches!(
                    step,
                    SetupStep::AllowHost
                        | SetupStep::AllowError
                        | SetupStep::Enabling
                        | SetupStep::MfaRequired
                        | SetupStep::Waiting
                ),
                |page| {
                    page.child(
                        div()
                            .h(px(46.))
                            .flex_none()
                            .px(px(24.))
                            .flex()
                            .items_center()
                            .child(text("Set up Remote", 14., 21., 430., t.text)),
                    )
                },
            )
            .child(page.flex_1().min_h_0())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn contain_rect_snaps_half_up() {
        let overlay = &super::ARTWORK.1[0];
        // refs/mobile/*@1440x900: 680x828 CSS box at 2x; Chrome draws the overlay at 163.
        assert_eq!(
            super::fit(1360, 1656, overlay, false),
            [(163., 1035.), (0., 1656.)]
        );
    }
}
