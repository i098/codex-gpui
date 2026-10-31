//! Native Codex URL parsing and page-family dispatch (SH-151..155).

use crate::{Root, Route, State, windows};
use gpui::{App, AppContext as _, Context, Focusable, Window};
use std::borrow::Cow;
use ui::screens::connections::Tab;
use url::Url;

mod navigation;

/// A supported native destination. Excluded products do not produce a route.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    NewThread {
        prompt: Option<String>,
        path: Option<String>,
        project_id: Option<String>,
        origin_url: Option<String>,
    },
    Thread {
        id: String,
        prompt: Option<String>,
        host_id: Option<String>,
        shared: bool,
    },
    Settings(Option<&'static str>),
    Connections(Tab),
    AddSsh {
        name: String,
        project_path: Option<String>,
        enabled: bool,
    },
    Scheduled,
    Automation {
        id: String,
        source: &'static str,
        edit: bool,
    },
    OAuth(String),
    Launch,
}

/// Parses the original native URL grammar without accepting excluded products.
pub fn parse(value: &str) -> Option<Link> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "codex" | "codex-dev")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let destination = url.host_str()?;
    let sub: Vec<_> = url.path().split('/').filter(|s| !s.is_empty()).collect();
    let params = Params(url.query_pairs().collect());
    match destination {
        "threads" if sub.as_slice() == ["new"] => new_thread(&params, false),
        "threads" => thread(sub.first()?, &params),
        "new" if sub.is_empty() => new_thread(&params, true),
        "settings" => Some(settings(&sub, &params)),
        "automations" if sub.is_empty() => Some(Link::Scheduled),
        "automations" if sub.len() == 1 => automation(sub[0], &params),
        "connector" if sub.as_slice() == ["oauth_callback"] => Some(Link::OAuth(value.to_owned())),
        "launch" if sub.is_empty() => Some(Link::Launch),
        _ => None,
    }
}

fn thread(id: &str, params: &Params<'_>) -> Option<Link> {
    if !uuid(id) {
        return None;
    }
    params.browser()?;
    let access = params.text("threadAccess");
    if access.is_some_and(|access| access != "shared") {
        return None;
    }
    Some(Link::Thread {
        id: id.to_owned(),
        prompt: params.value("prompt"),
        host_id: params.value("hostId"),
        shared: access == Some("shared"),
    })
}

fn automation(id: &str, params: &Params<'_>) -> Option<Link> {
    let source = match params.text("automationSource") {
        None | Some("cloud") => "cloud",
        Some("local") => "local",
        _ => return None,
    };
    Some(Link::Automation {
        id: component(id)?,
        source,
        edit: params.raw("edit") == Some("true"),
    })
}

fn new_thread(params: &Params<'_>, gated: bool) -> Option<Link> {
    let keyed = ["browserUrl", "mode", "originUrl", "path", "prompt"]
        .iter()
        .any(|key| params.raw(key).is_some());
    if !gated && !keyed {
        return Some(Link::NewThread {
            prompt: None,
            path: None,
            project_id: None,
            origin_url: None,
        });
    }
    if params.0.iter().filter(|(key, _)| key == "mode").count() > 1
        || params
            .raw("mode")
            .is_some_and(|mode| !matches!(mode, "codex" | "chat"))
    {
        return None;
    }
    params.browser()?;
    let prompt = params.value("prompt");
    let path = params.value("path");
    let project_id = params.value("projectId");
    let origin_url = params.value("originUrl");
    if prompt.is_none()
        && path.is_none()
        && project_id.is_none()
        && origin_url.is_none()
        && params.text("browserUrl").is_none()
        && params.raw("mode").is_none()
    {
        return None;
    }
    Some(Link::NewThread {
        prompt,
        path,
        project_id,
        origin_url,
    })
}

