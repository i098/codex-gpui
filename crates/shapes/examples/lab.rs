//! Measurement lab for crates/shapes (macOS, `--features lab`; driven by tools/shapes/run.sh).
//!
//! - `lab pages <dir>`: `verify.html` (one ref-sized, ref-colored sample per layer key of
//!   tools/shapes/cases.json); prints its size in CSS px.
//! - `lab verify <dir>`: paints the verify cells in GPUI with `.rounded()/.bg()/.border()/
//!   .shadow()` (before) and `shapes::decoration` (after), diffs both against Chromium's
//!   `verify.png` per cell and prints the table.

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    AnyElement, AppContext, BoxShadow, Corners, HeadlessAppContext, Hsla, IntoElement,
    ParentElement as _, Render, Rgba, Styled, Window, div, point, px, size,
};
use image::{Rgba as Px, RgbaImage};
use serde::Deserialize;
use std::{fmt::Write as _, fs, path::Path, sync::Arc};

#[derive(Deserialize, Clone)]
struct Case {
    kind: String,
    corner: String,
    radii: [f32; 4],
    width: Option<f32>,
    x: Option<f32>,
    y: Option<f32>,
    blur: Option<f32>,
    spread: Option<f32>,
    inset: Option<bool>,
    count: u32,
    reach: i32,
    sample: Sample,
}

#[derive(Deserialize, Clone)]
struct Sample {
    w: i32,
    h: i32,
    color: [u8; 4],
    backdrop: [u8; 4],
}

/// One box on a page, device px.
#[derive(Clone)]
struct Cell {
    case: usize,
    /// Border box origin and size.
    at: [i32; 2],
    size: [i32; 2],
    /// The cell (box + reach + gap) the box owns.
    rect: [i32; 4],
}

const GAP: i32 = 8;
const PAGE_W: i32 = 4000;

/// Shelf-packs `(case, box size)` items, shifting boxes to odd device px every other cell.
fn pack(cases: &[Case], items: Vec<(usize, [i32; 2])>) -> (Vec<Cell>, [i32; 2]) {
    let (mut x, mut y, mut row) = (0, 0, 0);
    let mut cells = Vec::new();
    for (n, (case, size)) in items.into_iter().enumerate() {
        let pad = cases[case].reach + GAP;
        let even = |v: i32| v + v % 2;
        let (w, h) = (even(size[0] + 2 * pad + 1), even(size[1] + 2 * pad + 1));
        if x + w > PAGE_W {
            (x, y, row) = (0, y + row, 0);
        }
        let shift = [n as i32 % 2, n as i32 / 2 % 2];
        cells.push(Cell {
            case,
            at: [x + pad + shift[0], y + pad + shift[1]],
            size,
            rect: [x, y, w, h],
        });
        x += w;
        row = row.max(h);
    }
    (cells, [PAGE_W, y + row])
}

fn verify_layout(cases: &[Case]) -> (Vec<Cell>, [i32; 2]) {
    let items = cases
        .iter()
        .enumerate()
        .map(|(i, c)| (i, [c.sample.w, c.sample.h]))
        .collect();
    pack(cases, items)
}

fn css(v: impl Into<f64>) -> f64 {
    v.into() / 2.
}

fn rgba_css([r, g, b, a]: [u8; 4]) -> String {
    format!("rgba({r},{g},{b},{})", f64::from(a) / 255.)
}

fn box_html(c: &Case, cell: &Cell, color: [u8; 4]) -> String {
    let r = c.radii.map(css);
    let mut s = format!(
        "position:absolute;box-sizing:border-box;left:{}px;top:{}px;width:{}px;height:{}px;\
         border-radius:{}px {}px {}px {}px;",
        css(cell.at[0]),
        css(cell.at[1]),
        css(cell.size[0]),
        css(cell.size[1]),
        r[0],
        r[1],
        r[2],
        r[3]
    );
    if c.corner == "squircle" {
        s += "corner-shape:superellipse(1.5);";
    }
    let col = rgba_css(color);
    match c.kind.as_str() {
        "fill" => write!(s, "background:{col};"),
        "border" => write!(s, "border:{}px solid {col};", css(c.width.unwrap())),
        _ => write!(
            s,
            "box-shadow:{}px {}px {}px {}px {col}{};",
            css(c.x.unwrap()),
            css(c.y.unwrap()),
            css(c.blur.unwrap()),
            css(c.spread.unwrap()),
            if c.inset.unwrap() { " inset" } else { "" }
        ),
    }
    .unwrap();
    format!("<div style=\"{s}\"></div>\n")
}

