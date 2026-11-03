//! Each thread keeps its own composer draft, as the original's per-thread `draftKey` does:
//! switching chats stores the text, chips and attachments of the one left and puts back the
//! draft of the one opened; `state.json` keeps the text for a restart.

mod common;

use anyhow::{Result, ensure};
use common::{EXPLAIN, Fx, SUBTRACT, script};

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

fn open(fx: &mut Fx, label: &str, id: &str) -> Result<()> {
    fx.click(fx.at("home", label)?)?;
    fx.wait_thread(id)
}

#[test]
fn switching_threads_keeps_each_draft() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |_| {})?;
    // Each chat opens twice; the recording loads each once.
    let ids: Vec<_> = lines
        .iter()
        .filter(|l| {
            l["dir"] == "send"
                && [SUBTRACT, EXPLAIN]
                    .contains(&l["msg"]["params"]["threadId"].as_str().unwrap_or_default())
        })
        .map(|l| l["msg"]["id"].clone())
        .collect();
    let again: Vec<_> = lines
        .iter()
        .filter(|l| ids.contains(&l["msg"]["id"]))
        .cloned()
        .collect();
    lines.extend(again);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open(&mut fx, "Add a subtract", SUBTRACT)?;
    fx.focus_composer()?;
    fx.type_text("Subtract draft")?;

    open(&mut fx, "In two sentences", EXPLAIN)?;
    ensure!(
        fx.composer_text()?.is_empty(),
        "the other chat showed {:?}",
        fx.composer_text()?
    );
    fx.focus_composer()?;
    fx.type_text("Explain draft")?;
    fx.capture_evidence("CGX_THREAD_DRAFT_EVIDENCE", "explain-draft")?;

    open(&mut fx, "Add a subtract", SUBTRACT)?;
    ensure!(
        fx.composer_text()? == "Subtract draft",
        "subtract draft {:?}",
        fx.composer_text()?
    );
    fx.capture_evidence("CGX_THREAD_DRAFT_EVIDENCE", "subtract-draft")?;
    ensure!(
        fx.read(|root, _| root.saved_drafts().get(EXPLAIN).cloned())
            .as_deref()
            == Some("Explain draft"),
        "state.json does not keep the left chat's draft"
    );

    open(&mut fx, "In two sentences", EXPLAIN)?;
    ensure!(
        fx.composer_text()? == "Explain draft",
        "explain draft {:?}",
        fx.composer_text()?
    );
    Ok(())
}