fn settings(sub: &[&str], params: &Params<'_>) -> Link {
    match sub {
        ["connections"] | ["connections", "computer"] => Link::Connections(Tab::Computer),
        ["connections", "devices"] => Link::Connections(Tab::Devices),
        ["connections", "ssh", "add"] => {
            let valid = params.0.iter().all(|(key, _)| {
                matches!(key.as_ref(), "name" | "alias" | "projectPath" | "enabled")
                    && params.0.iter().filter(|(other, _)| other == key).count() == 1
            }) && !(params.raw("name").is_some() && params.raw("alias").is_some())
                && params
                    .raw("enabled")
                    .is_none_or(|v| matches!(v, "true" | "false"));
            let name = params.text("name").or_else(|| params.text("alias"));
            match name.filter(|_| valid) {
                Some(name) => Link::AddSsh {
                    name: name.trim().to_owned(),
                    project_path: params
                        .text("projectPath")
                        .map(|path| path.trim().to_owned()),
                    enabled: params.raw("enabled") == Some("true"),
                },
                None => Link::Connections(Tab::Ssh),
            }
        }
        ["connections", "ssh"] => Link::Connections(Tab::Ssh),
        ["browser-use"] => Link::Settings(Some("Browser")),
        _ => Link::Settings(None),
    }
}

struct Params<'a>(Vec<(Cow<'a, str>, Cow<'a, str>)>);

impl Params<'_> {
    fn raw(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_ref())
    }

    fn text(&self, name: &str) -> Option<&str> {
        self.raw(name).filter(|value| !value.trim().is_empty())
    }

    fn value(&self, name: &str) -> Option<String> {
        self.text(name).map(str::to_owned)
    }

    fn browser(&self) -> Option<()> {
        let browser = self.text("browserUrl");
        if let Some(value) = browser {
            let url = Url::parse(value).ok()?;
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
        }
        if self.raw("browserTabId").is_some() && browser.is_none() {
            return None;
        }
        if let Some(active) = self.raw("browserActive")
            && (browser.is_none() || !matches!(active, "true" | "false"))
        {
            return None;
        }
        Some(())
    }
}

/// `decodeURIComponent` plus trim, like the original's `tD`; invalid escapes and empty
/// results are `None`.
fn component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let h = *bytes.get(i + 1)?;
            let l = *bytes.get(i + 2)?;
            let hex = |b: u8| (b as char).is_ascii_hexdigit().then_some(hex_val(b));
            out.push(hex(h)? * 16 + hex(l)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(out).ok()?;
    let trimmed = decoded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        _ => b - b'A' + 10,
    }
}

impl Root {
    /// Navigates to `link` in this window: the deep-link route, then a prompt prefill
    /// (and focus) of the route's composer.
    pub fn open_deep_link(&mut self, link: &Link, window: &mut Window, cx: &mut Context<Self>) {
        if self.host_id() != "local"
            && !matches!(link, Link::Thread { .. } | Link::OAuth(_) | Link::Launch)
        {
            self.open_link_host("local", link, window, cx);
            return;
        }
        let mut prompt = None;
        match link {
            Link::NewThread {
                prompt,
                path,
                project_id,
                origin_url,
            } => {
                self.open_link_project(
                    project_id.clone(),
                    path.clone(),
                    origin_url.clone(),
                    prompt.clone(),
                    window,
                    cx,
                );
                return;
            }
            Link::Thread {
                id,
                prompt: prefill,
                host_id,
                shared,
            } => {
                let host_id = host_id.as_deref().unwrap_or("local");
                if host_id != self.host_id() {
                    self.open_link_host(host_id, link, window, cx);
                    return;
                }
                self.bridge.set_thread_access(id, *shared);
                self.navigate(Route::Thread(id.clone()), window, cx);
                prompt = prefill.clone();
            }
            Link::Settings(None) => self.navigate(Route::Settings, window, cx),
            Link::Settings(Some(label)) => self.open_settings_section(label, window, cx),
            Link::Connections(tab) => self.open_connections(*tab, window, cx),
            Link::AddSsh {
                name,
                project_path,
                enabled,
            } => {
                self.open_ssh_link(name.clone(), project_path.clone(), *enabled, window, cx);
                return;
            }
            Link::Scheduled => self.navigate(Route::Scheduled, window, cx),
            Link::Automation { id, source, edit } => {
                self.open_automation_link(id.clone(), source, *edit, window, cx);
                return;
            }
            Link::OAuth(value) => {
                self.receive_oauth_redirect(value, window, cx);
                return;
            }
            Link::Launch => {}
        }
        if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
            let composer = self.composer(cx);
            composer.update(cx, |composer, cx| composer.set_text(&prompt, cx));
            let focus = composer.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
    }
}

