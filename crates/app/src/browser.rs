use anyhow::{Context as _, Result, bail, ensure};
use gpui::{BorrowAppContext as _, Bounds, Context, Entity, PromptLevel, point, px};
use serde_json::{Value, json};
use ui::browser::BrowserTab;
use ui::screens::settings::browser_settings::BROWSER_TOOL_GRANTS as GRANTS;
use url::Url;

use crate::{Root, host_settings::Settings};

const NAMESPACE: &str = "in_app_browser";

pub(super) fn tools() -> Vec<Value> {
    let mut tools = vec![crate::terminal::read_thread_terminal_tool()];
    if !cfg!(target_os = "macos") {
        return tools;
    }
    for (name, description, fields) in [
        (
            "navigate",
            "Navigate the selected in-app browser tab within its current site.",
            &["url"][..],
        ),
        (
            "click",
            "Click a visible link or button in the selected in-app browser tab.",
            &["selector"],
        ),
        (
            "type",
            "Enter text in a visible text field in the selected in-app browser tab.",
            &["selector", "text"],
        ),
        (
            "read_page",
            "Read the title and visible text of the selected in-app browser tab.",
            &[],
        ),
        (
            "screenshot",
            "Capture the visible content of the selected in-app browser tab.",
            &[],
        ),
    ] {
        let properties: serde_json::Map<_, _> = fields
            .iter()
            .map(|&field| (field.to_owned(), json!({"type": "string"})))
            .collect();
        tools.push(json!({
            "namespace": NAMESPACE, "name": name, "description": description,
            "inputSchema": {"type":"object", "properties": properties, "required": fields, "additionalProperties": false},
            "deferLoading": false,
        }));
    }
    tools
}

#[derive(Debug)]
enum Action {
    Navigate(Url),
    Click(String),
    Type { selector: String, text: String },
    ReadPage,
    Screenshot,
}

fn site(url: &Url) -> Result<String> {
    ensure!(
        matches!(url.scheme(), "https" | "http")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none(),
        "Browser tools require an HTTP or HTTPS site without embedded credentials."
    );
    Ok(url.origin().ascii_serialization())
}

impl Action {
    fn parse(params: &Value) -> Result<Self> {
        let args = params["arguments"]
            .as_object()
            .context("Missing browser tool arguments")?;
        let field = |name: &str, max_len| -> Result<&str> {
            args.get(name)
                .and_then(Value::as_str)
                .filter(|value| value.len() <= max_len)
                .with_context(|| format!("Invalid browser tool argument: {name}"))
        };
        Ok(match params["tool"].as_str() {
            Some("navigate") if args.len() == 1 => Self::Navigate(Url::parse(field("url", 4096)?)?),
            Some("click") if args.len() == 1 => Self::Click(field("selector", 512)?.to_owned()),
            Some("type") if args.len() == 2 => Self::Type {
                selector: field("selector", 512)?.to_owned(),
                text: field("text", 4096)?.to_owned(),
            },
            Some("read_page") if args.is_empty() => Self::ReadPage,
            Some("screenshot") if args.is_empty() => Self::Screenshot,
            _ => bail!("Invalid browser tool call"),
        })
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Navigate(_) => "Navigate",
            Self::Click(_) => "Click an element",
            Self::Type { .. } => "Enter text",
            Self::ReadPage => "Read page text",
            Self::Screenshot => "Capture a screenshot",
        }
    }

    fn script(&self, page_url: &Url) -> Result<String> {
        let expected = json!(page_url.as_str());
        let visible = "const visible = e => e && e.getClientRects().length && getComputedStyle(e).visibility === 'visible';";
        let body = match self {
            Self::Navigate(target) => {
                ensure!(site(target)? == site(page_url)?, "Open the other site in the browser before using tools there.");
                format!("location.assign({}); return 'Navigation started';", json!(target.as_str()))
            }
            Self::Click(selector) => format!(
                "{visible} const e = document.querySelector({});
                 if (!visible(e) || !e.matches('a[href],button:not([disabled]),[role=button]')) throw Error('No visible link or button');
                 if (e.matches('a') && (e.hasAttribute('download') || e.target && e.target !== '_self' || new URL(e.href).origin !== location.origin)) throw Error('Unsupported link');
                 e.click(); return 'Clicked';",
                json!(selector)
            ),
            Self::Type { selector, text } => format!(
                "{visible} const e = document.querySelector({});
                 if (!visible(e) || !e.matches('input,textarea') || !['text','search','textarea'].includes(e.type) || e.readOnly || e.disabled) throw Error('No editable text field');
                 e.value = {}; e.dispatchEvent(new Event('input', {{ bubbles: true }})); return 'Entered text';",
                json!(selector), json!(text)
            ),
            Self::ReadPage => "return document.title.slice(0, 512) + '\\n' + (document.body?.innerText ?? '').slice(0, 12000);".to_owned(),
            Self::Screenshot => "return 'Ready';".to_owned(),
        };
        Ok(format!(
            "(() => {{ if (location.href !== {expected}) throw Error('The browser page changed'); {body} }})()"
        ))
    }
}

