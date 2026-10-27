//! `codex://` deep links (SH-151..155): parse like the original renderer's bootstrap
//! `LE`, then bring up the primary window and navigate it like the host's
//! `queueCodexDeepLinkUrl` flush.
//!
//! Destinations whose products are not built (plugins, skills, space, sites, teams,
//! mini/spotlight, chatgpt.com shares) are recognized below and parse to `None`: the
//! original routes each to its own page, which this port does not carry. `space` and
//! `sites` are excluded products; the rest wait on their owning families.

use crate::{Root, Route, State, windows};
use gpui::{App, Context, Focusable, Window};
use ui::screens::connections::Tab;

/// A parsed `codex://` link this port can navigate.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// `threads/new` / `new`: a fresh thread, optionally with a prefilled prompt and a
    /// working directory (the prompt prefills and focuses the composer).
    NewThread {
        prompt: Option<String>,
        path: Option<String>,
    },
    /// `threads/<uuid>`: a local conversation, optionally with a prefilled prompt.
    Thread { id: String, prompt: Option<String> },
    /// `settings` or `settings/<section>`: Settings, opened at the nav `label` when the
    /// section has one.
    Settings(Option<&'static str>),
    /// `settings/connections[/computer|/devices|/ssh|/ssh/add]`: Settings > Connections at
    /// that tab. `ssh/add` opens the SSH tab without its add dialog.
    Connections(Tab),
    /// `automations`: the Scheduled page.
    Scheduled,
    /// `automations/<id>`: that task's thread (`automationId`, `automationSource`).
    Automation(String),
    /// `launch`: show the window without navigating.
    Launch,
}

/// `settings/<section>` deep-link sections (the original's `DE` allow-list) mapped to
/// this port's Settings nav labels; `connections/*` parse to [`Link::Connections`].
/// Sections without a page here (`chronicle`, `personal-vault`) open Settings itself,
/// like an unknown section.
const SETTINGS: &[(&str, &str)] = &[("browser-use", "Browser")];

/// Parses one `codex://` (or dev `codex-dev://`) URL. `None` for anything else, exactly
/// like the original's `LE` returning `null`.
pub fn parse(url: &str) -> Option<Link> {
    let url = url.trim();
    let rest = strip_scheme(url)?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut segments = rest.split('/').filter(|s| !s.is_empty());
    let destination = segments.next()?.to_ascii_lowercase();
    let sub: Vec<&str> = segments.collect();
    let params = Params::parse(query);
    match destination.as_str() {
        "threads" => threads(&sub, &params),
        "new" => new_thread(&params, true),
        "settings" => settings(&sub),
        "automations" => automations(&sub, &params),
        // The original's `launch` brings the window up and navigates nowhere; its
        // query is ignored.
        "launch" => sub.is_empty().then_some(Link::Launch),
        _ => None,
    }
}

fn strip_scheme(url: &str) -> Option<&str> {
    // `new URL` accepts `codex://x`, `codex:/x` and `codex:x`; the scheme is
    // case-insensitive, and any leading slashes drop from the path.
    for scheme in ["codex:", "codex-dev:"] {
        if url.len() >= scheme.len() && url[..scheme.len()].eq_ignore_ascii_case(scheme) {
            return Some(url[scheme.len()..].trim_start_matches('/'));
        }
    }
    None
}

fn threads(sub: &[&str], params: &Params) -> Option<Link> {
    if sub.first().is_some_and(|s| s.eq_ignore_ascii_case("new")) && sub.len() == 1 {
        return new_thread(params, false);
    }
    let id = *sub.first()?;
    if !uuid(id) {
        return None;
    }
    // Extra path segments and `view=review` stay accepted, like
    // `allowExtraPathSegments`; the review opens with the thread view this port has.
    // `hostId` and `threadAccess` are accepted like the original's free-form values;
    // this port has the one local host.
    params.http("browserUrl")?;
    let prompt = params.value("prompt");
    Some(Link::Thread {
        id: id.to_owned(),
        prompt,
    })
}

/// The original's `qE`: `gated` marks the bare `new` destination, where a link
/// without parameters is null (`threads/new` without parameters is a plain new thread).
fn new_thread(params: &Params, gated: bool) -> Option<Link> {
    let keyed = ["browserUrl", "mode", "originUrl", "path", "prompt"]
        .iter()
        .any(|k| params.raw(k).is_some());
    if !keyed {
        return (!gated).then_some(Link::NewThread {
            prompt: None,
            path: None,
        });
    }
    let prompt = params.value("prompt");
    // `path` goes through the original's absolute-path schema; anything else nulls
    // the link.
    let path = match params.value("path") {
        None => None,
        Some(path) if path.starts_with('/') => Some(path),
        Some(_) => return None,
    };
    params.http("browserUrl")?;
    params.http("originUrl")?;
    if let Some(active) = params.raw("browserActive")
        && !matches!(active, "true" | "false")
    {
        return None;
    }
    // Every parameter decoded to null: `qE` rejects the link.
    if prompt.is_none()
        && path.is_none()
        && params.value("browserUrl").is_none()
        && params.value("originUrl").is_none()
        && params.raw("mode").is_none()
        && params.raw("projectId").is_none()
    {
        return None;
    }
    // `mode` selects an app mode this port does not carry; the link still opens the
    // new thread, as the original does for every valid mode.
    Some(Link::NewThread { prompt, path })
}

