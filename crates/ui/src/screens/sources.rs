//! `Sources` side panel tab (`local-conversation-sources-side-panel-tab`): the files the chat
//! was given, read, created or updated, its web searches and its tool use counts, newest
//! first, derived from the thread's items.

use bridge::types::{ThreadItem, Turn, UserInput};
use gpui::{
    Context, Div, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::*, px,
};
use serde_json::Value;
use theme::{ActiveTheme as _, Theme};

use crate::icon::icon;
use crate::screens::diff::panel::OpenFile;
use crate::screens::thread::view::ThreadView;
use crate::text;

/// How the chat used a file (`codex.localConversation.sources.panel.activity.*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activity {
    Provided,
    Read,
    Created,
    Updated,
}

impl Activity {
    fn label(self) -> &'static str {
        match self {
            Activity::Provided => "Attached to the conversation",
            Activity::Read => "Read during the chat",
            Activity::Created => "Created during the chat",
            Activity::Updated => "Updated during the chat",
        }
    }
}

/// `webSearch` items: distinct queries, every query sent, and the distinct pages opened.
#[derive(Debug, Default, PartialEq)]
pub struct WebSearch {
    pub queries: Vec<String>,
    pub query_count: usize,
    pub pages: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Sources {
    /// `(path, activities)`, newest first.
    pub files: Vec<(String, Vec<Activity>)>,
    /// `(MCP server, [(tool, calls)])`, newest first.
    pub tools: Vec<(String, Vec<(String, usize)>)>,
    pub web: Option<WebSearch>,
}

impl Sources {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.tools.is_empty() && self.web.is_none()
    }

    fn file(&mut self, path: String, activity: Activity) {
        match self.files.iter_mut().find(|(known, _)| *known == path) {
            Some((_, activities)) if !activities.contains(&activity) => activities.push(activity),
            Some(_) => {}
            None => self.files.push((path, vec![activity])),
        }
    }

    fn tool(&mut self, server: &str, tool: &str) {
        let tools = match self.tools.iter().position(|(known, _)| known == server) {
            Some(ix) => &mut self.tools[ix].1,
            None => {
                self.tools.push((server.to_owned(), Vec::new()));
                &mut self.tools.last_mut().unwrap().1
            }
        };
        match tools.iter_mut().find(|(known, _)| known == tool) {
            Some((_, calls)) => *calls += 1,
            None => tools.push((tool.to_owned(), 1)),
        }
    }
}

/// The sources of `turns`, walking the newest item first as the original's model does.
pub fn sources(turns: &[Turn]) -> Sources {
    let mut out = Sources::default();
    let mut web = WebSearch::default();
    let mut searched = false;
    for item in turns.iter().rev().flat_map(|turn| turn.items.iter().rev()) {
        match item {
            ThreadItem::UserMessage { content, .. } => {
                for input in content {
                    if let UserInput::LocalImage { path } = input {
                        out.file(path.clone(), Activity::Provided);
                    }
                }
            }
            ThreadItem::CommandExecution {
                cwd,
                command_actions,
                ..
            } => {
                for action in command_actions.iter().filter(|a| a.kind == "read") {
                    let Some(path) = action.path.as_deref().or(action.name.as_deref()) else {
                        continue;
                    };
                    let path = if path.starts_with('/') {
                        path.to_owned()
                    } else {
                        format!("{}/{path}", cwd.trim_end_matches('/'))
                    };
                    out.file(path, Activity::Read);
                }
            }
            ThreadItem::FileChange {
                changes, status, ..
            } if !matches!(status.as_str(), "declined" | "failed") => {
                for change in changes {
                    let activity = if change.kind["type"] == "add" {
                        Activity::Created
                    } else {
                        Activity::Updated
                    };
                    let path = change.kind["move_path"].as_str().unwrap_or(&change.path);
                    out.file(path.to_owned(), activity);
                }
            }
            // `codex_app` is the host's own tool server (scheduled task cards), not a source.
            ThreadItem::McpToolCall { server, tool, .. } if server != "codex_app" => {
                out.tool(server, tool)
            }
            ThreadItem::WebSearch { query, action, .. } => {
                searched = true;
                let mut search = |query: &str| {
                    let query = query.trim();
                    if !query.is_empty() {
                        web.query_count += 1;
                        if !web.queries.iter().any(|known| known == query) {
                            web.queries.push(query.to_owned());
                        }
                    }
                };
                match action.as_ref().map(|a| (a["type"].as_str(), a)) {
                    None => search(query),
                    Some((Some("search"), action)) => {
                        let queries = action["queries"].as_array().filter(|q| !q.is_empty());
                        match queries {
                            Some(queries) => {
                                queries.iter().filter_map(Value::as_str).for_each(search)
                            }
                            None => search(action["query"].as_str().unwrap_or(query)),
                        }
                    }
                    Some((Some("openPage" | "findInPage"), action)) => {
                        if let Some(url) = action["url"].as_str()
                            && !web.pages.iter().any(|known| known == url)
                        {
                            web.pages.push(url.to_owned());
                        }
                    }
                    Some(_) => search(query),
                }
            }
            _ => {}
        }
    }
    if searched && (web.query_count > 0 || !web.pages.is_empty()) {
        out.web = Some(web);
    }
    out
}

