//! Text-rasterization lab. `measure page <dir>` writes `<dir>/page.html` (one text run per row).
//! `tools/text/measure.sh` renders it with chrome-headless-shell into `chrome.png` + `dom.json`,
//! then `measure compare <dir>` paints the same runs in GPUI (stock AppleTextSystem = before,
//! `textsys::text_system()` = after) as GPUI text elements and diffs inside the text boxes.

use anyhow::{Context as _, Result, bail};
use gpui::{
    App, AppContext, Bounds, Corners, Font, FontFeatures, FontStyle, FontWeight,
    HeadlessAppContext, IntoElement, ParentElement, PlatformTextSystem, Render, Rgba, SharedString,
    Styled, TextAlign, TextRun, Window, canvas, div, fill, point, px, size,
};
use image::RgbaImage;
use std::{cell::RefCell, fmt::Write as _, fs, path::Path, rc::Rc, sync::Arc};

const W: f32 = 900.;
const SANS: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";
const MONO: &str = "ui-monospace, SFMono-Regular, 'SF Mono', menlo, monospace";

struct Run {
    mono: bool,
    size: f32,
    weight: f32,
    color: Rgba,
    bg: [u8; 3],
    text: &'static str,
    /// CSS `letter-spacing`, px.
    spacing: f32,
    /// CSS `font-feature-settings`.
    features: &'static [(&'static str, u32)],
}

const SAMPLES: [&str; 4] = [
    "Skip to content New thread Automations 0123456789",
    "Fix failing tests in the parser module, 12m ago",
    "Ask Codex anything. Use @ to add files; gpt-5-codex High",
    "Settings General Appearance Keyboard shortcuts Ww Qq",
];
const MONO_SAMPLE: &str = "fn main() { let x = 42; } // diff +12 -3 [ok]";

/// A wrapped paragraph with a fractional line height (14px/22.75px, 8 lines at 300px).
const PARAGRAPH: &str = "Codex read the failing test output, traced the panic to an off-by-one \
    in the tokenizer, and patched the boundary check. It then reran the parser suite, updated \
    two snapshots that depended on the old behavior, and summarized the change with links to \
    every touched file so you can review it before merging.";
const PARAGRAPH_WIDTH: f32 = 300.;
const PARAGRAPH_HEIGHT: f32 = 8. * 22.75 + 16.;

/// Top of the paragraph block (below the runs).
fn paragraph_top(rows: &[(f32, f32, f32, f32)]) -> f32 {
    rows.last().map_or(0., |r| r.0 + r.1)
}