fn settings(sub: &[&str]) -> Option<Link> {
    let section = sub.join("/");
    let tab = match section.as_str() {
        "connections" | "connections/computer" => Some(Tab::Computer),
        "connections/devices" => Some(Tab::Devices),
        "connections/ssh" | "connections/ssh/add" => Some(Tab::Ssh),
        _ => None,
    };
    if let Some(tab) = tab {
        return Some(Link::Connections(tab));
    }
    let label = SETTINGS
        .iter()
        .find(|(path, _)| *path == section)
        .map(|&(_, label)| label);
    Some(Link::Settings(label))
}

fn automations(sub: &[&str], params: &Params) -> Option<Link> {
    if sub.is_empty() {
        // The original's bare `automations` ignores its query.
        return Some(Link::Scheduled);
    }
    let id = component(sub[0])?;
    if sub.len() > 1 {
        return None;
    }
    if let Some(source) = params.raw("automationSource")
        && !matches!(source, "cloud" | "local")
    {
        return None;
    }
    // `edit=true` opens the task's edit dialog in the original; this port opens the
    // task's thread, which carries that edit surface.
    Some(Link::Automation(id))
}

/// Query parameters, read like the original's `URLSearchParams`.
struct Params<'a> {
    query: &'a str,
}

impl<'a> Params<'a> {
    fn parse(query: &'a str) -> Self {
        Self { query }
    }

