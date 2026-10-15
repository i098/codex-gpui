//! Pixel-parity harness: renders registered GPUI cases headless at the size of each
//! reference screenshot, diffs them against the ref and writes a JSON + HTML report.
//!
//! Gate (plan Choice 7): exact RGBA compare. Pixels outside text boxes must all match;
//! inside text boxes (spec.json `texts[].rect`, inflated 1 device px) at most
//! `MAX_TEXT_DIFF_RATIO` may differ.

use anyhow::{Context as _, Result, ensure};
use gpui::{
    AnyView, App, AppContext, AssetSource, HeadlessAppContext, IntoElement, Render, Window, px,
    size,
};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{fmt::Write as _, fs, path::Path, sync::Arc};

pub const MAX_TEXT_DIFF_RATIO: f64 = 0.005;

/// One renderable app state, matched to `refs/<screen>/<theme>@<W>x<H>.png`.
pub struct Case {
    pub screen: &'static str,
    /// `"dark"` or `"light"`.
    pub theme: &'static str,
    pub build: fn(&mut Window, &mut App) -> AnyView,
}

#[derive(Serialize, Default, Debug)]
pub struct Counts {
    pub total_px: u64,
    pub diff_px_outside_text: u64,
    pub text_px: u64,
    pub diff_px_inside_text: u64,
    pub text_diff_ratio: f64,
    pub diff_px_tol2_outside_text: u64,
    pub max_channel_delta: u8,
}

#[derive(Serialize, Debug)]
pub struct Metrics {
    pub screen: String,
    pub theme: String,
    pub width: u32,
    pub height: u32,
    #[serde(flatten)]
    pub counts: Counts,
    pub pass: bool,
    /// Image paths relative to the report dir.
    pub ref_png: String,
    pub port_png: String,
    pub diff_png: String,
}

#[derive(Deserialize)]
struct Spec {
    viewport: [f64; 3],
    texts: Vec<SpecText>,
}

#[derive(Deserialize)]
struct SpecText {
    rect: [f64; 4],
}

struct Root(AnyView);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

fn headless(assets: Arc<dyn AssetSource>) -> HeadlessAppContext {
    let text_system = textsys::text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, assets, || {
        gpui_platform::current_headless_renderer()
    });
    cx.update(theme::load_fonts);
    cx
}

fn render_in(cx: &mut HeadlessAppContext, case: &Case, w: u32, h: u32) -> Result<RgbaImage> {
    cx.update(|cx| {
        cx.set_global(match case.theme {
            "light" => theme::Theme::light(),
            _ => theme::Theme::dark(),
        })
    });
    let window = cx.open_window(size(px(w as f32), px(h as f32)), |window, cx| {
        let view = (case.build)(window, cx);
        cx.new(|_| Root(view))
    })?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(image)
}

/// Renders one case at `w` x `h` logical px (the headless window is 2x).
pub fn render(case: &Case, assets: Arc<dyn AssetSource>, w: u32, h: u32) -> Result<RgbaImage> {
    render_in(&mut headless(assets), case, w, h)
}

