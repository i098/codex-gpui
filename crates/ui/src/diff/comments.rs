use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Left => "L",
            Self::Right => "R",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub path: String,
    pub side: Side,
    pub line: NonZeroU32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<NonZeroU32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_side: Option<Side>,
}

impl Position {
    pub fn range(&self) -> String {
        let start = self.start_line.unwrap_or(self.line).get();
        let end = self.line.get();
        let start_side = self.start_side.unwrap_or(self.side);
        if start_side != self.side {
            format!("{}{start}-{}{end}", start_side.marker(), self.side.marker())
        } else if start == end {
            end.to_string()
        } else {
            format!("{}-{}", start.min(end), start.max(end))
        }
    }

    pub fn display_range(&self) -> String {
        let start = self.start_line.unwrap_or(self.line);
        let start_side = self.start_side.unwrap_or(self.side);
        if start == self.line && start_side == self.side {
            format!("{}{}", self.side.marker(), self.line)
        } else {
            format!(
                "{}{} to {}{}",
                start_side.marker(),
                start,
                self.side.marker(),
                self.line
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub content_type: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub content: Vec<Text>,
    pub position: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_diff_hunk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_workspace_root: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Comment {
    pub fn new(
        position: Position,
        text: String,
        hunk: Option<String>,
        workspace: Option<String>,
    ) -> Self {
        Self {
            kind: Some("comment".into()),
            content: vec![Text {
                content_type: "text".into(),
                text,
            }],
            position,
            local_diff_hunk: hunk,
            local_workspace_root: workspace,
            extra: Default::default(),
        }
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|part| part.content_type == "text")
            .map(|part| part.text.as_str())
            .collect()
    }

    pub fn key(&self) -> String {
        let key = format!(
            "{}{}|{}|{}|{}",
            self.local_workspace_root
                .as_ref()
                .map_or(String::new(), |root| format!("{root}|")),
            self.position.path,
            match self.position.side {
                Side::Left => "left",
                Side::Right => "right",
            },
            self.position.range(),
            self.text()
        );
        if let Some(url) = self
            .extra
            .get("localPullRequestDiscussionUrl")
            .and_then(|value| value.as_str())
        {
            return url.to_owned();
        }
        match self
            .extra
            .get("localPullRequestReviewKey")
            .and_then(|value| value.as_str())
        {
            Some(review) => format!("{review}|{key}"),
            None => key,
        }
    }

    pub fn prompt(&self, number: usize) -> String {
        let side = self.position.start_side.unwrap_or(self.position.side);
        let side = if side == self.position.side {
            side.marker().to_owned()
        } else {
            format!("{}-{}", side.marker(), self.position.side.marker())
        };
        let mut out = format!("\n## User Comment {number}\nFile: {}\n", self.position.path);
        if let Some(root) = &self.local_workspace_root {
            out.push_str(&format!("Workspace: {root}\n"));
        }
        out.push_str(&format!("Side: {side}\nLines: {}\n", self.position.range()));
        if let Some(hunk) = &self.local_diff_hunk
            && !hunk.trim().is_empty()
        {
            out.push_str(&format!("Diff hunk:\n```diff\n{hunk}\n```\n"));
        }
        out.push_str(&format!("Comment:\n{}\n", self.text()));
        out
    }
}

fn attributes(input: &str) -> Option<(BTreeMap<String, String>, usize)> {
    let mut chars = input.char_indices().peekable();
    let mut result = BTreeMap::new();
    loop {
        while chars.peek().is_some_and(|(_, ch)| ch.is_whitespace()) {
            chars.next();
        }
        let (start, ch) = chars.next()?;
        if ch == '}' {
            return Some((result, start + 1));
        }
        let mut key = String::from(ch);
        while let Some(&(_, ch)) = chars.peek() {
            if ch == '=' || ch.is_whitespace() {
                break;
            }
            key.push(ch);
            chars.next();
        }
        while chars.peek().is_some_and(|(_, ch)| ch.is_whitespace()) {
            chars.next();
        }
        if chars.next()?.1 != '=' {
            return None;
        }
        while chars.peek().is_some_and(|(_, ch)| ch.is_whitespace()) {
            chars.next();
        }
        let (_, first) = chars.next()?;
        let mut value = String::new();
        if matches!(first, '\'' | '"') {
            loop {
                let (_, ch) = chars.next()?;
                if ch == first {
                    break;
                }
                if ch == '\\' {
                    value.push(match chars.next()?.1 {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        ch => ch,
                    });
                } else {
                    value.push(ch);
                }
            }
        } else {
            value.push(first);
            while let Some(&(_, ch)) = chars.peek() {
                if ch.is_whitespace() || ch == '}' {
                    break;
                }
                value.push(ch);
                chars.next();
            }
        }
        result.insert(key, value);
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(part),
        }
    }
    normalized
}

pub fn model_comments(text: &str, cwd: &str, roots: &[String]) -> Vec<Comment> {
    let excluded = pulldown_cmark::Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                pulldown_cmark::Event::Code(_)
                    | pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(_))
            )
            .then_some(range)
        })
        .collect::<Vec<_>>();
    let mut comments = Vec::new();
    for (offset, _) in text.match_indices("::code-comment{") {
        if excluded.iter().any(|range| range.contains(&offset)) {
            continue;
        }
        let before = &text[..offset];
        if !before
            .rsplit_once('\n')
            .map_or(before, |(_, line)| line)
            .trim()
            .is_empty()
        {
            continue;
        }
        let Some((values, _)) = attributes(&text[offset + "::code-comment{".len()..]) else {
            continue;
        };
        let (Some(title), Some(body), Some(file)) =
            (values.get("title"), values.get("body"), values.get("file"))
        else {
            continue;
        };
        if title.trim().is_empty() || body.trim().is_empty() || file.trim().is_empty() {
            continue;
        }
        let number = |key: &str, fallback: u32| -> u32 {
            values
                .get(key)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|n| n.is_finite())
                .map(|n| n.trunc().clamp(1., u32::MAX as f64) as u32)
                .unwrap_or(fallback)
        };
        let start = number("start", 1);
        let end = number("end", start).max(start);
        let mut title = title.trim().to_owned();
        if let Some(priority) = values
            .get("priority")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            && !(title
                .as_bytes()
                .get(0..2)
                .is_some_and(|start| start.eq_ignore_ascii_case(b"[p"))
                && title.as_bytes().get(2).is_some_and(u8::is_ascii_digit)
                && title.as_bytes().get(3) == Some(&b']'))
        {
            title = format!("[P{}] {title}", priority.trunc());
        }
        let absolute = normalize(&Path::new(cwd).join(file));
        let root = roots
            .iter()
            .filter(|root| absolute.starts_with(root.as_str()))
            .max_by_key(|root| root.len())
            .map_or(cwd, String::as_str);
        let path = absolute
            .strip_prefix(root)
            .unwrap_or(&absolute)
            .to_string_lossy()
            .into_owned();
        comments.push(Comment {
            kind: None,
            content: vec![Text {
                content_type: "text".into(),
                text: format!("{title}\n\n{}", body.trim()),
            }],
            position: Position {
                path,
                side: Side::Right,
                line: NonZeroU32::new(end).unwrap(),
                start_line: (start != end).then(|| NonZeroU32::new(start).unwrap()),
                start_side: None,
            },
            local_diff_hunk: None,
            local_workspace_root: (!root.is_empty()).then(|| root.to_owned()),
            extra: Default::default(),
        });
    }
    comments
}

