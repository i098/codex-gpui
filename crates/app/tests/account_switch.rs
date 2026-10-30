//! Profile menu account rows through the real `Root`: the signed-in ChatGPT account's email,
//! `Add account` opening the sign-in screen over it, and `Cancel` returning to the session.
mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::Fx;
use ui::ProfileAccount;

fn avatar(fx: &Fx) -> (f32, f32) {
    (26., fx.size.1 - 26.)
}

/// `Add account`: the row above `Settings` (bottom-anchored menu, 32 px rows).
fn switch_row(fx: &Fx) -> (f32, f32) {
    (118., fx.size.1 - 224.)
}

/// The sign-in screen's `Cancel`, 16 px from the right edge and 44 px from the top.
const CANCEL: (f32, f32) = (1230., 60.);

#[test]
fn add_account_opens_sign_in_and_cancel_keeps_the_session() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.wait("account read", |fx| fx.shell(|s| s.account.is_some()))?;
    let account = fx.shell(|s| s.account.clone());
    ensure!(
        matches!(&account, Some(ProfileAccount::Chatgpt(Some(_)))),
        "{account:?}"
    );
    fx.click(avatar(&fx))?;
    ensure!(fx.shell(|s| s.profile_menu));
    fx.capture_evidence("CGX_ACCOUNT_SHOTS", "profile-menu")?;
    fx.click(switch_row(&fx))?;
    ensure!(fx.read(|root, _| root.signed_out()), "no sign-in screen");
    ensure!(fx.shell(|s| !s.profile_menu));
    fx.capture_evidence("CGX_ACCOUNT_SHOTS", "sign-in-cancelable")?;
    fx.click(CANCEL)?;
    ensure!(fx.read(|root, _| !root.signed_out()), "Cancel kept sign-in");
    ensure!(fx.route() == Route::Home, "route {:?}", fx.route());
    ensure!(fx.shell(|s| s.account == account));
    Ok(())
}
