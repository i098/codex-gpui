#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_SIGNIN_SHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        fx.shot()?.save(directory.join(format!("{name}.png")))?;
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

fn open_setup(path: &str, host: std::sync::Arc<OnboardingHost>) -> Result<Fx> {
    let route = app::onboarding::route(path).context("Unknown onboarding path")?;
    let mut fx = Fx::with_route(route)?;
    fx.wait("initial setup response", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| page.read(cx).error().is_some())
        })
    })?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("tab enter")?;
    fx.wait("onboarding reload", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| !page.read(cx).loading() && page.read(cx).error().is_none())
        })
    })?;
    Ok(fx)
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
    let mut fx = open_setup("/welcome", host.clone())?;
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
    drop(fx);
    let mut fx = open_setup("/onboarding", host.clone())?;
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

#[test]
fn project_menu_opens_and_dismisses_without_creating_a_project() -> Result<()> {
    common::install_panic_hook();
    if std::env::var_os("CGX_WORKSPACE_MENU_CHILD").is_some() {
        return workspace_menu_replay();
    }
    let home = tempfile::tempdir()?;
    let status = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "project_menu_opens_and_dismisses_without_creating_a_project",
            "--nocapture",
        ])
        .env("CGX_WORKSPACE_MENU_CHILD", "1")
        .env("HOME", home.path())
        .env("CODEX_HOME", home.path().join(".codex"))
        .status()?;
    ensure!(
        status.success(),
        "Isolated workspace menu replay failed: {status}"
    );
    Ok(())
}

fn workspace_menu_replay() -> Result<()> {
    use ui::screens::onboarding::Page;

    let route = app::onboarding::route("/select-workspace").context("Workspace route")?;
    let mut fx = Fx::with_route(route)?;
    fx.wait("workspace options", |fx| {
        fx.read(|root, cx| {
            root.onboarding()
                .is_some_and(|page| !page.read(cx).loading())
        })
    })?;
    shot(&mut fx, "workspace-empty")?;
    fx.hover((640., 496.))?;
    fx.click((640., 496.))?;
    ensure!(fx.read(|root, cx| root.onboarding().unwrap().read(cx).project_menu_open()));
    shot(&mut fx, "workspace-add-menu")?;
    fx.keys("escape")?;
    ensure!(!fx.read(|root, cx| root.onboarding().unwrap().read(cx).project_menu_open()));
    ensure!(fx.route() == app::Route::Onboarding(Page::SelectWorkspace));
    Ok(())
}

#[test]
fn account_completion_is_checked_after_a_previous_local_completion() -> Result<()> {
    use common::{exchange, script};
    use ui::screens::onboarding::Page;
    use ui::screens::signin::policy::Policy;

    common::install_panic_hook();
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s034b.json"))?;
    let login = &recording["result"]["login"]["loginStart"];
    let mut lines = script(None, |_| {})?;
    let account = ui::fixture::response("readback.jsonl", "account/read", 0);
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            line["msg"]["result"] = json!({"account":null,"requiresOpenaiAuth":true});
        }
    }
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements":{"allowedLoginMethods":["api","chatgpt"]}}),
    ));
    lines.extend(exchange(
        "account/login/start",
        json!({"type":"chatgpt","loginId":login["loginId"],"authUrl":login["authUrl"]}),
    ));
    lines.extend(exchange("account/read", account));
    let mut settings = common::recorded_host_settings()?;
    settings.write("electron:onboarding-projectless-completed", json!(true))?;
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    let host = std::sync::Arc::new(OnboardingHost {
        context: json!({"status":200,"bodyJsonString":json!({
            "desktop_onboarding_completed_at":null,"role":"engineering"
        }).to_string()}),
        writes: Default::default(),
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.wait("sign-in requirements", |fx| {
        fx.read(|root, cx| {
            root.login().is_some_and(|login| {
                matches!(login.read(cx).policy(), Policy::Ready { chatgpt: true, .. })
            })
        })
    })?;
    fx.keys("tab tab enter")?;
    fx.wait("browser handoff", |fx| fx.cx.take_opened_url().is_some())?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "account/login/completed".into(),
                params: json!({"loginId":login["loginId"],"success":true}),
            },
            cx,
        )
    });
    fx.wait("account-specific onboarding", |fx| {
        fx.route() == app::Route::Onboarding(Page::Welcome)
            && fx.read(|root, cx| {
                root.onboarding()
                    .is_some_and(|page| !page.read(cx).loading() && page.read(cx).error().is_none())
            })
    })?;
    Ok(())
}
