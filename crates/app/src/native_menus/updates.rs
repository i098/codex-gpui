use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, PromptLevel};
use semver::Version;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/i098/codex-gpui/releases";
// shortcut: unauthenticated, so the check reports 404 while the repository is private; send a token if it stays private.
const LATEST_RELEASE: &str = "https://api.github.com/repos/i098/codex-gpui/releases/latest";

/// Compares the newest published release with this build and reports both versions.
pub(super) fn check(cx: &mut App) {
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let latest = cx.background_spawn(async { latest_release() });
    cx.spawn(async move |cx| {
        let (message, detail, answers, newer) = match latest.await {
            Ok(latest) if latest > current() => (
                format!("Codex GPUI {latest} is available"),
                format!("You have version {VERSION}."),
                &["Download", "Later"][..],
                true,
            ),
            Ok(_) => (
                "You're up to date".to_owned(),
                format!("Codex GPUI {VERSION} is the newest version."),
                &["OK"][..],
                false,
            ),
            Err(error) => (
                "Cannot check for updates".to_owned(),
                format!("You have version {VERSION}. {error:#}"),
                &["OK"][..],
                false,
            ),
        };
        let answer = window
            .update(cx, |_, window, cx| {
                window.prompt(PromptLevel::Info, &message, Some(&detail), answers, cx)
            })
            .ok();
        if let Some(answer) = answer
            && answer.await == Ok(0)
            && newer
        {
            cx.update(|cx| cx.open_url(RELEASES));
        }
    })
    .detach();
}

fn current() -> Version {
    Version::parse(VERSION).expect("the workspace version is semver")
}

fn latest_release() -> Result<Version> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let body = agent
        .get(LATEST_RELEASE)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", format!("codex-gpui/{VERSION}"))
        .call()
        .context("the release server is unavailable")?
        .body_mut()
        .read_to_string()?;
    release_version(&body)
}

fn release_version(body: &str) -> Result<Version> {
    let release: serde_json::Value = serde_json::from_str(body)?;
    let tag = release["tag_name"]
        .as_str()
        .context("the release has no tag")?;
    Ok(Version::parse(tag.trim_start_matches('v'))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tag_parses_and_orders_against_this_build() {
        let newer = release_version(r#"{"tag_name":"v999.0.0"}"#).unwrap();
        assert_eq!(newer, Version::new(999, 0, 0));
        assert!(newer > current());
        assert_eq!(
            release_version(&format!(r#"{{"tag_name":"v{VERSION}"}}"#)).unwrap(),
            current()
        );
        assert!(release_version(r#"{"name":"untagged"}"#).is_err());
        assert!(release_version(r#"{"tag_name":"nightly"}"#).is_err());
    }
}
