//! Lab for `shapes::backdrop_blur` (macOS, `--features lab`; driven by tools/shapes/blur.sh):
//! the refs' `backdrop-filter: blur()` boxes (composer model picker, diff header capsules) on
//! their backdrops at their ref positions, in Chromium and GPUI.
//!
//! - `blur pages <dir>`: `blur-<theme>.html`.
//! - `blur verify <dir>`: renders the scenes with plain `shapes::decoration` boxes (before)
//!   and `shapes::backdrop_blur` (after), diffs both against Chromium's `blur-<theme>.png` per
//!   blurred box and prints the table.

mod common;

use anyhow::{Result, bail, ensure};
use common::{hsla, rgba_css};
use gpui::{
    AnyElement, AppContext, Corners, HeadlessAppContext, IntoElement, ParentElement as _, Render,
    Styled, Window, div, point, px, size,
};
use image::RgbaImage;
use std::{fmt::Write as _, fs, path::Path, sync::Arc};

const PAGE: [f32; 2] = [1280., 832.];

/// A CSS box at its ref position: `[x, y, w, h]` CSS px.
#[derive(Clone)]
struct Node {
    rect: [f32; 4],
    radius: [f32; 4],
    squircle: bool,
    fill: Option<[u8; 4]>,
    /// The fill is a `linear-gradient(c, c)` background image.
    gradient: bool,
    /// `[x, y, blur, spread]`, color, inset; CSS order.
    shadows: Vec<([f32; 4], [u8; 4], bool)>,
    blur: Option<f32>,
    children: Vec<Node>,
    label: &'static str,
}

fn node(rect: [f32; 4], r: f32) -> Node {
    Node {
        rect,
        radius: [r; 4],
        squircle: false,
        fill: None,
        gradient: false,
        shadows: Vec::new(),
        blur: None,
        children: Vec::new(),
        label: "",
    }
}

fn scene(dark: bool) -> Vec<Node> {
    let c = |dark_c: [u8; 4], light_c: [u8; 4]| if dark { dark_c } else { light_c };
    let surface = Node {
        fill: Some(c([24, 24, 24, 255], [255, 255, 255, 255])),
        ..node([340., 44., 936., 784.], 0.)
    };
    let bar = Node {
        fill: Some(c([40, 40, 40, 255], [246, 246, 246, 255])),
        ..node([453.25, 676., 710., 42.], 0.)
    };
    let card = Node {
        radius: [22., 22., 0., 22.],
        fill: Some(c([54, 54, 54, 255], [255, 255, 255, 255])),
        shadows: if dark {
            vec![([0., 0., 1., 0.], [255, 255, 255, 51], true)]
        } else {
            vec![
                ([0., 0., 0., 1.], [0, 0, 0, 10], false),
                ([0., 2., 8., 0.], [0, 0, 0, 10], false),
                ([0., 4., 80., 8.], [0, 0, 0, 6], false),
            ]
        },
        ..node([440.25, 714., 736., 98.], 22.)
    };
    let ring = c([255, 255, 255, 21], [26, 28, 31, 20]);
    let popup = Node {
        squircle: true,
        fill: Some(c([45, 45, 45, 230], [255, 255, 255, 230])),
        shadows: vec![
            ([0., 0., 0., 0.5], ring, false),
            ([0., 0., 0., 0.5], ring, false),
            ([0., 8., 16., -4.], [0, 0, 0, 31], false),
        ],
        blur: Some(8.),
        label: "composer popup",
        ..node([902.5, 671., 254., 96.], 20.)
    };
    let capsule = |rect: [f32; 4], buttons: Vec<[f32; 4]>, label| Node {
        fill: Some(c([45, 45, 45, 255], [255, 255, 255, 245])),
        gradient: true,
        shadows: if dark {
            vec![
                ([0., 0., 0., 0.5], [255, 255, 255, 21], false),
                ([0., 0., 0., 0.5], [255, 255, 255, 21], false),
                ([0., 8., 16., -4.], [0, 0, 0, 31], false),
            ]
        } else {
            vec![
                ([0., 0., 0., 1.], [26, 28, 31, 12], false),
                ([0., 4., 16., 0.], [0, 0, 0, 13], false),
            ]
        },
        blur: Some(24.),
        label,
        children: buttons
            .into_iter()
            .map(|r| Node {
                blur: Some(24.),
                ..node(r, 9999.)
            })
            .collect(),
        ..node(rect, 9999.)
    };
    // Nesting probes: a button at each end and in the middle, without and with the
    // capsule's shadows.
    let probe = |x: f32, y: f32, w: f32, bx: f32, shadows: bool, label| {
        let mut n = capsule([x, y, w, 32.], vec![[x + bx, y + 2., 28., 28.]], label);
        n.gradient = false;
        if !shadows {
            n.shadows.clear();
        }
        n
    };
    vec![
        surface,
        bar,
        card,
        popup,
        capsule(
            [669.5, 52., 136.2, 32.],
            vec![[671.5, 54., 90.95, 28.]],
            "diff capsule (review source)",
        ),
        capsule(
            [1176., 52., 92., 32.],
            vec![[1238., 54., 28., 28.]],
            "diff capsule (controls)",
        ),
        probe(400., 200., 92., 2., false, "probe: button left, no shadows"),
        probe(
            600.,
            200.,
            92.,
            32.,
            false,
            "probe: button middle, no shadows",
        ),
        probe(
            800.,
            200.,
            92.,
            62.,
            false,
            "probe: button right, no shadows",
        ),
        probe(400., 300., 92., 2., true, "probe: button left, shadows"),
        probe(600., 300., 92., 32., true, "probe: button middle, shadows"),
        probe(800., 300., 92., 62., true, "probe: button right, shadows"),
        probe(
            400.,
            400.,
            200.,
            2.,
            true,
            "probe: wide, button left, shadows",
        ),
    ]
}