pub fn prompt_body(text: &str) -> &str {
    if text.starts_with("# Diff comments:\n") {
        text.rsplit_once("\n## My request:\n")
            .map_or(text, |(_, body)| body)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_directives_preserve_ranges_and_skip_code_examples() {
        let text = "```\n::code-comment{title=\"Example\" body=\"not feedback\" file=\"example.rs\"}\n```\n::code-comment{title=\"Fix the branch\" body=\"A } stays in quoted text\" file=\"src/a.rs\" start=4 end=6 priority=2}";
        let comments = model_comments(text, "/repo", &[]);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].position.path, "src/a.rs");
        assert_eq!(comments[0].position.range(), "4-6");
        assert_eq!(
            comments[0].text(),
            "[P2] Fix the branch\n\nA } stays in quoted text"
        );
    }

    #[test]
    fn local_comment_prompt_keeps_side_and_selected_hunk() {
        let comment = Comment::new(
            Position {
                path: "a.rs".into(),
                side: Side::Right,
                line: NonZeroU32::new(9).unwrap(),
                start_line: NonZeroU32::new(3),
                start_side: Some(Side::Left),
            },
            "Use the checked value".into(),
            Some("-old\n+new".into()),
            Some("/repo".into()),
        );
        assert_eq!(
            comment.prompt(1),
            "\n## User Comment 1\nFile: a.rs\nWorkspace: /repo\nSide: L-R\nLines: L3-R9\nDiff hunk:\n```diff\n-old\n+new\n```\nComment:\nUse the checked value\n"
        );
        assert_eq!(comment.position.display_range(), "L3 to R9");
        let prompt = format!(
            "# Diff comments:\n{}\n## My request:\nFix this.",
            comment.prompt(1)
        );
        assert_eq!(prompt_body(&prompt), "Fix this.");
        assert_eq!(
            prompt_body("## My request:\nKeep this literal"),
            "## My request:\nKeep this literal"
        );
    }
}
