mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use ui::screens::plugins::Modal;
use ui::screens::plugins::{Action, Resource};

#[test]
fn proxy() {
    proxy_child();
}

fn catalog_fixture() -> Result<Vec<Value>> {
    let mut lines = script(None, |_| {})?;
    for method in ["skills/list", "plugin/list", "config/read"] {
        let response: Value = ui::fixture::response("readback.jsonl", method, 0);
        lines.extend(exchange(method, response));
    }
    Ok(lines)
}

#[cfg(target_os = "macos")]
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
    let draft = fx.read(|root, cx| root.composer(cx).read(cx).prompt());
    ensure!(draft.starts_with("[$skill-creator](") && draft.ends_with("help me create a skill"));
    ensure!(
        !fx.composer_text()?.contains("]("),
        "The draft displayed raw mention markup"
    );
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message["method"] != "turn/start")
    );
    Ok(())
}

#[cfg(target_os = "macos")]
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

#[test]
fn unsupported_host_never_reads_local_catalogs() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), catalog_fixture()?)?;
    fx.action(Box::new(ui::OpenPlugins))?;
    fx.wait("local catalog loaded", |fx| {
        fx.read(|root, cx| {
            root.plugins_page()
                .is_some_and(|page| matches!(page.read(cx).data.skills, Resource::Ready(_)))
        })
    })?;
    fx.wait("local manager snapshot", |fx| {
        fx.read(|_, cx| {
            cx.global::<ui::screens::settings::integrations::State>()
                .gates_loaded
        })
    })?;
    let data_reads = |messages: Vec<Value>| {
        messages
            .into_iter()
            .filter(|message| {
                matches!(
                    message["method"].as_str(),
                    Some("plugin/list" | "plugin/installed" | "skills/list" | "app/list")
                )
            })
            .collect::<Vec<_>>()
    };
    let before = data_reads(fx.sent()?);
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("local page")?;
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.act(Action::Navigate("/skills?hostId=unsupported".into()), cx)
        })
    });
    fx.wait("unsupported host", |fx| {
        fx.read(|root, cx| {
            root.plugins_page().is_some_and(|page| {
                page.read(cx).host_id == "unsupported"
                    && matches!(page.read(cx).data.skills, Resource::Failed(_))
            })
        })
    })?;
    let page = fx
        .read(|root, _| root.plugins_page().cloned())
        .context("unsupported page")?;
    fx.read(|_, cx| {
        let page = page.read(cx);
        ensure!(page.data.personal_marketplace.is_none());
        ensure!(matches!(page.data.plugins, Resource::Failed(_)));
        ensure!(matches!(page.data.apps, Resource::Failed(_)));
        Ok::<_, anyhow::Error>(())
    })?;
    fx.cx
        .update(|cx| page.update(cx, |page, cx| page.act(Action::Refresh, cx)));
    fx.cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.act(Action::Navigate("/plugins".into()), cx)
        })
    });
    fx.wait("host-preserving navigation", |fx| {
        fx.read(|root, cx| {
            root.plugins_page().is_some_and(|page| {
                page.read(cx).tab == ui::screens::plugins::Tab::Plugins
                    && page.read(cx).host_id == "unsupported"
                    && matches!(page.read(cx).data.plugins, Resource::Failed(_))
            })
        })
    })?;
    ensure!(
        data_reads(fx.sent()?) == before,
        "Unsupported host read a local catalog"
    );
    Ok(())
}