fn page(body: String) -> String {
    format!(
        "<!doctype html><html><body style=\"margin:0;background:#000\">\n{body}</body></html>\n"
    )
}

fn load_cases() -> Result<Vec<Case>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/shapes/cases.json");
    Ok(serde_json::from_slice(
        &fs::read(&path).context("cases.json")?,
    )?)
}

fn pages(dir: &Path, cases: &[Case]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let (cells, vsize) = verify_layout(cases);
    let body = cells
        .iter()
        .map(|cell| {
            let c = &cases[cell.case];
            let [x, y, w, h] = cell.rect.map(css);
            format!(
                "<div style=\"position:absolute;left:{x}px;top:{y}px;width:{w}px;height:{h}px;background:{}\"></div>\n{}",
                rgba_css(c.sample.backdrop),
                box_html(c, cell, c.sample.color)
            )
        })
        .collect();
    fs::write(dir.join("verify.html"), page(body))?;
    println!("{} {}", css(vsize[0]), css(vsize[1]));
    Ok(())
}

fn hsla([r, g, b, a]: [u8; 4]) -> Hsla {
    let c = |v: u8| f32::from(v) / 255.;
    Rgba {
        r: c(r),
        g: c(g),
        b: c(b),
        a: c(a),
    }
    .into()
}

fn decoration(c: &Case) -> shapes::Decoration {
    let color = hsla(c.sample.color);
    let r = |i: usize| px(c.radii[i] / 2.);
    let mut d = shapes::Decoration {
        radius: Corners {
            top_left: r(0),
            top_right: r(1),
            bottom_right: r(2),
            bottom_left: r(3),
        },
        corner: if c.corner == "squircle" {
            shapes::Corner::Squircle
        } else {
            shapes::Corner::Round
        },
        ..Default::default()
    };
    match c.kind.as_str() {
        "fill" => d.fill = Some(color),
        "border" => d.border = Some((px(c.width.unwrap() / 2.), color)),
        _ => d.shadows.push(shapes::Shadow {
            color,
            offset: point(px(c.x.unwrap() / 2.), px(c.y.unwrap() / 2.)),
            blur: px(c.blur.unwrap() / 2.),
            spread: px(c.spread.unwrap() / 2.),
            inset: c.inset.unwrap(),
        }),
    }
    d
}

/// The same box with GPUI's own styles (what the screens use without crates/shapes).
fn gpui_styles(mut b: gpui::Div, d: shapes::Decoration) -> gpui::Div {
    let s = b.style();
    s.corner_radii.top_left = Some(d.radius.top_left.into());
    s.corner_radii.top_right = Some(d.radius.top_right.into());
    s.corner_radii.bottom_right = Some(d.radius.bottom_right.into());
    s.corner_radii.bottom_left = Some(d.radius.bottom_left.into());
    if let Some((w, color)) = d.border {
        for e in [
            &mut s.border_widths.top,
            &mut s.border_widths.right,
            &mut s.border_widths.bottom,
            &mut s.border_widths.left,
        ] {
            *e = Some(w.into());
        }
        s.border_color = Some(color);
    }
    if let Some(fill) = d.fill {
        b = b.bg(fill);
    }
    b.shadow(
        d.shadows
            .iter()
            .map(|s| BoxShadow {
                color: s.color,
                offset: s.offset,
                blur_radius: s.blur,
                spread_radius: s.spread,
                inset: s.inset,
            })
            .collect(),
    )
}

struct View {
    cases: Vec<Case>,
    cells: Vec<Cell>,
    after: bool,
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let at = |v: i32| px(css(v) as f32);
        let mut root = div().size_full().bg(hsla([0, 0, 0, 255]));
        for cell in &self.cells {
            let c = &self.cases[cell.case];
            let [x, y, w, h] = cell.rect;
            let backdrop = div()
                .absolute()
                .left(at(x))
                .top(at(y))
                .w(at(w))
                .h(at(h))
                .bg(hsla(c.sample.backdrop));
            let b = div()
                .absolute()
                .left(at(cell.at[0]))
                .top(at(cell.at[1]))
                .w(at(cell.size[0]))
                .h(at(cell.size[1]));
            let b: AnyElement = if self.after {
                b.child(shapes::decoration(decoration(c)))
                    .into_any_element()
            } else {
                gpui_styles(b, decoration(c)).into_any_element()
            };
            root = root.child(backdrop).child(b);
        }
        root
    }
}