/// Layer runs, below the paragraph; the whole row is diffed. Chromium blits the content of a
/// CSS `opacity` into a transparent layer and blends that onto the tile (`saveLayerAlpha`),
/// and rasterizes a composited layer (here `will-change: transform`; the app's sidebar is one)
/// into a transparent tile that the compositor blends onto the page.
struct LayerRun {
    name: &'static str,
    bg: u8,
    /// A `rgba(24, 24, 24, .65)` box holds the content; true: a composited layer.
    boxed: Option<bool>,
    opacity: bool,
    /// A rounded fill (radius 8) under the text.
    fill: Option<Rgba>,
    /// Text, size, weight, alpha (white).
    text: Option<(&'static str, f32, f32, f32)>,
}

const LAYER_ROW: f32 = 40.;

fn layer_runs() -> [LayerRun; 7] {
    let white = |a| rgb(255, 255, 255, a);
    let no_projects = Some(("No projects", 14., 430., 0.498));
    let recents = Some(("Recents", 14., 500., 0.498));
    [
        LayerRun {
            name: "opacity .5 text .498",
            bg: 29,
            boxed: None,
            opacity: true,
            fill: None,
            text: no_projects,
        },
        LayerRun {
            name: "opacity .5 text",
            bg: 24,
            boxed: None,
            opacity: true,
            fill: None,
            text: Some((SAMPLES[1], 13., 400., 1.)),
        },
        LayerRun {
            name: "opacity .5 rounded fill",
            bg: 24,
            boxed: None,
            opacity: true,
            fill: Some(white(1.)),
            text: None,
        },
        LayerRun {
            name: "translucent box, text",
            bg: 36,
            boxed: Some(false),
            opacity: false,
            fill: None,
            text: recents,
        },
        LayerRun {
            name: "composited translucent box, text",
            bg: 36,
            boxed: Some(true),
            opacity: false,
            fill: None,
            text: recents,
        },
        LayerRun {
            name: "composited translucent box, opacity .5 text",
            bg: 36,
            boxed: Some(true),
            opacity: true,
            fill: None,
            text: no_projects,
        },
        LayerRun {
            name: "composited translucent box, fill .1, text",
            bg: 36,
            boxed: Some(true),
            opacity: false,
            fill: Some(white(0.1)),
            text: Some((SAMPLES[2], 13., 400., 1.)),
        },
    ]
}

/// Box, fill and text geometry of a layer run (CSS px): box in the row, content in the box.
const LAYER_BOX: [f32; 4] = [4., 2., 600., 36.];
const LAYER_FILL: [f32; 4] = [6., 6., 120., 24.];
const LAYER_TEXT: (f32, f32) = (12.25, 7.5);

fn layers_top(rows: &[(f32, f32, f32, f32)]) -> f32 {
    paragraph_top(rows) + PARAGRAPH_HEIGHT
}

fn rgb(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    let c = |v: u8| f32::from(v) / 255.;
    Rgba {
        r: c(r),
        g: c(g),
        b: c(b),
        a,
    }
}

fn runs() -> Vec<Run> {
    let fonts = [
        (13., 400.),
        (12., 430.),
        (13., 430.),
        (14., 430.),
        (14., 400.),
        (13., 500.),
        (14., 500.),
        (16., 430.),
        (16., 400.),
        (18., 600.),
        (28., 400.),
        (28., 430.),
    ];
    let mut out = Vec::new();
    let mut n = 0;
    let mut text = || {
        n += 1;
        SAMPLES[n % SAMPLES.len()]
    };
    for dark in [true, false] {
        let (bg, bg2, fg) = if dark {
            ([24, 24, 24], [40, 40, 40], (255, 255, 255))
        } else {
            ([255, 255, 255], [243, 243, 243], (26, 28, 31))
        };
        let primary = rgb(fg.0, fg.1, fg.2, 1.);
        for (size, weight) in fonts {
            let text = text();
            out.push(Run {
                mono: false,
                size,
                weight,
                color: primary,
                bg,
                text,
                spacing: 0.,
                features: &[],
            });
        }
        let alpha = |a| rgb(fg.0, fg.1, fg.2, a);
        let accents = if dark {
            [
                rgb(86, 133, 209, 1.),
                rgb(250, 66, 62, 1.),
                rgb(64, 201, 119, 1.),
            ]
        } else {
            [
                rgb(40, 88, 164, 1.),
                rgb(186, 38, 35, 1.),
                rgb(0, 162, 64, 1.),
            ]
        };
        let a2 = if dark { 0.498 } else { 0.494 };
        for color in [alpha(a2), alpha(0.65), alpha(0.85), alpha(0.6)]
            .into_iter()
            .chain(accents)
        {
            for (size, weight) in [(14., 430.), (13., 400.)] {
                let text = text();
                out.push(Run {
                    mono: false,
                    size,
                    weight,
                    color,
                    bg,
                    text,
                    spacing: 0.,
                    features: &[],
                });
            }
        }
        for color in [primary, alpha(a2)] {
            let text = text();
            out.push(Run {
                mono: false,
                size: 14.,
                weight: 430.,
                color,
                bg: bg2,
                text,
                spacing: 0.,
                features: &[],
            });
        }
        let (ibg, ifg) = if dark {
            ([255, 255, 255], rgb(45, 45, 45, 1.))
        } else {
            ([13, 13, 13], rgb(255, 255, 255, 1.))
        };
        let text = text();
        out.push(Run {
            mono: false,
            size: 14.,
            weight: 500.,
            color: ifg,
            bg: ibg,
            text,
            spacing: 0.,
            features: &[],
        });
        for (size, weight) in [(12., 430.), (12.88, 430.), (13., 400.)] {
            out.push(Run {
                mono: true,
                size,
                weight,
                color: primary,
                bg,
                text: MONO_SAMPLE,
                spacing: 0.,
                features: &[],
            });
        }
    }
    // CSS letter-spacing as refs/automations uses it.
    for (size, weight, spacing) in [(18., 600., -0.45), (16., 430., -0.32), (14., 430., -0.14)] {
        for (fg, bg) in [
            ((255, 255, 255), [24, 24, 24]),
            ((26, 28, 31), [255, 255, 255]),
        ] {
            out.push(Run {
                mono: false,
                size,
                weight,
                color: rgb(fg.0, fg.1, fg.2, 1.),
                bg,
                text: SAMPLES[0],
                spacing,
                features: &[],
            });
        }
    }
    // Diff stats: tabular, alternate digits.
    for (fg, bg) in [
        ((255, 255, 255), [24, 24, 24]),
        ((26, 28, 31), [255, 255, 255]),
    ] {
        out.push(Run {
            mono: false,
            size: 16.,
            weight: 430.,
            color: rgb(fg.0, fg.1, fg.2, 1.),
            bg,
            text: "+1234 -567 0123456789 1,111",
            spacing: 0.,
            features: &[("tnum", 1), ("cv01", 1), ("cv02", 1)],
        });
    }
    // Color emoji (Apple Color Emoji bitmaps) as refs/automations: 28 px cards, 48 px hero with
    // `tracking-tight` (-0.025em), at two x phases each.
    for (size, weight, spacing, text) in [
        (28., 430., 0., "🥗"),
        (28., 430., 0., "😴💰"),
        (48., 400., -1.2, "🕗"),
        (48., 400., -1.2, "🕗"),
    ] {
        out.push(Run {
            mono: false,
            size,
            weight,
            color: rgb(255, 255, 255, 1.),
            bg: [24, 24, 24],
            text,
            spacing,
            features: &[],
        });
    }
    out
}

fn line_height(run: &Run) -> f32 {
    (run.size * 1.5).ceil()
}

/// Row layout shared by the page and the GPUI render: (row top, row height, text left, text top).
/// Text origins step through fractional CSS offsets to hit every subpixel/baseline phase.
fn rows(runs: &[Run]) -> Vec<(f32, f32, f32, f32)> {
    let mut y = 0.;
    runs.iter()
        .enumerate()
        .map(|(i, run)| {
            let h = line_height(run) + 8.;
            let row = (
                y,
                h,
                10. + (i % 8) as f32 * 0.125,
                y + 4. + (i % 4) as f32 * 0.25,
            );
            y += h;
            row
        })
        .collect()
}

fn css_color(c: Rgba) -> String {
    let v = |x: f32| (x * 255.).round();
    format!("rgba({}, {}, {}, {})", v(c.r), v(c.g), v(c.b), c.a)
}

fn page(dir: &Path) -> Result<()> {
    let runs = runs();
    let rows = rows(&runs);
    let height = (layers_top(&rows) + layer_runs().len() as f32 * LAYER_ROW).ceil();
    let mut html = format!(
        "<!doctype html><html><head><meta charset=utf-8><style>\
         html{{-webkit-font-smoothing:antialiased}}\
         body{{margin:0;width:{W}px;height:{height}px;position:relative;overflow:hidden}}\
         .r{{position:absolute;left:0;width:{W}px}}.t{{position:absolute;white-space:nowrap}}\
         #m{{display:none}}</style></head><body>"
    );
    for (run, &(top, h, x, y)) in runs.iter().zip(&rows) {
        let [r, g, b] = run.bg;
        write!(
            html,
            "<div class=r style=\"top:{top}px;height:{h}px;background:rgb({r},{g},{b})\"></div>\
             <span class=t style=\"left:{x}px;top:{y}px;font:{} {}px/{}px {};color:{};letter-spacing:{}px;font-feature-settings:{}\">\
             {}</span>",
            run.weight,
            run.size,
            line_height(run),
            if run.mono { MONO } else { SANS },
            css_color(run.color),
            run.spacing,
            if run.features.is_empty() {
                "normal".to_string()
            } else {
                run.features
                    .iter()
                    .map(|(t, v)| format!("'{t}' {v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            },
            run.text
        )?;
    }
    let top = paragraph_top(&rows);
    write!(
        html,
        "<div class=r style=\"top:{top}px;height:{PARAGRAPH_HEIGHT}px;background:rgb(24,24,24)\"></div>\
         <span class=t style=\"left:10px;top:{}px;width:{PARAGRAPH_WIDTH}px;white-space:normal;\
         font:430 14px/22.75px {SANS};color:#fff\">{PARAGRAPH}</span>",
        top + 8.
    )?;
    for (j, run) in layer_runs().iter().enumerate() {
        let top = layers_top(&rows) + j as f32 * LAYER_ROW;
        let g = run.bg;
        write!(
            html,
            "<div class=r style=\"top:{top}px;height:{LAYER_ROW}px;background:rgb({g},{g},{g})\"></div>"
        )?;
        let mut inner = String::new();
        if let Some(c) = run.fill {
            let [x, y, w, h] = LAYER_FILL;
            write!(
                inner,
                "<div style=\"position:absolute;left:{x}px;top:{y}px;width:{w}px;height:{h}px;\
                 border-radius:8px;background:{}\"></div>",
                css_color(c)
            )?;
        }
        if let Some((text, size, weight, a)) = run.text {
            let (x, y) = LAYER_TEXT;
            write!(
                inner,
                "<div style=\"position:absolute;left:{x}px;top:{y}px;white-space:nowrap;\
                 font:{weight} {size}px/21px {SANS};color:rgba(255,255,255,{a})\">{text}</div>"
            )?;
        }
        if run.opacity {
            inner = format!("<div style=\"position:absolute;inset:0;opacity:.5\">{inner}</div>");
        }
        let [x, y, w, h] = match run.boxed {
            Some(_) => LAYER_BOX,
            None => [0., 0., W, LAYER_ROW],
        };
        write!(
            html,
            "<div style=\"position:absolute;left:{x}px;top:{}px;width:{w}px;height:{h}px;{}\">{inner}</div>",
            top + y,
            match run.boxed {
                Some(true) => "background:rgba(24,24,24,.65);will-change:transform",
                Some(false) => "background:rgba(24,24,24,.65)",
                None => "",
            }
        )?;
    }
    html.push_str(
        "<pre id=m></pre><script>\
         const r=document.createRange();\
         document.getElementById('m').textContent=JSON.stringify([...document.querySelectorAll('.t')].map(s=>{\
         const t=s.firstChild;r.selectNodeContents(t);const box=r.getBoundingClientRect();const xs=[];\
         for(let i=0;i<t.length;i++){r.setStart(t,i);r.setEnd(t,i+1);xs.push(r.getBoundingClientRect().left)}\
         return {box:[box.x,box.y,box.width,box.height],xs}}))\
         </script></body></html>",
    );
    fs::write(dir.join("page.html"), html)?;
    println!("{W} {height}");
    Ok(())
}

struct Measured {
    rect: [f32; 4],
    xs: Vec<f32>,
}

fn dom(dir: &Path) -> Result<Vec<Measured>> {
    let raw = fs::read_to_string(dir.join("dom.html"))?;
    let json = raw
        .split_once("<pre id=\"m\">")
        .and_then(|(_, rest)| rest.split_once("</pre>"))
        .context("no #m in dom.html")?
        .0
        .replace("&amp;", "&");
    let value: serde_json::Value = serde_json::from_str(&json)?;
    let f = |v: &serde_json::Value| v.as_f64().unwrap_or(f64::NAN) as f32;
    Ok(value
        .as_array()
        .context("dom json")?
        .iter()
        .map(|m| Measured {
            rect: [0, 1, 2, 3].map(|i| f(&m["box"][i])),
            xs: m["xs"].as_array().into_iter().flatten().map(f).collect(),
        })
        .collect())
}

fn gpui_font(run: &Run) -> Font {
    Font {
        family: SharedString::new_static(if run.mono { "Menlo" } else { ".SystemUIFont" }),
        features: FontFeatures(Arc::new(
            textsys::letter_spacing(run.spacing)
                .tag_value_list()
                .iter()
                .filter(|_| run.spacing != 0.)
                .cloned()
                .chain(run.features.iter().map(|&(t, v)| (t.to_string(), v)))
                .collect(),
        )),
        weight: FontWeight(run.weight),
        style: FontStyle::Normal,
        fallbacks: None,
    }
}

/// Per run: max |GPUI glyph x - Chromium char x| (CSS px) and the GPUI text box.
type RunOut = (f32, [f32; 4]);

struct Lab {
    measured: Arc<Vec<Measured>>,
    measured_out: Rc<RefCell<Vec<RunOut>>>,
}

impl Render for Lab {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let measured = self.measured.clone();
        let measured_out = self.measured_out.clone();
        let paragraph = paragraph_top(&rows(&runs()));
        let text = canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                let runs = runs();
                window.paint_quad(fill(
                    Bounds::new(
                        point(px(0.), px(paragraph)),
                        size(px(W), px(PARAGRAPH_HEIGHT)),
                    ),
                    rgb(24, 24, 24, 1.),
                ));
                for ((run, row), m) in runs.iter().zip(rows(&runs)).zip(measured.iter()) {
                    let [r, g, b] = run.bg;
                    window.paint_quad(fill(
                        Bounds::new(point(px(0.), px(row.0)), size(px(W), px(row.1))),
                        Rgba {
                            r: r as f32 / 255.,
                            g: g as f32 / 255.,
                            b: b as f32 / 255.,
                            a: 1.,
                        },
                    ));
                    let color = run.color.into();
                    let line = window.text_system().shape_line(
                        run.text.into(),
                        px(run.size),
                        &[TextRun {
                            len: run.text.len(),
                            font: gpui_font(run),
                            color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    // Placed as a GPUI text element would be: line box top-left = CSS span box.
                    let lh = px(line_height(run));
                    let origin = point(px(row.2), px(row.3));
                    let top = (f32::from(lh - line.ascent - line.descent)).floor() / 2.;
                    let rect = [
                        m.rect[0],
                        row.3 + top,
                        m.rect[2],
                        f32::from(line.ascent + line.descent),
                    ];
                    let mut err = 0f32;
                    for glyph in line.runs.iter().flat_map(|r| &r.glyphs) {
                        let ci = run.text[..glyph.index].encode_utf16().count();
                        if let Some(x) = m.xs.get(ci) {
                            err = err.max((f32::from(glyph.position.x) + row.2 - x).abs());
                        }
                    }
                    measured_out.borrow_mut().push((err, rect));
                    line.paint(origin, lh, TextAlign::Left, None, window, cx)
                        .unwrap();
                }
            },
        )
        .absolute()
        .size_full();
        let layers = layers_top(&rows(&runs()));
        div()
            .size_full()
            .child(text)
            .child(
                div()
                    .absolute()
                    .left(px(10.))
                    .top(px(paragraph + 8.))
                    .w(px(PARAGRAPH_WIDTH))
                    .font_family(".SystemUIFont")
                    .font_weight(FontWeight(430.))
                    .text_size(px(14.))
                    .line_height(px(22.75))
                    .text_color(rgb(255, 255, 255, 1.))
                    .child(PARAGRAPH),
            )
            .children(
                layer_runs()
                    .iter()
                    .enumerate()
                    .map(|(j, run)| layer_row(run, layers + j as f32 * LAYER_ROW)),
            )
    }
}

fn abs([x, y, w, h]: [f32; 4]) -> gpui::Div {
    div().absolute().left(px(x)).top(px(y)).w(px(w)).h(px(h))
}

fn layer_row(run: &LayerRun, top: f32) -> gpui::Div {
    let mut inner = abs([0., 0., W, LAYER_ROW]);
    if let Some(c) = run.fill {
        inner = inner.child(
            abs(LAYER_FILL).child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(8.)),
                fill: Some(c.into()),
                ..Default::default()
            })),
        );
    }
    if let Some((text, size, weight, a)) = run.text {
        let (x, y) = LAYER_TEXT;
        inner = inner.child(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .whitespace_nowrap()
                .font_family(".SystemUIFont")
                .font_weight(FontWeight(weight))
                .text_size(px(size))
                .line_height(px(21.))
                .text_color(rgb(255, 255, 255, a))
                .child(text),
        );
    }
    if run.opacity {
        inner = inner.opacity(0.5);
    }
    let content = match run.boxed {
        Some(composited) => {
            let b = abs(LAYER_BOX).child(shapes::decoration(shapes::Decoration {
                fill: Some(rgb(24, 24, 24, 0.65).into()),
                ..Default::default()
            }));
            let b = if composited { b.layer() } else { b };
            b.child(inner)
        }
        None => inner,
    };
    abs([0., top, W, LAYER_ROW])
        .bg(rgb(run.bg, run.bg, run.bg, 1.))
        .child(content)
}

