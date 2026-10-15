//! `SkiaMatchedTextSystem`: a CoreText `PlatformTextSystem` that picks, shapes and rasterizes
//! fonts the way Chromium on macOS does (Blink font matching, Skia `SkScalerContext_Mac`), so
//! GPUI text lands on the same pixels as the Codex desktop app's renderer.
//!
//! - Fonts: `system-ui` is `CTFontCreateUIFontForLanguage` + bold trait at >= 600 + `wght`
//!   variation (Blink `MatchSystemUIFont`); other families use CSS weight matching over the
//!   family members. Every size gets `opsz` = CSS size and no `trak` (Skia
//!   `SkCTFontCreateExactCopy`), so advances equal Chromium's.
//! - Glyph masks: `-webkit-font-smoothing: antialiased` = Skia A8, unhinted: CoreGraphics draws
//!   black on white without font smoothing at the device size, 4 horizontal subpixel offsets,
//!   no gamma pre-blend (Chromium-Mac does not define `SK_GAMMA_APPLY_TO_A8`).

use core_foundation::{
    attributed_string::CFMutableAttributedString,
    base::{CFRange, CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    base::{
        CGFloat, kCGBitmapByteOrder32Little, kCGImageAlphaNoneSkipFirst,
        kCGImageAlphaPremultipliedLast,
    },
    color_space::CGColorSpace,
    context::{CGContext, CGTextDrawingMode},
    geometry::{CGAffineTransform, CGPoint},
};
use core_text::{
    font::{
        CTFont, CTFontRef, kCTFontSystemFontType, new_from_descriptor, new_ui_font_for_language,
    },
    font_collection,
    font_descriptor::{
        CTFontDescriptor, CTFontDescriptorRef, TraitAccessors, kCTFontBoldTrait,
        kCTFontDefaultOrientation, kCTFontItalicTrait, kCTFontVariationAttribute,
        new_from_attributes,
    },
    font_manager,
    line::CTLine,
    string_attributes::kCTFontAttributeName,
};
use gpui::{
    Bounds, DevicePixels, Font, FontId, FontMetrics, FontRun, FontStyle, GlyphId, LineLayout,
    Pixels, PlatformTextSystem, RenderGlyphParams, Result, SUBPIXEL_VARIANTS_X, ShapedGlyph,
    ShapedRun, Size, TextRenderingMode, point, px, size,
};
use parking_lot::Mutex;

#[path = "shape.rs"]
mod shape;
use std::{borrow::Cow, collections::HashMap, ptr};

#[derive(Default)]
pub struct SkiaMatchedTextSystem(Mutex<State>);

impl SkiaMatchedTextSystem {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Default)]
struct State {
    faces: Vec<Face>,
    by_font: HashMap<Font, FontId>,
    by_postscript_name: HashMap<String, FontId>,
    /// Descriptors of `add_fonts` data, searched before installed families.
    added: Vec<CTFontDescriptor>,
    /// `(face, size bits)` -> font at that CSS size.
    sized: HashMap<(usize, u32), CTFont>,
    shapers: HashMap<(usize, u32), shape::HbFont>,
    /// Per face: CSS `letter-spacing` (from [`crate::letter_spacing`]) and the other
    /// `FontFeatures` as OpenType (tag, value) pairs for HarfBuzz.
    shaping: HashMap<usize, (f32, Vec<(u32, u32)>)>,
}

enum Face {
    System {
        weight: f32,
        italic: bool,
    },
    /// A matched family member or a CoreText fallback font, at any size.
    Fixed(CTFont),
}

/// `layout_line` gets no scale factor; Blink rounds line metrics in device px.
/// ponytail: fixed 2x (the reference displays); pass the window scale if 1x parity matters.
const DEVICE_SCALE: f64 = 2.;