fn origin_matches(pattern: &str, url: &Url) -> Option<bool> {
    let pattern = Url::parse(pattern).ok()?;
    let host = pattern.host_str()?.trim_end_matches('.');
    let target = url.host_str()?.trim_end_matches('.');
    let wildcard = host
        .strip_prefix("**.")
        .map(|suffix| (suffix, true))
        .or_else(|| host.strip_prefix("*.").map(|suffix| (suffix, false)));
    let suffix = wildcard.map_or(host, |(suffix, _)| suffix);
    if suffix.is_empty() || suffix.contains('*') {
        return None;
    }
    if pattern.scheme() != url.scheme() || pattern.port() != url.port() {
        return Some(false);
    }
    Some(match wildcard {
        None => target == host,
        Some((suffix, apex)) => {
            (apex && target == suffix)
                || target
                    .strip_suffix(suffix)
                    .is_some_and(|head| head.ends_with('.'))
        }
    })
}

fn policy_allows(config: &Value, url: &Url) -> bool {
    let browser = &config["browser_use"];
    let mut site_allowed = false;
    for (pattern, policy) in browser["origins"].as_object().into_iter().flatten() {
        let matches = origin_matches(pattern, url);
        if policy["access"] == "deny" && matches != Some(false) {
            return false;
        }
        site_allowed |= policy["access"] == "allow" && matches == Some(true);
    }
    site_allowed || browser["default_origin_policy"]["access"] != "deny"
}

fn reply(result: Result<Value>) -> Value {
    match result {
        Ok(item) => json!({"success": true, "contentItems": [item]}),
        Err(error) => json!({"success": false, "contentItems": [text(error.to_string())]}),
    }
}

fn text(text: String) -> Value {
    json!({"type": "inputText", "text": text})
}

impl Root {
    pub(super) fn browser_tool_call(
        &mut self,
        id: &Value,
        params: &Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if params["namespace"] != NAMESPACE {
            return false;
        }
        let start = (|| -> Result<_> {
            ensure!(cfg!(target_os = "macos"), "No native browser is available.");
            ensure!(
                self.browser_tool_pending.is_none(),
                "A browser tool is already running."
            );
            let thread = params["threadId"]
                .as_str()
                .context("Missing browser thread")?;
            let turn = params["turnId"].as_str().context("Missing browser turn")?;
            let (tab, page_url) = self.browser_tool_tab(thread, turn, cx)?;
            let action = Action::parse(params)?;
            let script = action.script(&page_url)?;
            let origin = site(&page_url)?;
            Ok((
                thread.to_owned(),
                turn.to_owned(),
                tab,
                page_url,
                action,
                script,
                origin,
            ))
        })();
        let bridge = self.bridge.clone();
        let (thread, turn, tab, page_url, action, script, origin) = match start {
            Ok(prepared) => prepared,
            Err(error) => {
                if self.browser_tool_pending.as_ref() != Some(id) {
                    let _ = bridge.respond(id.clone(), reply(Err(error)));
                }
                return true;
            }
        };
        self.browser_tool_pending = Some(id.clone());
        let id = id.clone();
        cx.spawn(async move |root, cx| {
            let output = async {
                ensure!(policy_allows(&bridge.config_read().await?.config, &page_url), "Browser access is blocked in Settings.");
                let requirements = bridge.config_requirements().await?;
                let requirements = &requirements["requirements"];
                ensure!(
                    requirements["allowBrowserAndComputerUse"] != false
                        && requirements["featureRequirements"]["browser_use"] != false
                        && requirements["network"].is_null()
                        && requirements["browserUse"].is_null(),
                    "Browser tools are unavailable under the current managed policy."
                );
                let check = |root: &Root, cx: &gpui::App| -> Result<()> {
                    let (current, url) = root.browser_tool_tab(&thread, &turn, cx)?;
                    ensure!(current == tab && url == page_url, "The browser page changed.");
                    Ok(())
                };
                let prompt = root.update(cx, |root, cx| -> Result<_> {
                    check(root, cx)?;
                    if cx.try_global::<Settings>().is_some_and(|settings| {
                        settings.read_all().get(GRANTS).is_some_and(|grants| grants[&origin] == true)
                    }) {
                        return Ok(None);
                    }
                    let detail = format!("{} on {origin}. Always allow lets Codex read and interact with this site without asking again.", action.label());
                    Ok(Some(root.context_window.update(cx, |_, window, cx| {
                        window.prompt(PromptLevel::Warning, "Allow Codex to use this site?", Some(&detail), &["Deny", "Always ask", "Always allow"], cx)
                    })?))
                })??;
                let always = if let Some(prompt) = prompt {
                    match prompt.await.ok() {
                        Some(1) => false,
                        Some(2) => true,
                        _ => bail!("Browser access was denied."),
                    }
                } else {
                    false
                };
                let eval = root.update(cx, |root, cx| -> Result<_> {
                    check(root, cx)?;
                    if always {
                        ensure!(cx.has_global::<Settings>(), "Browser settings are not available.");
                        cx.update_global::<Settings, _>(|settings, _| {
                            settings.update(GRANTS, |value| {
                                let mut origins = value.and_then(Value::as_object).cloned().unwrap_or_default();
                                origins.insert(origin, Value::Bool(true));
                                Ok(Value::Object(origins))
                            })
                        })?;
                    }
                    Ok(tab.read(cx).evaluate_in_client_world(&script))
                })??;
                let timeout = cx.background_executor().timer(std::time::Duration::from_secs(30));
                futures_lite::future::race(async {
                    let result = eval.await?;
                    if !matches!(action, Action::Screenshot) {
                        return Ok(text(result));
                    }
                    let capture = root.update(cx, |root, cx| -> Result<_> {
                        check(root, cx)?;
                        let tab = tab.read(cx);
                        let size = tab.page_bounds().size;
                        Ok(tab.snapshot(Bounds { origin: point(px(0.), px(0.)), size }))
                    })??;
                    let png = capture.await?;
                    root.update(cx, |root, cx| check(root, cx))??;
                    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
                    Ok(json!({"type": "inputImage", "imageUrl": format!("data:image/png;base64,{encoded}")}))
                }, async {
                    timeout.await;
                    bail!("WebKit did not confirm the result. Check the page before retrying.")
                }).await
            }.await;
            let _ = root.update(cx, |root, _| root.browser_tool_pending = None);
            if let Err(error) = bridge.respond(id, reply(output)) {
                eprintln!("codex-gpui: browser tool response: {error:#}");
            }
        }).detach();
        true
    }

