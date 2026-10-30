//! Rail `Plugins` (docs/flows.md §14.1 row 4): the rail opens `/plugins` on the app-server
//! catalog, and the directory reads, installs and uninstalls a plugin through the original's
//! `plugin/list`, `plugin/read`, `plugin/install` and `plugin/uninstall` requests.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::plugins::{Action, Page, Plugin, Resource};

const RAIL_PLUGINS: (f32, f32) = (26., 202.);
const LINEAR: &str = "linear@openai-curated";
/// Linear's row under the Customize sidebar's `Installed`, after the three bundled plugins.
const INSTALLED_LINEAR: (f32, f32) = (150., 312.);
const LINEAR_REMOTE_ID: &str = "plugin_asdk_app_69a089a326dc8191b32a3f2553f5be2c";

#[test]
fn proxy() {
    proxy_child();
}

/// The recorded `plugin/list` result, with Linear's marketplace served remotely as the signed-in
/// app-server does (no `path`, a `remotePluginId`), and Linear's `installed` set to `installed`.
fn catalog(installed: bool) -> Value {
    let mut catalog: Value = ui::fixture::response("readback.jsonl", "plugin/list", 0);
    for market in catalog["marketplaces"].as_array_mut().into_iter().flatten() {
        let Some(plugins) = market["plugins"].as_array_mut() else {
            continue;
        };
        let Some(plugin) = plugins.iter_mut().find(|plugin| plugin["id"] == LINEAR) else {
            continue;
        };
        plugin["installed"] = json!(installed);
        plugin["enabled"] = json!(installed);
        plugin["remotePluginId"] = json!(LINEAR_REMOTE_ID);
        plugin["source"] = json!({"type": "remote"});
        market["path"] = Value::Null;
    }
    catalog
}

fn linear(fx: &Fx, page: &gpui::Entity<Page>) -> Option<Arc<Plugin>> {
    fx.read(|_, cx| {
        page.read(cx)
            .data
            .plugins
            .value()
            .into_iter()
            .flatten()
            .flat_map(|market| &market.plugins)
            .find(|plugin| plugin.id() == LINEAR)
            .cloned()
    })
}

fn page(fx: &Fx) -> Result<gpui::Entity<Page>> {
    fx.read(|root, _| root.plugins_page().cloned())
        .context("plugin page")
}

/// Waits until the current page has no request in flight and its catalog and any detail are read.
fn settle(fx: &mut Fx, what: &str) -> Result<gpui::Entity<Page>> {
    fx.wait(what, |fx| {
        fx.read(|root, cx| {
            root.plugins_page().is_some_and(|page| {
                let page = page.read(cx);
                !page.pending
                    && matches!(page.data.plugins, Resource::Ready(_))
                    && (page.detail.is_none() || !matches!(page.detail_data, Resource::Loading))
            })
        })
    })?;
    page(fx)
}

fn act(fx: &mut Fx, page: &gpui::Entity<Page>, action: Action) {
    fx.cx
        .update(|cx| page.update(cx, |page, cx| page.act(action, cx)));
}

