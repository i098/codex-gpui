mod common;

use anyhow::{Result, ensure};
use app::SidePanel;
use common::{Fx, SUBTRACT};
use ui::workspace::WorkspaceLayout;

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(path) = std::env::var_os("CGX_WORKSPACE_LAYOUT_OUT") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path)?;
        fx.shot()?.save(path.join(format!("{name}.png")))?;
    }
    Ok(())
}

#[test]
fn layout_cycle_retains_the_selected_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.action(Box::new(ui::OpenChangesPanel))?;
    capture(&mut fx, "split")?;
    for (keys, layout, name) in [
        ("cmd-shift-f", WorkspaceLayout::Full, "full"),
        ("cmd-shift-b", WorkspaceLayout::Split, "split-restored"),
        ("cmd-shift-b", WorkspaceLayout::Chat, "hidden"),
        ("cmd-shift-b", WorkspaceLayout::Split, "tabs-restored"),
    ] {
        fx.keys(keys)?;
        ensure!(fx.read(|root, _| {
            root.workspace_layout() == layout
                && root.side_tabs() == [SidePanel::Changes]
                && root.side_panel() == Some(SidePanel::Changes)
        }));
        capture(&mut fx, name)?;
    }
    fx.keys("cmd-shift-b")?;
    ensure!(fx.composer_focused()?, "Hiding tabs must focus Chat");
    fx.action(Box::new(ui::ToggleChangesPanel))?;
    ensure!(fx.read(|root, _| root.workspace_layout() == WorkspaceLayout::Split));
    fx.keys("cmd-w")?;
    ensure!(fx.read(|root, _| root.side_tabs().is_empty()));
    ensure!(fx.composer_focused()?);
    Ok(())
}

#[test]
fn full_view_does_not_route_typing_to_the_hidden_composer() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.action(Box::new(ui::OpenChangesPanel))?;
    fx.keys("cmd-shift-f")?;
    fx.type_text("x")?;
    ensure!(fx.composer_text()?.is_empty());
    ensure!(!fx.composer_focused()?);
    fx.keys("cmd-f")?;
    fx.type_text("subtract")?;
    ensure!(fx.read(|root, cx| {
        root.diff_panel()
            .and_then(|panel| panel.read(cx).search())
            .is_some_and(|(query, count)| query == "subtract" && count > 0)
            && root.thread().unwrap().read(cx).find_state().is_none()
    }));
    capture(&mut fx, "full-find")?;
    fx.keys("escape")?;
    fx.keys("cmd-shift-f")?;
    ensure!(fx.read(|root, _| root.workspace_layout() == WorkspaceLayout::Split));
    Ok(())
}

#[test]
fn empty_workspace_does_not_create_a_content_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-shift-f cmd-shift-b")?;
    ensure!(fx.read(|root, _| root.side_tabs().is_empty()));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn layout_controls_accept_tab_enter_and_mouse_clicks() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.action(Box::new(ui::OpenChangesPanel))?;
    let mut reached = false;
    for _ in 0..64 {
        fx.keys("tab")?;
        let image = fx.shot()?;
        let scale = image.width() / 1280;
        reached = (8 * scale..36 * scale).any(|y| {
            (1210 * scale..1238 * scale).any(|x| image.get_pixel(x, y).0[..3] == [58, 131, 247])
        });
        if reached {
            break;
        }
    }
    ensure!(reached, "Tab must reach the Full view button");
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.workspace_layout() == WorkspaceLayout::Full));
    for layout in [
        WorkspaceLayout::Split,
        WorkspaceLayout::Chat,
        WorkspaceLayout::Split,
    ] {
        fx.click((1258., 22.))?;
        ensure!(fx.read(|root, _| root.workspace_layout() == layout));
    }
    Ok(())
}