fn render(
    text_system: Arc<dyn PlatformTextSystem>,
    measured: Arc<Vec<Measured>>,
    w: u32,
    h: u32,
) -> Result<(RgbaImage, Vec<RunOut>)> {
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), || {
        gpui_platform::current_headless_renderer()
    });
    let measured_out: Rc<RefCell<Vec<RunOut>>> = Rc::default();
    let lab = Lab {
        measured,
        measured_out: measured_out.clone(),
    };
    let window = cx.open_window(size(px(w as f32), px(h as f32)), |_, cx: &mut App| {
        cx.new(|_| lab)
    })?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    let out = measured_out.borrow().clone();
    Ok((image, out))
}

/// Differing px inside the run's text box (inflated 1 device px, as the parity gate does).
fn diff(chrome: &RgbaImage, port: &RgbaImage, rect: [f32; 4], out: &mut RgbaImage) -> (u32, u32) {
    let [x, y, w, h] = rect.map(|v| v * 2.);
    let (x0, x1) = ((x.floor() - 1.) as u32, ((x + w).ceil() + 1.) as u32);
    let (y0, y1) = ((y.floor() - 1.) as u32, ((y + h).ceil() + 1.) as u32);
    let mut n = 0;
    for py in y0..y1.min(chrome.height()) {
        for px in x0..x1.min(chrome.width()) {
            let (a, b) = (chrome.get_pixel(px, py), port.get_pixel(px, py));
            if a != b {
                n += 1;
                out.put_pixel(px, py, image::Rgba([255, 255, 0, 255]));
            }
        }
    }
    (n, (x1 - x0) * (y1 - y0))
}