fn html(n: &Node, parent: [f32; 2], out: &mut String) {
    let [x, y, w, h] = n.rect;
    let r = n.radius;
    let mut s = format!(
        "position:absolute;left:{}px;top:{}px;width:{w}px;height:{h}px;\
         border-radius:{}px {}px {}px {}px;",
        x - parent[0],
        y - parent[1],
        r[0],
        r[1],
        r[2],
        r[3]
    );
    if n.squircle {
        s += "corner-shape:superellipse(1.5);";
    }
    if let Some(c) = n.fill {
        let c = rgba_css(c);
        if n.gradient {
            write!(s, "background-image:linear-gradient({c},{c});").unwrap();
        } else {
            write!(s, "background:{c};").unwrap();
        }
    }
    if !n.shadows.is_empty() {
        let list: Vec<String> = n
            .shadows
            .iter()
            .map(|(v, c, inset)| {
                format!(
                    "{}{}px {}px {}px {}px {}",
                    if *inset { "inset " } else { "" },
                    v[0],
                    v[1],
                    v[2],
                    v[3],
                    rgba_css(*c)
                )
            })
            .collect();
        write!(s, "box-shadow:{};", list.join(",")).unwrap();
    }
    if let Some(b) = n.blur {
        write!(s, "backdrop-filter:blur({b}px);").unwrap();
    }
    write!(out, "<div style=\"{s}\">").unwrap();
    for c in &n.children {
        html(c, [x, y], out);
    }
    out.push_str("</div>\n");
}

fn pages(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    for (theme, dark) in [("dark", true), ("light", false)] {
        let mut body = String::new();
        for n in scene(dark) {
            html(&n, [0., 0.], &mut body);
        }
        fs::write(
            dir.join(format!("blur-{theme}.html")),
            format!(
                "<!doctype html><html><body style=\"margin:0;background:#000\">\n{body}</body></html>\n"
            ),
        )?;
    }
    Ok(())
}

