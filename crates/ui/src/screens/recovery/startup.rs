use crate::{icon, motion};
use gpui::{AnyElement, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};
use std::{
    sync::{LazyLock, OnceLock},
    time::Duration,
};
use theme::{Appearance, Theme};

const DURATION: f64 = 2.2;
// ponytail: cache 120 frames per second; use shader uniforms if higher refresh rates need exact motion.
const FRAMES: usize = 264;
type SvgFrame = (SharedString, Vec<u8>);
// Each cell needs the frame index selected at runtime.
static SHIMMER: LazyLock<[OnceLock<SvgFrame>; FRAMES]> =
    LazyLock::new(|| std::array::from_fn(|_| OnceLock::new()));
static PATHS: LazyLock<String> = LazyLock::new(|| {
    let source = include_str!("../../../../../assets/icons/startup-openai.svg");
    let (_, paths) = source.split_once('>').expect("startup SVG root");
    paths
        .trim()
        .strip_suffix("</svg>")
        .expect("startup SVG end")
        .replace("fill=\"currentColor\"", "fill=\"url(#shimmer)\"")
});

fn frame(index: usize) -> &'static SvgFrame {
    SHIMMER[index].get_or_init(|| {
        let angle = 112_f64.to_radians();
        let (dx, dy) = (angle.sin(), -angle.cos());
        let (width, height) = (21. * 2.2, 21.);
        let length = width * dx.abs() + height * dy.abs();
        let (y1, y2) = (height / 2. - length * dy / 2., height / 2. + length * dy / 2.);
        let progress = motion::EASE_IN_OUT(index as f32 / FRAMES as f32) as f64;
        let offset = (21. - width) * (1.4 - 2.45 * progress);
        let (x1, x2) = (offset + width / 2. - length * dx / 2., offset + width / 2. + length * dx / 2.);
        let paths = &*PATHS;
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><defs><linearGradient id="shimmer" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}"><stop offset="22%" stop-color="white" stop-opacity="0"/><stop offset="38%" stop-color="white" stop-opacity=".04"/><stop offset="49%" stop-color="white" stop-opacity=".48"/><stop offset="56%" stop-color="white" stop-opacity=".08"/><stop offset="74%" stop-color="white" stop-opacity="0"/></linearGradient><clipPath id="image"><rect x="{offset}" y="0" width="{width}" height="{height}"/></clipPath></defs><g clip-path="url(#image)">{paths}</g></svg>"#);
        (format!("startup-shimmer-{index}").into(), svg.into_bytes())
    })
}

pub(super) fn indicator(elapsed: Duration, reduced: bool, theme: &Theme) -> AnyElement {
    let opacity = if reduced {
        1.
    } else {
        motion::cubic_bezier(0., 0., 0.58, 1.)((elapsed.as_secs_f32() - 0.06) / 0.18)
    };
    if opacity == 0. {
        return div().size(px(56.)).into_any_element();
    }
    let index = if reduced {
        0
    } else {
        ((elapsed.as_secs_f64() % DURATION) / DURATION * FRAMES as f64) as usize % FRAMES
    };
    let (key, data) = frame(index);
    let key = key.clone();
    let base = match theme.appearance {
        Appearance::Dark => gpui::white().opacity(0.68),
        Appearance::Light => gpui::black().opacity(0.24),
    };
    div()
        .relative()
        .size(px(56.))
        .opacity(opacity)
        .child(icon::icon("startup-openai", 56., base).runtime())
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, (), window, cx| {
                    window
                        .paint_svg(
                            bounds,
                            key,
                            Some(data),
                            gpui::TransformationMatrix::default(),
                            gpui::white(),
                            cx,
                        )
                        .ok();
                },
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}
