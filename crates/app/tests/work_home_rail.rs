//! The Work home rail's `Files`, `Plugins` and run-location controls through the real `Root`:
//! the Library reads and download use the original's requests, plugin picks become chips.
mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::Route;
use app::host::{Host, Identity, Request};
use base64::Engine as _;
use common::{Fx, exchange, profile_row as center, script};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use ui::screens::home::{LibraryFile, LibraryLoad};

const FILES: &str = "work-home-files-control";
const PLUGINS: &str = "composer-work-home-plugins-control";
const RUN: &str = "composer-action-bar-run-location-dropdown";
/// The Library's 45 MiB local limit, plus one byte.
const TOO_LARGE: u64 = 47_185_921;

/// The ChatGPT Library in the desktop fetch wire format: `POST /files/library` (500 while
/// `fail`), one `/files/download` URL and that file's bytes.
#[derive(Default)]
struct Library {
    fail: Mutex<bool>,
    bodies: Mutex<Vec<Value>>,
}

fn wire(status: u16, body: Value) -> Value {
    json!({"value": {"response": {"__response": {
        "status": status, "statusText": "",
        "headers": [["content-type", "application/json"]],
        "body": body.to_string(),
    }}}})
}

impl Host for Library {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(http) = request else {
            bail!("unexpected host request: {request:?}");
        };
        match (
            http["method"].as_str(),
            http["url"].as_str().unwrap_or_default(),
        ) {
            (Some("POST"), "/files/library") => {
                let body: Value = serde_json::from_str(http["body"].as_str().context("body")?)?;
                self.bodies.lock().unwrap().push(body.clone());
                if *self.fail.lock().unwrap() {
                    return Ok(wire(500, json!({"detail": "error"})));
                }
                let plan = json!({"id": "libfile_plan", "file_id": "file_plan",
                    "file_name": "Q3 plan.txt", "file_size_bytes": 7});
                let video = json!({"id": "libfile_video", "file_id": "file_video",
                    "file_name": "launch.mov", "file_size_bytes": TOO_LARGE});
                let items = match body["q"].as_str() {
                    None => vec![plan, video],
                    Some("plan") => vec![plan],
                    Some(_) => vec![],
                };
                Ok(wire(200, json!({"items": items, "cursor": null})))
            }
            (Some("GET"), "/files/download/file_plan?inline=true") => Ok(wire(
                200,
                json!({"status": "success", "download_url": "https://files.example/plan"}),
            )),
            (Some("GET"), "https://files.example/plan") => {
                ensure!(
                    http["headers"] == json!({}),
                    "credentials sent to another origin"
                );
                Ok(json!({"value": {"response": {"__response": {
                    "status": 200, "statusText": "", "headers": [],
                    "base64": base64::engine::general_purpose::STANDARD.encode("Q3 plan"),
                }}}}))
            }
            (method, url) => bail!("unexpected host request: {method:?} {url}"),
        }
    }

    fn identity(&self) -> Option<Identity> {
        Some(Identity {
            account_id: "00000000-0000-4000-8000-000000000000".into(),
            user_id: "user-000000000000000000000000".into(),
            device_id: "device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        Ok(Vec::new())
    }
}

/// Clicks the center of the element with debug selector `id`.
fn tap(fx: &mut Fx, id: &str) -> Result<()> {
    let at = center(fx, id)?;
    fx.click(at)
}

/// Draws frames until `id` stops moving: a menu follows its trigger one frame late.
fn settle(fx: &mut Fx, id: &str) -> Result<()> {
    let mut last = center(fx, id)?;
    for _ in 0..10 {
        fx.advance(std::time::Duration::ZERO)?;
        let now = center(fx, id)?;
        if now == last {
            return Ok(());
        }
        last = now;
    }
    bail!("{id} keeps moving")
}

fn files(fx: &Fx) -> (bool, String, LibraryLoad) {
    fx.read(|root, cx| {
        let files = &root.home().read(cx).work.files;
        (files.open, files.query.clone(), files.load.clone())
    })
}

fn names(load: &LibraryLoad) -> Vec<String> {
    match load {
        LibraryLoad::Ready(files) => files.iter().map(|f: &LibraryFile| f.name.clone()).collect(),
        _ => Vec::new(),
    }
}

fn attached(fx: &Fx) -> Vec<std::path::PathBuf> {
    fx.read(|root, cx| {
        let home = root.home().read(cx);
        let composer = home.composer.read(cx);
        home.work
            .files
            .attached
            .iter()
            .filter(|(_, path)| composer.has_attachment(path))
            .map(|(_, path)| path.clone())
            .collect()
    })
}

#[test]
fn files_menu_reads_searches_and_attaches_library_files() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("fs/createDirectory", json!({})));
    lines.extend(exchange("fs/writeFile", json!({})));
    lines.extend(exchange("fs/remove", json!({})));
    let mut fx = Fx::scripted(lines)?;
    let host = Arc::new(Library::default());
    *host.fail.lock().unwrap() = true;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("ctrl-2")?;

    // A failed read shows `Couldn't load Library files`; the request is the original's body.
    tap(&mut fx, FILES)?;
    fx.wait("refused read", |fx| files(fx).2 == LibraryLoad::Failed)?;
    ensure!(
        host.bodies.lock().unwrap()[0]
            == json!({"limit": 10, "cursor": null, "state": "ready", "q": null})
    );
    fx.keys("escape")?;
    ensure!(!files(&fx).0, "Escape closes the menu");

    // Reopening reads again.
    *host.fail.lock().unwrap() = false;
    tap(&mut fx, FILES)?;
    fx.wait("recent files", |fx| {
        matches!(files(fx).2, LibraryLoad::Ready(_))
    })?;
    ensure!(names(&files(&fx).2) == ["Q3 plan.txt", "launch.mov"]);
    fx.capture_evidence("CGX_WORK_RAIL_SHOTS", "files-menu")?;

    // A file over the local limit stays disabled.
    tap(&mut fx, "library-file-1")?;
    fx.cx.run_until_parked();
    ensure!(attached(&fx).is_empty(), "the large file stays disabled");

    // The search asks the server with `q` after the debounce.
    fx.type_text("plan")?;
    fx.advance(std::time::Duration::from_millis(300))?;
    fx.wait("searched files", |fx| {
        let (_, query, load) = files(fx);
        query == "plan" && matches!(load, LibraryLoad::Ready(_))
    })?;
    ensure!(host.bodies.lock().unwrap().last().unwrap()["q"] == "plan");
    ensure!(names(&files(&fx).2) == ["Q3 plan.txt"]);

    // Picking downloads the file and attaches it; picking it again removes it.
    tap(&mut fx, "library-file-0")?;
    fx.wait("attached file", |fx| !attached(fx).is_empty())?;
    let path = attached(&fx).remove(0);
    ensure!(path.ends_with("Q3 plan.txt"), "{path:?}");
    ensure!(files(&fx).0, "the menu stays open for more picks");
    // The attachment card grew the composer: the menu follows the moved pill.
    settle(&mut fx, "library-file-0")?;
    tap(&mut fx, "library-file-0")?;
    ensure!(attached(&fx).is_empty(), "the second pick removes the file");
    Ok(())
}