/// GPUI rounds glyph origins half toward zero (x to 1/4 device px, y to 1 px), Skia rounds ties
/// up. Positions sit on a grid of 1/64 device px (layout) plus size/2048-em steps (advances),
/// so this bias (far below both) only moves exact ties up.
const TIE_BIAS: f32 = 1. / 4096.;

/// Sizes for size-independent queries (metrics, advances), scaled back to font units.
const METRICS_SIZE: f64 = 64.;

impl State {
    fn font(&mut self, id: FontId, size: f32) -> CTFont {
        let key = (id.0, size.to_bits());
        if let Some(font) = self.sized.get(&key) {
            return font.clone();
        }
        let size = f64::from(size);
        let base = match &self.faces[id.0] {
            Face::System { weight, italic } => system_font(*weight, *italic, size),
            Face::Fixed(font) => font.clone(),
        };
        let font = exact_copy(&base, size, size);
        self.sized.insert(key, font.clone());
        font
    }

    fn hb(&mut self, id: FontId, size: f32) -> &shape::HbFont {
        let font = self.font(id, size);
        self.shapers
            .entry((id.0, size.to_bits()))
            .or_insert_with(|| shape::HbFont::new(&font, f64::from(size), DEVICE_SCALE))
    }

    fn face_id(&mut self, face: Face, postscript_name: Option<String>) -> FontId {
        let id = FontId(self.faces.len());
        self.faces.push(face);
        if let Some(name) = postscript_name {
            self.by_postscript_name.insert(name, id);
        }
        id
    }

    fn match_family(&self, family: &str, weight: f32, italic: bool) -> Option<CTFont> {
        let added: Vec<CTFontDescriptor> = self
            .added
            .iter()
            .filter(|d| d.family_name() == family)
            .cloned()
            .collect();
        let members = if added.is_empty() {
            font_collection::create_for_family(family)?
                .get_descriptors()?
                .iter()
                .map(|d| d.clone())
                .collect()
        } else {
            added
        };
        let desired = weight.round() as i32;
        let mut best: Option<(&CTFontDescriptor, (bool, i32))> = None;
        for member in &members {
            let traits = member.traits();
            let slant_ok = (traits.normalized_slant() != 0.) == italic;
            let w = css_weight(traits.normalized_weight());
            if best.is_none_or(|(_, (best_slant, best_w))| {
                (slant_ok && !best_slant)
                    || (slant_ok == best_slant && better_weight(desired, best_w, w))
            }) {
                best = Some((member, (slant_ok, w)));
            }
        }
        Some(new_from_descriptor(best?.0, 16.))
    }
}

/// Blink `MatchSystemUIFont`.
fn system_font(weight: f32, italic: bool, size: f64) -> CTFont {
    let mut font = new_ui_font_for_language(kCTFontSystemFontType, size, None);
    let traits = if italic { kCTFontItalicTrait } else { 0 }
        | if weight >= 600. { kCTFontBoldTrait } else { 0 };
    if traits != 0 {
        font = font
            .clone_with_symbolic_traits(traits, traits)
            .unwrap_or(font);
    }
    if weight != 400. {
        let variation = CFDictionary::from_CFType_pairs(&[(
            CFNumber::from(i64::from(u32::from_be_bytes(*b"wght"))),
            CFNumber::from(f64::from(weight)),
        )]);
        let key = unsafe { CFString::wrap_under_get_rule(kCTFontVariationAttribute) };
        font = copy_with(&font, size, &[(key, variation.as_CFType())]);
    }
    font
}

/// `font` at `size` with optical size `opsz` (Skia `SkCTFontCreateExactCopy`; its no-`trak`
/// attribute is left out because Blink's advances include `trak`).
fn exact_copy(font: &CTFont, size: f64, opsz: f64) -> CTFont {
    let key = CFString::from_static_string("NSCTFontOpticalSizeAttribute");
    copy_with(font, size, &[(key, CFNumber::from(opsz).as_CFType())])
}

