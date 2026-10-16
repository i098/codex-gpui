//! Blink's shaper: HarfBuzz with its own (`hb-ot`) advances, the font's variation position,
//! scale = device font size and `ptem` = CSS size (`HarfBuzzFace::GetScaledFont`).

use core_foundation::{
    base::{CFType, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::CFString,
};
use core_text::font::{CTFont, CTFontRef};
use harfbuzz_sys as hb;
use std::{ffi::c_void, ptr};

/// A HarfBuzz font for one CoreText font at one size, and its AAT `trak` advance (hb units).
pub struct HbFont(*mut hb::hb_font_t, i64);

// The font is only used under the text system's mutex.
unsafe impl Send for HbFont {}

impl Drop for HbFont {
    fn drop(&mut self) {
        unsafe { hb::hb_font_destroy(self.0) };
    }
}

unsafe extern "C" fn reference_table(
    _: *mut hb::hb_face_t,
    tag: hb::hb_tag_t,
    font: *mut c_void,
) -> *mut hb::hb_blob_t {
    let font = unsafe { &*(font as *const CTFont) };
    let Some(data) = font.get_font_table(tag) else {
        return ptr::null_mut();
    };
    let bytes = data.bytes();
    unsafe {
        hb::hb_blob_create(
            bytes.as_ptr().cast(),
            bytes.len() as u32,
            hb::HB_MEMORY_MODE_DUPLICATE,
            ptr::null_mut(),
            None,
        )
    }
}

unsafe extern "C" fn drop_font(font: *mut c_void) {
    drop(unsafe { Box::from_raw(font as *mut CTFont) });
}

impl HbFont {
    /// `font` is the CSS-size font; its variation position (with `opsz` = CSS size, as Blink
    /// sets it) becomes the HarfBuzz variation.
    pub fn new(font: &CTFont, css_size: f64, device_scale: f64) -> Self {
        unsafe {
            let face = hb::hb_face_create_for_tables(
                Some(reference_table),
                Box::into_raw(Box::new(font.clone())).cast(),
                Some(drop_font),
            );
            let hb_font = hb::hb_font_create(face);
            hb::hb_face_destroy(face);
            let variations = variations(font, css_size);
            hb::hb_font_set_variations(hb_font, variations.as_ptr(), variations.len() as u32);
            let scale = (css_size * device_scale * 65536.) as i32;
            hb::hb_font_set_scale(hb_font, scale, scale);
            // The bundled HarfBuzz also shifts glyphs by half the `trak` value; Chromium's
            // does not. Shape without `trak` and add its advance per cluster instead.
            let space = |ptem| {
                hb::hb_font_set_ptem(hb_font, ptem);
                shape_raw(hb_font, 0, " ", &[]).1
            };
            let tracking = space(css_size as f32) - space(0.);
            hb::hb_font_set_ptem(hb_font, 0.);
            Self(hb_font, tracking)
        }
    }

    /// Glyphs and the total advance, CSS px. `features` are OpenType (tag, value) pairs.
    /// `letter_spacing` (CSS px) goes after every cluster, and optional ligatures are off when
    /// it is set, as in Blink. Glyphs are (id, x, y down, byte cluster), CSS px relative to the
    /// run start.
    pub fn shape(
        &self,
        text: &str,
        device_scale: f32,
        letter_spacing: f32,
        features: &[(u32, u32)],
    ) -> (Vec<(u32, f32, f32, usize)>, f32) {
        let to_css = |v: i64| v as f32 / 65536. / device_scale;
        let spacing = (letter_spacing * device_scale * 65536.) as i64;
        let no_ligatures =
            [b"liga", b"clig", b"dlig", b"hlig", b"calt"].map(|tag| (u32::from_be_bytes(*tag), 0));
        let features: Vec<hb::hb_feature_t> = features
            .iter()
            .chain(if spacing != 0 { &no_ligatures[..] } else { &[] })
            .map(|&(tag, value)| hb::hb_feature_t {
                tag,
                value,
                start: 0,
                end: u32::MAX,
            })
            .collect();
        let (glyphs, width) = shape_raw(self.0, self.1 + spacing, text, &features);
        let glyphs = glyphs
            .into_iter()
            .map(|(id, x, y, cluster)| (id, to_css(x), -to_css(y), cluster))
            .collect();
        (glyphs, to_css(width))
    }
}

/// (glyph, x, y up, cluster) and the total advance in hb units, `trak` added per cluster.
fn shape_raw(
    font: *mut hb::hb_font_t,
    tracking: i64,
    text: &str,
    features: &[hb::hb_feature_t],
) -> (Vec<(u32, i64, i64, usize)>, i64) {
    unsafe {
        let buffer = hb::hb_buffer_create();
        hb::hb_buffer_add_utf8(
            buffer,
            text.as_ptr().cast(),
            text.len() as i32,
            0,
            text.len() as i32,
        );
        hb::hb_buffer_guess_segment_properties(buffer);
        hb::hb_shape(font, buffer, features.as_ptr(), features.len() as u32);
        let mut len = 0;
        let infos = hb::hb_buffer_get_glyph_infos(buffer, &mut len);
        let positions = hb::hb_buffer_get_glyph_positions(buffer, &mut len);
        let (infos, positions) = (
            std::slice::from_raw_parts(infos, len as usize),
            std::slice::from_raw_parts(positions, len as usize),
        );
        let mut pen = 0;
        let mut cluster = u32::MAX;
        let glyphs = infos
            .iter()
            .zip(positions)
            .map(|(info, pos)| {
                let glyph = (
                    info.codepoint,
                    pen + i64::from(pos.x_offset),
                    i64::from(pos.y_offset),
                    info.cluster as usize,
                );
                pen += i64::from(pos.x_advance);
                if info.cluster != cluster {
                    pen += tracking;
                    cluster = info.cluster;
                }
                glyph
            })
            .collect();
        hb::hb_buffer_destroy(buffer);
        (glyphs, pen)
    }
}

/// Skia `ctvariation_from_SkFontArguments`: every axis at its current value, `opsz` at the
/// CSS size.
fn variations(font: &CTFont, css_size: f64) -> Vec<hb::hb_variation_t> {
    let Some(axes) = font.get_variation_axes() else {
        return Vec::new();
    };
    let current = unsafe { CTFontCopyVariation(font.as_concrete_TypeRef()) };
    let current: Option<CFDictionary<CFNumber, CFNumber>> =
        (!current.is_null()).then(|| unsafe { CFDictionary::wrap_under_create_rule(current) });
    let number = |axis: &CFDictionary<CFString, CFType>, key: &'static str| {
        axis.find(CFString::from_static_string(key))
            .and_then(|v| v.downcast::<CFNumber>())
    };
    axes.iter()
        .filter_map(|axis| {
            let tag = number(&axis, "NSCTVariationAxisIdentifier")?;
            let tag_value = tag.to_i64()? as u32;
            let value = if tag_value == u32::from_be_bytes(*b"opsz") {
                css_size
            } else {
                current
                    .as_ref()
                    .and_then(|c| c.find(&tag).and_then(|v| v.to_f64()))
                    .or_else(|| number(&axis, "NSCTVariationAxisDefaultValue")?.to_f64())?
            };
            Some(hb::hb_variation_t {
                tag: tag_value,
                value: value as f32,
            })
        })
        .collect()
}

unsafe extern "C" {
    fn CTFontCopyVariation(font: CTFontRef) -> CFDictionaryRef;
}