/// Opens a native URL in its owning window, or the primary window.
pub fn open(url: &str, cx: &mut App) {
    let Some(link) = parse(url) else {
        return;
    };
    let owner = match &link {
        Link::OAuth(url) => Url::parse(url).ok().and_then(|url| {
            let state = url
                .query_pairs()
                .find(|(key, _)| key == "state")?
                .1
                .into_owned();
            cx.windows()
                .into_iter()
                .filter_map(|window| window.downcast::<Root>())
                .find(|window| {
                    window
                        .read(cx)
                        .is_ok_and(|root| root.owns_oauth_state(&state))
                })
        }),
        Link::Thread { host_id, .. } => {
            navigation::host_window(host_id.as_deref().unwrap_or("local"), None, cx)
        }
        _ => navigation::host_window("local", None, cx),
    };
    if let Some(handle) =
        owner.or_else(|| windows::primary(cx).and_then(|window| window.downcast::<Root>()))
    {
        let result = handle
            .update(cx, |root, window, cx| {
                root.open_deep_link(&link, window, cx);
            })
            .and_then(|()| windows::show(handle.into(), windows::Presentation::Foreground, cx));
        if let Err(error) = result {
            eprintln!("codex-gpui: cannot open deep link: {error:#}");
        }
        return;
    }
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if let Ok(handle) = windows::open_primary(State::load(), cwd, cx) {
        let _ = handle.update(cx, |root, window, cx| {
            root.open_deep_link(&link, window, cx);
        });
    }
}