    fn browser_tool_tab(
        &self,
        thread: &str,
        turn: &str,
        cx: &gpui::App,
    ) -> Result<(Entity<BrowserTab>, Url)> {
        ensure!(
            self.thread.as_ref().is_some_and(|view| {
                let view = view.read(cx);
                view.id() == thread
                    && view.turns().last().is_some_and(|active| {
                        active.id == turn && active.status == bridge::types::TurnStatus::InProgress
                    })
            }) && matches!(&self.state.route, crate::Route::Thread(id) if id == thread)
                && self.page.is_none(),
            "Select the tool's chat before using its browser."
        );
        let tab = self
            .active_browser_tab()
            .context("Select a browser tab and open a site first.")?;
        let url = tab
            .read(cx)
            .browser_tool_url()
            .cloned()
            .context("Open a site and wait for it to load before using browser tools.")?;
        Ok((tab, url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_does_not_reuse_approval_across_origins() {
        let page = Url::parse("https://example.com/path").unwrap();
        for target in [
            "http://example.com/",
            "https://example.com:8443/",
            "https://example.com.evil.test/",
        ] {
            let action = Action::Navigate(Url::parse(target).unwrap());
            assert!(action.script(&page).is_err(), "{target}");
        }
        let action = Action::Navigate(Url::parse("https://example.com/next").unwrap());
        assert!(action.script(&page).is_ok());
    }

    #[test]
    fn tool_arguments_cannot_add_script_fields() {
        assert!(
            Action::parse(&json!({"tool":"read_page", "arguments":{"script":"console.log(1)"}}))
                .is_err()
        );
    }

    #[test]
    fn site_denials_take_precedence_over_browsing_permission() {
        let origin = "https://example.com";
        let url = Url::parse(origin).unwrap();
        let other = Url::parse("https://other.example").unwrap();
        let config = json!({"browser_use":{"default_origin_policy":{"access":"allow"},"origins":{origin:{"access":"deny"}}}});
        assert!(!policy_allows(&config, &url));
        assert!(policy_allows(&config, &other));

        let config = json!({"browser_use":{"default_origin_policy":{"access":"deny"},"origins":{origin:{"access":"allow"}}}});
        assert!(policy_allows(&config, &url));
        assert!(!policy_allows(&config, &other));
    }

    #[test]
    fn wildcard_site_rule_preserves_scheme_port_and_domain_boundaries() {
        let config =
            json!({"browser_use":{"origins":{"https://*.example.com:8443":{"access":"deny"}}}});
        let allowed = |url| policy_allows(&config, &Url::parse(url).unwrap());
        assert!(!allowed("https://sub.example.com:8443"));
        assert!(allowed("https://example.com:8443"));
        assert!(allowed("https://notexample.com:8443"));
        assert!(allowed("https://sub.example.com"));
        assert!(allowed("http://sub.example.com:8443"));

        let config =
            json!({"browser_use":{"origins":{"https://**.example.com":{"access":"deny"}}}});
        assert!(!policy_allows(
            &config,
            &Url::parse("https://example.com").unwrap()
        ));
    }
}