/// `"<theme>@<W>x<H>.png"` -> `(W, H)`.
fn ref_size(file_name: &str, theme: &str) -> Option<(u32, u32)> {
    let (w, h) = file_name
        .strip_prefix(theme)?
        .strip_prefix('@')?
        .strip_suffix(".png")?
        .split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn text_mask(spec: &Spec, w: u32, h: u32) -> Vec<bool> {
    let dpr = spec.viewport[2];
    let mut mask = vec![false; (w * h) as usize];
    let clamp = |v: f64, max: u32| v.clamp(0., max as f64) as u32;
    for text in &spec.texts {
        let [x, y, tw, th] = text.rect;
        let (x0, x1) = (
            clamp((x * dpr).floor() - 1., w),
            clamp(((x + tw) * dpr).ceil() + 1., w),
        );
        let (y0, y1) = (
            clamp((y * dpr).floor() - 1., h),
            clamp(((y + th) * dpr).ceil() + 1., h),
        );
        for row in y0..y1 {
            mask[(row * w + x0) as usize..(row * w + x1) as usize].fill(true);
        }
    }
    mask
}

/// Compares two equal-size images. Diff image: ref as 30% gray, differing pixels magenta
/// outside text boxes and yellow inside.
fn compare(reference: &RgbaImage, port: &RgbaImage, mask: &[bool]) -> (Counts, RgbaImage) {
    let (w, h) = reference.dimensions();
    let mut c = Counts {
        total_px: u64::from(w) * u64::from(h),
        ..Counts::default()
    };
    let mut diff = RgbaImage::new(w, h);
    let pixels = reference.pixels().zip(port.pixels()).zip(mask);
    for (((r, p), &inside), d) in pixels.zip(diff.pixels_mut()) {
        c.text_px += u64::from(inside);
        // Exact byte compare; both-opaque pixels have equal alpha, so alpha only counts
        // when one side is translucent.
        let delta = (0..4).map(|i| r[i].abs_diff(p[i])).max().unwrap_or(0);
        c.max_channel_delta = c.max_channel_delta.max(delta);
        *d = if delta == 0 {
            let luma = 0.299 * f32::from(r[0]) + 0.587 * f32::from(r[1]) + 0.114 * f32::from(r[2]);
            let gray = (luma * 0.3).round() as u8;
            Rgba([gray, gray, gray, 255])
        } else if inside {
            c.diff_px_inside_text += 1;
            Rgba([255, 255, 0, 255])
        } else {
            c.diff_px_outside_text += 1;
            c.diff_px_tol2_outside_text += u64::from(delta > 2);
            Rgba([255, 0, 255, 255])
        };
    }
    if c.text_px > 0 {
        c.text_diff_ratio = c.diff_px_inside_text as f64 / c.text_px as f64;
    }
    (c, diff)
}

/// Renders every case against each matching ref under `refs` and writes
/// `<out>/<screen>/<theme>@<W>x<H>.{ref,port,diff}.png`, `report.json` and `report.html`.
pub fn run(
    cases: &[Case],
    assets: Arc<dyn AssetSource>,
    refs: &Path,
    out: &Path,
    screen: Option<&str>,
) -> Result<Vec<Metrics>> {
    let mut cx = headless(assets);
    let mut report = Vec::new();
    for case in cases
        .iter()
        .filter(|c| screen.is_none_or(|s| s == c.screen))
    {
        let dir = refs.join(case.screen);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut sizes: Vec<_> = entries
            .filter_map(|e| ref_size(e.ok()?.file_name().to_str()?, case.theme))
            .collect();
        sizes.sort();
        fs::create_dir_all(out.join(case.screen))?;
        for (w, h) in sizes {
            let stem = format!("{}/{}@{w}x{h}", case.screen, case.theme);
            let ref_path = refs.join(format!("{stem}.png"));
            let reference = image::open(&ref_path)
                .with_context(|| format!("reading {}", ref_path.display()))?
                .into_rgba8();
            let spec_path = refs.join(format!("{stem}.spec.json"));
            let spec: Spec = serde_json::from_slice(
                &fs::read(&spec_path)
                    .with_context(|| format!("reading {}", spec_path.display()))?,
            )
            .with_context(|| format!("parsing {}", spec_path.display()))?;
            let port = render_in(&mut cx, case, w, h)?;
            ensure!(
                port.dimensions() == reference.dimensions(),
                "{stem}: port renders {:?} px, ref is {:?} px",
                port.dimensions(),
                reference.dimensions()
            );
            let (rw, rh) = reference.dimensions();
            let (counts, diff) = compare(&reference, &port, &text_mask(&spec, rw, rh));
            let m = Metrics {
                screen: case.screen.into(),
                theme: case.theme.into(),
                width: w,
                height: h,
                pass: counts.diff_px_outside_text == 0
                    && counts.text_diff_ratio <= MAX_TEXT_DIFF_RATIO,
                counts,
                ref_png: format!("{stem}.ref.png"),
                port_png: format!("{stem}.port.png"),
                diff_png: format!("{stem}.diff.png"),
            };
            fs::copy(&ref_path, out.join(&m.ref_png))?;
            port.save(out.join(&m.port_png))?;
            diff.save(out.join(&m.diff_png))?;
            println!(
                "{} {} {w}x{h} outside={} text={:.3}% {}",
                m.screen,
                m.theme,
                m.counts.diff_px_outside_text,
                m.counts.text_diff_ratio * 100.,
                if m.pass { "PASS" } else { "FAIL" }
            );
            report.push(m);
        }
    }
    fs::create_dir_all(out)?;
    fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    fs::write(out.join("report.html"), html(&report))?;
    Ok(report)
}

fn html(report: &[Metrics]) -> String {
    let mut s = String::from(
        "<!doctype html><meta charset=utf-8><title>codex-gpui parity</title><style>\
         body{font:13px system-ui;background:#1b1b1b;color:#ddd;margin:16px}\
         td{padding:1px 10px 1px 0}.PASS{color:#4c4}.FAIL{color:#e55}\
         .imgs{display:flex;gap:8px}.imgs figure{flex:1;margin:0}img{width:100%}</style>\
         <h1>codex-gpui parity</h1>",
    );
    for m in report {
        let c = &m.counts;
        let verdict = if m.pass { "PASS" } else { "FAIL" };
        let _ = write!(
            s,
            "<section><h2>{} {} {}x{} <span class={verdict}>{verdict}</span></h2><table>\
             <tr><td>total_px<td>{}<tr><td>diff_px_outside_text (gate 0)<td>{}\
             <tr><td>text_px<td>{}<tr><td>diff_px_inside_text<td>{}\
             <tr><td>text_diff_ratio (gate &le; {MAX_TEXT_DIFF_RATIO})<td>{:.5}\
             <tr><td>diff_px_tol2_outside_text<td>{}<tr><td>max_channel_delta<td>{}</table>\
             <div class=imgs><figure><figcaption>ref</figcaption><a href=\"{r}\"><img src=\"{r}\"></a></figure>\
             <figure><figcaption>port</figcaption><a href=\"{p}\"><img src=\"{p}\"></a></figure>\
             <figure><figcaption>diff</figcaption><a href=\"{d}\"><img src=\"{d}\"></a></figure></div></section>",
            m.screen,
            m.theme,
            m.width,
            m.height,
            c.total_px,
            c.diff_px_outside_text,
            c.text_px,
            c.diff_px_inside_text,
            c.text_diff_ratio,
            c.diff_px_tol2_outside_text,
            c.max_channel_delta,
            r = m.ref_png,
            p = m.port_png,
            d = m.diff_png,
        );
    }
    s
}
