pub(crate) mod generation;
mod workflow;

use crate::{GitRun, host};
use anyhow::{Result, bail};
use gpui::AsyncWindowContext;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    sync::Mutex,
    time::{Duration, Instant},
};

const CODEX_AUTHOR: &str = "Codex <noreply@openai.com>";

#[derive(Default)]
pub(crate) struct Attribution {
    preference: Mutex<Option<(Instant, Option<Value>)>>,
}

impl Attribution {
    fn preference(&self, host: &dyn host::Host) -> Option<Value> {
        let mut cached = self.preference.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((fetched, value)) = &*cached
            && fetched.elapsed() < Duration::from_secs(300)
        {
            return value.clone();
        }
        let response = host
            .fetch(&host::Request::Http(json!({
                "url": "/wham/settings/user",
                "method": "GET",
                "headers": {"X-OpenAI-Attach-Auth": "1"},
                "returnErrorResponse": true,
            })))
            .and_then(|response| host::json_body(&response));
        match response {
            Ok(response) => {
                let value = response.get("commit_attribution_enabled").cloned();
                *cached = Some((Instant::now(), value.clone()));
                value
            }
            Err(_) => Some(Value::Null),
        }
    }
}

pub(crate) async fn attribution(run: &GitRun, cx: &AsyncWindowContext) -> Option<String> {
    let host = run.host.clone();
    let attribution = run.attribution.clone();
    let preference = cx
        .background_executor()
        .spawn(async move { attribution.preference(&*host) })
        .await;
    if let Some(preference) = preference {
        return (preference == true).then(|| CODEX_AUTHOR.to_owned());
    }
    let config = run
        .bridge
        .request::<_, bridge::types::ConfigReadResponse>(
            "config/read",
            json!({"cwd": run.cwd, "includeLayers": false}),
        )
        .await
        .ok()?
        .config;
    configured_attribution(&config).map(str::to_owned)
}

fn configured_attribution(config: &Value) -> Option<&str> {
    let enabled = config
        .get("features.codex_git_commit")
        .filter(|value| value.is_boolean())
        .unwrap_or(&config["features"]["codex_git_commit"]);
    if enabled != true {
        return None;
    }
    match config.get("commit_attribution") {
        None => Some(CODEX_AUTHOR),
        Some(Value::Null) => None,
        Some(Value::String(author)) => {
            let author = author.trim();
            (!author.is_empty()).then_some(author)
        }
        Some(_) => Some(CODEX_AUTHOR),
    }
}

pub(crate) fn commit_message<'a>(
    message: &'a str,
    attribution: Option<&Value>,
) -> Result<Cow<'a, str>> {
    let author = match attribution {
        None => Some(CODEX_AUTHOR),
        Some(Value::Null) => None,
        Some(Value::String(author)) => Some(author.trim()).filter(|author| !author.is_empty()),
        Some(_) => bail!("commitAttribution must be a string or null"),
    };
    let Some(author) = author else {
        return Ok(Cow::Borrowed(message));
    };
    let trailer = format!("Co-authored-by: {author}");
    if message
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&trailer))
    {
        return Ok(Cow::Borrowed(message));
    }
    let message = message.trim_end();
    if message.is_empty() {
        return Ok(Cow::Owned(trailer));
    }
    let trailers = message
        .lines()
        .rev()
        .take_while(|line| !line.trim().is_empty())
        .all(is_trailer);
    let separator = if trailers { "\n" } else { "\n\n" };
    Ok(Cow::Owned(format!("{message}{separator}{trailer}")))
}

fn is_trailer(line: &str) -> bool {
    let Some((key, value)) = line.split_once(':') else {
        return false;
    };
    key.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && value.starts_with(char::is_whitespace)
        && !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_obeys_flat_feature_precedence_and_explicit_disable() {
        assert_eq!(configured_attribution(&json!({})), None);
        assert_eq!(
            configured_attribution(&json!({"features": {"codex_git_commit": true}})),
            Some(CODEX_AUTHOR)
        );
        assert_eq!(
            configured_attribution(
                &json!({"features.codex_git_commit": false, "features": {"codex_git_commit": true}})
            ),
            None
        );
        assert_eq!(
            configured_attribution(
                &json!({"features.codex_git_commit": true, "commit_attribution": null})
            ),
            None
        );
        assert_eq!(
            configured_attribution(
                &json!({"features.codex_git_commit": true, "commit_attribution": "  "})
            ),
            None
        );
        assert_eq!(
            configured_attribution(
                &json!({"features.codex_git_commit": true, "commit_attribution": " Team <team@example.invalid> "})
            ),
            Some("Team <team@example.invalid>")
        );
    }
}
