use crate::{focus, text};
use gpui::{
    AnyElement, App, Bounds, Corners, Div, Global, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Pixels, RenderImage, StatefulInteractiveElement, Styled, Window,
    canvas, div, img, point, px, size,
};
use std::sync::Arc;
use theme::ActiveTheme as _;

use serde::{Deserialize, Serialize};

/// The original locked-use gates, supplied by feature and account requirements.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUsePolicy {
    pub locked_use_enabled: bool,
    pub slingshot_enabled: bool,
    pub remote_control_available: Option<bool>,
    pub remote_control_access_required: bool,
    pub allow_locked_computer_use: Option<bool>,
}

impl LockedUsePolicy {
    pub fn allowed(&self) -> bool {
        self.locked_use_enabled
            && self.slingshot_enabled
            && self.remote_control_available != Some(false)
            && !self.remote_control_access_required
            && self.allow_locked_computer_use != Some(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUseState {
    pub enabled: Option<bool>,
    #[serde(rename = "computerIconDataURL")]
    pub computer_icon_data_url: Option<String>,
    #[serde(rename = "lockIconDataURL")]
    pub lock_icon_data_url: Option<String>,
}

impl LockedUseState {
    pub fn visible(&self, policy: &LockedUsePolicy) -> bool {
        policy.allowed() && self.enabled.is_some()
    }
}

pub type LockedUseDispatch = Arc<dyn Fn(bool, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct NativeSettingsState {
    pub policy: LockedUsePolicy,
    pub locked_use: LockedUseState,
    pub pending: bool,
    pub error: Option<String>,
    pub dispatch: Option<LockedUseDispatch>,
}

impl Global for NativeSettingsState {}

fn change_locked_use(enabled: bool, cx: &mut App) {
    let Some(state) = cx.try_global::<NativeSettingsState>() else {
        return;
    };
    if state.pending || !state.locked_use.visible(&state.policy) {
        return;
    }
    if let Some(dispatch) = state.dispatch.clone() {
        dispatch(enabled, cx);
    }
}

type IconRaster = (Bounds<Pixels>, f32, Hsla, Bounds<Pixels>, Arc<RenderImage>);

struct SystemIcons {
    sources: [Arc<gpui::Image>; 2],
    pixels: [image::RgbaImage; 2],
    raster: Option<IconRaster>,
}

fn icon_cache(window: &mut Window, cx: &mut App) -> gpui::Entity<Option<SystemIcons>> {
    window.use_keyed_state("locked-use-system-icons", cx, |window, cx| {
        cx.on_release_in(window, |entry: &mut Option<SystemIcons>, window, _| {
            if let Some((_, _, _, _, image)) = entry.take().and_then(|entry| entry.raster) {
                window.drop_image(image).ok();
            }
        })
        .detach();
        None::<SystemIcons>
    })
}

fn icon_origin(bounds: Bounds<Pixels>, scale: f32) -> gpui::Point<Pixels> {
    let wrapper = point(px(62.), px(63.));
    let unscaled = bounds.origin + (gpui::Point::from(bounds.size) - wrapper) / 2.;
    unscaled.map(|value| px((f32::from(value) * scale).round() / scale))
        + wrapper * ((1. - 0.65) / 2.)
}

fn grouped_icons(
    computer: &Arc<gpui::Image>,
    lock: &Arc<gpui::Image>,
    page: Hsla,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let cache = icon_cache(window, cx);
    let ready = cache.update(cx, |entry, _| {
        if entry.as_ref().is_some_and(|entry| {
            Arc::ptr_eq(&entry.sources[0], computer) && Arc::ptr_eq(&entry.sources[1], lock)
        }) {
            return true;
        }
        let decode = |source: &gpui::Image| {
            let mut pixels = image::load_from_memory(&source.bytes).ok()?.to_rgba8();
            shapes::premultiply(&mut pixels);
            Some(pixels)
        };
        let Some((computer_pixels, lock_pixels)) = decode(computer).zip(decode(lock)) else {
            return false;
        };
        if let Some((_, _, _, _, image)) = entry.take().and_then(|entry| entry.raster) {
            window.drop_image(image).ok();
        }
        *entry = Some(SystemIcons {
            sources: [computer.clone(), lock.clone()],
            pixels: [computer_pixels, lock_pixels],
            raster: None,
        });
        true
    });
    if !ready {
        return None;
    }
    Some(
        canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let image = cache.update(cx, |entry, _| {
                    let entry = entry.as_mut()?;
                    let scale = window.scale_factor();
                    if let Some((previous, previous_scale, previous_page, at, image)) =
                        &entry.raster
                        && *previous == bounds
                        && *previous_scale == scale
                        && *previous_page == page
                    {
                        return Some((*at, image.clone()));
                    }
                    // Blink snaps the 62x63 inline box before applying its CSS scale.
                    let origin = icon_origin(bounds, scale);
                    let destination = |source: &image::RgbaImage, x, y, width: f32, height: f32| {
                        let factor =
                            (width / source.width() as f32).min(height / source.height() as f32);
                        let width_scaled = source.width() as f32 * factor;
                        let height_scaled = source.height() as f32 * factor;
                        Bounds::new(
                            origin
                                + point(
                                    px((x + (width - width_scaled) / 2.) * 0.65),
                                    px((y + (height - height_scaled) / 2.) * 0.65),
                                ),
                            size(px(width_scaled * 0.65), px(height_scaled * 0.65)),
                        )
                    };
                    let image = Arc::new(shapes::raster_images(
                        &[
                            (
                                &entry.pixels[0],
                                destination(&entry.pixels[0], 4., 0., 53., 53.),
                            ),
                            (
                                &entry.pixels[1],
                                destination(&entry.pixels[1], 35., 24., 27., 33.),
                            ),
                        ],
                        bounds,
                        Corners::default(),
                        shapes::Corner::Round,
                        shapes::ImageComposition::Group(page),
                        window,
                    )?);
                    let device = |value: Pixels| (f32::from(value) * scale).round();
                    let (x, y) = (device(bounds.left()), device(bounds.top()));
                    let at = Bounds::new(
                        point(px(x / scale), px(y / scale)),
                        size(
                            px((device(bounds.right()) - x) / scale),
                            px((device(bounds.bottom()) - y) / scale),
                        ),
                    );
                    if let Some((_, _, _, _, old)) = entry.raster.take() {
                        window.drop_image(old).ok();
                    }
                    entry.raster = Some((bounds, scale, page, at, image.clone()));
                    Some((at, image))
                });
                if let Some((at, image)) = image {
                    window
                        .paint_image(at, at, Corners::default(), image, 0, false)
                        .ok();
                }
            },
        )
        .size(px(40.))
        .flex_none()
        .into_any_element(),
    )
}

pub fn locked_use_row(window: &mut Window, cx: &mut App) -> Option<Div> {
    let state = cx.try_global::<NativeSettingsState>()?;
    if !state.locked_use.visible(&state.policy) {
        return None;
    }
    let enabled = state.locked_use.enabled?;
    let disabled = state.pending || state.dispatch.is_none();
    let error = state.error.clone();
    let icons = state
        .locked_use
        .computer_icon_data_url
        .clone()
        .zip(state.locked_use.lock_icon_data_url.clone());
    let icons = icons.and_then(|(computer, lock)| {
        crate::screens::computer_use::data_image(
            &computer,
            "locked-use-computer-icon".into(),
            window,
            cx,
        )
        .zip(crate::screens::computer_use::data_image(
            &lock,
            "locked-use-lock-icon".into(),
            window,
            cx,
        ))
    });
    let t = cx.theme().clone();
    let leading = match icons {
        Some((computer, lock)) => grouped_icons(&computer, &lock, t.background_panel, window, cx)
            .or_else(|| {
                Some(
                    div()
                        .w(px(40.))
                        .h(px(40.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .relative()
                                .w(px(40.3))
                                .h(px(40.95))
                                .child(
                                    img(computer)
                                        .absolute()
                                        .left(px(2.6))
                                        .top_0()
                                        .w(px(34.45))
                                        .h(px(34.45)),
                                )
                                .child(
                                    img(lock)
                                        .absolute()
                                        .right_0()
                                        .bottom(px(3.9))
                                        .w(px(17.55))
                                        .h(px(21.45)),
                                ),
                        )
                        .into_any_element(),
                )
            }),
        _ => None,
    };
    let handle = focus::handle("locked-use-enabled", 0, window, cx);
    let control = focus::ring(
        crate::screens::settings::switch("locked-use-enabled", enabled, disabled, &t),
        &handle,
        px(10.),
        px(2.),
        cx,
    )
    .on_click(move |_, _, cx| {
        if !disabled {
            change_locked_use(!enabled, cx);
        }
    })
    .on_key_down(move |event: &KeyDownEvent, _, cx| {
        if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
            change_locked_use(!enabled, cx);
            cx.stop_propagation();
        }
    })
    .into_any_element();
    let help = "https://developers.openai.com/codex/app/computer-use#locked-use";
    let help_handle = focus::handle("locked-use-help", 0, window, cx);
    let link = focus::ring(
        div()
            .id("locked-use-help")
            .child(text("Learn more", 12., 16., 430., t.text_accent)),
        &help_handle,
        px(2.),
        px(1.),
        cx,
    )
    .on_click(move |_, _, cx| cx.open_url(help))
    .on_key_down(move |event: &KeyDownEvent, _, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            cx.open_url(help);
            cx.stop_propagation();
        }
    });
    let detail = div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .child(text(
                    "Let ChatGPT use your Mac when it's locked. ",
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                ))
                .child(link),
        )
        .children(error.map(|error| text(error, 12., 16., 430., t.text_error)));
    Some(
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .children(leading)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(text("Locked use", 13., 18.571_428, 500., t.text))
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .w(px(160.))
                    .max_w(gpui::relative(0.4))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .child(control),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, BorrowAppContext as _, Context, HeadlessAppContext, Render};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn app() -> HeadlessAppContext {
        let mut cx =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            cx.set_global(NativeSettingsState::default());
        });
        cx
    }

    fn click_toggle(window: &mut Window, cx: &mut App) {
        let position = point(px(700.), px(30.));
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    }

    struct LockedRow {
        present: bool,
    }

    impl Render for LockedRow {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let row = locked_use_row(window, cx);
            self.present = row.is_some();
            div().w_full().children(row)
        }
    }

    struct CachedIcons {
        image: Arc<RenderImage>,
        source: Arc<gpui::Image>,
        visible: bool,
    }

    impl Render for CachedIcons {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut content = div();
            if self.visible {
                let cache = icon_cache(window, cx);
                cache.update(cx, |entry, cx| {
                    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]));
                    *entry = Some(SystemIcons {
                        sources: [self.source.clone(), self.source.clone()],
                        pixels: [pixels.clone(), pixels],
                        raster: Some((
                            Bounds::default(),
                            1.,
                            cx.theme().background_panel,
                            Bounds::default(),
                            self.image.clone(),
                        )),
                    });
                });
                content = content.child(img(self.image.clone()).size(px(2.)));
            }
            content
        }
    }

    #[test]
    fn unscaled_icon_origin_snaps_before_the_css_transform() {
        let origin = icon_origin(
            Bounds::new(point(px(461.5), px(343.5)), size(px(40.), px(40.))),
            2.,
        );
        assert!((f32::from(origin.x) * 2. - 922.7).abs() < 0.0001);
        assert!((f32::from(origin.y) * 2. - 686.05).abs() < 0.0001);
    }

    #[test]
    fn leaving_the_page_releases_its_icon_atlas_entry() {
        let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 60, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let source = Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png.into_inner(),
        ));
        let image = Arc::new(RenderImage::new([image::Frame::new(pixels)]));
        let mut cx = app();
        let view = cx.update(|cx| {
            cx.new(|_| CachedIcons {
                image: image.clone(),
                source,
                visible: true,
            })
        });
        let window = cx
            .open_window(size(px(100.), px(100.)), |_, _| view.clone())
            .unwrap();
        for _ in 0..3 {
            cx.update(|cx| {
                view.update(cx, |view, cx| {
                    view.visible = true;
                    cx.notify();
                })
            });
            cx.run_until_parked();
            assert!(
                cx.update_window(window.into(), |_, window, _| {
                    window.has_image_atlas_entry(&image)
                })
                .unwrap()
            );
            cx.update(|cx| {
                view.update(cx, |view, cx| {
                    view.visible = false;
                    cx.notify();
                })
            });
            cx.run_until_parked();
            cx.update(|cx| view.update(cx, |_, cx| cx.notify()));
            cx.run_until_parked();
            assert!(
                !cx.update_window(window.into(), |_, window, _| {
                    window.has_image_atlas_entry(&image)
                })
                .unwrap()
            );
        }
    }

    #[test]
    fn locked_use_requires_confirmed_policy_and_rejects_repeated_pending_input() {
        let mut cx = app();
        let view = cx.update(|cx| cx.new(|_| LockedRow { present: false }));
        let window = cx
            .open_window(size(px(726.), px(100.)), |_, _| view.clone())
            .unwrap();
        let allowed = LockedUsePolicy {
            locked_use_enabled: true,
            slingshot_enabled: true,
            ..Default::default()
        };
        for (policy, enabled, present) in [
            (LockedUsePolicy::default(), Some(false), false),
            (allowed.clone(), None, false),
            (allowed.clone(), Some(false), true),
        ] {
            cx.update(|cx| {
                cx.set_global(NativeSettingsState {
                    policy,
                    locked_use: LockedUseState {
                        enabled,
                        ..Default::default()
                    },
                    ..Default::default()
                });
                view.update(cx, |_, cx| cx.notify());
            });
            cx.run_until_parked();
            cx.update(|cx| assert_eq!(view.read(cx).present, present));
        }
        cx.update_window(window.into(), |_, window, cx| click_toggle(window, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let state = cx.global::<NativeSettingsState>();
            assert!(!state.pending);
            assert_eq!(state.locked_use.enabled, Some(false));
        });

        let requests = Arc::new(AtomicUsize::new(0));
        let sent = requests.clone();
        cx.update(|cx| {
            cx.update_global::<NativeSettingsState, _>(|state, _| {
                state.dispatch = Some(Arc::new(move |enabled, cx| {
                    assert!(enabled);
                    sent.fetch_add(1, Ordering::Relaxed);
                    cx.update_global::<NativeSettingsState, _>(|state, _| state.pending = true);
                }));
            });
            view.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            for _ in 0..2 {
                click_toggle(window, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        cx.update(|cx| {
            let state = cx.global::<NativeSettingsState>();
            assert!(state.pending);
            assert_eq!(state.locked_use.enabled, Some(false));
        });
    }
}
