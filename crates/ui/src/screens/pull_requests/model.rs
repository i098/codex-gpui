use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub host_id: String,
    pub hostname: String,
    pub login: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub hostname: String,
    pub owner: String,
    pub repository: String,
    pub number: u64,
}

impl Identity {
    pub fn url(&self) -> String {
        format!(
            "https://{}/{}/{}/pull/{}",
            self.hostname, self.owner, self.repository, self.number
        )
    }

    pub fn valid(&self) -> bool {
        self.number > 0
            && self.number <= 9_007_199_254_740_991
            && valid_component(&self.hostname)
            && valid_component(&self.owner)
            && valid_component(&self.repository)
    }
}

pub fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

pub fn limit_query(value: &str) -> &str {
    let mut units = 0;
    let end = value
        .char_indices()
        .find_map(|(index, character)| {
            units += character.len_utf16();
            (units > 256).then_some(index)
        })
        .unwrap_or(value.len());
    &value[..end]
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub node_id: String,
    pub pull_request: Identity,
    pub title: String,
    pub state: String,
    pub head_branch: String,
    pub base_branch: String,
    pub author_login: Option<String>,
    pub author_avatar_url: Option<String>,
    pub updated_at: String,
    pub additions: u64,
    pub deletions: u64,
    pub has_diff_stats: bool,
    pub is_draft: bool,
    pub ci_status: String,
    pub can_merge: bool,
    pub merge_blocker: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<Item>,
    pub end_cursor: Option<String>,
    pub has_next_page: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum View {
    #[default]
    All,
    Reviewing,
    Authored,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Repository {
    pub owner: String,
    pub repository: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    pub lifecycle: String,
    pub relationship: String,
    pub repository: Option<Repository>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_query: Option<String>,
}

const QUALIFIERS: &str = "archived assignee author base checks closed commenter comments created draft head in interactions involves is label language linked merged mentions milestone no org project reactions repo review review-involves review-requested reviewed-by sort state status team team-review-requested type updated user user-review-requested";
const RELATIONSHIPS: &str = "assignee author commenter involves mentions review-involves review-requested reviewed-by team team-review-requested user-review-requested";
pub const SECTION_KEYS: [&str; 4] = [
    "user_review_requested",
    "review_requested",
    "reviewed",
    "authored",
];
pub const SECTION_TITLES: [&str; 4] = [
    "Needs my review",
    "Needs my team’s review",
    "Previously reviewed",
    "Authored",
];

pub fn qualifiers(text: &str) -> Vec<(&str, &str)> {
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    let mut out = Vec::new();
    for (at, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
        }
        if !quoted && (ch.is_whitespace() || ch == '(' || ch == ')') {
            let word = text[start..at].trim_start_matches('-');
            if let Some((key, value)) = word.split_once(':') {
                out.push((key, value));
            }
            start = at + ch.len_utf8();
        }
    }
    out
}

pub fn filters(
    text: &str,
    view: View,
    lifecycle: &str,
    repository: Option<Repository>,
) -> (Filters, bool, bool) {
    let words = qualifiers(text);
    let known = |key: &str| {
        QUALIFIERS
            .split_whitespace()
            .any(|q| q.eq_ignore_ascii_case(key))
    };
    let relationship = words.iter().any(|(key, _)| {
        RELATIONSHIPS
            .split_whitespace()
            .any(|q| q.eq_ignore_ascii_case(key))
    });
    let explicit_lifecycle = words.iter().any(|(key, value)| {
        ["closed", "draft", "merged", "state"]
            .iter()
            .any(|q| key.eq_ignore_ascii_case(q))
            || key.eq_ignore_ascii_case("is")
                && ["closed", "draft", "merged", "open", "unmerged"]
                    .iter()
                    .any(|q| value.eq_ignore_ascii_case(q))
    });
    let repository = if words.iter().any(|(key, _)| {
        ["org", "repo", "user"]
            .iter()
            .any(|q| key.eq_ignore_ascii_case(q))
    }) {
        None
    } else {
        repository
    };
    (
        Filters {
            lifecycle: if explicit_lifecycle {
                "all"
            } else if view == View::Reviewing {
                "open"
            } else {
                lifecycle
            }
            .to_owned(),
            relationship: "all".into(),
            repository,
            text: text.to_owned(),
            raw_query: words
                .iter()
                .any(|(key, _)| known(key))
                .then(|| text.to_owned()),
        },
        relationship,
        explicit_lifecycle,
    )
}

impl Filters {
    pub fn search_query(&self) -> String {
        let words = qualifiers(self.raw_query.as_deref().unwrap_or(""));
        let has = |key: &str| words.iter().any(|(q, _)| q.eq_ignore_ascii_case(key));
        let mut query = vec!["is:pr".to_owned()];
        if !has("archived") {
            query.push("archived:false".into());
        }
        let relation = match self.relationship.as_str() {
            "authored" => Some("author:@me"),
            "review_requested" => Some("review-requested:@me"),
            "reviewed" => Some("reviewed-by:@me"),
            "user_review_requested" => Some("user-review-requested:@me"),
            _ => None,
        };
        if let Some(relation) = relation {
            query.push(relation.into());
        }
        match self.lifecycle.as_str() {
            "open" => query.push("is:open".into()),
            "merged" => query.push("is:merged".into()),
            "closed" => query.extend(["is:closed".into(), "is:unmerged".into()]),
            _ => {}
        }
        if let Some(repo) = &self.repository {
            query.push(format!("repo:{}/{}", repo.owner, repo.repository));
        }
        if let Some(text) = self.query_text() {
            query.push(text);
        }
        if !has("sort") {
            query.push("sort:updated-desc".into());
        }
        query.join(" ")
    }

    fn query_text(&self) -> Option<String> {
        if let Some(raw) = &self.raw_query {
            return Some(raw.trim().to_owned());
        }
        let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty())
            .then(|| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")))
    }
}

#[derive(Clone, Debug)]
pub struct Stream {
    pub key: &'static str,
    pub title: &'static str,
    pub items: Vec<Item>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub cursor: Option<String>,
    pub expanded: bool,
}

impl Stream {
    pub fn new(key: &'static str, title: &'static str) -> Self {
        Self {
            key,
            title,
            items: Vec::new(),
            loaded: false,
            loading: true,
            error: None,
            cursor: None,
            expanded: true,
        }
    }

    pub fn apply(&mut self, result: Result<SearchPage, String>, more: bool) {
        self.loading = false;
        match result {
            Ok(page) => {
                self.error = None;
                self.loaded = true;
                self.cursor = if page.has_next_page {
                    page.end_cursor
                } else {
                    None
                };
                if !more {
                    self.items.clear();
                }
                let mut seen: HashSet<_> = self.items.iter().map(|p| p.node_id.clone()).collect();
                self.items.extend(
                    page.items
                        .into_iter()
                        .filter(|p| seen.insert(p.node_id.clone())),
                );
            }
            Err(error) => self.error = Some(error),
        }
    }
}

pub fn section_items(streams: &[Stream]) -> Vec<Vec<&Item>> {
    if streams.is_empty() {
        return Vec::new();
    }
    if streams.len() == 1 {
        return vec![streams[0].items.iter().collect()];
    }
    let failed = streams[0].error.is_some();
    let mut seen = HashSet::new();
    let mut sections: Vec<Vec<&Item>> = streams.iter().map(|_| Vec::new()).collect();
    for (i, stream) in streams.iter().enumerate() {
        for item in &stream.items {
            if seen.insert(&item.node_id) {
                if i == 1 && !failed && !streams[0].loaded {
                    continue;
                }
                let target = if i == 1 && failed { 0 } else { i };
                sections[target].push(item);
            }
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str) -> Item {
        Item {
            node_id: id.into(),
            pull_request: Identity {
                hostname: "github.com".into(),
                owner: "owner".into(),
                repository: "repo".into(),
                number: 1,
            },
            title: "Review".into(),
            state: "open".into(),
            head_branch: "change".into(),
            base_branch: "main".into(),
            author_login: None,
            author_avatar_url: None,
            updated_at: "2026-10-20T00:00:00Z".into(),
            additions: 1,
            deletions: 0,
            has_diff_stats: true,
            is_draft: false,
            ci_status: "none".into(),
            can_merge: false,
            merge_blocker: None,
        }
    }

    #[test]
    fn explicit_qualifiers_override_controls_without_reading_quoted_text() {
        let repo = Some(Repository {
            owner: "owner".into(),
            repository: "repo".into(),
        });
        let (query, relation, lifecycle) = filters(
            "author:me is:closed repo:x/y sort:created-desc",
            View::Reviewing,
            "open",
            repo.clone(),
        );
        assert!(relation && lifecycle);
        assert_eq!(query.repository, None);
        assert_eq!(
            query.search_query(),
            "is:pr archived:false author:me is:closed repo:x/y sort:created-desc"
        );
        let (query, relation, lifecycle) =
            filters("\"author:me is:closed\"", View::Reviewing, "merged", repo);
        assert!(!relation && !lifecycle);
        assert_eq!(query.lifecycle, "open");
        assert!(query.raw_query.is_none());
    }

    #[test]
    fn failed_pagination_retains_rows_and_cursor() {
        let mut stream = Stream::new("authored", "Authored");
        stream.apply(
            Ok(SearchPage {
                items: vec![item("first")],
                end_cursor: Some("next".into()),
                has_next_page: true,
            }),
            false,
        );
        stream.loading = true;
        stream.apply(Err("network".into()), true);
        assert!(stream.loaded);
        assert!(!stream.loading);
        assert_eq!(stream.cursor.as_deref(), Some("next"));
        assert_eq!(stream.error.as_deref(), Some("network"));
        assert_eq!(stream.items[0].node_id, "first");
        assert_eq!(stream.items.len(), 1);
    }

    #[test]
    fn direct_review_failure_uses_team_results_without_duplicate_authored_rows() {
        let mut streams: Vec<_> = SECTION_KEYS
            .iter()
            .zip(SECTION_TITLES)
            .map(|(key, title)| Stream::new(key, title))
            .collect();
        streams[0].apply(Err("direct query failed".into()), false);
        streams[1].apply(
            Ok(SearchPage {
                items: vec![item("team")],
                ..Default::default()
            }),
            false,
        );
        streams[2].apply(
            Ok(SearchPage {
                items: vec![item("team"), item("reviewed")],
                ..Default::default()
            }),
            false,
        );
        streams[3].apply(
            Ok(SearchPage {
                items: vec![item("reviewed"), item("authored")],
                ..Default::default()
            }),
            false,
        );
        let rows: Vec<Vec<&str>> = section_items(&streams)
            .iter()
            .map(|items| items.iter().map(|item| item.node_id.as_str()).collect())
            .collect();
        assert_eq!(
            rows,
            vec![vec!["team"], vec![], vec!["reviewed"], vec!["authored"]]
        );
    }

    #[test]
    fn search_limit_counts_utf16_without_splitting_a_character() {
        let value = format!("{}a𐐀", "𐐀".repeat(127));
        assert_eq!(limit_query(&value), format!("{}a", "𐐀".repeat(127)));
        assert_eq!(limit_query(&value).encode_utf16().count(), 255);
    }
}
