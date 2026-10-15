//! `parity [--refs refs] [--out target/parity] [--screen <name>] [--gate] [--self-test]`

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{AnyView, App, AppContext, IntoElement, Render, Styled, Window, div, rgb};
use image::Rgba;
use parity::Case;
use std::{fs, path::PathBuf, sync::Arc};

const USAGE: &str =
    "usage: parity [--refs refs] [--out target/parity] [--screen <name>] [--gate] [--self-test]";

fn main() -> Result<()> {
    let (mut refs, mut out) = (PathBuf::from("refs"), PathBuf::from("target/parity"));
    let (mut screen, mut gate) = (None, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--refs" => refs = args.next().context(USAGE)?.into(),
            "--out" => out = args.next().context(USAGE)?.into(),
            "--screen" => screen = Some(args.next().context(USAGE)?),
            "--gate" => gate = true,
            "--self-test" => return self_test(),
            _ => bail!(USAGE),
        }
    }
    // The `ui` crate's `parity_cases()` plugs in here once it exists.
    let report = parity::run(&[], Arc::new(()), &refs, &out, screen.as_deref())?;
    println!("{}", out.join("report.html").display());
    if gate && report.iter().any(|m| !m.pass) {
        std::process::exit(1);
    }
    Ok(())
}

struct Solid;

impl Render for Solid {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x336699))
    }
}

fn solid(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| Solid).into()
}

/// Renders a solid view, saves it as a ref with 3 pixels changed outside its one text box
/// and 10 inside, then checks `run` counts exactly those.
fn self_test() -> Result<()> {
    let case = Case {
        screen: "selftest",
        theme: "dark",
        build: solid,
    };
    let root = PathBuf::from("target/parity-self-test");
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("refs/selftest");
    fs::create_dir_all(&dir)?;

    let mut fake = parity::render(&case, Arc::new(()), 64, 48)?;
    ensure!(fake.dimensions() == (128, 96), "headless window not 2x");
    let red = Rgba([255, 0, 0, 255]);
    for x in 0..3 {
        fake.put_pixel(x, 0, red);
    }
    // Text box [10, 10, 10, 5] CSS px -> device px x 19..41, y 19..31 after 1 px inflation.
    for x in 25..35 {
        fake.put_pixel(x, 25, red);
    }
    fake.save(dir.join("dark@64x48.png"))?;
    fs::write(
        dir.join("dark@64x48.spec.json"),
        r#"{"viewport":[64,48,2],"texts":[{"rect":[10,10,10,5]}]}"#,
    )?;

    let report = parity::run(
        &[case],
        Arc::new(()),
        &root.join("refs"),
        &root.join("out"),
        None,
    )?;
    let [m] = report.as_slice() else {
        bail!("expected 1 report entry, got {}", report.len());
    };
    let c = &m.counts;
    assert_eq!(c.total_px, 128 * 96);
    assert_eq!(c.diff_px_outside_text, 3);
    assert_eq!(c.diff_px_tol2_outside_text, 3);
    assert_eq!(c.text_px, 22 * 12);
    assert_eq!(c.diff_px_inside_text, 10);
    assert!(!m.pass);
    println!("self-test ok: {c:?}");
    Ok(())
}