#[test]
fn rail_plugins_lists_reads_installs_and_uninstalls() -> Result<()> {
    let skills: Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let mut lines = script(None, |_| {})?;
    // Every directory load reads the remote catalog below instead of the recorded local one.
    let send = lines
        .iter()
        .position(|line| line["dir"] == "send" && line["msg"]["method"] == "plugin/list")
        .context("recorded plugin/list")?;
    let id = lines[send]["msg"]["id"].clone();
    let reply = send
        + lines[send..]
            .iter()
            .position(|line| line["dir"] == "recv" && line["msg"]["id"] == id)
            .context("recorded plugin/list result")?;
    lines.remove(reply);
    lines.remove(send);
    // Four directory loads: the rail, the detail route, after install, after uninstall.
    for _ in 0..4 {
        lines.extend(exchange("skills/list", skills.clone()));
    }
    for installed in [false, false, true, false] {
        lines.extend(exchange("plugin/list", catalog(installed)));
    }
    for _ in 0..3 {
        lines.extend(exchange(
            "plugin/read",
            json!({"plugin": {"summary": {"id": LINEAR}, "skills": [], "apps": []}}),
        ));
    }
    lines.extend(exchange(
        "plugin/install",
        json!({"authPolicy": "ON_USE", "appsNeedingAuth": []}),
    ));
    lines.extend(exchange("plugin/uninstall", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;

    fx.click(RAIL_PLUGINS)?;
    ensure!(
        fx.route() == app::Route::Plugins("/plugins".into()),
        "route {:?}",
        fx.route()
    );
    ensure!(fx.shell(|shell| shell.selected_rail == Some("plugins") && !shell.new_tab));
    let directory = settle(&mut fx, "plugin catalog")?;
    let plugin = linear(&fx, &directory).context("recorded Linear plugin")?;
    ensure!(!plugin.installed());

    act(&mut fx, &directory, Action::Read(plugin.clone()));
    let detail = settle(&mut fx, "plugin detail")?;
    ensure!(fx.route() == app::Route::Plugins(format!("/plugins/{LINEAR}")));
    fx.read(|_, cx| {
        let page = detail.read(cx);
        ensure!(page.detail.as_ref().is_some_and(|p| p.id() == LINEAR));
        ensure!(
            page.detail_data
                .value()
                .is_some_and(|d| d["summary"]["id"] == LINEAR)
        );
        Ok::<_, anyhow::Error>(())
    })?;

    act(&mut fx, &detail, Action::Install(plugin));
    fx.wait("installed catalog", |fx| {
        page(fx)
            .ok()
            .and_then(|page| linear(fx, &page))
            .is_some_and(|p| p.installed())
    })?;
    let installed_page = settle(&mut fx, "installed detail")?;
    let installed = linear(&fx, &installed_page).context("installed Linear")?;
    ensure!(installed.can_uninstall());

    act(&mut fx, &installed_page, Action::Uninstall(installed));
    fx.wait("uninstalled catalog", |fx| {
        page(fx)
            .ok()
            .and_then(|page| linear(fx, &page))
            .is_some_and(|p| !p.installed())
    })?;
    let final_page = settle(&mut fx, "uninstalled detail")?;
    ensure!(fx.read(|_, cx| final_page.read(cx).error.is_none()));

    let sent = fx.sent()?;
    let params = |method: &str| -> Vec<Value> {
        sent.iter()
            .filter(|message| message["method"] == method)
            .map(|message| message["params"].clone())
            .collect()
    };
    // The original's remote params (plugin-installation-content-*.js): the backend's
    // `/ps/plugins/{pluginName}/install` answers 404 for the plain name `linear`.
    let expected = json!({
        "pluginName": LINEAR_REMOTE_ID,
        "remoteMarketplaceName": "openai-curated",
    });
    ensure!(
        params("plugin/install") == [expected.clone()],
        "{:?}",
        params("plugin/install")
    );
    ensure!(params("plugin/read").iter().all(|p| *p == expected));
    // The catalog `id` answers `{}` but leaves a remote plugin installed.
    ensure!(
        params("plugin/uninstall") == [json!({"pluginId": LINEAR_REMOTE_ID})],
        "{:?}",
        params("plugin/uninstall")
    );
    ensure!(params("plugin/list").len() == 4);
    Ok(())
}

/// The Customize sidebar's `Installed` section lists the installed Linear plugin, and its row
/// opens the plugin's detail.
#[test]
fn customize_installed_opens_an_installed_plugin() -> Result<()> {
    let skills: Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let mut lines = script(None, |_| {})?;
    let send = lines
        .iter()
        .position(|line| line["dir"] == "send" && line["msg"]["method"] == "plugin/list")
        .context("recorded plugin/list")?;
    let id = lines[send]["msg"]["id"].clone();
    let reply = send
        + lines[send..]
            .iter()
            .position(|line| line["dir"] == "recv" && line["msg"]["id"] == id)
            .context("recorded plugin/list result")?;
    lines.remove(reply);
    lines.remove(send);
    // Two directory loads: the rail, then the detail route.
    for _ in 0..2 {
        lines.extend(exchange("skills/list", skills.clone()));
        lines.extend(exchange("plugin/list", catalog(true)));
    }
    lines.extend(exchange(
        "plugin/read",
        json!({"plugin": {"summary": {"id": LINEAR}, "skills": [], "apps": []}}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;

    fx.click(RAIL_PLUGINS)?;
    settle(&mut fx, "plugin catalog")?;
    fx.capture_evidence("CGX_PLUGINS_SHOTS", "customize-installed")?;
    fx.click(INSTALLED_LINEAR)?;
    ensure!(
        fx.route() == app::Route::Plugins(format!("/plugins/{LINEAR}")),
        "route {:?}",
        fx.route()
    );
    let detail = settle(&mut fx, "plugin detail")?;
    ensure!(fx.read(|_, cx| {
        detail
            .read(cx)
            .detail
            .as_ref()
            .is_some_and(|p| p.id() == LINEAR)
    }));
    fx.capture_evidence("CGX_PLUGINS_SHOTS", "customize-installed-detail")?;
    Ok(())
}
