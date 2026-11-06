//! `configRequirements/read` disables the permission presets that `requirements.toml` does not
//! allow, with the original's `Disabled by requirements.toml` tooltip, and leaves only
//! `Managed` (tooltip `Managed by enterprise policy`) when no preset is allowed.

mod common;

use anyhow::{Result, ensure};
use common::{Fx, exchange, script};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc, time::Duration};
use ui::composer::Permission;

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

fn app(requirements: Value) -> Result<Fx> {
    let mut lines = script(None, |_| {})?;
    lines.retain(|l| l["msg"]["method"] != "configRequirements/read");
    lines.extend(exchange(
        "configRequirements/read",
        json!({ "requirements": requirements }),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    // The defaults enable every preset; the read requirements disable one or leave `Managed`.
    fx.wait("permission rows", |fx| {
        fx.read(|root, cx| {
            let c = root.composer(cx).read(cx);
            c.permission_disabled("Managed").is_some()
                || c.permission_disabled("Full access")
                    == Some(Some("Disabled by requirements.toml"))
        })
    })?;
    Ok(fx)
}

/// Moves the pointer up from the trigger until a tooltip with `label` shows.
fn hover_until(fx: &mut Fx, from: (f32, f32), label: &str) -> Result<bool> {
    for step in 0..40 {
        fx.hover((from.0 + 40., from.1 - 8. * step as f32))?;
        fx.advance(Duration::from_secs(1))?;
        if fx
            .cx
            .update(|cx| ui::tooltip::shown(cx).map(|s| s.label.to_string()))
            .as_deref()
            == Some(label)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[test]
fn requirements_disable_full_access() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = app(json!({ "allowedApprovalPolicies": ["on-request"] }))?;
    let disabled = fx.read(|root, cx| {
        let c = root.composer(cx).read(cx);
        [
            c.permission_disabled("Ask for approval"),
            c.permission_disabled("Full access"),
        ]
    });
    ensure!(
        disabled == [Some(None), Some(Some("Disabled by requirements.toml"))],
        "rows {disabled:?}"
    );
    let trigger = fx.at("home", "Change permissions")?;
    fx.click(trigger)?;
    ensure!(
        hover_until(&mut fx, trigger, "Disabled by requirements.toml")?,
        "no disabled tooltip"
    );
    if let Some(dir) = std::env::var_os("CGX_PERMISSION_EVIDENCE") {
        std::fs::create_dir_all(&dir)?;
        fx.shot()?
            .save(std::path::Path::new(&dir).join("full-access-disabled.png"))?;
    }
    let before = fx.read(|root, cx| root.composer(cx).read(cx).permission());
    fx.keys("f u l l enter")?;
    ensure!(
        fx.read(|root, cx| root.composer(cx).read(cx).permission()) == before,
        "a disabled option was picked"
    );
    Ok(())
}

#[test]
fn no_allowed_preset_leaves_managed() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = app(json!({ "allowedApprovalPolicies": ["untrusted"] }))?;
    ensure!(
        fx.read(|root, cx| root.composer(cx).read(cx).permission()) == Permission::Managed,
        "not managed"
    );
    let trigger = fx.at("home", "Change permissions")?;
    fx.hover(trigger)?;
    fx.advance(Duration::from_secs(1))?;
    let tip = fx
        .cx
        .update(|cx| ui::tooltip::shown(cx).map(|s| s.label.to_string()));
    ensure!(
        tip.as_deref() == Some("Managed by enterprise policy"),
        "tooltip {tip:?}"
    );
    if let Some(dir) = std::env::var_os("CGX_PERMISSION_EVIDENCE") {
        std::fs::create_dir_all(&dir)?;
        fx.shot()?
            .save(std::path::Path::new(&dir).join("managed.png"))?;
    }
    let sent = Rc::new(RefCell::new(None));
    let slot = sent.clone();
    fx.composer().update(&mut fx.cx, |c, _| {
        c.on_submit(move |s, _, _| *slot.borrow_mut() = Some(s.permission.clone()))
    });
    fx.focus_composer()?;
    fx.type_text("Hi")?;
    fx.keys("enter")?;
    ensure!(
        *sent.borrow() == Some(Permission::Managed),
        "sent {:?}",
        sent.borrow()
    );
    Ok(())
}
