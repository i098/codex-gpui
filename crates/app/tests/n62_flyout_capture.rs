mod common;

use anyhow::Result;
use common::Fx;
use gpui::BorrowAppContext as _;
use ui::screens::connections::{Connection, ConnectionsState};

#[test]
fn remote_flyout() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |_, cx| {
        cx.update_global::<ConnectionsState, _>(|state, _| {
            state.connections = Some(vec![Connection {
                host_id: "ssh:dev".into(),
                display_name: "Build host".into(),
                source: "ssh".into(),
                state: "connected".into(),
                ..Default::default()
            }]);
        });
    });
    fx.click(fx.at("home", "This computer")?)?;
    fx.capture_evidence("CGX_N62_SHOTS", "after-work-in")?;
    fx.keys("down down enter")?;
    fx.capture_evidence("CGX_N62_SHOTS", "after-remote-picker")?;
    fx.keys("down enter")?;
    anyhow::ensure!(fx.read(|root, cx| root.home().read(cx).location.remote_host.as_deref() == Some("ssh:dev")));
    Ok(())
}
