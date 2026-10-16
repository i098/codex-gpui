//! TH-073 (S009): a file link in an answer opens the file viewer side tab, as the original
//! recorded in refs/interactions/s009.json. The original's host could not read the file
//! offline (`Could not open file`); the port replays both outcomes: the fixture file on the
//! Mac (`/tmp/cgx-mac/proj/src/math.py`, the subtract session's result) fills the viewer, a
//! missing file shows the message and the toast.
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::*;
use serde_json::Value;
use ui::screens::diff::panel::{CloseSideTab, OpenFile};

const MATH_PY: &str =
    "def add(a, b):\n    return a + b\n\n\ndef subtract(a, b):\n    return a - b\n";

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn file_link_opens_the_file_viewer_tab() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s009: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s009.json"))?;
    let viewport = &rec["viewport"];
    let mut fx = Fx::captured(
        "dark",
        (
            viewport["w"].as_f64().context("w")? as f32,
            viewport["h"].as_f64().context("h")? as f32,
        ),
        script(None, |_| {})?,
    )?;
    // Step 0: the Recents row; the port's row sits where the Home reference has it.
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(
        fx.read(|r, _| r.side_panel()).is_none(),
        "no side panel at rest"
    );
    let before = fx.shot()?;

    // Steps 1 and 2: hover, then click the link (at the thread reference's position: the
    // recording's Changes tab was open, which narrows its chat column).
    let link = fx.at("thread", "src/math.py")?;
    fx.hover(link)?;
    fx.click(link)?;
    fx.wait("file viewer contents", |fx| {
        fx.read(|r, cx| r.file_view().is_some_and(|f| f.read(cx).contents.is_some()))
    })?;
    fx.hover((1270., 820.))?;
    let after = fx.shot()?;
    after.save(format!("{}/s009-port.png", env!("CARGO_TARGET_TMPDIR")))?;

    let tabs = rec["result"]["dom"]
        .as_array()
        .context("dom")?
        .iter()
        .find(|d| d["selector"] == "[role=tab]" && d["state"] != "rest")
        .context("tab entry")?;
    let selected = tabs["tabs"]
        .as_array()
        .context("tabs")?
        .iter()
        .find(|t| t["selected"] == "true")
        .context("selected tab")?;
    ensure!(
        fx.read(|r, _| r.side_panel()) == Some(SidePanel::File),
        "{:?}",
        fx.read(|r, _| r.side_panel())
    );
    let (name, crumbs, path, contents) = fx.read(|r, cx| {
        let f = r.file_view().unwrap().read(cx);
        (
            f.name().to_string(),
            f.crumbs.join(" > "),
            f.path.clone(),
            f.contents.clone(),
        )
    });
    ensure!(
        name == selected["text"],
        "tab {name:?}, original {}",
        selected["text"]
    );
    let crumb = &rec["result"]["dom"]
        .as_array()
        .context("dom")?
        .iter()
        .find(|d| d["selector"] == "[aria-label=\"File path\"]")
        .context("crumb entry")?["crumbs"][0]["text"];
    ensure!(crumbs == *crumb, "breadcrumb {crumbs:?}, original {crumb}");
    let recorded_path = rec["fixture"]["path"].as_str().context("fixture path")?;
    ensure!(
        path.rsplit("/proj/").next() == recorded_path.rsplit("/proj/").next(),
        "{path:?} vs {recorded_path:?}"
    );
    ensure!(contents == Some(Ok(MATH_PY.into())), "{contents:?}");
    // The panel (the original's tabpanel box) now shows the viewer instead of the chat.
    let panel = &rec["result"]["dom"]
        .as_array()
        .context("dom")?
        .iter()
        .find(|d| d["selector"] == "[role=tabpanel]")
        .context("tabpanel entry")?["panels"][0]["bbox"];
    let rect = [
        panel["x"].as_f64().context("x")? as f32,
        panel["y"].as_f64().context("y")? as f32,
        panel["w"].as_f64().context("w")? as f32,
        panel["h"].as_f64().context("h")? as f32,
    ];
    let diff = changed(&before, &after, rect);
    ensure!(diff > 20_000, "panel region changed by only {diff} px");

    // The original's outcome offline: the read fails, the viewer says so, and a toast repeats
    // it. The port reads a file that does not exist, from the link's own action.
    fx.action(Box::new(CloseSideTab(0)))?;
    fx.action(Box::new(OpenFile {
        path: "src/missing.py".into(),
        line: None,
    }))?;
    fx.wait("file viewer error", |fx| {
        fx.read(|r, cx| {
            r.file_view()
                .is_some_and(|f| matches!(f.read(cx).contents, Some(Err(_))))
        })
    })?;
    let empty = rec["result"]["dom"]
        .as_array()
        .context("dom")?
        .iter()
        .find(|d| d["selector"] == "Could not open file")
        .context("empty entry")?;
    let message = empty["element"]["text"].as_str().context("text")?;
    let toasts = fx.read(|r, cx| r.toasts(cx));
    ensure!(
        toasts.iter().any(|t| t.as_ref() == message),
        "toasts {toasts:?}, original {message:?}"
    );
    println!(
        "s009: file link opens the `{name}` viewer tab; read-file fills it, a failed read says {message:?}"
    );
    Ok(())
}