fn decoration(n: &Node) -> shapes::Decoration {
    shapes::Decoration {
        radius: Corners {
            top_left: px(n.radius[0]),
            top_right: px(n.radius[1]),
            bottom_right: px(n.radius[2]),
            bottom_left: px(n.radius[3]),
        },
        corner: if n.squircle {
            shapes::Corner::Squircle
        } else {
            shapes::Corner::Round
        },
        fill: n.fill.map(hsla),
        shadows: n
            .shadows
            .iter()
            .map(|(v, c, inset)| shapes::Shadow {
                color: hsla(*c),
                offset: point(px(v[0]), px(v[1])),
                blur: px(v[2]),
                spread: px(v[3]),
                inset: *inset,
            })
            .collect(),
        ..Default::default()
    }
}

fn element(n: &Node, parent: [f32; 2], after: bool) -> AnyElement {
    let [x, y, w, h] = n.rect;
    let d = decoration(n);
    let b = div()
        .absolute()
        .left(px(x - parent[0]))
        .top(px(y - parent[1]))
        .w(px(w))
        .h(px(h));
    let b = match n.blur {
        Some(blur) if after => {
            let fill = if n.gradient {
                shapes::Fill::Gradient
            } else {
                shapes::Fill::Color
            };
            b.child(shapes::backdrop_blur(px(blur), fill, false, d))
        }
        _ => b.child(shapes::decoration(d)),
    };
    b.children(n.children.iter().map(|c| element(c, [x, y], after)))
        .into_any_element()
}

struct View {
    dark: bool,
    after: bool,
}

impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().bg(hsla([0, 0, 0, 255])).children(
            scene(self.dark)
                .iter()
                .map(|n| element(n, [0., 0.], self.after)),
        )
    }
}

fn verify(dir: &Path) -> Result<()> {
    let mut cx = HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(()), || {
        gpui_platform::current_headless_renderer()
    });
    println!("| theme | box | blur | device px | diff px before | diff px after |");
    println!("|---|---|---|---|---|---|");
    for (theme, dark) in [("dark", true), ("light", false)] {
        let chrome = image::open(dir.join(format!("blur-{theme}.png")))?.into_rgba8();
        let mut shots: Vec<RgbaImage> = Vec::new();
        for after in [false, true] {
            let window = cx.open_window(size(px(PAGE[0]), px(PAGE[1])), |_, cx| {
                cx.new(|_| View { dark, after })
            })?;
            cx.run_until_parked();
            let shot = cx.capture_screenshot(window.into())?;
            cx.update_window(window.into(), |_, window, _| window.remove_window())?;
            ensure!(shot.dimensions() == chrome.dimensions(), "GPUI shot size");
            shots.push(shot);
        }
        for n in scene(dark).iter().filter(|n| n.blur.is_some()) {
            // The box and the reach of its shadows, device px.
            let [x, y, w, h] = n.rect.map(|v| v * 2.);
            let r = [x - 48., y - 48., x + w + 48., y + h + 64.].map(|v| v.round() as u32);
            let r = [
                r[0],
                r[1],
                r[2].min(chrome.width()),
                r[3].min(chrome.height()),
            ];
            let d: Vec<u32> = shots
                .iter()
                .map(|shot| {
                    (r[1]..r[3])
                        .flat_map(|y| (r[0]..r[2]).map(move |x| (x, y)))
                        .filter(|&(x, y)| chrome.get_pixel(x, y) != shot.get_pixel(x, y))
                        .count() as u32
                })
                .collect();
            println!(
                "| {theme} | {} | {}px | {}x{} | {} | {} |",
                n.label,
                n.blur.unwrap(),
                w,
                h,
                d[0],
                d[1]
            );
        }
        for (name, img) in ["before", "after"].iter().zip(&shots) {
            img.save(dir.join(format!("blur-{theme}-{name}.png")))?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [cmd, dir] = args.as_slice() else {
        bail!("usage: blur pages|verify <dir>");
    };
    let dir = Path::new(dir);
    match cmd.as_str() {
        "pages" => pages(dir),
        "verify" => verify(dir),
        _ => bail!("unknown command {cmd}"),
    }
}