fn copy_with(font: &CTFont, size: f64, attributes: &[(CFString, CFType)]) -> CTFont {
    let descriptor = new_from_attributes(&CFDictionary::from_CFType_pairs(attributes));
    unsafe {
        CTFont::wrap_under_create_rule(CTFontCreateCopyWithAttributes(
            font.as_concrete_TypeRef(),
            size,
            ptr::null(),
            descriptor.as_concrete_TypeRef(),
        ))
    }
}

/// Blink `ToCSSFontWeight`.
fn css_weight(ct_weight: f64) -> i32 {
    const RANGES: [(f64, i32); 9] = [
        (-0.70, 100),
        (-0.45, 200),
        (-0.10, 300),
        (0.10, 400),
        (0.27, 500),
        (0.35, 600),
        (0.50, 700),
        (0.60, 800),
        (1.0, 900),
    ];
    RANGES
        .iter()
        .find(|(upper, _)| ct_weight <= *upper)
        .map_or(400, |&(_, w)| w)
}

/// Blink `BetterWeightMatch` (CSS Fonts 4 §5.2 weight search order).
fn better_weight(desired: i32, chosen: i32, candidate: i32) -> bool {
    let priority = |w: i32| -> (i32, i32) {
        if desired > 500 {
            if w >= desired {
                (0, w - desired)
            } else {
                (1, desired - w)
            }
        } else if desired < 400 {
            if w <= desired {
                (0, desired - w)
            } else {
                (1, w - desired)
            }
        } else if w >= desired && w <= 500 {
            (0, w - desired)
        } else if w < desired {
            (1, desired - w)
        } else {
            (2, w - desired)
        }
    };
    priority(candidate) < priority(chosen)
}

fn is_system_family(family: &str) -> bool {
    matches!(
        family,
        ".SystemUIFont"
            | ".AppleSystemUIFont"
            | "system-ui"
            | "-apple-system"
            | "BlinkMacSystemFont"
    )
}

/// Skia glyph bounds (`SkScalerContext_Mac::generateMetrics`): device px, y down, relative
/// to the integer glyph origin.
fn skia_bounds(font: &CTFont, glyph: u16, sub_x: f64) -> Bounds<DevicePixels> {
    let r = font.get_bounding_rects_for_glyphs(kCTFontDefaultOrientation, &[glyph]);
    if r.size.width <= 0. || r.size.height <= 0. {
        return Bounds::default();
    }
    let (left, top) = (r.origin.x.floor(), (-r.origin.y - r.size.height).floor());
    let (right, bottom) = (
        (r.origin.x + r.size.width + sub_x).ceil(),
        (-r.origin.y).ceil(),
    );
    Bounds {
        origin: point(DevicePixels(left as i32 - 1), DevicePixels(top as i32 - 1)),
        size: size(
            DevicePixels((right - left) as i32 + 2),
            DevicePixels((bottom - top) as i32 + 2),
        ),
    }
}

impl SkiaMatchedTextSystem {
    /// The glyph's font at device size (Skia scaler context font).
    fn device_font(&self, params: &RenderGlyphParams) -> CTFont {
        let css = f32::from(params.font_size);
        let font = self.0.lock().font(params.font_id, css);
        let css = f64::from(css);
        exact_copy(&font, css * f64::from(params.scale_factor), css)
    }

    fn sub_x(params: &RenderGlyphParams) -> f64 {
        f64::from(params.subpixel_variant.x) / f64::from(SUBPIXEL_VARIANTS_X)
    }
}

