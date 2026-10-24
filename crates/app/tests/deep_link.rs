//! Deep-link navigation (SH-151..154): a `codex://` link navigates the primary window
//! like the original's deep-link flush, and a prompt prefill focuses the composer.

mod common;

use anyhow::Result;
use common::{Fx, GREETING};

fn open(fx: &mut Fx, url: &str) -> Result<()> {
    let link = app::deep_link::parse(url).expect("parses");
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn new_thread_link_prefills_and_focuses_the_composer() -> Result<()> {
    let mut fx = Fx::new()?;
    open(&mut fx, "codex://threads/new?prompt=Fix%20the%20greeting")?;
    assert_eq!(fx.route(), app::Route::Home);
    assert_eq!(fx.composer_text()?, "Fix the greeting");
    assert!(fx.composer_focused()?);
    Ok(())
}

#[test]
fn thread_link_opens_the_conversation() -> Result<()> {
    let mut fx = Fx::new()?;
    open(&mut fx, &format!("codex://threads/{GREETING}"))?;
    fx.wait_thread(GREETING)?;
    Ok(())
}

#[test]
fn settings_automations_and_launch_links_route() -> Result<()> {
    let mut fx = Fx::new()?;
    open(&mut fx, "codex://settings/connections")?;
    fx.wait("settings", |fx| fx.route() == app::Route::Settings)?;

    let mut fx = Fx::new()?;
    open(&mut fx, "codex://automations")?;
    fx.wait("scheduled", |fx| fx.route() == app::Route::Scheduled)?;

    // `launch` navigates nowhere.
    let mut fx = Fx::new()?;
    let route = fx.route();
    open(&mut fx, "codex://launch")?;
    assert_eq!(fx.route(), route);
    Ok(())
}
