mod common;

use anyhow::Result;
use app::{Route, host::ReplayHost};
use common::Fx;
use std::sync::Arc;
use ui::screens::pull_requests::{Availability, Event};

#[test]
fn missing_host_response_keeps_retry_and_chat_available() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(ReplayHost::from_recording(r#"{"exchanges":[]}"#)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
    })?;
    fx.wait("GitHub access failure", |fx| {
        fx.read(|root, cx| {
            root.pull_requests_page().is_some_and(|page| {
                matches!(page.read(cx).availability, Availability::Unavailable(_))
            })
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    capture(&mut fx, "access-error")?;
    let page = fx.read(|root, _| root.pull_requests_page().unwrap().clone());
    let generation = fx.read(|_, cx| page.read(cx).generation);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Retry));
    fx.wait("GitHub retry failure", |fx| {
        fx.read(|_, cx| {
            page.read(cx).generation > generation
                && matches!(page.read(cx).availability, Availability::Unavailable(_))
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    page.update(&mut fx.cx, |_, cx| {
        cx.emit(Event::AskCodex(
            ui::screens::pull_requests::INSTALL_PROMPT.into(),
        ))
    });
    fx.wait("installation draft", |fx| fx.route() == Route::Home)?;
    assert_eq!(
        fx.composer_text()?,
        ui::screens::pull_requests::INSTALL_PROMPT
    );
    capture(&mut fx, "install-draft")?;
    Ok(())
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_PULL_REQUEST_EVIDENCE") {
        std::fs::create_dir_all(&directory)?;
        fx.shot()?
            .save(std::path::Path::new(&directory).join(format!("{name}.png")))?;
    }
    Ok(())
}