    /// The raw (still encoded) first value of `name`; a bare key counts, as
    /// `URLSearchParams.has`.
    fn raw(&self, name: &str) -> Option<&'a str> {
        self.query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then_some(value)
        })
    }

    /// The trimmed non-empty decoded value of `name`, like the original's `Z`.
    fn value(&self, name: &str) -> Option<String> {
        // `URLSearchParams` decodes `+` as a space before percent escapes.
        component(&self.raw(name)?.trim().replace('+', " "))
    }

    /// An `http(s)` URL value, like the original's `rD`; any other value makes the
    /// link invalid. Absent parameters pass.
    fn http(&self, name: &str) -> Option<()> {
        match self.value(name) {
            None => Some(()),
            Some(url) => (url.starts_with("http://") || url.starts_with("https://")).then_some(()),
        }
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
        let mut prompt = None;
        match link {
            Link::NewThread {
                prompt: prefill,
                path: _,
            } => {
                self.navigate(Route::Home, window, cx);
                prompt = prefill.clone();
            }
            Link::Thread {
                id,
                prompt: prefill,
            } => {
                self.navigate(Route::Thread(id.clone()), window, cx);
                prompt = prefill.clone();
            }
            Link::Settings(None) => self.navigate(Route::Settings, window, cx),
            Link::Settings(Some(label)) => self.open_settings_section(label, window, cx),
            Link::Connections(tab) => self.open_connections(*tab, window, cx),
            Link::Scheduled => self.navigate(Route::Scheduled, window, cx),
            Link::Automation(id) => self.navigate(Route::Automation(id.clone()), window, cx),
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

/// Opens `url` in the last active primary window, starting one at the link when none
/// exists — the original's `ensurePrimaryWindowVisible` then `navigateToRoute`.
pub fn open(url: &str, cx: &mut App) {
    let Some(link) = parse(url) else {
        return;
    };
    if let Some(handle) = windows::primary(cx).and_then(|handle| handle.downcast::<Root>()) {
        let result = handle
            .update(cx, |root, window, cx| {
                root.open_deep_link(&link, window, cx);
            })
            .and_then(|()| windows::show(handle.into(), windows::Presentation::Foreground, cx));
        if let Err(error) = result {
            eprintln!("codex-gpui: deep link {url}: {error:#}");
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

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a13de9-c48a-7b20-8382-db900881cb99";

    #[test]
    fn parses_thread_links() {
        assert_eq!(
            parse(&format!("codex://threads/{ID}")),
            Some(Link::Thread {
                id: ID.into(),
                prompt: None
            })
        );
        assert_eq!(
            parse(&format!("codex://threads/{ID}?prompt=Hi%20there")),
            Some(Link::Thread {
                id: ID.into(),
                prompt: Some("Hi there".into())
            })
        );
        // Empty prompts are absent, like the original's `Z`.
        assert_eq!(
            parse(&format!("codex://threads/{ID}?prompt=%20")),
            Some(Link::Thread {
                id: ID.into(),
                prompt: None
            })
        );
        // Extra segments stay accepted (`allowExtraPathSegments`), as does `hostId`.
        assert!(parse(&format!("codex://threads/{ID}/review")).is_some());
        assert!(
            parse(&format!(
                "codex://threads/{ID}?hostId=remote&threadAccess=shared"
            ))
            .is_some()
        );
        assert_eq!(parse("codex://threads/not-a-uuid"), None);
        assert_eq!(parse("codex://threads"), None);
        // A browser URL that is not http(s) makes the link null, like `rD`.
        assert_eq!(
            parse(&format!("codex://threads/{ID}?browserUrl=ftp://x")),
            None
        );
    }

    #[test]
    fn parses_new_thread_links() {
        assert_eq!(
            parse("codex://threads/new"),
            Some(Link::NewThread {
                prompt: None,
                path: None
            })
        );
        assert_eq!(
            parse("codex://threads/new?prompt=Fix%20it&path=/tmp/proj"),
            Some(Link::NewThread {
                prompt: Some("Fix it".into()),
                path: Some("/tmp/proj".into())
            })
        );
        assert_eq!(
            parse("codex://new?prompt=Hi"),
            Some(Link::NewThread {
                prompt: Some("Hi".into()),
                path: None
            })
        );
        // A relative `path` fails the original's path schema, so the link is null.
        assert_eq!(parse("codex://threads/new?path=relative"), None);
        // `browserActive` is outside the `threads/new` parameter gate, so the original
        // never validates it there; on the bare `new` destination it nulls the link.
        assert_eq!(
            parse("codex://threads/new?browserActive=maybe"),
            Some(Link::NewThread {
                prompt: None,
                path: None
            })
        );
        assert_eq!(parse("codex://new?prompt=Hi&browserActive=maybe"), None);
        // `qE` rejects a link whose every parameter decoded to null, and the bare
        // `new` destination is always `qE`.
        assert_eq!(parse("codex://new"), None);
        assert_eq!(parse("codex://threads/new?prompt="), None);
        // `projectId` is not in `threads/new`'s parameter gate: a bare new thread.
        assert_eq!(
            parse("codex://threads/new?projectId=x"),
            Some(Link::NewThread {
                prompt: None,
                path: None
            })
        );
    }

    #[test]
    fn parses_settings_links() {
        assert_eq!(parse("codex://settings"), Some(Link::Settings(None)));
        assert_eq!(
            parse("codex://settings/connections"),
            Some(Link::Connections(Tab::Computer))
        );
        assert_eq!(
            parse("codex://settings/connections/devices"),
            Some(Link::Connections(Tab::Devices))
        );
        assert_eq!(
            parse("codex://settings/connections/ssh/add"),
            Some(Link::Connections(Tab::Ssh))
        );
        // `wsl` is outside the original's allow-list.
        assert_eq!(
            parse("codex://settings/connections/wsl"),
            Some(Link::Settings(None))
        );
        assert_eq!(
            parse("codex://settings/browser-use"),
            Some(Link::Settings(Some("Browser")))
        );
        // Unknown sections open Settings itself, like the original's fallback.
        assert_eq!(
            parse("codex://settings/chronicle"),
            Some(Link::Settings(None))
        );
    }

    #[test]
    fn parses_automation_links() {
        // The original's bare `automations` ignores its query.
        assert_eq!(parse("codex://automations?x=1"), Some(Link::Scheduled));
        assert_eq!(
            parse("codex://automations/task%202?automationSource=local"),
            Some(Link::Automation("task 2".into()))
        );
        assert_eq!(parse("codex://automations/a/b"), None);
        assert_eq!(parse("codex://automations/a?automationSource=bogus"), None);
    }

    #[test]
    fn parses_launch_and_rejects_the_rest() {
        assert_eq!(parse("codex://launch"), Some(Link::Launch));
        // `launch` ignores its query, like the original.
        assert_eq!(parse("codex://launch?x=1"), Some(Link::Launch));
        assert_eq!(parse("codex://launch/x"), None);
        assert_eq!(parse("codex://plugins"), None);
        assert_eq!(parse("codex://skills"), None);
        assert_eq!(parse("codex://space"), None);
        assert_eq!(parse("codex://sites/edit"), None);
        assert_eq!(
            parse("codex://shared-thread/cx_0123456789abcdef0123456789abcdef"),
            None
        );
        assert_eq!(parse("codex://connector/callback"), None);
        assert_eq!(parse("https://chatgpt.com/"), None);
        assert_eq!(parse("not a url"), None);
        // The dev scheme and mixed case parse like the packaged one.
        assert_eq!(parse("codex-dev://automations"), Some(Link::Scheduled));
        assert_eq!(parse("CODEX://Settings"), Some(Link::Settings(None)));
        // `new URL` accepts the single-slash and no-slash forms too.
        assert_eq!(parse("codex:/settings"), Some(Link::Settings(None)));
        assert_eq!(parse("CODEX:automations"), Some(Link::Scheduled));
    }

    #[test]
    fn component_matches_decode_uri_component() {
        assert_eq!(component("a%20b"), Some("a b".into()));
        assert_eq!(component("%C3%A9"), Some("é".into()));
        assert_eq!(component("a+b"), Some("a+b".into()));
        assert_eq!(component("%4"), None);
        assert_eq!(component("%zz"), None);
        assert_eq!(component("%C3"), None);
        assert_eq!(component("  "), None);
    }
}
