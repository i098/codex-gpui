//! PANEL-21: `⌘⇧T` reopens the last closed side tab, and a tab's right-click menu moves the
//! panel to the left pane and back, as the original's `tab-context-menu` does. The menu's
//! `Close other tabs` and `Close tabs to the right` close through `close_side`, so `⌘⇧T`
//! brings the tabs back.
mod common;

use anyhow::{Result, ensure};
use app::SidePanel;
use common::*;
use std::path::Path;
use ui::screens::diff::panel::{CloseSideTab, CloseSideTabs, OpenFile, content_left};

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn tab_menu_moves_panes_and_reopens_closed_tab() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("tab_menu: the Metal replay requires macOS");
        return Ok(());
    }
    let src = Path::new(CWD).join("src");
    std::fs::create_dir_all(&src)?;
    std::fs::write(src.join("math.py"), "def add(a, b):\n    return a + b\n")?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.action(Box::new(OpenFile {
        path: "src/math.py".into(),
        line: None,
    }))?;
    fx.wait("file tab", |fx| {
        fx.read(|r, cx| {
            matches!(r.side_tabs(), [SidePanel::File(_)])
                && r.file_view()
                    .is_some_and(|f| matches!(f.read(cx).contents, Some(Ok(_))))
        })
    })?;
    fx.action(Box::new(CloseSideTab(0)))?;
    ensure!(fx.read(|r, _| r.side_tabs().is_empty()), "tab not closed");
    fx.keys("cmd-shift-t")?;
    ensure!(
        fx.read(|r, _| matches!(r.side_tabs(), [SidePanel::File(_)])),
        "cmd-shift-t did not reopen the file tab"
    );

    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    // The strip starts 8 px into the panel, 1280 - 4 - 664 px from the left.
    fx.right_click((680., 22.))?;
    fx.shot()?.save(proof.join("tab-menu.png"))?;
    fx.keys("down")?;
    fx.keys("enter")?;
    ensure!(fx.read(|_, cx| content_left(cx)), "panel did not move left");
    fx.shot()?.save(proof.join("tab-menu-left.png"))?;

    // On the left the strip starts where the thread header did, after the sidebar.
    fx.right_click((400., 22.))?;
    fx.shot()?.save(proof.join("tab-menu-from-left.png"))?;
    fx.keys("down")?;
    fx.keys("enter")?;
    ensure!(
        !fx.read(|_, cx| content_left(cx)),
        "panel did not move back"
    );
    println!("tab_menu: shots in {}", proof.display());
    Ok(())
}

/// The strip's slots start 8 px into the panel (1280 - 4 - 664 px wide): 620 px, then 860 px
/// for two 240 px slots.
const TABS: [&str; 3] = ["math.py", "other.py", "third.py"];

fn open_files(fx: &mut Fx, names: &[&str]) -> Result<()> {
    let src = Path::new(CWD).join("src");
    std::fs::create_dir_all(&src)?;
    for name in names {
        std::fs::write(src.join(name), "def add(a, b):\n    return a + b\n")?;
        fx.action(Box::new(OpenFile {
            path: format!("src/{name}"),
            line: None,
        }))?;
    }
    fx.wait("file tabs", |fx| fx.read(|r, _| !r.side_tabs().is_empty()))
}

#[test]
fn tab_menu_closes_tabs_to_the_right_and_the_others() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("tab_menu: the Metal replay requires macOS");
        return Ok(());
    }
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;

    // Right-click the first tab, then take the menu's first row, `Close other tabs`.
    open_files(&mut fx, &TABS[..2])?;
    fx.wait("two file tabs", |fx| {
        fx.read(|r, _| r.side_tabs().len() == 2)
    })?;
    let first = fx.read(|r, _| r.side_tabs()[0]);
    fx.right_click((650., 22.))?;
    fx.keys("down")?;
    fx.keys("enter")?;
    ensure!(
        fx.read(|r, _| r.side_tabs() == [first]),
        "Close other tabs kept the wrong tab: {:?}",
        fx.read(|r, _| r.side_tabs().to_vec())
    );

    // `⌘⇧T` gives the closed tab back, as `Reopen closed tab` does.
    fx.keys("cmd-shift-t")?;
    ensure!(
        fx.read(|r, _| r.side_tabs().len() == 2),
        "cmd-shift-t did not reopen the closed tab"
    );

    // `Close tabs to the right` of the first of three tabs keeps the ones on its left.
    open_files(&mut fx, &TABS)?;
    fx.wait("three file tabs", |fx| {
        fx.read(|r, _| r.side_tabs().len() == 3)
    })?;
    let kept = fx.read(|r, _| r.side_tabs()[0]);
    fx.action(Box::new(CloseSideTabs {
        keep: 0,
        right_only: true,
    }))?;
    ensure!(
        fx.read(|r, _| r.side_tabs() == [kept]),
        "Close tabs to the right kept the wrong tabs: {:?}",
        fx.read(|r, _| r.side_tabs().to_vec())
    );
    Ok(())
}
