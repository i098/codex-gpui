#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::{Value, json};
use ui::screens::plugins::{Action, Modal, Resource};

#[test]
fn proxy() {
    proxy_child();
}

fn catalog_fixture() -> Result<Vec<Value>> {
    let mut lines = script(None, |_| {})?;
    let skills: Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    lines.extend(exchange("skills/list", skills));
    Ok(lines)
}

#[test]
fn failed_install_keeps_the_recorded_plugin_uninstalled() -> Result<()> {
    let mut lines = catalog_fixture()?;
    lines.push(json!({"dir":"send","msg":{"id":900,"method":"plugin/install","params":{}}}));
    lines.push(json!({"dir":"recv","msg":{"id":900,"error":{"code":-32000,"message":"Install denied by policy"}}}));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    fx.wait("plugin catalog", |fx| {
        fx.read(|root, cx| {
            root.plugins_page()
                .is_some_and(|page| matches!(page.read(cx).data.plugins, Resource::Ready(_)))
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("plugin page")?;
    let plugin = fx
        .read(|_, cx| {
            page.read(cx)
                .data
                .plugins
                .value()
                .into_iter()
                .flatten()
                .flat_map(|market| &market.plugins)
                .find(|plugin| plugin.id() == "linear@openai-curated")
                .cloned()
        })
        .context("recorded Linear plugin")?;
    ensure!(!plugin.installed());
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.modal = Some(Modal::Install(plugin.clone()));
            page.act(Action::Install(plugin.clone()), cx);
        })
    });
    fx.wait("failed installation", |fx| {
        fx.read(|_, cx| !page.read(cx).pending)
    })?;
    fx.read(|_, cx| {
        let state = page.read(cx);
        ensure!(
            state
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Install denied by policy"))
        );
        ensure!(matches!(state.modal, Some(Modal::Install(_))));
        ensure!(
            !state
                .data
                .plugins
                .value()
                .unwrap()
                .iter()
                .flat_map(|market| &market.plugins)
                .find(|plugin| plugin.id() == "linear@openai-curated")
                .unwrap()
                .installed()
        );
        Ok::<_, anyhow::Error>(())
    })?;
    if let Some(directory) = std::env::var_os("CGX_PLUGINS_PROOF") {
        std::fs::create_dir_all(&directory)?;
        fx.shot()?
            .save(std::path::Path::new(&directory).join("failed-install.png"))?;
    }
    Ok(())
}

#[test]
fn new_skill_prepares_an_editable_draft_without_sending_a_turn() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), catalog_fixture()?)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    fx.wait("skills loaded", |fx| {
        fx.read(|root, cx| {
            root.plugins_page()
                .is_some_and(|page| matches!(page.read(cx).data.skills, Resource::Ready(_)))
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("plugin page")?;
    fx.cx
        .update(|cx| page.update(cx, |page, cx| page.act(Action::NewSkill, cx)));
    fx.wait("new skill draft", |fx| fx.route() == app::Route::Home)?;
    let draft = fx.composer_text()?;
    ensure!(draft.starts_with("[$skill-creator](") && draft.ends_with("help me create a skill"));
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message["method"] != "turn/start")
    );
    Ok(())
}

#[test]
fn opening_share_does_not_publish_a_plugin() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), catalog_fixture()?)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    fx.wait("plugin catalog", |fx| {
        fx.read(|root, cx| {
            root.plugins_page()
                .is_some_and(|page| matches!(page.read(cx).data.plugins, Resource::Ready(_)))
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("plugin page")?;
    let plugin = fx.read(|_, cx| {
        let state = page.read(cx);
        let mut plugin = (**state
            .data
            .plugins
            .value()
            .unwrap()
            .iter()
            .flat_map(|market| &market.plugins)
            .find(|plugin| plugin.id() == "chrome@openai-bundled")
            .unwrap())
        .clone();
        plugin.marketplace_path = Some(
            state
                .data
                .personal_marketplace
                .as_ref()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        );
        std::sync::Arc::new(plugin)
    });
    fx.cx
        .update(|cx| page.update(cx, |page, cx| page.act(Action::Share(plugin), cx)));
    fx.wait("share dialog", |fx| {
        fx.read(|_, cx| matches!(page.read(cx).modal, Some(Modal::Share(_))))
    })?;
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message["method"] != "plugin/share/save")
    );
    Ok(())
}

/// The search box sends the original's `plugin/search`; `See more` loads the next page with the
/// returned cursor.
#[test]
fn plugin_search_sends_the_query_and_pages_the_catalog() -> Result<()> {
    let entry = |id: &str| {
        json!({
            "marketplaceName": "openai-curated",
            "marketplacePath": "/tmp/curated/.agents/plugins/marketplace.json",
            "plugin": {"id": id, "name": id, "installed": false, "enabled": false},
        })
    };
    let mut lines = catalog_fixture()?;
    lines.extend(exchange(
        "plugin/search",
        json!({
            "data": [entry("linear@openai-curated"), entry("notion@openai-curated")],
            "nextCursor": "page-2",
        }),
    ));
    lines.extend(exchange(
        "plugin/search",
        json!({"data": [entry("slack@openai-curated")], "nextCursor": null}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    fx.wait("plugin catalog", |fx| {
        fx.read(|root, cx| {
            root.plugins_page()
                .is_some_and(|page| matches!(page.read(cx).data.plugins, Resource::Ready(_)))
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("plugin page")?;
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.query
                .update(cx, |query, cx| query.set_text("  line ", cx))
        })
    });
    fx.advance(std::time::Duration::from_millis(250))?;
    fx.wait("first search page", |fx| {
        fx.read(|_, cx| {
            page.read(cx)
                .search
                .as_ref()
                .is_some_and(|search| matches!(search.results, Resource::Ready(_)))
        })
    })?;
    let (found, next) = fx.read(|_, cx| {
        let search = page.read(cx).search.as_ref().unwrap();
        let Resource::Ready(found) = &search.results else {
            panic!("search results are not ready");
        };
        (
            found
                .iter()
                .map(|plugin| plugin.id().to_owned())
                .collect::<Vec<_>>(),
            search.next.clone(),
        )
    });
    ensure!(
        found == ["linear@openai-curated", "notion@openai-curated"],
        "found {found:?}"
    );
    ensure!(next.as_deref() == Some("page-2"));

    let sent = fx.sent()?;
    let requests: Vec<&Value> = sent
        .iter()
        .filter(|message| message["method"] == "plugin/search")
        .collect();
    ensure!(requests.len() == 1, "sent {} searches", requests.len());
    let params = &requests[0]["params"];
    ensure!(params["searchTerm"] == "line", "trimmed term: {params}");
    ensure!(params["scope"] == "global", "public scope: {params}");
    ensure!(
        params["limit"] == 50 && params["cursor"].is_null(),
        "first page: {params}"
    );
    ensure!(
        params["cwds"]
            .as_array()
            .is_some_and(|cwds| cwds.len() == 1),
        "search roots: {params}"
    );

    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.act(Action::SearchMore("line".into(), "page-2".into()), cx)
        })
    });
    fx.wait("second search page", |fx| {
        fx.read(|_, cx| {
            page.read(cx).search.as_ref().is_some_and(|search| {
                search.next.is_none()
                    && matches!(&search.results, Resource::Ready(list) if list.len() == 3)
            })
        })
    })?;
    let sent = fx.sent()?;
    let last = sent
        .iter()
        .filter(|message| message["method"] == "plugin/search")
        .last()
        .context("second plugin/search")?;
    ensure!(
        last["params"]["cursor"] == "page-2",
        "cursor: {}",
        last["params"]
    );
    Ok(())
}