fn compare(dir: &Path) -> Result<()> {
    let chrome = image::open(dir.join("chrome.png"))?.to_rgba8();
    let measured = Arc::new(dom(dir)?);
    let (w, h) = (chrome.width() / 2, chrome.height() / 2);
    let stock: Arc<dyn PlatformTextSystem> = gpui_platform::current_platform(true).text_system();
    let (before, _) = render(stock, measured.clone(), w, h)?;
    let (after, out) = render(textsys::text_system(), measured.clone(), w, h)?;
    before.save(dir.join("before.png"))?;
    after.save(dir.join("after.png"))?;
    let runs = runs();
    let mut diff_img = chrome.clone();
    let mut scratch = chrome.clone();
    let mut table = String::from(
        "| # | font | color | bg | before | after | max adv err px |\n|---|---|---|---|---|---|---|\n",
    );
    let (mut tb, mut ta, mut tt) = (0, 0, 0);
    for (i, run) in runs.iter().enumerate() {
        let (err, rect) = out[i];
        let (nb, total) = diff(&chrome, &before, rect, &mut scratch);
        let (na, _) = diff(&chrome, &after, rect, &mut diff_img);
        (tb, ta, tt) = (tb + nb, ta + na, tt + total);
        let pct = |n: u32| 100. * n as f64 / total as f64;
        writeln!(
            table,
            "| {i} | {}{} {} | {} | {:?} | {:.2}% | {:.2}% | {:.3} |",
            if run.mono { "mono " } else { "" },
            run.weight,
            run.size,
            css_color(run.color),
            run.bg,
            pct(nb),
            pct(na),
            err
        )?;
    }
    let rect = measured
        .get(runs.len())
        .context("paragraph not measured")?
        .rect;
    let (nb, total) = diff(&chrome, &before, rect, &mut scratch);
    let (na, _) = diff(&chrome, &after, rect, &mut diff_img);
    (tb, ta, tt) = (tb + nb, ta + na, tt + total);
    let pct = |n: u32| 100. * n as f64 / total as f64;
    writeln!(
        table,
        "| p | 430 14/22.75 wrapped | rgba(255, 255, 255, 1) | [24, 24, 24] | {:.2}% | {:.2}% | |",
        pct(nb),
        pct(na)
    )?;
    let layers = layers_top(&rows(&runs));
    for (j, run) in layer_runs().iter().enumerate() {
        // The whole row (`diff` inflates the rect by one device px).
        let rect = [0., layers + j as f32 * LAYER_ROW + 0.5, W, LAYER_ROW - 1.];
        let (nb, total) = diff(&chrome, &before, rect, &mut scratch);
        let (na, _) = diff(&chrome, &after, rect, &mut diff_img);
        (tb, ta, tt) = (tb + nb, ta + na, tt + total);
        let pct = |n: u32| 100. * n as f64 / total as f64;
        writeln!(
            table,
            "| L{j} | {} | | {} | {:.2}% | {:.2}% | |",
            run.name,
            run.bg,
            pct(nb),
            pct(na)
        )?;
    }
    let pct = |n: u32| 100. * n as f64 / tt as f64;
    writeln!(table, "| all | | | | {:.2}% | {:.2}% | |", pct(tb), pct(ta))?;
    diff_img.save(dir.join("diff.png"))?;
    fs::write(dir.join("table.md"), &table)?;
    print!("{table}");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match &args[..] {
        [cmd, dir] if cmd == "page" => page(Path::new(dir)),
        [cmd, dir] if cmd == "compare" => compare(Path::new(dir)),
        _ => bail!("usage: measure page|compare <dir>"),
    }
}