/// `{count, plural, one {once} other {# times}}`.
fn times(count: usize) -> String {
    if count == 1 {
        "once".into()
    } else {
        format!("{count} times")
    }
}

/// The side panel tab: re-derived from the thread on every render, so it follows the turn
/// live.
pub struct SourcesView {
    thread: Entity<ThreadView>,
}

impl SourcesView {
    pub fn new(thread: Entity<ThreadView>, cx: &mut Context<Self>) -> Self {
        cx.observe(&thread, |_, _, cx| cx.notify()).detach();
        Self { thread }
    }

    pub fn thread(&self) -> &Entity<ThreadView> {
        &self.thread
    }
}

/// `SourceDetails`: an icon, the label and its secondary meta line, then the detail lines.
fn details(
    id: impl Into<gpui::ElementId>,
    icon_name: &'static str,
    label: impl Into<SharedString>,
    meta: Option<String>,
    lines: Vec<String>,
    t: &Theme,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .px(px(8.))
        .py(px(6.))
        .rounded(Theme::RADIUS_SM)
        .flex()
        .gap(px(8.))
        .child(
            div()
                .pt(px(1.))
                .child(icon(icon_name, 16., t.text_secondary)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(text(label, 13., 18., 500., t.text))
                .children(meta.map(|meta| text(meta, 12., 16., 400., t.text_tertiary)))
                .children(
                    lines
                        .into_iter()
                        .map(|line| text(line, 13., 20., 400., t.text_secondary)),
                ),
        )
}

impl Render for SourcesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let sources = sources(self.thread.read(cx).turns());
        let hover = t.ghost_hover;
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (ix, (path, activities)) in sources.files.into_iter().enumerate() {
            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
            let image = ["png", "jpg", "jpeg", "gif", "webp"]
                .iter()
                .any(|ext| path.to_lowercase().ends_with(&format!(".{ext}")));
            let lines = activities.iter().map(|a| a.label().to_owned()).collect();
            let open = path.clone();
            rows.push(
                details(
                    ("source-file", ix),
                    if image {
                        "diff-file-image"
                    } else {
                        "diff-file-default"
                    },
                    name,
                    Some(path),
                    lines,
                    &t,
                )
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(OpenFile {
                            path: open.clone(),
                            line: None,
                        }),
                        cx,
                    )
                })
                .into_any_element(),
            );
        }
        for (ix, (server, tools)) in sources.tools.into_iter().enumerate() {
            let lines = tools
                .iter()
                .map(|(tool, calls)| format!("{tool} {}", times(*calls)))
                .collect();
            rows.push(
                details(("source-tool", ix), "thread-mcp", server, None, lines, &t)
                    .into_any_element(),
            );
        }
        if let Some(web) = sources.web {
            let mut lines = Vec::new();
            if web.query_count > 0 {
                lines.push(if web.query_count == 1 {
                    "Searched once".to_owned()
                } else {
                    format!("Searched {} times", web.query_count)
                });
                lines.extend(web.queries.iter().map(|q| format!("· {q}")));
            }
            rows.push(
                details("source-web", "cloud-web", "Web search", None, lines, &t)
                    .into_any_element(),
            );
            if !web.pages.is_empty() {
                rows.push(
                    div()
                        .pl(px(32.))
                        .child(text(
                            if web.pages.len() == 1 {
                                "Opened one page".to_owned()
                            } else {
                                format!("Opened {} pages", web.pages.len())
                            },
                            13.,
                            20.,
                            400.,
                            t.text_secondary,
                        ))
                        .into_any_element(),
                );
                for (ix, url) in web.pages.into_iter().enumerate() {
                    let open = url.clone();
                    rows.push(
                        div()
                            .id(("source-page", ix))
                            .ml(px(24.))
                            .px(px(8.))
                            .py(px(2.))
                            .rounded(Theme::RADIUS_SM)
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .on_click(move |_, _, cx| cx.open_url(&open))
                            .child(text(url, 13., 20., 400., t.text_link))
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .id("sources-panel")
            .role(gpui::Role::List)
            .aria_label("Sources")
            .size_full()
            .overflow_y_scroll()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(rows.is_empty(), |d| {
                d.child(div().p(px(8.)).child(text(
                    "No sources yet",
                    13.,
                    18.,
                    400.,
                    t.text_tertiary,
                )))
            })
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turns(items: Value) -> Vec<Turn> {
        serde_json::from_value(json!([{
            "id": "t1", "items": items, "status": "completed", "error": null,
            "startedAt": null, "completedAt": null, "durationMs": null
        }]))
        .unwrap()
    }

    #[test]
    fn files_merge_activities_newest_first() {
        let sources = sources(&turns(json!([
            {"type": "userMessage", "id": "u", "content": [
                {"type": "text", "text": "fix"}, {"type": "localImage", "path": "/w/shot.png"}]},
            {"type": "commandExecution", "id": "c", "command": "cat a.py", "cwd": "/w/",
             "status": "completed", "aggregatedOutput": null, "exitCode": 0, "durationMs": 1,
             "commandActions": [{"type": "read", "command": "cat a.py", "name": "a.py"},
                                {"type": "search", "command": "rg x", "name": null}]},
            {"type": "fileChange", "id": "f", "status": "completed", "changes": [
                {"path": "/w/a.py", "kind": {"type": "update"}, "diff": ""},
                {"path": "/w/b.py", "kind": {"type": "add"}, "diff": ""}]},
            {"type": "fileChange", "id": "g", "status": "declined", "changes": [
                {"path": "/w/c.py", "kind": {"type": "add"}, "diff": ""}]}
        ])));
        assert_eq!(
            sources.files,
            [
                ("/w/a.py".into(), vec![Activity::Updated, Activity::Read]),
                ("/w/b.py".into(), vec![Activity::Created]),
                ("/w/shot.png".into(), vec![Activity::Provided]),
            ]
        );
    }

    #[test]
    fn tools_count_calls_per_server_and_skip_host_tools() {
        let call = |server: &str, tool: &str| {
            json!({"type": "mcpToolCall", "id": tool, "server": server, "tool": tool,
                   "status": "completed", "result": null, "error": null, "durationMs": 1})
        };
        let sources = sources(&turns(json!([
            call("github", "search"),
            call("github", "search"),
            call("github", "get_pr"),
            call("codex_app", "automation_update"),
        ])));
        assert_eq!(
            sources.tools,
            [(
                "github".into(),
                vec![("get_pr".into(), 1), ("search".into(), 2)]
            )]
        );
    }

    #[test]
    fn web_search_counts_queries_and_distinct_pages() {
        let sources = sources(&turns(json!([
            {"type": "webSearch", "id": "w1", "query": "rust gpui"},
            {"type": "webSearch", "id": "w2", "query": "",
             "action": {"type": "search", "queries": ["rust gpui", "zed"]}},
            {"type": "webSearch", "id": "w3", "query": "",
             "action": {"type": "openPage", "url": "https://zed.dev"}},
            {"type": "webSearch", "id": "w4", "query": "",
             "action": {"type": "findInPage", "url": "https://zed.dev", "pattern": "x"}}
        ])));
        assert_eq!(
            sources.web,
            Some(WebSearch {
                queries: vec!["rust gpui".into(), "zed".into()],
                query_count: 3,
                pages: vec!["https://zed.dev".into()],
            })
        );
        assert!(super::sources(&turns(json!([]))).is_empty());
    }
}
