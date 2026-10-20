mod common;

use anyhow::{Result, ensure};
use app::{Route, SidePanel};
use common::{Fx, ROOT, SUBTRACT};
use image::RgbaImage;
use serde_json::Value;
use std::{env, fmt::Write as _, fs, path::Path};

#[test]
#[ignore = "run through tools/parity/proof.py on macOS"]
fn capture_cell() -> Result<()> {
    ensure!(cfg!(target_os = "macos"), "proof capture requires macOS");
    let surface = env::var("CGX_PROOF_SURFACE")?;
    let state = env::var("CGX_PROOF_STATE")?;
    let theme = match env::var("CGX_PROOF_THEME")?.as_str() {
        "dark" => "dark",
        "light" => "light",
        _ => anyhow::bail!("invalid theme"),
    };
    let width: u32 = env::var("CGX_PROOF_WIDTH")?.parse()?;
    let height: u32 = env::var("CGX_PROOF_HEIGHT")?.parse()?;
    let output = env::var("CGX_PROOF_OUT")?;
    let output = Path::new(&output);
    let mut fx = Fx::with(theme, (width as f32, height as f32), None, |_| {})?;
    match (surface.as_str(), state.as_str()) {
        ("home", "rest") => ensure!(fx.route() == Route::Home),
        ("terminal", "rest") => open_terminal(&mut fx)?,
        ("settings", "configuration") => {
            fx.keys("cmd-,")?;
            fx.click(fx.at("settings", "Configuration")?)?;
            ensure!(fx.route() == Route::Settings);
        }
        _ => anyhow::bail!("no native recipe for {surface}/{state}"),
    }
    fx.cx.run_until_parked();
    let native = fx.reference_shot()?;
    let metrics = parity::diff_ref(&output.join("refs"), output, &surface, theme, &native)?;
    fs::write(
        output.join("metrics.json"),
        serde_json::to_vec_pretty(&metrics)?,
    )?;
    fs::copy(output.join(&metrics.ref_png), output.join("original.png"))?;
    fs::rename(output.join(&metrics.port_png), output.join("native.png"))?;
    fs::rename(output.join(&metrics.diff_png), output.join("diff.png"))?;
    let diff = image::open(output.join("diff.png"))?.into_rgba8();
    let mut report = format!(
        "# {surface} / {state}\n\nTheme: {theme}.\nSize: {width}x{height} CSS pixels, native 2x.\n\n\
         Gate: {}.\nOutside text: {} pixels (limit 0).\nInside text: {} / {} pixels ({:.6}%, limit 0.5%).\n\n\
         The unchanged `parity::diff_ref` sets the gate and the text mask.\n\n\
         | Region (device x, y, width, height) | Pixels | Inside text | Outside text |\n\
         |---|---:|---:|---:|\n",
        if metrics.pass { "PASS" } else { "FAIL" },
        metrics.counts.diff_px_outside_text,
        metrics.counts.diff_px_inside_text,
        metrics.counts.text_px,
        metrics.counts.text_diff_ratio * 100.,
    );
    let regions = regions(&diff);
    for region in &regions {
        writeln!(
            report,
            "| {}, {}, {}, {} | {} | {} | {} |",
            region.x,
            region.y,
            region.width,
            region.height,
            region.inside + region.outside,
            region.inside,
            region.outside
        )?;
    }
    if regions.is_empty() {
        report.push_str("| No differences | 0 | 0 | 0 |\n");
    } else {
        report.push_str("\nRegions join adjacent differing pixels across edges and corners.\nThe report does not infer a source cause from pixels.\n");
    }
    fs::write(output.join("differences.md"), report)?;
    ensure!(metrics.pass, "strict pixel gate failed; see differences.md");
    Ok(())
}

fn open_terminal(fx: &mut Fx) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let statuses: Vec<Value> =
        fs::read_to_string(format!("{ROOT}/fixtures/sessions/subtract.jsonl"))?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<serde_json::Result<Vec<_>>>()?
            .into_iter()
            .filter(|line| line["msg"]["method"] == "thread/status/changed")
            .collect();
    ensure!(statuses.len() == 2, "expected active and idle statuses");
    let mut events = ui::screens::terminal::running_command_replay();
    for event in &mut events {
        if let bridge::Event::Notification { params, .. } = event
            && params["turn"].is_object()
        {
            params["turn"]["rootTurnId"] = params["turn"]["id"].clone();
        }
    }
    let status = |line: &Value| bridge::Event::Notification {
        method: "thread/status/changed".into(),
        params: line["msg"]["params"].clone(),
    };
    events.insert(0, status(&statuses[0]));
    events.push(status(&statuses[1]));
    for event in events {
        fx.root
            .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
        fx.cx.run_until_parked();
    }
    fx.click((fx.size.0 - 63., 22.))?;
    fx.click((fx.size.0 - 202., 179.7))?;
    ensure!(fx.read(|root, _| root.side_panel()) == Some(SidePanel::Terminal));
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Region {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    inside: u64,
    outside: u64,
}

fn regions(diff: &RgbaImage) -> Vec<Region> {
    let (width, height) = diff.dimensions();
    let mut seen = vec![false; (width * height) as usize];
    let mut pending = Vec::new();
    let mut result = Vec::new();
    let kind = |x, y| match diff.get_pixel(x, y).0 {
        [255, 255, 0, 255] => Some(true),
        [255, 0, 255, 255] => Some(false),
        _ => None,
    };
    for y in 0..height {
        for x in 0..width {
            if seen[(y * width + x) as usize] || kind(x, y).is_none() {
                continue;
            }
            let mut region = Region {
                x,
                y,
                width: 1,
                height: 1,
                inside: 0,
                outside: 0,
            };
            let (mut right, mut bottom) = (x, y);
            seen[(y * width + x) as usize] = true;
            pending.push((x, y));
            while let Some((px, py)) = pending.pop() {
                region.x = region.x.min(px);
                region.y = region.y.min(py);
                right = right.max(px);
                bottom = bottom.max(py);
                if kind(px, py) == Some(true) {
                    region.inside += 1;
                } else {
                    region.outside += 1;
                }
                for ny in py.saturating_sub(1)..=(py + 1).min(height - 1) {
                    for nx in px.saturating_sub(1)..=(px + 1).min(width - 1) {
                        let index = (ny * width + nx) as usize;
                        if !seen[index] && kind(nx, ny).is_some() {
                            seen[index] = true;
                            pending.push((nx, ny));
                        }
                    }
                }
            }
            region.width = right - region.x + 1;
            region.height = bottom - region.y + 1;
            result.push(region);
        }
    }
    result
}

#[test]
fn regions_keep_diagonal_text_and_geometry_counts() {
    let mut image = RgbaImage::new(5, 3);
    image.put_pixel(0, 0, image::Rgba([255, 255, 0, 255]));
    image.put_pixel(1, 1, image::Rgba([255, 0, 255, 255]));
    image.put_pixel(4, 2, image::Rgba([255, 0, 255, 255]));
    assert_eq!(
        regions(&image),
        vec![
            Region {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                inside: 1,
                outside: 1
            },
            Region {
                x: 4,
                y: 2,
                width: 1,
                height: 1,
                inside: 0,
                outside: 1
            },
        ]
    );
}
