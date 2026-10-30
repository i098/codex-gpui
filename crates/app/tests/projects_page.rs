//! Explore ▸ `Projects` (`/projects`, issue #1090) through the real `Root` over a `LocalHost`:
//! the menu opens the page, the page lists the stored `local-projects` newest first, `Edit
//! project` ▸ `Save` renames the stored project, and `Start new chat in project` opens Home in
//! that project.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use app::host::LocalHost;
use common::Fx;
use serde_json::json;
use std::{fs, sync::Arc};
use ui::screens::projects::Load;

const RAIL_EXPLORE: (f32, f32) = (26., 246.);
const MENU_PROJECTS: (f32, f32) = (100., 245.);
/// The first row's `Project actions`, its menu's `Edit project`, and `Start new chat in project`.
const FIRST_ACTIONS: (f32, f32) = (1166., 196.);
const EDIT_PROJECT: (f32, f32) = (1100., 232.);
const FIRST_NEW_CHAT: (f32, f32) = (1202., 196.);

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

#[test]
fn projects_page_lists_renames_and_opens() -> Result<()> {
    let home = tempfile::tempdir()?;
    let proj = home.path().join("proj");
    let website = home.path().join("website");
    fs::create_dir_all(&proj)?;
    fs::create_dir_all(&website)?;
    // Ages against the capture clock, so the rows read `12h` and `1w`.
    let now = common::CAPTURE_CLOCK.pinned_now.unwrap_or_default() * 1000;
    let stored = |id: &str, name: &str, root: &std::path::Path, hours: i64| {
        json!({"id": id, "name": name, "rootPaths": [root], "createdAt": 1,
               "updatedAt": now - hours * 3_600_000})
    };
    fs::write(
        home.path().join(".codex-global-state.json"),
        serde_json::to_vec(&json!({
            "local-projects": {
                "website": stored("website", "Website", &website, 8 * 24),
                "proj": stored("proj", "proj", &proj, 12),
            },
            "project-order": ["website", "proj"],
        }))?,
    )?;
    let mut fx = Fx::new()?;
    fx.click(RAIL_EXPLORE)?;
    ensure!(
        fx.shell(|s| s.explore_menu),
        "Explore did not open its menu"
    );
    fx.click(MENU_PROJECTS)?;
    ensure!(fx.route() == Route::Projects, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.selected_rail == Some("explore") && !s.sidebar_open));
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(LocalHost::new(home.path())), cx)
    });
    fx.wait("projects listed", |fx| {
        fx.read(|root, cx| {
            root.projects_page()
                .is_some_and(|page| matches!(page.read(cx).load, Load::Ready(_)))
        })
    })?;
    ensure!(labels(&fx) == ["proj", "Website"], "rows {:?}", labels(&fx));
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "after-projects")?;

    fx.click(FIRST_ACTIONS)?;
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "after-actions-menu")?;
    fx.click(EDIT_PROJECT)?;
    ensure!(
        fx.read(|root, _| root.new_project().is_some()),
        "Edit project did not open"
    );
    fx.capture_evidence("CGX_PROJECTS_EVIDENCE", "after-edit-dialog")?;
    fx.keys("backspace backspace backspace backspace")?;
    fx.type_text("Renamed")?;
    fx.keys("enter")?;
    fx.wait("renamed row", |fx| {
        labels(fx).first().map(String::as_str) == Some("Renamed")
    })?;
    ensure!(
        fx.read(|root, _| root.new_project().is_none()),
        "dialog stayed open"
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(home.path().join(".codex-global-state.json"))?)?;
    ensure!(
        state["local-projects"]["proj"]["name"] == "Renamed",
        "stored {state}"
    );

    fx.click(FIRST_NEW_CHAT)?;
    ensure!(fx.route() == Route::Home, "new chat: {:?}", fx.route());
    Ok(())
}
