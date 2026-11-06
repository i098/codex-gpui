mod common;

use anyhow::{Context as _, Result, ensure};
use app::{
    SidePanel,
    host::{Host, ReplayHost, Request, json_body},
};
use common::{Fx, SUBTRACT};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use ui::screens::pull_requests::Event;
use ui::screens::thread::view::GitAction;

#[test]
fn proxy() {
    common::proxy_child();
}

/// The recorded Code Review host, with the recording's review metadata merged into each detail
/// as the original's detail read returns it, and `gh-pr-state` bodies kept for the test.
struct PullRequestHost {
    replay: ReplayHost,
    review_metadata: Value,
    states: Mutex<Vec<Value>>,
}

impl Host for PullRequestHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        if let Request::Fetch { url, body, .. } = request
            && url == "vscode://codex/gh-pr-state"
        {
            self.states
                .lock()
                .unwrap()
                .push(serde_json::from_str(body)?);
            return Ok(json!({
                "type": "fetch-response", "responseType": "success", "status": 200, "headers": {},
                "bodyJsonString": json!({"success": true}).to_string()
            }));
        }
        let mut response = self.replay.fetch(request)?;
        if let Request::Fetch { url, .. } = request
            && url == "vscode://codex/gh-pr-detail"
        {
            let mut body = json_body(&response)?;
            let number = body["number"]
                .as_u64()
                .context("detail number")?
                .to_string();
            if let Some(metadata) = self.review_metadata[&number].as_object() {
                body.as_object_mut()
                    .context("detail object")?
                    .extend(metadata.clone());
            }
            response["bodyJsonString"] = body.to_string().into();
        }
        Ok(response)
    }
}

/// Branch actions ▸ `View PR` opens the `PR #n` side panel tab with the draft pull request's
/// recorded GitHub detail; `Ready for review` sends `gh-pr-state` and reads the detail again.
#[test]
fn view_pr_opens_the_pull_request_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let recording =
        std::fs::read_to_string(format!("{}/fixtures/pull-requests.json", common::ROOT))?;
    let mut data: Value = serde_json::from_str(&recording)?;
    let host = Arc::new(PullRequestHost {
        replay: ReplayHost::from_recording(&recording)?,
        review_metadata: data["reviewMetadata"].take(),
        states: Mutex::new(Vec::new()),
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.cx.run_until_parked();
    let shots = std::env::var("CGX_PR_TAB_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
        fx.shot()?.save(format!("{out}/1-thread.png"))?;
    }
    let thread = fx.read(|root, _| root.thread().cloned());
    let thread = thread.context("no open thread")?;
    thread.update(&mut fx.cx, |view, cx| {
        view.set_pull_request(
            Some(json!({"url": "https://github.com/i098/codex-gpui/pull/372"})),
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
    let page = fx.read(|root, _| root.pull_request_tab().cloned().unwrap());
    let detail = fx.read(|_, cx| page.read(cx).detail.clone().unwrap().unwrap());
    ensure!(
        detail["number"] == 372 && detail["isDraft"] == true,
        "detail {detail}"
    );
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/2-pr-tab.png"))?;
    }
    let generation = fx.read(|_, cx| page.read(cx).detail_generation);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::SetState("ready")));
    fx.wait("detail read again after the state action", |fx| {
        fx.read(|_, cx| {
            let page = page.read(cx);
            page.detail_generation > generation && matches!(page.detail, Some(Ok(_)))
        })
    })?;
    let states = host.states.lock().unwrap().clone();
    ensure!(
        states
            == [json!({
                "account": {"hostId": "local", "hostname": "github.com", "login": "i098"},
                "pullRequest": {"hostname": "github.com", "owner": "i098", "repository": "codex-gpui", "number": 372},
                "action": "ready",
            })],
        "gh-pr-state requests {states:?}"
    );
    Ok(())
}
