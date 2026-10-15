//! Text-rasterization lab. `measure page <dir>` writes `<dir>/page.html` (one text run per row).
//! `tools/text/measure.sh` renders it with chrome-headless-shell into `chrome.png` + `dom.json`,
//! then `measure compare <dir>` paints the same runs in GPUI (stock AppleTextSystem = before,
//! `textsys::text_system()` = after) as GPUI text elements and diffs inside the text boxes.

use anyhow::{Context as _, Result, bail};
use gpui::{
    App, AppContext, Bounds, Font, FontFeatures, FontStyle, FontWeight, HeadlessAppContext,
    IntoElement, PlatformTextSystem, Render, Rgba, SharedString, Styled, TextAlign, TextRun,
    Window, canvas, fill, point, px, size,
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
}

const SAMPLES: [&str; 4] = [
    "Skip to content New thread Automations 0123456789",
    "Fix failing tests in the parser module, 12m ago",
    "Ask Codex anything. Use @ to add files; gpt-5-codex High",
    "Settings General Appearance Keyboard shortcuts Ww Qq",
];
const MONO_SAMPLE: &str = "fn main() { let x = 42; } // diff +12 -3 [ok]";

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
            });
        }
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
    let height = rows.last().map_or(0., |r| r.0 + r.1).ceil();
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
             <span class=t style=\"left:{x}px;top:{y}px;font:{} {}px/{}px {};color:{};letter-spacing:{}px\">\
             {}</span>",
            run.weight,
            run.size,
            line_height(run),
            if run.mono { MONO } else { SANS },
            css_color(run.color),
            run.spacing,
            run.text
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
        features: if run.spacing != 0. {
            textsys::letter_spacing(run.spacing)
        } else {
            FontFeatures::default()
        },
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
        canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                let runs = runs();
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
                    let top = (lh - line.ascent - line.descent) / 2.;
                    let rect = [
                        m.rect[0],
                        row.3 + f32::from(top),
                        m.rect[2],
                        f32::from(line.ascent + line.descent),
                    ];
                    let mut err = 0f32;
                    for glyph in line.runs.iter().flat_map(|r| &r.glyphs) {
                        let ci = run.text[..glyph.index].chars().count();
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
        .size_full()
    }
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
