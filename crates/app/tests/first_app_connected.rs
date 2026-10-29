#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, bail, ensure};
use app::host::{Host, Request};
use common::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct RefreshHost {
    executor: gpui::BackgroundExecutor,
    domains: Mutex<Vec<Value>>,
}

impl Host for RefreshHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Fetch { url, body, .. } = request else {
            bail!("unexpected host request");
        };
        if url != "vscode://codex/ambient-suggestions-refresh" {
            return Ok(json!({"status":200,"body":"{}"}));
        }
        ensure!(
            !self.executor.is_main_thread(),
            "refresh blocked the UI thread"
        );
        let params: Value = serde_json::from_str(body)?;
        ensure!(params["mode"] == "first-plugin-connect");
        ensure!(params["hostId"] == "local");
        ensure!(params["projectRoot"] == "~");
        self.domains.lock().unwrap().push(params["domain"].clone());
        if params["domain"].is_null() {
            bail!("default refresh failed");
        }
        Ok(json!({"status":200,"body":"{}"}))
    }
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn only_a_known_first_connection_refreshes_both_domains_despite_failure() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(RefreshHost {
        executor: fx.cx.background_executor().clone(),
        domains: Mutex::new(Vec::new()),
    });
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.set_host(host.clone(), cx))
    });
    let connected = vec![json!({"isAccessible":true,"isEnabled":true})];
    let disconnected = vec![
        json!({"isAccessible":true,"isEnabled":false}),
        json!({"isAccessible":false,"isEnabled":true}),
        json!({"isAccessible":true}),
        json!({"isEnabled":true}),
    ];
    fx.cx.update(|cx| {
        fx.root.update(cx, |root, cx| {
            root.first_app_connected(None, &connected, cx);
            root.first_app_connected(Some(&connected), &connected, cx);
            root.first_app_connected(Some(&[]), &disconnected, cx);
            root.first_app_connected(Some(&disconnected), &[], cx);
            root.first_app_connected(Some(&disconnected), &connected, cx);
        });
    });
    fx.wait("both suggestion domains", |_| {
        host.domains.lock().unwrap().len() >= 2
    })?;
    ensure!(*host.domains.lock().unwrap() == vec![Value::Null, json!("science")]);
    Ok(())
}
