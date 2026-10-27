#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result};
use app::host::{Host, LocalHost, Request};
use common::Fx;
use serde_json::json;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[test]
fn host_context_files_preserve_the_draft_and_deduplicate_attachments() -> Result<()> {
    let mut fx = Fx::new()?;
    let home = tempfile::tempdir()?;
    let first = home.path().join("first.rs");
    let second = home.path().join("second.rs");
    std::fs::write(&first, "fn first() {}")?;
    std::fs::write(&second, "fn second() {}")?;
    let host = Arc::new(LocalHost::new(home.path()));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    let submitted = Rc::new(RefCell::new(None));
    let observed = submitted.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer
            .on_submit(move |submission, _, _| *observed.borrow_mut() = Some(submission.clone()));
    });
    fx.focus_composer()?;
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_text("before after", cx)
    });
    fx.keys("left left left left left")?;
    let request = |path: &std::path::Path| {
        Request::post(
            "add-context-file",
            Some(json!({ "hostId": "local", "path": path })),
        )
    };
    host.fetch(&request(&first))?;
    host.fetch(&request(&first))?;
    host.fetch(&request(&second))?;
    fx.cx.run_until_parked();
    assert_eq!(fx.composer_text()?, "before after");
    fx.type_text("!")?;
    fx.keys("enter")?;
    let submission = submitted
        .borrow_mut()
        .take()
        .context("context-file submission")?;
    assert_eq!(submission.text, "before !after");
    assert_eq!(submission.context_files, [first, second]);
    assert!(submission.attachments.is_empty());
    Ok(())
}

#[test]
fn replacing_the_host_disconnects_its_context_file_source() -> Result<()> {
    let mut fx = Fx::new()?;
    let home = tempfile::tempdir()?;
    let old = Arc::new(LocalHost::new(home.path()));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(old.clone(), cx));
    let current = Arc::new(LocalHost::new(home.path()));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(current.clone(), cx));
    let submitted = Rc::new(RefCell::new(None));
    let observed = submitted.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer
            .on_submit(move |submission, _, _| *observed.borrow_mut() = Some(submission.clone()));
    });
    let _ = old.fetch(&Request::post(
        "add-context-file",
        Some(json!({ "path": "/work/old.rs" })),
    ));
    current.fetch(&Request::post(
        "add-context-file",
        Some(json!({ "path": "/work/current.rs" })),
    ))?;
    fx.cx.run_until_parked();
    fx.keys("enter")?;
    let submission = submitted
        .borrow_mut()
        .take()
        .context("current-host submission")?;
    assert_eq!(
        submission.context_files,
        [std::path::PathBuf::from("/work/current.rs")]
    );
    Ok(())
}

#[test]
fn failed_send_restores_context_files_without_uploading_them() -> Result<()> {
    common::install_panic_hook();
    let mut lines = common::script(None, |_| {})?;
    lines.extend(common::exchange(
        "thread/resume",
        json!({ "thread": common::recorded_thread(common::SUBTRACT)? }),
    ));
    let mut refused = common::exchange("turn/start", json!({}));
    refused[1]["msg"]
        .as_object_mut()
        .context("turn response")?
        .remove("result");
    refused[1]["msg"]["error"] = json!({ "code": -32000, "message": "context test refusal" });
    lines.extend(refused);
    let mut fx = Fx::scripted(lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    let home = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(home.path()));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.focus_composer()?;
    fx.type_text("Keep this request")?;
    host.fetch(&Request::post(
        "add-context-file",
        Some(json!({ "path": "/work/failure.rs" })),
    ))?;
    fx.cx.run_until_parked();
    fx.keys("enter")?;
    fx.wait("failed send", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text.as_ref() == "Error submitting message")
        })
    })?;
    assert_eq!(fx.composer_text()?, "Keep this request");
    let submitted = Rc::new(RefCell::new(None));
    let observed = submitted.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer
            .on_submit(move |submission, _, _| *observed.borrow_mut() = Some(submission.clone()));
    });
    fx.keys("enter")?;
    let submission = submitted.borrow_mut().take().context("restored draft")?;
    assert_eq!(
        submission.context_files,
        [std::path::PathBuf::from("/work/failure.rs")]
    );
    assert!(submission.attachments.is_empty());
    Ok(())
}

#[test]
fn settings_do_not_receive_context_files_for_a_hidden_composer() -> Result<()> {
    let mut fx = Fx::new()?;
    let home = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(home.path()));
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-,")?;
    assert_eq!(fx.route(), app::Route::Settings);
    host.fetch(&Request::post(
        "add-context-file",
        Some(json!({ "path": "/work/hidden.rs" })),
    ))?;
    fx.cx.run_until_parked();
    fx.keys("cmd-[")?;
    assert_eq!(fx.route(), app::Route::Home);
    fx.focus_composer()?;
    fx.type_text("Visible request")?;
    let submitted = Rc::new(RefCell::new(None));
    let observed = submitted.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer
            .on_submit(move |submission, _, _| *observed.borrow_mut() = Some(submission.clone()));
    });
    fx.keys("enter")?;
    let submission = submitted.borrow_mut().take().context("visible draft")?;
    assert_eq!(submission.text, "Visible request");
    assert!(submission.context_files.is_empty());
    Ok(())
}
