mod common;

use anyhow::{Result, ensure};
use app::{SidePanel, host::ReplayHost};
use common::{Fx, SUBTRACT};
use std::sync::Arc;
use ui::screens::thread::view::GitAction;

#[test]
fn proxy() {
    common::proxy_child();
}

/// Branch actions ▸ `View PR` opens the `PR #n` side panel tab with the pull request's recorded
/// GitHub detail (status, reviewers, comments, checks) and its state action.
#[test]
fn view_pr_opens_the_pull_request_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let recording =
        std::fs::read_to_string(format!("{}/fixtures/pull-requests.json", common::ROOT))?;
    let host = Arc::new(ReplayHost::from_recording(&recording)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.run_until_parked();
    let shots = std::env::var("CGX_PR_TAB_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
        fx.shot()?.save(format!("{out}/1-thread.png"))?;
    }
    let thread = fx.read(|root, _| root.thread().cloned());
    let thread = thread.ok_or_else(|| anyhow::anyhow!("no open thread"))?;
    thread.update(&mut fx.cx, |view, cx| {
        view.set_pull_request(
            Some(serde_json::json!({"url": "https://github.com/i098/codex-gpui/pull/380"})),
            cx,
        );
        cx.emit(GitAction::ViewPr);
    });
    fx.wait("recorded pull request detail", |fx| {
        fx.read(|root, cx| {
            root.pull_request_tab()
                .is_some_and(|page| matches!(page.read(cx).detail, Some(Ok(_))))
        })
    })?;
    ensure!(
        fx.read(|root, _| root.side_panel()) == Some(SidePanel::PullRequest),
        "side panel {:?}",
        fx.read(|root, _| root.side_panel())
    );
    let (number, title) = fx.read(|root, cx| {
        let page = root.pull_request_tab().unwrap().read(cx);
        let detail = page.detail.as_ref().unwrap().as_ref().unwrap();
        (detail["number"].clone(), detail["title"].clone())
    });
    ensure!(number == 380, "detail number {number} ({title})");
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/2-pr-tab.png"))?;
    }
    Ok(())
}