#[test]
fn browse_all_opens_the_library_dialog_and_adds_picked_files() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("fs/createDirectory", json!({})));
    lines.extend(exchange("fs/writeFile", json!({})));
    lines.extend(exchange("fs/remove", json!({})));
    let mut fx = Fx::scripted(lines)?;
    let host = Arc::new(Library::default());
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("ctrl-2")?;
    tap(&mut fx, FILES)?;
    fx.wait("recent files", |fx| {
        matches!(files(fx).2, LibraryLoad::Ready(_))
    })?;
    tap(&mut fx, "work-files-browse-0")?;
    let browsed = |fx: &Fx| {
        fx.read(|root, cx| {
            let browse = &root.home().read(cx).work.browse;
            browse.open && matches!(browse.load, LibraryLoad::Ready(_))
        })
    };
    fx.wait("dialog files", |fx| browsed(fx))?;
    ensure!(!files(&fx).0, "the menu closes");
    ensure!(host.bodies.lock().unwrap().last().unwrap()["limit"] == 50);
    fx.capture_evidence("CGX_WORK_RAIL_SHOTS", "browse-dialog")?;

    tap(&mut fx, "library-browse-file-0")?;
    tap(&mut fx, "library-browse-add")?;
    fx.wait("attached file", |fx| !attached(fx).is_empty())?;
    ensure!(attached(&fx)[0].ends_with("Q3 plan.txt"));
    ensure!(!fx.read(|root, cx| root.home().read(cx).work.browse.open));
    Ok(())
}

