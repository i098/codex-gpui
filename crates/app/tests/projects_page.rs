//! Explore ▸ `Projects` (`/projects`, issue #1090) through the real `Root`: the page lists the
//! app-server's `project/list` newest first, `Edit project` ▸ `Save` sends `project/update`
//! and stays on the page, and `Start new chat in project` opens Home in that project.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::LocalHost;
use bridge::Bridge;
use common::{CAPTURE_CLOCK, Fx, exchange, script};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use ui::screens::projects::{Event, Load};

const RAIL_EXPLORE: (f32, f32) = (26., 246.);
/// The first row's `Start new chat in project`.
const FIRST_NEW_CHAT: (f32, f32) = (1164., 196.);

#[test]
fn proxy() {
    common::proxy_child();
}

fn labels(fx: &Fx) -> Vec<String> {
    fx.read(|root, cx| {
        root.projects_page()
            .map(|page| {
                page.read(cx)
                    .rows(cx)
                    .iter()
                    .map(|row| row.label.to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn emit(fx: &mut Fx, event: Event) -> Result<()> {
    let page = fx.read(|root, _| root.projects_page().cloned()).unwrap();
    page.update(&mut fx.cx, |_, cx| cx.emit(event));
    fx.advance(std::time::Duration::ZERO)
}

/// The project's app-server `project/list` entry rooted in `home/<id>`, updated `hours` before
/// the capture clock.
fn project(home: &Path, id: &str, name: &str, hours: i64) -> Value {
    let now = CAPTURE_CLOCK.pinned_now.unwrap_or_default();
    json!({"id": id, "name": name, "roots": [{"path": home.join(id)}],
           "metadata": {}, "createdAt": 1, "updatedAt": now - hours * 3600})
}

#[test]
fn projects_page_lists_renames_and_opens() -> Result<()> {
    let home = tempfile::tempdir()?;
    for id in ["website", "proj"] {
        std::fs::create_dir_all(home.path().join(id))?;
    }
    let list = json!({"data": [project(home.path(), "website", "Website", 8 * 24),
                               project(home.path(), "proj", "proj", 12)],
                      "nextCursor": null});
    // The host's own app-server answers `initialize` and every `project/list` the page and
    // sidebar send.
    let init = json!({"userAgent": "fixture", "codexHome": home.path(), "platformOs": "macos"});
    let replies: Vec<Value> = exchange("initialize", init)
        .into_iter()
        .chain((0..8).flat_map(|_| exchange("project/list", list.clone())))
        .collect();
    let path = tempfile::NamedTempFile::new()?;
    std::fs::write(
        path.path(),
        replies
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )?;
    let (host_bridge, _events) = Bridge::replay(path.path())?;

    let mut lines = script(None, |_| {})?;
    let edited = project(home.path(), "proj", "proj", 12);
    for _ in 0..2 {
        lines.extend(exchange("project/read", json!({"project": edited})));
    }
    lines.extend(exchange("project/update", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            Arc::new(LocalHost::new(home.path()).with_bridge(Arc::new(host_bridge))),
            cx,
        )
    });
    fx.click(RAIL_EXPLORE)?;
    ensure!(
        fx.shell(|s| s.explore_menu),
        "Explore did not open its menu"
    );
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "explore-menu")?;
    fx.click(RAIL_EXPLORE)?;
    fx.action(Box::new(ui::OpenProjects))?;
    ensure!(fx.route() == Route::Projects, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("explore") && !s.sidebar_open));
    fx.wait("projects listed", |fx| {
        fx.read(|root, cx| {
            root.projects_page()
                .is_some_and(|page| matches!(page.read(cx).load, Load::Ready(_)))
        })
    })?;
    ensure!(labels(&fx) == ["proj", "Website"], "rows {:?}", labels(&fx));
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "projects")?;

    emit(&mut fx, Event::Edit("proj".into()))?;
    fx.wait("project editor", |fx| {
        fx.read(|root, _| root.new_project().is_some())
    })?;
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "edit-project")?;
    fx.keys("backspace backspace backspace backspace")?;
    fx.type_text("Renamed")?;
    fx.keys("enter")?;
    fx.wait("project saved", |fx| {
        fx.read(|root, _| root.new_project().is_none())
    })?;
    let update = fx
        .sent()?
        .into_iter()
        .find(|request| request["method"] == "project/update")
        .unwrap();
    ensure!(update["params"]["name"] == "Renamed", "update {update}");
    ensure!(update["params"]["projectId"] == "proj", "update {update}");
    ensure!(
        fx.route() == Route::Projects,
        "rename left the page: {:?}",
        fx.route()
    );

    fx.click(FIRST_NEW_CHAT)?;
    ensure!(fx.route() == Route::Home, "new chat: {:?}", fx.route());
    Ok(())
}
