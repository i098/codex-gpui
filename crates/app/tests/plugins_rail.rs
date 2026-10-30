//! Rail `Plugins` (docs/flows.md §14.1 row 4): the rail opens `/plugins` on the app-server
//! catalog, and the directory reads, installs and uninstalls a plugin through the original's
//! `plugin/list`, `plugin/read`, `plugin/install` and `plugin/uninstall` requests.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use bridge::Event;
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::plugins::{Action, Page, Plugin, Resource};

const RAIL_PLUGINS: (f32, f32) = (26., 202.);
const LINEAR: &str = "linear@openai-curated";
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

#[test]
fn remote_plugin_skill_preview_reads_server_content() -> Result<()> {
    let path = "/remote/linear/SKILL.md";
    let content = "# Linear\n\nRemote preview content.";
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "plugin/skill/read",
        json!({"skill_md_contents": content}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(RAIL_PLUGINS)?;
    let page = settle(&mut fx, "plugin catalog")?;
    fx.capture_evidence("CGX_PLUGIN_EVIDENCE_DIR", "remote-skill-before")?;

    let skill = json!({
        "name": "linear",
        "path": path,
        "description": "Remote Linear skill",
        "remotePluginSkill": {
            "remotePluginId": LINEAR_REMOTE_ID,
            "remoteMarketplaceName": "openai-curated",
            "skillName": "linear",
        },
    });
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.modal = Some(ui::screens::plugins::Modal::Skill(skill));
            cx.notify();
        })
    });
    act(&mut fx, &page, Action::ReadSkill(path.to_owned()));
    fx.wait("remote skill contents", |fx| {
        fx.read(|_, cx| {
            page.read(cx).skill_file.as_ref().is_some_and(|file| {
                file.read(cx)
                    .contents
                    .as_ref()
                    .is_some_and(|contents| contents.as_ref().is_ok_and(|text| text == content))
            })
        })
    })?;
    fx.capture_evidence("CGX_PLUGIN_EVIDENCE_DIR", "remote-skill-after")?;
    let sent = fx.sent()?;
    let request = sent
        .iter()
        .find(|request| request["method"] == "plugin/skill/read")
        .context("remote plugin skill read request")?;
    ensure!(
        request["params"]
            == json!({
                "remotePluginId": LINEAR_REMOTE_ID,
                "remoteMarketplaceName": "openai-curated",
                "skillName": "linear",
            }),
        "{request}"
    );
    Ok(())
}

#[test]
fn a_connected_app_reconciles_remote_plugin_suggestions() -> Result<()> {
    let skills: Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("app/list", json!({"data": [], "nextCursor": null})));
    lines.extend(exchange(
        "app/list",
        json!({
            "data": [{"id": "linear", "isAccessible": true}],
            "nextCursor": null,
        }),
    ));
    lines.extend(exchange(
        "plugin/reconcile",
        json!({
            "failedRemotePluginIds": [],
            "failedMaterializationRemotePluginIds": [],
        }),
    ));
    lines.extend(exchange("plugin/list", catalog(false)));
    lines.extend(exchange("skills/list", skills));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(RAIL_PLUGINS)?;
    let page = settle(&mut fx, "plugin catalog")?;
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.modal = Some(ui::screens::plugins::Modal::Connecting(json!({
                "id": "linear",
            })));
            cx.notify();
        })
    });

    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: "app/list/updated".into(),
                params: json!({}),
            },
            cx,
        )
    });
    fx.wait("plugin suggestions reconciled", |fx| {
        fx.read(|_, cx| page.read(cx).modal.is_none())
    })?;
    fx.read(|_, cx| {
        let page = page.read(cx);
        ensure!(page.error.is_none(), "{:?}", page.error);
        ensure!(
            page.data.apps.value().is_some_and(|apps| {
                apps.iter()
                    .any(|app| app["id"] == "linear" && app["isAccessible"] == true)
            }),
            "connected app was not loaded"
        );
        Ok::<_, anyhow::Error>(())
    })?;
    let sent = fx.sent()?;
    let reconciliations = sent
        .iter()
        .filter(|request| request["method"] == "plugin/reconcile")
        .collect::<Vec<_>>();
    ensure!(reconciliations.len() == 1, "{reconciliations:?}");
    ensure!(
        reconciliations[0]["params"] == json!({"reason": "plugin_suggestion_connected"}),
        "{}",
        reconciliations[0]
    );
    Ok(())
}