fn plugin(id: &str, title: &str, installed: bool) -> Value {
    json!({
        "id": id, "name": id, "installed": installed, "enabled": installed,
        "interface": { "displayName": title, "shortDescription": format!("{title} plugin") },
    })
}

#[test]
fn plugins_picker_mentions_connected_plugins() -> Result<()> {
    let list = json!({ "marketplaces": [{ "name": "openai-curated", "plugins": [
        plugin("github@openai-curated", "GitHub", true),
        plugin("gmail@openai-curated", "Gmail", false),
    ] }] });
    let mut lines = script(None, |_| {})?;
    lines.retain(|l| l["msg"]["method"] != "plugin/list");
    for _ in 0..3 {
        lines.extend(exchange("plugin/list", list.clone()));
    }
    let mut fx = Fx::scripted(lines)?;
    fx.keys("ctrl-2")?;
    let loaded = |fx: &Fx| {
        fx.read(|root, cx| {
            let plugins = &root.home().read(cx).work.plugins;
            plugins.open && matches!(plugins.load, Some(Ok(_)))
        })
    };
    let mentioned =
        |fx: &Fx| fx.read(|root, cx| root.home().read(cx).composer.read(cx).mentioned_plugins());

    tap(&mut fx, PLUGINS)?;
    fx.wait("connected plugins", |fx| loaded(fx))?;
    fx.capture_evidence("CGX_WORK_RAIL_SHOTS", "plugins-menu")?;
    // Only the installed, enabled GitHub is listed.
    ensure!(center(&mut fx, "work-plugin-1").is_err());
    tap(&mut fx, "work-plugin-0")?;
    ensure!(mentioned(&fx) == ["github@openai-curated"]);
    ensure!(!fx.read(|root, cx| root.home().read(cx).work.plugins.open));

    // The checked row unchecks: its chip goes.
    tap(&mut fx, PLUGINS)?;
    fx.wait("connected plugins", |fx| loaded(fx))?;
    tap(&mut fx, "work-plugin-0")?;
    ensure!(mentioned(&fx).is_empty());

    // `Connect plugins` ▸ `Browse all plugins` opens the directory.
    tap(&mut fx, PLUGINS)?;
    fx.wait("connected plugins", |fx| loaded(fx))?;
    tap(&mut fx, "work-plugins-connect-0")?;
    tap(&mut fx, "work-plugins-browse-0")?;
    ensure!(matches!(fx.route(), Route::Plugins(_)), "{:?}", fx.route());
    Ok(())
}

#[test]
fn run_location_menu_keeps_work_on_this_computer() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("ctrl-2")?;
    fx.capture_evidence("CGX_WORK_RAIL_SHOTS", "work-home")?;
    let open = |fx: &Fx| fx.read(|root, cx| root.home().read(cx).work.run_open);
    tap(&mut fx, RUN)?;
    ensure!(open(&fx));
    fx.capture_evidence("CGX_WORK_RAIL_SHOTS", "run-location-menu")?;
    // `In the cloud` is disabled: this app runs no cloud Work.
    tap(&mut fx, "run-location-cloud")?;
    ensure!(open(&fx));
    tap(&mut fx, "run-location-local")?;
    ensure!(!open(&fx));
    Ok(())
}