impl PlatformTextSystem for SkiaMatchedTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let mut state = self.0.lock();
        for bytes in fonts {
            let descriptor = font_manager::create_font_descriptor(&bytes)
                .map_err(|()| anyhow::anyhow!("invalid font data"))?;
            state.added.push(descriptor);
        }
        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names: Vec<String> = font_manager::copy_available_font_family_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        names.extend(self.0.lock().added.iter().map(|d| d.family_name()));
        names.sort();
        names.dedup();
        names
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        let mut state = self.0.lock();
        if let Some(&id) = state.by_font.get(font) {
            return Ok(id);
        }
        let italic = font.style != FontStyle::Normal;
        let id = if is_system_family(&font.family) {
            let face = Face::System {
                weight: font.weight.0,
                italic,
            };
            state.face_id(face, None)
        } else {
            let matched = state
                .match_family(&font.family, font.weight.0, italic)
                .ok_or_else(|| anyhow::anyhow!("no font family {:?}", font.family))?;
            let name = matched.postscript_name();
            state.face_id(Face::Fixed(matched), Some(name))
        };
        let mut shaping = (0., Vec::new());
        for (tag, value) in font.features.tag_value_list() {
            if tag == crate::LETTER_SPACING_TAG {
                shaping.0 = *value as i32 as f32 / 100.;
            } else if let Ok(tag) = <[u8; 4]>::try_from(tag.as_bytes()) {
                shaping.1.push((u32::from_be_bytes(tag), *value));
            }
        }
        state.shaping.insert(id.0, shaping);
        state.by_font.insert(font.clone(), id);
        Ok(id)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        let font = self.0.lock().font(font_id, METRICS_SIZE as f32);
        let upem = font.units_per_em();
        let s = upem as f64 / METRICS_SIZE;
        let b = font.bounding_box();
        let u = |v: CGFloat| (v * s) as f32;
        FontMetrics {
            units_per_em: upem,
            ascent: u(font.ascent()),
            descent: -u(font.descent()),
            line_gap: u(font.leading()),
            underline_position: u(font.underline_position()),
            underline_thickness: u(font.underline_thickness()),
            cap_height: u(font.cap_height()),
            x_height: u(font.x_height()),
            bounding_box: Bounds {
                origin: point(u(b.origin.x), u(b.origin.y)),
                size: size(u(b.size.width), u(b.size.height)),
            },
        }
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let font = self.0.lock().font(font_id, METRICS_SIZE as f32);
        let s = font.units_per_em() as f64 / METRICS_SIZE;
        let r = font.get_bounding_rects_for_glyphs(kCTFontDefaultOrientation, &[glyph_id.0 as u16]);
        Ok(Bounds {
            origin: point((r.origin.x * s) as f32, (r.origin.y * s) as f32),
            size: size((r.size.width * s) as f32, (r.size.height * s) as f32),
        })
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        let font = self.0.lock().font(font_id, METRICS_SIZE as f32);
        let s = font.units_per_em() as f64 / METRICS_SIZE;
        let mut advance = core_graphics::geometry::CGSize::new(0., 0.);
        unsafe {
            font.get_advances_for_glyphs(
                kCTFontDefaultOrientation,
                &(glyph_id.0 as u16),
                &mut advance,
                1,
            );
        }
        Ok(size(
            (advance.width * s) as f32,
            (advance.height * s) as f32,
        ))
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let font = self.0.lock().font(font_id, METRICS_SIZE as f32);
        let mut utf16 = [0u16; 2];
        let units = ch.encode_utf16(&mut utf16).len();
        let mut glyphs = [0u16; 2];
        let found = unsafe {
            font.get_glyphs_for_characters(utf16.as_ptr(), glyphs.as_mut_ptr(), units as _)
        };
        (found && glyphs[0] != 0).then(|| GlyphId(u32::from(glyphs[0])))
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        Ok(skia_bounds(
            &self.device_font(params),
            params.glyph_id.0 as u16,
            Self::sub_x(params),
        ))
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        let (w, h) = (bounds.size.width.0 as usize, bounds.size.height.0 as usize);
        anyhow::ensure!(w > 0 && h > 0, "glyph bounds are empty");
        let font = self.device_font(params);
        let glyph = params.glyph_id.0 as u16;
        let sub_x = Self::sub_x(params);
        // `SkScalerContext_Mac::Offscreen::getCG`: origin bottom-left, glyph origin at
        // (-left + subX, top + height).
        let origin = CGPoint::new(
            -f64::from(bounds.origin.x.0) + sub_x,
            f64::from(bounds.origin.y.0 + bounds.size.height.0),
        );
        let rgb = CGColorSpace::create_device_rgb();
        let mut pixels;
        let cx;
        if params.is_emoji {
            pixels = vec![0u8; w * h * 4];
            cx = CGContext::create_bitmap_context(
                Some(pixels.as_mut_ptr().cast()),
                w,
                h,
                8,
                w * 4,
                &rgb,
                kCGImageAlphaPremultipliedLast,
            );
        } else {
            pixels = vec![0xFFu8; w * h * 4];
            cx = CGContext::create_bitmap_context(
                Some(pixels.as_mut_ptr().cast()),
                w,
                h,
                8,
                w * 4,
                &rgb,
                kCGImageAlphaNoneSkipFirst | kCGBitmapByteOrder32Little,
            );
            cx.set_gray_fill_color(0., 1.);
        }
        cx.set_allows_font_subpixel_quantization(false);
        cx.set_should_subpixel_quantize_fonts(false);
        cx.set_allows_font_subpixel_positioning(true);
        cx.set_should_subpixel_position_fonts(true);
        cx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
        cx.set_should_antialias(true);
        cx.set_should_smooth_fonts(false);
        cx.set_text_matrix(&CGAffineTransform::new(1., 0., 0., 1., 0., 0.));
        font.draw_glyphs(&[glyph], &[origin], cx);
        if params.is_emoji {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                // Premultiplied, as Skia blends it (PolychromeSprite::skia).
                pixel.swap(0, 2);
            }
            return Ok((bounds.size, pixels));
        }
        // Little-endian xRGB: bytes are B, G, R, x. Gray, so coverage = 255 - B.
        let mask = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| 255 - p[0])
            .collect();
        Ok((bounds.size, mask))
    }

    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        let mut state = self.0.lock();
        let css = f32::from(font_size);
        let mut string = CFMutableAttributedString::new();
        // (utf16 end, requested font id, postscript name of the font set on the range)
        let mut requested = Vec::with_capacity(runs.len());
        let (mut ascent, mut descent) = (0f32, 0f32);
        let mut rest = text;
        for (i, run) in runs.iter().enumerate() {
            let (run_text, tail) = rest.split_at(run.len);
            rest = tail;
            let start = string.char_len();
            string.replace_str(&CFString::new(run_text), CFRange::init(start, 0));
            let end = string.char_len();
            let font = state.font(run.font_id, css);
            // Blink lays out in device px (zoom for DSF) and rounds the primary font's
            // ascent and descent there.
            let device = |v: CGFloat| (v * DEVICE_SCALE).round();
            let (a, d) = (device(font.ascent()), device(font.descent()));
            ascent = ascent.max((a / DEVICE_SCALE) as f32);
            descent = descent.max((d / DEVICE_SCALE) as f32);
            // Alternate a 1-ulp size change so CoreText never forms ligatures across runs.
            let font = if i % 2 == 0 {
                exact_copy(&font, f64::from(css.next_up()), f64::from(css))
            } else {
                font
            };
            requested.push((end, run.font_id, font.postscript_name()));
            unsafe {
                string.set_attribute(
                    CFRange::init(start, end - start),
                    kCTFontAttributeName,
                    &font,
                );
            }
        }

        let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
        let mut shaped: Vec<ShapedRun> = Vec::new();
        let utf8_offsets: Vec<usize> = text
            .char_indices()
            .flat_map(|(i, c)| std::iter::repeat_n(i, c.len_utf16()))
            .chain([text.len()])
            .collect();
        // Glyphs and advances of the requested fonts come from HarfBuzz (Blink's shaper);
        // CoreText only itemizes and shapes fallback fonts.
        let mut pen = 0f32;
        for run in line.glyph_runs().iter() {
            let font = unsafe {
                run.attributes()
                    .and_then(|a| a.find(kCTFontAttributeName).map(|f| f.downcast::<CTFont>()))
                    .flatten()
            };
            let Some(mut font) = font else { continue };
            // Blink falls back to "Apple Color Emoji" for emoji, not CoreText's UI variant
            // (whose advances are wider), and shapes it with HarfBuzz.
            let is_emoji = match font.postscript_name().as_str() {
                ".AppleColorEmojiUI" => {
                    font = core_text::font::new_from_name("AppleColorEmoji", font.pt_size())
                        .unwrap_or(font);
                    true
                }
                name => name == "AppleColorEmoji",
            };
            let range = unsafe { CTRunGetStringRange(run.as_concrete_TypeRef()) };
            let utf8 = |i: isize| utf8_offsets[(i as usize).min(utf8_offsets.len() - 1)];
            let (start, end) = (utf8(range.location), utf8(range.location + range.length));
            let name = font.postscript_name();
            // The CSS run covering this one: letter-spacing and font features are the
            // element's, so fallback fonts (emoji) take them too.
            let covering = requested.iter().find(|(end, ..)| range.location < *end);
            let requested_id = covering
                .filter(|(_, _, requested_name)| *requested_name == name)
                .map(|(_, id, _)| *id);
            let font_id =
                requested_id.unwrap_or_else(|| match state.by_postscript_name.get(&name) {
                    Some(&id) => id,
                    None => state.face_id(Face::Fixed(font.clone()), Some(name.clone())),
                });
            let mut glyphs = Vec::new();
            if requested_id.is_some() || is_emoji {
                let (spacing, features) = covering
                    .and_then(|(_, id, _)| state.shaping.get(&id.0).cloned())
                    .unwrap_or_default();
                let (hb_glyphs, width) = state.hb(font_id, css).shape(
                    &text[start..end],
                    DEVICE_SCALE as f32,
                    spacing,
                    &features,
                );
                glyphs.extend(
                    hb_glyphs
                        .into_iter()
                        .map(|(id, x, y, cluster)| (id, pen + x, y, start + cluster)),
                );
                pen += width;
            } else {
                let positions = run.positions();
                let x0 = positions.first().map_or(0., |p| p.x);
                for ((&id, p), &index) in run
                    .glyphs()
                    .iter()
                    .zip(positions.iter())
                    .zip(run.string_indices().iter())
                {
                    glyphs.push((
                        u32::from(id),
                        pen + (p.x - x0) as f32,
                        -p.y as f32,
                        utf8(index),
                    ));
                }
                pen += run.get_typographic_bounds().width as f32;
            }
            let out = match shaped.last_mut() {
                Some(last) if last.font_id == font_id => &mut last.glyphs,
                _ => {
                    shaped.push(ShapedRun {
                        font_id,
                        glyphs: Vec::new(),
                    });
                    &mut shaped.last_mut().unwrap().glyphs
                }
            };
            out.extend(glyphs.into_iter().map(|(id, x, y, index)| ShapedGlyph {
                id: GlyphId(id),
                position: point(px(x + TIE_BIAS), px(y + TIE_BIAS)),
                index,
                is_emoji,
            }));
        }
        LineLayout {
            font_size,
            width: px(pen),
            ascent: px(ascent),
            descent: px(descent),
            runs: shaped,
            len: text.len(),
        }
    }

    fn recommended_rendering_mode(&self, _: FontId, _: Pixels) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}

unsafe extern "C" {
    fn CTRunGetStringRange(run: core_text::run::CTRunRef) -> CFRange;
    fn CTFontCreateCopyWithAttributes(
        font: CTFontRef,
        size: CGFloat,
        matrix: *const CGAffineTransform,
        attributes: CTFontDescriptorRef,
    ) -> CTFontRef;
}