/// Opens a supported path in the primary window.
pub fn open_path(path: &str, cx: &mut App) -> bool {
    let Some(route) = Route::remote_path(path)
        .or_else(|| Route::pull_request_path(path))
        .or_else(|| crate::onboarding::route(path))
    else {
        return false;
    };
    if let Some(handle) = windows::primary(cx).and_then(|window| window.downcast::<Root>()) {
        let result = handle
            .update(cx, |root, window, cx| root.navigate(route, window, cx))
            .and_then(|()| windows::show(handle.into(), windows::Presentation::Foreground, cx));
        if let Err(error) = result {
            eprintln!("codex-gpui: cannot open launch path: {error:#}");
        }
        return true;
    }

    let mut state = State::load();
    state.route = route;
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if let Err(error) = windows::open_primary(state, cwd, cx) {
        eprintln!("codex-gpui: cannot open launch path: {error:#}");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn preserves_thread_host_access_and_prompt() {
        assert_eq!(
            parse(&format!(
                "codex://threads/{ID}/review?hostId=ssh%3Adev&threadAccess=shared&prompt=%20a%2Bb%20"
            )),
            Some(Link::Thread {
                id: ID.into(),
                host_id: Some("ssh:dev".into()),
                shared: true,
                prompt: Some(" a+b ".into()),
            })
        );
        assert!(parse(&format!("codex://threads/{ID}?threadAccess=private")).is_none());
    }

    #[test]
    fn preserves_new_thread_project_precedence_inputs() {
        assert_eq!(
            parse("codex://new?projectId=p%2Fq&path=relative&originUrl=repo&prompt=hi"),
            Some(Link::NewThread {
                project_id: Some("p/q".into()),
                path: Some("relative".into()),
                origin_url: Some("repo".into()),
                prompt: Some("hi".into()),
            })
        );
        assert!(parse("codex://threads/new?mode=invalid").is_none());
        assert!(parse("codex://threads/new?mode=chat&mode=chat").is_none());
    }

    #[test]
    fn preserves_automation_source_and_edit() {
        assert_eq!(
            parse("codex://automations/a%2Fb?edit=true"),
            Some(Link::Automation {
                id: "a/b".into(),
                source: "cloud",
                edit: true,
            })
        );
        assert_eq!(
            parse("codex://automations/task?automationSource=local"),
            Some(Link::Automation {
                id: "task".into(),
                source: "local",
                edit: false,
            })
        );
    }

    #[test]
    fn preserves_ssh_registration_and_oauth_callback() {
        assert_eq!(
            parse(
                "codex://settings/connections/ssh/add?alias=dev&projectPath=%2Fwork&enabled=true"
            ),
            Some(Link::AddSsh {
                name: "dev".into(),
                project_path: Some("/work".into()),
                enabled: true
            })
        );
        let callback = "codex://connector/oauth_callback?state=s&code=a%2Bb&returnTo=%2Fsettings";
        assert_eq!(parse(callback), Some(Link::OAuth(callback.into())));
    }

    #[test]
    fn matches_original_native_acceptance() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/deep_links.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let value = case["url"].as_str().unwrap();
            let parsed = parse(value);
            let expected = &case["result"];
            assert_eq!(parsed.is_some(), !expected.is_null(), "{value}");
            match parsed {
                Some(Link::NewThread {
                    prompt,
                    path,
                    project_id,
                    origin_url,
                }) => {
                    assert_eq!(expected["kind"], "newThread", "{value}");
                    for (actual, key) in [
                        (prompt, "prompt"),
                        (path, "path"),
                        (project_id, "projectId"),
                        (origin_url, "originUrl"),
                    ] {
                        assert_eq!(actual.as_deref(), expected[key].as_str(), "{value}: {key}");
                    }
                }
                Some(Link::Thread {
                    id,
                    prompt,
                    host_id,
                    shared,
                }) => {
                    assert_eq!(expected["kind"], "localConversation", "{value}");
                    assert_eq!(id, expected["conversationId"].as_str().unwrap(), "{value}");
                    assert_eq!(prompt.as_deref(), expected["prompt"].as_str(), "{value}");
                    assert_eq!(host_id.as_deref(), expected["hostId"].as_str(), "{value}");
                    assert_eq!(shared, expected["threadAccess"] == "shared", "{value}");
                }
                Some(Link::Automation { id, source, edit }) => {
                    assert_eq!(expected["kind"], "automation", "{value}");
                    assert_eq!(id, expected["automationId"].as_str().unwrap(), "{value}");
                    assert_eq!(
                        source,
                        expected["automationSource"].as_str().unwrap(),
                        "{value}"
                    );
                    assert_eq!(edit, expected["edit"].as_bool().unwrap(), "{value}");
                }
                Some(Link::OAuth(url)) => {
                    assert_eq!(url, expected["fullRedirectUrl"].as_str().unwrap())
                }
                Some(Link::Scheduled) => assert_eq!(expected["kind"], "automations", "{value}"),
                Some(Link::Settings(_) | Link::Connections(_) | Link::AddSsh { .. }) => {
                    assert_eq!(expected["kind"], "settings", "{value}")
                }
                Some(Link::Launch) | None => {}
            }
        }
        for value in [
            "codex://plugins",
            "codex://skills",
            "codex://space",
            "codex://sites/edit",
            "codex://shared-thread/cx_0123456789abcdef0123456789abcdef",
            "codex://connector/callback",
            "https://chatgpt.com/",
            "codex://launch/x",
        ] {
            assert!(parse(value).is_none(), "{value}");
        }
    }
}
