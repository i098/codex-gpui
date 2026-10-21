mod common;

use anyhow::{Result, ensure};
use common::Fx;
use std::time::Duration;
use ui::keyboard::{Change, KeyboardState};

fn change(fx: &mut Fx, change: Change) -> Result<()> {
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(change, cx);
    });
    fx.cx.run_until_parked();
    // The dialog observes the store on the next frame.
    fx.advance(Duration::ZERO)
}

fn titles(fx: &Fx) -> Vec<&'static str> {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).visible_titles())
}

#[test]
fn the_open_shortcuts_dialog_follows_live_bindings() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-/")?;
    ensure!(titles(&fx).contains(&"Archive chat"));
    change(
        &mut fx,
        Change::Bindings {
            command: "archiveThread".into(),
            keys: Vec::new(),
        },
    )?;
    ensure!(
        !titles(&fx).contains(&"Archive chat"),
        "an unbound command leaves the open dialog"
    );
    change(&mut fx, Change::ResetCommand("archiveThread".into()))?;
    ensure!(titles(&fx).contains(&"Archive chat"));
    Ok(())
}
