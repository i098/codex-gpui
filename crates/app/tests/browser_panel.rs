mod common;

use anyhow::{Result, ensure};
use app::SidePanel;
use common::{Fx, SUBTRACT};

/// With `CGX_BROWSER_PROOF=<dir>` (macOS), saves the window as `<dir>/<name>.png`.
fn proof(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(out) = std::env::var_os("CGX_BROWSER_PROOF") {
        std::fs::create_dir_all(&out)?;
        fx.shot()?
            .save(std::path::Path::new(&out).join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn cmd_t_opens_a_browser_tab_that_loads_the_address_and_closes_with_cmd_w() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    proof(&mut fx, "before")?;
    fx.keys("cmd-t")?;
    ensure!(fx.read(|root, _| root.side_panel()) == Some(SidePanel::Browser(1)));
    let title = |fx: &mut Fx| {
        fx.read(|root, cx| root.active_browser_tab().map(|tab| tab.read(cx).title()))
    };
    ensure!(title(&mut fx).as_deref() == Some("New tab"));
    proof(&mut fx, "new-tab")?;
    // ⌘T focused the address bar: typing goes there, Enter loads it.
    fx.type_text("example.com/docs")?;
    fx.keys("enter")?;
    let url = fx.read(|root, cx| {
        root.active_browser_tab()
            .and_then(|tab| tab.read(cx).url().map(ToString::to_string))
    });
    ensure!(url.as_deref() == Some("https://example.com/docs"), "{url:?}");
    ensure!(title(&mut fx).as_deref() == Some("example.com"));
    proof(&mut fx, "loaded")?;
    fx.keys("cmd-w")?;
    ensure!(fx.read(|root, _| root.browser_tab(1).is_none()));
    ensure!(fx.read(|root, _| root.side_panel()) != Some(SidePanel::Browser(1)));
    Ok(())
}
