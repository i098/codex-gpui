mod navigation;
pub use navigation::listen;
pub(crate) use navigation::{Navigation, flush};

use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Link {
    Launch,
    Thread {
        id: String,
        host: Option<String>,
        shared: bool,
        prompt: Option<String>,
        review: Option<Review>,
    },
    NewThread {
        prompt: Option<String>,
        path: Option<String>,
        origin: Option<String>,
        project: Option<String>,
        mode: Option<String>,
    },
    Settings(String),
    Automations {
        id: Option<String>,
        source: String,
        edit: bool,
    },
    Family(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Review {
    pub filter: Option<String>,
    pub path: Option<String>,
}

pub(crate) fn parse(raw: &str) -> Option<Link> {
    let mut url = Url::parse(raw).ok()?;
    if crate::qa::ENABLED && url.scheme() == "codex-gpui-qa" {
        url.set_scheme("codex").ok()?;
    }
    if !matches!(url.scheme(), "codex" | "codex-dev")
        || !url.username().is_empty()
        || url.password().is_some_and(|password| !password.is_empty())
        || url.port().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let segments: Vec<_> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let value = |name: &str| query(name).filter(|value| !value.trim().is_empty());
    match host {
        "launch" if segments.is_empty() => Some(Link::Launch),
        "threads" if segments.first() != Some(&"new") => {
            if url.scheme() != "codex" {
                return None;
            }
            let id = *segments.first()?;
            if !uuid(id) || !browser_parameters(&url) {
                return None;
            }
            let shared = match query("threadAccess").as_deref() {
                None => false,
                Some("shared") => true,
                _ => return None,
            };
            let review = (query("view").as_deref() == Some("review")).then(|| {
                let filter = query("diffFilter")
                    .filter(|filter| matches!(filter.as_str(), "branch" | "last-turn"));
                let path = filter
                    .as_ref()
                    .and_then(|_| query("path").filter(|path| !path.is_empty()));
                Review { filter, path }
            });
            Some(Link::Thread {
                id: id.to_owned(),
                host: value("hostId"),
                shared,
                prompt: value("prompt"),
                review,
            })
        }
        "new" | "threads" => {
            let has_options = host == "new"
                || url.query_pairs().any(|(key, _)| {
                    matches!(
                        key.as_ref(),
                        "browserUrl" | "mode" | "originUrl" | "path" | "prompt"
                    )
                });
            if !has_options {
                return Some(Link::NewThread {
                    prompt: None,
                    path: None,
                    origin: None,
                    project: None,
                    mode: None,
                });
            }
            let modes: Vec<_> = url
                .query_pairs()
                .filter(|(key, _)| key == "mode")
                .map(|(_, value)| value.into_owned())
                .collect();
            if modes.len() > 1
                || modes
                    .first()
                    .is_some_and(|mode| !matches!(mode.as_str(), "chat" | "work" | "codex"))
                || !browser_parameters(&url)
            {
                return None;
            }
            let path = value("path");
            if path.as_ref().is_some_and(|path| {
                path.as_bytes()
                    .get(..2)
                    .is_some_and(|prefix| prefix.iter().all(|byte| matches!(*byte, b'/' | b'\\')))
                    || path.contains('\0')
            }) {
                return None;
            }
            let prompt = value("prompt");
            let origin = value("originUrl");
            let project = value("projectId");
            if prompt.is_none()
                && path.is_none()
                && origin.is_none()
                && project.is_none()
                && modes.is_empty()
                && query("browserUrl").is_none()
            {
                return None;
            }
            Some(Link::NewThread {
                prompt,
                path,
                origin,
                project,
                mode: modes.into_iter().next(),
            })
        }
        "settings" => {
            let path = segments.join("/");
            let mut section = match path.as_str() {
                "connections/ssh/add" => ssh_add_route(&url),
                "browser-use"
                | "chronicle"
                | "connections"
                | "connections/computer"
                | "connections/devices"
                | "connections/ssh" => path,
                "personal-vault" => "local-environments?view=personal-vault".to_owned(),
                _ if segments
                    .first()
                    .is_some_and(|part| matches!(*part, "mcp-app" | "plugins")) =>
                {
                    if !plugin_tool_link(&url, &segments[1..]) {
                        return None;
                    }
                    return Some(Link::Family(url.into()));
                }
                _ => String::new(),
            };
            if !section.is_empty()
                && let Some(fragment) = url.fragment()
            {
                section.push('#');
                section.push_str(fragment);
            }
            Some(Link::Settings(section))
        }
        "automations" => {
            if segments.is_empty() {
                return Some(Link::Automations {
                    id: None,
                    source: "cloud".into(),
                    edit: false,
                });
            }
            if segments.len() > 1 {
                return None;
            }
            let id = segments
                .first()
                .map(|id| decode_segment(id))
                .transpose()
                .ok()?;
            let source = query("automationSource").unwrap_or_else(|| "cloud".to_owned());
            if !matches!(source.as_str(), "local" | "cloud") {
                return None;
            }
            Some(Link::Automations {
                id,
                source,
                edit: query("edit").as_deref() == Some("true"),
            })
        }
        "shared-thread" => {
            let id = *segments.first()?;
            if segments.len() != 1
                || id.len() != 35
                || !id.starts_with("cx_")
                || !id[3..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                || url.fragment().is_some()
            {
                return None;
            }
            let parameters: Vec<_> = url.query_pairs().collect();
            if parameters.len() > 1
                || parameters
                    .iter()
                    .any(|(key, value)| key != "handoff_id" || !uuid(value))
            {
                return None;
            }
            Some(Link::Family(url.into()))
        }
        "plugins" if segments.is_empty() => None,
        "plugins" if segments.len() == 1 => {
            let id = decode_segment(segments[0]).ok()?;
            if installed_plugin_id(&id) || local_plugin_link(&url) {
                Some(Link::Family(url.into()))
            } else {
                None
            }
        }
        "mcp-app" if plugin_tool_link(&url, &segments) => Some(Link::Family(url.into())),
        "plugins" | "skills" | "space" | "sites" | "connector" | "cloud-environments"
        | "browser" | "chatgpt" | "teams" | "codex-app" | "pets" | "activate" | "orbit"
        | "dots" => Some(Link::Family(url.into())),
        _ => None,
    }
}

fn plugin_tool_link(url: &Url, segments: &[&str]) -> bool {
    if segments.len() < 2 || url.fragment().is_some() {
        return false;
    }
    decode_segment(segments[0]).is_ok_and(|id| installed_plugin_id(&id))
        && segments[1..]
            .iter()
            .all(|segment| decode_segment(segment).is_ok())
}

fn local_plugin_link(url: &Url) -> bool {
    if url
        .query_pairs()
        .any(|(key, _)| !matches!(key.as_ref(), "marketplacePath" | "mode"))
    {
        return false;
    }
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    if query("mode").is_some_and(|mode| mode != "share") {
        return false;
    }
    query("marketplacePath").is_some_and(|path| {
        let bytes = path.as_bytes();
        (path.starts_with('/') && !path.starts_with("//"))
            || (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\'))
    })
}

fn ssh_add_route(url: &Url) -> String {
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let name = query("name");
    let alias = query("alias");
    let project = query("projectPath");
    let enabled = query("enabled");
    let value = name.as_ref().or(alias.as_ref());
    let invalid = value.is_none()
        || name
            .as_ref()
            .zip(alias.as_ref())
            .is_some_and(|(name, alias)| name != alias)
        || url
            .query_pairs()
            .any(|(key, _)| !matches!(key.as_ref(), "name" | "alias" | "projectPath" | "enabled"))
        || (url.query_pairs().any(|(key, _)| key == "projectPath") && project.is_none())
        || (url.query_pairs().any(|(key, _)| key == "enabled")
            && !enabled
                .as_deref()
                .is_some_and(|value| matches!(value, "true" | "false")));
    if invalid {
        return "connections/ssh".into();
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("name", value.unwrap());
    if let Some(project) = project {
        query.append_pair("projectPath", &project);
    }
    if let Some(enabled) = enabled {
        query.append_pair("enabled", &enabled);
    }
    format!("connections/ssh/add?{}", query.finish())
}

fn installed_plugin_id(id: &str) -> bool {
    if let Some((name, marketplace)) = id.rsplit_once('@') {
        return !name.is_empty()
            && matches!(
                marketplace,
                "openai-bundled"
                    | "openai-curated"
                    | "openai-primary-runtime"
                    | "openai-internal-testing"
            );
    }
    for prefix in ["plugins_", "Plugin_", "plugins~Plugin_"] {
        if let Some(suffix) = id.strip_prefix(prefix) {
            return suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        }
    }
    id.strip_prefix("plugin_").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 248
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    })
}

fn browser_parameters(url: &Url) -> bool {
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let browser = query("browserUrl");
    let tab = query("browserTabId");
    let active = query("browserActive");
    if browser.as_ref().is_some_and(|url| {
        Url::parse(url)
            .ok()
            .is_none_or(|url| !matches!(url.scheme(), "http" | "https"))
    }) || tab.as_ref().is_some_and(|tab| {
        tab.is_empty()
            || tab.len() > 128
            || !tab
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }) || active
        .as_ref()
        .is_some_and(|active| !matches!(active.as_str(), "true" | "false"))
        || (browser.is_none() && (tab.is_some() || active.is_some()))
        || (active.is_some() && tab.is_none())
    {
        return false;
    }
    if active.as_deref() == Some("false") {
        return false;
    }
    true
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn decode_segment(segment: &str) -> Result<String, ()> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair =
                std::str::from_utf8(bytes.get(index + 1..index + 3).ok_or(())?).map_err(|_| ())?;
            decoded.push(u8::from_str_radix(pair, 16).map_err(|_| ())?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| ())?;
    let value = decoded.trim();
    if value.is_empty() {
        Err(())
    } else {
        Ok(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_urls_do_not_navigate_or_cross_hosts() {
        for url in [
            "https://threads/01a13de9-c48a-7b20-8382-db900881cb99",
            "codex://user@threads/01a13de9-c48a-7b20-8382-db900881cb99",
            "codex://threads:3000/01a13de9-c48a-7b20-8382-db900881cb99",
            "codex://threads/not-a-thread",
            "codex://threads/new?path=%2F%2Fserver%2Fshare",
            "codex://threads/new?mode=work&mode=codex",
            "codex://automations/%ZZ",
            "codex://automations/a/b",
        ] {
            assert_eq!(parse(url), None, "{url}");
        }
    }

    #[test]
    fn thread_links_keep_host_access_prefill_and_review_state() {
        assert_eq!(
            parse(
                "codex://threads/01a13de9-c48a-7b20-8382-db900881cb99?hostId=remote&threadAccess=shared&prompt=Review+this&view=review&diffFilter=branch&path=src%2Flib.rs"
            ),
            Some(Link::Thread {
                id: "01a13de9-c48a-7b20-8382-db900881cb99".into(),
                host: Some("remote".into()),
                shared: true,
                prompt: Some("Review this".into()),
                review: Some(Review {
                    filter: Some("branch".into()),
                    path: Some("src/lib.rs".into())
                }),
            })
        );
    }

    #[test]
    fn new_thread_and_automation_links_decode_user_text() {
        assert_eq!(
            parse("codex://threads/new?prompt=Fix+%CE%BB&path=%2Ftmp%2Fproject"),
            Some(Link::NewThread {
                prompt: Some("Fix λ".into()),
                path: Some("/tmp/project".into()),
                origin: None,
                project: None,
                mode: None
            })
        );
        assert_eq!(
            parse("codex://automations/task%20one?automationSource=local&edit=true"),
            Some(Link::Automations {
                id: Some("task one".into()),
                source: "local".into(),
                edit: true
            })
        );
    }

    #[test]
    fn ssh_add_links_preserve_valid_fields_and_reject_conflicts() {
        assert_eq!(
            parse(
                "codex://settings/connections/ssh/add?name=box&projectPath=%2Ftmp%2Fwork&enabled=false"
            ),
            Some(Link::Settings(
                "connections/ssh/add?name=box&projectPath=%2Ftmp%2Fwork&enabled=false".into()
            )),
        );
        for url in [
            "codex://settings/connections/ssh/add?name=box&alias=other",
            "codex://settings/connections/ssh/add?name=box&enabled=no",
        ] {
            assert_eq!(parse(url), Some(Link::Settings("connections/ssh".into())));
        }
    }

    #[test]
    fn plugin_links_require_an_original_plugin_identifier() {
        assert_eq!(parse("codex://plugins"), None);
        assert_eq!(parse("codex://plugins/example"), None);
        assert!(matches!(
            parse("codex://plugins/example@openai-curated"),
            Some(Link::Family(_))
        ));
    }

    #[test]
    fn settings_links_keep_original_defaults_and_validate_plugin_tools() {
        assert_eq!(
            parse("codex://settings/computer-use"),
            Some(Link::Settings(String::new())),
        );
        for url in [
            "codex://settings/plugins",
            "codex://settings/plugins/example@openai-curated",
            "codex://settings/mcp-app/computer",
            "codex://mcp-app/example@openai-curated",
        ] {
            assert_eq!(parse(url), None, "{url}");
        }
        assert!(matches!(
            parse("codex://settings/plugins/example@openai-curated/tool"),
            Some(Link::Family(_)),
        ));
    }

    #[test]
    fn qa_callback_links_use_the_original_scheme_only_in_qa_builds() {
        assert_eq!(
            parse("codex-gpui-qa://connector/oauth_callback?state=pending"),
            crate::qa::ENABLED
                .then(|| Link::Family("codex://connector/oauth_callback?state=pending".into())),
        );
    }
}
