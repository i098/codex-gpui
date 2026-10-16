//! TH-073 (S009): a file link in an answer opens the file viewer side tab, as the original
//! recorded in refs/interactions/s009.json. The original's host could not read the file
//! offline (`Could not open file`); the port replays both outcomes: a host that answers
//! `read-file` fills the viewer, a host that fails shows the message and the toast.
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use app::host::{Host, Request};
use common::*;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;

const MATH_PY: &str =
    "def add(a, b):\n    return a + b\n\n\ndef subtract(a, b):\n    return a - b\n";

/// Answers `read-file` with [`MATH_PY`] when `ok`, else fails; records every request.
struct FileHost {
    ok: bool,
    requests: Mutex<Vec<Value>>,
}

impl Host for FileHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Fetch { url, body, .. } = request else {
            anyhow::bail!("unexpected request");
        };
        self.requests
            .lock()
            .push(json!({ "url": url, "body": serde_json::from_str::<Value>(body)? }));
        ensure!(self.ok && url == "vscode://codex/read-file", "no such file");
        Ok(
            json!({"type": "fetch-response", "responseType": "success", "status": 200,
            "headers": {"content-type": "application/json"},
            "bodyJsonString": json!({ "contents": MATH_PY }).to_string()}),
        )
    }
}

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
    let host = Arc::new(FileHost {
        ok: true,
        requests: Mutex::new(Vec::new()),
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));

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
    let requests = host.requests.lock().clone();
    ensure!(
        requests.len() == 1
            && requests[0]["url"] == "vscode://codex/read-file"
            && requests[0]["body"]["path"] == path,
        "{requests:?}"
    );
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

    // The original's outcome offline: the host fails, the viewer says so, and a toast repeats it.
    let failing = Arc::new(FileHost {
        ok: false,
        requests: Mutex::new(Vec::new()),
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(failing, cx));
    // The open panel narrowed the chat: close its only tab, so the link is back at the
    // reference spot.
    fx.action(Box::new(ui::screens::diff::panel::CloseSideTab(0)))?;
    fx.click(link)?;
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
