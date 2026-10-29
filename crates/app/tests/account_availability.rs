#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::Fx;
use serde_json::json;
use ui::screens::signin::policy::Policy;

#[test]
fn account_read_failure_uses_login_without_recovery_controls() -> Result<()> {
    common::install_panic_hook();
    let mut lines = common::script(None, |_| {})?;
    for row in &mut lines {
        if row["dir"] == "recv" && row["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            row["msg"].as_object_mut().unwrap().remove("result");
            row["msg"]["error"] = json!({"code":-32603,"message":"owned account read failure"});
        }
    }
    lines.extend(common::exchange(
        "configRequirements/read",
        json!({"requirements": null}),
    ));
    lines.extend(common::exchange(
        "config/read",
        ui::fixture::response("readback.jsonl", "config/read", 0),
    ));
    let mut fx = Fx::scripted(lines)?;
    ensure!(fx.read(|root, _| root.signed_out()));
    ensure!(fx.read(|root, _| root.login().is_some()));
    ensure!(fx.route() == Route::Home);
    fx.wait("sign-in methods", |fx| {
        fx.read(|root, cx| {
            root.login().is_some_and(|login| {
                matches!(login.read(cx).policy(), Policy::Ready { chatgpt: true, .. })
            })
        })
    })?;
    fx.capture_evidence("CGX_ACCOUNT_EVIDENCE", "account-read-login")?;
    Ok(())
}

#[test]
fn coding_home_does_not_require_access_inputs() -> Result<()> {
    common::install_panic_hook();
    let mut fx = Fx::new()?;
    ensure!(!fx.read(|root, _| root.signed_out()));
    ensure!(fx.route() == Route::Home);
    fx.focus_composer()?;
    fx.type_text("Coding Home stays available")?;
    ensure!(fx.composer_text()? == "Coding Home stays available");
    fx.capture_evidence("CGX_ACCOUNT_EVIDENCE", "coding-home")?;
    Ok(())
}