fn render(cx: &mut HeadlessAppContext, view: View, page: [i32; 2]) -> Result<RgbaImage> {
    let window = cx.open_window(
        size(px(css(page[0]) as f32), px(css(page[1]) as f32)),
        |_, cx| cx.new(|_| view),
    )?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    Ok(image)
}

/// Differing px inside `rect`, and a copy of `port` with them magenta.
fn diff(chrome: &RgbaImage, port: &RgbaImage, rect: [i32; 4], marked: &mut RgbaImage) -> u32 {
    let [x0, y0, w, h] = rect.map(|v| v as u32);
    let mut n = 0;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if chrome.get_pixel(x, y) != port.get_pixel(x, y) {
                n += 1;
                marked.put_pixel(x, y, Px([255, 0, 255, 255]));
            }
        }
    }
    n
}

fn params(c: &Case) -> String {
    let r = c.radii.map(css);
    let r = if r.iter().all(|&v| v == r[0]) {
        format!("r {}", r[0])
    } else {
        format!("r {r:?}")
    };
    match c.kind.as_str() {
        "fill" => r,
        "border" => format!("{r}, {}px", css(c.width.unwrap())),
        _ => format!(
            "{r}, {}{} {} {} {}",
            if c.inset.unwrap() { "inset " } else { "" },
            css(c.x.unwrap()),
            css(c.y.unwrap()),
            css(c.blur.unwrap()),
            css(c.spread.unwrap())
        ),
    }
}

fn verify(dir: &Path, cases: Vec<Case>) -> Result<()> {
    let chrome = image::open(dir.join("verify.png"))?.into_rgba8();
    let (cells, page) = verify_layout(&cases);
    ensure!(
        [chrome.width() as i32, chrome.height() as i32] == page,
        "verify.png size"
    );
    let mut cx = HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(()), || {
        gpui_platform::current_headless_renderer()
    });
    let mut shots = Vec::new();
    for after in [false, true] {
        let view = View {
            cases: cases.clone(),
            cells: cells.clone(),
            after,
        };
        let shot = render(&mut cx, view, page)?;
        ensure!(shot.dimensions() == chrome.dimensions(), "GPUI shot size");
        shots.push(shot);
    }
    let mut marked = [shots[0].clone(), shots[1].clone()];
    println!(
        "| kind | params (CSS px) | corner | ref nodes | box (device px) | diff px before | diff px after |"
    );
    println!("|---|---|---|---|---|---|---|");
    let mut total = [0; 2];
    for cell in &cells {
        let c = &cases[cell.case];
        let d = [0, 1].map(|i| diff(&chrome, &shots[i], cell.rect, &mut marked[i]));
        total = [total[0] + d[0], total[1] + d[1]];
        if d[1] > 0 {
            // Inspection aid (stderr): the first differing px, box-relative, Chromium vs port.
            eprintln!(
                "cell {} {} {} at {:?}",
                c.kind,
                params(c),
                c.corner,
                cell.at
            );
            let [x0, y0, w, h] = cell.rect;
            let px = (y0..y0 + h)
                .flat_map(|y| (x0..x0 + w).map(move |x| (x as u32, y as u32)))
                .filter(|&(x, y)| chrome.get_pixel(x, y) != shots[1].get_pixel(x, y));
            for (x, y) in px.take(16) {
                let (a, b) = (chrome.get_pixel(x, y).0, shots[1].get_pixel(x, y).0);
                let rel = [x as i32 - cell.at[0], y as i32 - cell.at[1]];
                eprintln!("  {rel:?} chrome {:?} port {:?}", &a[..3], &b[..3]);
            }
        }
        println!(
            "| {} | {} | {} | {} | {}x{} | {} | {} |",
            c.kind,
            params(c),
            c.corner,
            c.count,
            cell.size[0],
            cell.size[1],
            d[0],
            d[1]
        );
    }
    println!("| total | | | | | {} | {} |", total[0], total[1]);
    for (name, img) in ["before", "after"].iter().zip(&shots) {
        img.save(dir.join(format!("{name}.png")))?;
    }
    for (name, img) in ["before-diff", "after-diff"].iter().zip(&marked) {
        img.save(dir.join(format!("{name}.png")))?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [cmd, dir] = args.as_slice() else {
        bail!("usage: lab pages|verify <dir>");
    };
    let (dir, cases) = (Path::new(dir), load_cases()?);
    match cmd.as_str() {
        "pages" => pages(dir, &cases),
        "verify" => verify(dir, cases),
        _ => bail!("unknown command {cmd}"),
    }
}
