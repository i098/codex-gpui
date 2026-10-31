//! `git.toggleBlame`: the file viewer's active-line Git blame (`data-pierre-editor-inline-blame`).

use super::*;
use serde::Deserialize;
use serde_json::Value;

/// `git-active-line-blame-enabled`: whether the active line shows its blame. Off by default.
/// shortcut: in memory only, the original persists it; store it with the app settings when a
/// persisted UI preference store lands.
#[derive(Default)]
pub struct GitBlame(pub bool);

impl Global for GitBlame {}

impl GitBlame {
    pub fn enabled(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|blame| blame.0)
    }
}

/// One `git blame --porcelain` line, as the host's `git-blame-file` reports it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    pub line_number: u32,
    pub commit_sha: String,
    pub author: Option<String>,
    pub author_time: Option<i64>,
    pub summary: Option<String>,
}

impl BlameLine {
    /// `author · summary · sha8 · date` (`Intl.DateTimeFormat` `dateStyle: medium`), or `None`
    /// for a line with no author or an uncommitted (all-zero) commit, which the original hides.
    pub fn label(&self) -> Option<String> {
        let author = self.author.as_deref().filter(|author| !author.is_empty())?;
        if self.commit_sha.is_empty() || self.commit_sha.bytes().all(|b| b == b'0') {
            return None;
        }
        let date = self
            .author_time
            .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%b %-d, %Y")
                    .to_string()
            });
        let sha = &self.commit_sha[..self.commit_sha.len().min(8)];
        Some(
            [
                Some(author),
                self.summary.as_deref(),
                Some(sha),
                date.as_deref(),
            ]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        )
    }
}

impl FileView {
    /// The host's `git-blame-file` answer for `revision`; an error shows no blame.
    pub fn set_blame(
        &mut self,
        revision: u64,
        body: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        if self.revision != revision {
            return;
        }
        self.blame = body
            .ok()
            .and_then(|body| serde_json::from_value(body["lines"].clone()).ok())
            .unwrap_or_default();
        cx.notify();
    }

    /// The active line's blame text, while blame is on.
    pub fn active_blame(&self, cx: &App) -> Option<String> {
        if !GitBlame::enabled(cx) {
            return None;
        }
        let line = self.line?;
        self.blame
            .iter()
            .find(|blame| blame.line_number == line)
            .and_then(BlameLine::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(sha: &str, author: Option<&str>) -> BlameLine {
        BlameLine {
            line_number: 1,
            commit_sha: sha.into(),
            author: author.map(Into::into),
            author_time: None,
            summary: Some("Add math".into()),
        }
    }

    #[test]
    fn label_hides_uncommitted_and_authorless_lines() {
        assert_eq!(
            line("0123456789abcdef", Some("Ada")).label().as_deref(),
            Some("Ada · Add math · 01234567")
        );
        assert_eq!(
            line(&"0".repeat(40), Some("Not Committed Yet")).label(),
            None
        );
        assert_eq!(line("0123456789abcdef", None).label(), None);
    }
}
