mod common;

use anyhow::{Result, ensure};
use app::SidePanel;
use common::{Fx, SUBTRACT};
use ui::browser::LoadState;

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
    let title =
        |fx: &mut Fx| fx.read(|root, cx| root.active_browser_tab().map(|tab| tab.read(cx).title()));
    ensure!(title(&mut fx).as_deref() == Some("New tab"));
    proof(&mut fx, "new-tab")?;
    // ⌘T focused the address bar: typing goes there, Enter loads it.
    fx.type_text("example.com/docs")?;
    fx.keys("enter")?;
    let url = fx.read(|root, cx| {
        root.active_browser_tab()
            .and_then(|tab| tab.read(cx).url().map(ToString::to_string))
    });
    ensure!(
        url.as_deref() == Some("https://example.com/docs"),
        "{url:?}"
    );
    ensure!(title(&mut fx).as_deref() == Some("example.com"));
    proof(&mut fx, "loaded")?;
    fx.keys("cmd-w")?;
    ensure!(fx.read(|root, _| root.browser_tab(1).is_none()));
    ensure!(fx.read(|root, _| root.side_panel()) != Some(SidePanel::Browser(1)));
    Ok(())
}

/// The selected browser tab's committed URL, load state and failed address.
fn page(fx: &mut Fx) -> (Option<String>, LoadState, Option<String>) {
    fx.read(|root, cx| {
        let tab = root.active_browser_tab().expect("a browser tab");
        let tab = tab.read(cx);
        (
            tab.url().map(ToString::to_string),
            tab.load_state(),
            tab.load_error().map(|error| error.url.to_string()),
        )
    })
}

/// A toolbar control's center: Back, Next and Reload sit left of the address bar, 29 px
/// apart, in the 40 px row over the page.
fn toolbar_control(fx: &mut Fx, index: f32) -> (f32, f32) {
    let bounds = fx.read(|root, cx| root.active_browser_tab().unwrap().read(cx).page_bounds());
    (
        f32::from(bounds.origin.x) + 22. + 29. * index,
        f32::from(bounds.origin.y) - 20.,
    )
}

fn load(fx: &mut Fx, address: &str) -> Result<()> {
    fx.keys("cmd-l")?;
    fx.type_text(address)?;
    fx.keys("enter")
}

#[test]
fn back_next_reload_and_the_error_page_follow_the_tab_history() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-t")?;
    load(&mut fx, "example.com/a")?;
    load(&mut fx, "example.com/b")?;
    let b = Some("https://example.com/b".to_owned());
    ensure!(page(&mut fx) == (b.clone(), LoadState::Loaded, None));
    proof(&mut fx, "loaded")?;
    let back = toolbar_control(&mut fx, 0.);
    let next = toolbar_control(&mut fx, 1.);
    fx.click(back)?;
    let a = Some("https://example.com/a".to_owned());
    ensure!(page(&mut fx).0 == a, "{:?}", page(&mut fx));
    fx.click(next)?;
    ensure!(page(&mut fx).0 == b, "{:?}", page(&mut fx));
    // `.invalid` never resolves: the load fails and the tab shows its error page with the
    // failed address, over the page it came from.
    load(&mut fx, "nope.invalid")?;
    let failed = Some("https://nope.invalid/".to_owned());
    ensure!(page(&mut fx) == (b.clone(), LoadState::Failed, failed.clone()));
    proof(&mut fx, "error-page")?;
    // Reload tries the failed address again; Back leaves the error page.
    fx.keys("cmd-r")?;
    ensure!(page(&mut fx) == (b.clone(), LoadState::Failed, failed));
    fx.click(back)?;
    ensure!(page(&mut fx) == (b, LoadState::Loaded, None));
    Ok(())
}
