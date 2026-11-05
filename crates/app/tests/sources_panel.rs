mod common;

use anyhow::{Result, ensure};
use app::SidePanel;
use common::{Fx, SUBTRACT};

#[test]
fn proxy() {
    common::proxy_child();
}

/// Thread summary ▸ `Sources` opens the `Sources` tab with the file the subtract turn changed.
#[test]
fn summary_sources_open_sources_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let files = fx.read(|r, cx| {
        let turns = r.thread().map(|t| t.read(cx).turns()).unwrap_or_default();
        ui::screens::sources::sources(turns).files
    });
    ensure!(
        files.iter().any(|(path, _)| path.ends_with("math.py")),
        "sources {files:?}"
    );
    let shots = std::env::var("CGX_SOURCES_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
    }
    // `Toggle summary`, 63 px from the right edge.
    let w = fx.size.0;
    fx.click((w - 63., 22.))?;
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/1-summary.png"))?;
    }
    fx.action(Box::new(ui::OpenSourcesPanel))?;
    ensure!(
        fx.read(|r, _| r.side_panel()) == Some(SidePanel::Sources),
        "side panel {:?}",
        fx.read(|r, _| r.side_panel())
    );
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/2-sources-tab.png"))?;
    }
    Ok(())
}
