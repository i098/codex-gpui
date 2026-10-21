#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{Fx, exchange, script};
use gpui::AppContext as _;
use serde_json::{Value, json};
use ui::screens::signin::policy::Policy;

#[test]
fn proxy() {
    common::proxy_child();
}

fn signed_out(line: &mut Value) {
    if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
        line["msg"]["result"] = json!({"account":null,"requiresOpenaiAuth":true});
    }
    if line["dir"] == "send" && line["msg"]["method"] == "thread/list" {
        line["msg"]["params"] = json!({});
    }
}

fn policy(fx: &Fx) -> Option<Policy> {
    fx.read(|root, cx| root.login().map(|login| login.read(cx).policy()))
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_SIGNIN_SHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        fx.shot()?.save(directory.join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn managed_signin_restrictions_hide_methods_and_api_paste_submits_once() -> Result<()> {
    common::install_panic_hook();
    for (requirements, expected, name) in [
        (json!({}), Policy::Unsupported, "requirements-old-cli"),
        (
            json!({"allowedLoginMethods":[]}),
            Policy::Ready {
                api: false,
                chatgpt: false,
            },
            "requirements-disabled",
        ),
        (
            json!({"allowedLoginMethods":["unknown"]}),
            Policy::Error,
            "requirements-error",
        ),
        (
            json!({"allowedLoginMethods":["api"]}),
            Policy::Ready {
                api: true,
                chatgpt: false,
            },
            "requirements-api-only",
        ),
    ] {
        let mut lines = script(None, signed_out)?;
        lines.extend(exchange(
            "configRequirements/read",
            json!({"requirements":requirements}),
        ));
        let mut refused = exchange("account/login/start", Value::Null);
        refused[1]["msg"].as_object_mut().unwrap().remove("result");
        refused[1]["msg"]["error"] = json!({"code":-32000,"message":"Invalid fixture API key"});
        lines.extend(refused);
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        fx.wait("managed sign-in policy", |fx| policy(fx) == Some(expected))?;
        shot(&mut fx, name)?;
        if expected
            != (Policy::Ready {
                api: true,
                chatgpt: false,
            })
        {
            continue;
        }
        fx.keys("tab tab enter")?;
        let field = fx.read(|root, cx| root.login().unwrap().read(cx).api_key_field().clone());
        fx.cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "  sk-fixture-rejected  ".into(),
        ));
        fx.keys("cmd-v")?;
        ensure!(field.read(&fx.cx).field_text() == "  sk-fixture-rejected  ");
        shot(&mut fx, "api-key-paste")?;
        fx.keys("enter")?;
        fx.wait("API error", |fx| {
            fx.read(|root, cx| {
                root.login()
                    .is_some_and(|login| login.read(cx).error().is_some())
            })
        })?;
        shot(&mut fx, "api-key-error")?;
        let sent = fx.sent()?;
        let attempts: Vec<_> = sent
            .iter()
            .filter(|message| message["method"] == "account/login/start")
            .collect();
        ensure!(attempts.len() == 1);
        ensure!(attempts[0]["params"] == json!({"type":"apiKey","apiKey":"sk-fixture-rejected"}));
        fx.root.update(&mut fx.cx, |root, cx| {
            root.on_event(
                bridge::Event::Notification {
                    method: "account/login/completed".into(),
                    params: json!({"loginId":"unrelated","success":true}),
                },
                cx,
            )
        });
        ensure!(fx.read(|root, _| root.signed_out()));
    }
    Ok(())
}

struct OnboardingHost {
    context: Value,
    writes: std::sync::Mutex<Vec<Value>>,
}

impl app::host::Host for OnboardingHost {
    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        if let app::host::Request::Http(request) = request {
            if request["url"] == "/wham/onboarding/context" {
                return Ok(self.context.clone());
            }
            if request["url"] == "/wham/onboarding/desktop/complete" {
                self.writes.lock().unwrap().push(request.clone());
                anyhow::bail!("Offline completion is denied");
            }
        }
        anyhow::bail!("No recorded response for this onboarding request")
    }
    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        Ok(Vec::new())
    }
}

#[test]
fn onboarding_keeps_failed_saves_and_cancelled_skips_on_the_page() -> Result<()> {
    common::install_panic_hook();
    let original: Value = serde_json::from_str(&std::fs::read_to_string(format!(
        "{}/tools/route1/oracle.json",
        common::ROOT
    ))?)?;
    let context = original
        .as_object()
        .unwrap()
        .iter()
        .find(|(key, _)| key.contains("/wham/onboarding/context"))
        .unwrap()
        .1
        .clone();
    let host = std::sync::Arc::new(OnboardingHost {
        context,
        writes: Default::default(),
    });
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.show_onboarding(
                ui::screens::onboarding::Page::Welcome,
                window,
                cx,
            )
        })
    })?;
    fx.wait("onboarding context", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| !page.read(cx).loading())
        })
    })?;
    shot(&mut fx, "onboarding-role")?;
    for _ in 0..14 {
        fx.keys("tab")?;
    }
    fx.keys("enter")?;
    fx.wait("onboarding save error", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| page.read(cx).error().is_some())
        })
    })?;
    ensure!(fx.route() == app::Route::Onboarding(ui::screens::onboarding::Page::Welcome));
    ensure!(host.writes.lock().unwrap().len() == 1);
    shot(&mut fx, "onboarding-save-error")?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.show_onboarding(
                ui::screens::onboarding::Page::Onboarding,
                window,
                cx,
            )
        })
    })?;
    fx.wait("second onboarding context", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| !page.read(cx).loading())
        })
    })?;
    for _ in 0..15 {
        fx.keys("tab")?;
    }
    fx.keys("enter")?;
    ensure!(fx.read(|root, cx| root.onboarding().unwrap().read(cx).confirming_skip()));
    shot(&mut fx, "onboarding-skip-dialog")?;
    fx.keys("escape")?;
    ensure!(!fx.read(|root, cx| root.onboarding().unwrap().read(cx).confirming_skip()));
    ensure!(host.writes.lock().unwrap().len() == 1);
    Ok(())
}
