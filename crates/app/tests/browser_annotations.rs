#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::json;
use ui::composer::DiffCommentsChanged;
use ui::confirmation::Outcome;
use ui::screens::diff::panel::comments::Comment;

#[test]
fn proxy() {
    common::proxy_child();
}

fn comment(value: serde_json::Value) -> Comment {
    serde_json::from_value(value).expect("a stored comment")
}

fn browser_comment(text: &str) -> Comment {
    comment(json!({
        "content": [{"content_type": "text", "text": text}],
        "position": {"path": "browser:Save", "side": "right", "line": 1},
        "localBrowserCommentMetadata": {"kind": "element"},
        "localBrowserContext": {"pageUrl": "https://example.com/docs"}
    }))
}

/// The selected browser tab's annotation mode and pending count.
fn mode(fx: &Fx) -> (bool, usize) {
    fx.read(|root, cx| {
        let tab = root.active_browser_tab().expect("a browser tab");
        let tab = tab.read(cx);
        (tab.annotating(), tab.pending_annotations())
    })
}

fn set_comments(fx: &mut Fx, comments: Vec<Comment>) {
    let composer = fx.composer();
    composer.update(&mut fx.cx, |_, cx| cx.emit(DiffCommentsChanged(comments)));
    fx.cx.run_until_parked();
}

fn pending_texts(fx: &Fx) -> Vec<String> {
    fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .diff_comments()
            .iter()
            .map(Comment::text)
            .collect()
    })
}

/// A control of the annotation toolbar under the page: Exit and Discard on the left,
/// Send on the right, centered in its 44 px row.
fn toolbar(fx: &Fx, control: &str) -> (f32, f32) {
    let bounds = fx.read(|root, cx| root.active_browser_tab().unwrap().read(cx).page_bounds());
    let y = f32::from(bounds.origin.y + bounds.size.height) + 22.;
    let x = match control {
        "exit" => f32::from(bounds.origin.x) + 22.,
        "discard" => f32::from(bounds.origin.x) + 56.,
        _ => f32::from(bounds.origin.x + bounds.size.width) - 22.,
    };
    (x, y)
}

/// Cmd+Shift+. toggles annotation mode on a page; the toolbar counts the chat's pending
/// browser annotations, Exit keeps them, Send sends them as one turn and Discard drops them
/// after its confirmation. Diff comments stay pending throughout.
#[test]
fn annotation_mode_toolbar_sends_discards_and_exits() -> Result<()> {
    let evidence = "CGX_BROWSER_ANNOTATION_EVIDENCE";
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-t")?;
    // No page yet: the shortcut leaves the mode off.
    fx.keys("cmd-shift-.")?;
    ensure!(mode(&fx) == (false, 0), "{:?}", mode(&fx));
    fx.type_text("example.com/docs")?;
    fx.keys("enter")?;
    fx.capture_evidence(evidence, "before")?;

    fx.keys("cmd-shift-.")?;
    ensure!(mode(&fx) == (true, 0), "{:?}", mode(&fx));
    fx.capture_evidence(evidence, "annotating")?;

    let diff = comment(json!({
        "content": [{"content_type": "text", "text": "Keep this check"}],
        "position": {"path": "src/a.rs", "side": "right", "line": 3}
    }));
    set_comments(
        &mut fx,
        vec![
            browser_comment("Make Save bold"),
            diff.clone(),
            browser_comment("Fix the typo"),
        ],
    );
    ensure!(mode(&fx) == (true, 2), "{:?}", mode(&fx));
    fx.capture_evidence(evidence, "annotating-pending")?;

    let exit = toolbar(&fx, "exit");
    fx.click(exit)?;
    ensure!(
        mode(&fx) == (false, 2),
        "Exit dropped the pending annotations"
    );
    fx.keys("cmd-shift-.")?;
    ensure!(mode(&fx) == (true, 2), "{:?}", mode(&fx));

    let discard = toolbar(&fx, "discard");
    fx.click(discard)?;
    let dialog = fx.read(|root, _| root.diff_comment_confirmation().cloned());
    let dialog = dialog.ok_or_else(|| anyhow::anyhow!("Discard asked nothing"))?;
    ensure!(
        mode(&fx) == (true, 2),
        "the dialog changed the pending list early"
    );
    fx.capture_evidence(evidence, "discard-confirm")?;
    dialog.update(&mut fx.cx, |_, cx| cx.emit(Outcome::Confirm));
    fx.cx.run_until_parked();
    ensure!(mode(&fx) == (true, 0), "{:?}", mode(&fx));
    ensure!(pending_texts(&fx) == ["Keep this check"]);

    set_comments(
        &mut fx,
        vec![
            browser_comment("Make Save bold"),
            diff,
            browser_comment("Fix the typo"),
        ],
    );
    let send = toolbar(&fx, "send");
    fx.click(send)?;
    fx.wait("annotation turn", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|request| request["method"] == "turn/start"))
    })?;
    let requests = fx.sent()?;
    let text = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .and_then(|request| request["params"]["input"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|input| input["text"].as_str())
        .collect::<String>();
    ensure!(
        text.starts_with("# Browser comments:\n")
            && text.contains("Make Save bold")
            && text.contains("Fix the typo")
            && !text.contains("Keep this check")
            && text.ends_with("## My request:\n"),
        "The annotation turn lost its context: {text}"
    );
    ensure!(mode(&fx) == (false, 0), "{:?}", mode(&fx));
    ensure!(pending_texts(&fx) == ["Keep this check"]);
    Ok(())
}
