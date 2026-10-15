//! Helpers shared by the lab examples.

use gpui::{Hsla, Rgba};

pub fn rgba_css([r, g, b, a]: [u8; 4]) -> String {
    format!("rgba({r},{g},{b},{})", f64::from(a) / 255.)
}

pub fn hsla([r, g, b, a]: [u8; 4]) -> Hsla {
    let c = |v: u8| f32::from(v) / 255.;
    Rgba {
        r: c(r),
        g: c(g),
        b: c(b),
        a: c(a),
    }
    .into()
}
