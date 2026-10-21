mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, SUBTRACT};

fn clear_command(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("clear all unreads")?;
    fx.keys("enter")
}

#[test]
fn clear_unreads_requires_confirmation_and_remembers_the_opt_out() -> Result<()> {
    let mut fx = Fx::new()?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_unread(GREETING, cx));
    clear_command(&mut fx)?;
    fx.keys("escape")?;
    ensure!(fx.read(|root, cx| root.sidebar().read(cx).unread == [GREETING]));
    clear_command(&mut fx)?;
    fx.keys("space tab tab enter")?;
    fx.wait("confirmed unread clearing", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).unread.is_empty())
    })?;
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_unread(SUBTRACT, cx));
    clear_command(&mut fx)?;
    fx.wait("saved confirmation preference", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).unread.is_empty())
    })?;
    Ok(())
}

#[test]
fn attention_shortcut_skips_the_open_unread_chat() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| {
        sidebar.set_unread(GREETING, cx);
        sidebar.set_unread(SUBTRACT, cx);
    });
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(SUBTRACT)?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn shift_escape_opens_the_confirmation_over_home() -> Result<()> {
    common::install_panic_hook();
    let mut fx = Fx::new()?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_unread(GREETING, cx));
    fx.keys("shift-escape")?;
    if let Some(out) = std::env::var_os("CGX_SHORTCUT_PROOF") {
        std::fs::create_dir_all(&out)?;
        // `shot`, not `reference_shot`: a pointer move would clear the keyboard focus ring.
        fx.shot()?
            .save(std::path::Path::new(&out).join("clear-unreads-modal.png"))?;
    }
    fx.keys("escape")?;
    ensure!(fx.read(|root, cx| root.sidebar().read(cx).unread == [GREETING]));
    Ok(())
}
