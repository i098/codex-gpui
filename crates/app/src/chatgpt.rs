//! The ChatGPT backend as the desktop host fetches it: the web-frontend request, the Statsig
//! bootstrap and the named gates read from it.

use crate::host::{self, Identity, Request};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};

/// The original desktop build whose requests we mirror.
pub(crate) const ORIGINAL_APP_VERSION: &str = "26.1007.30912";

/// Statsig feature gates, by the id the original's bundle uses.
#[derive(Clone, Copy)]
pub(crate) enum Gate {
    ActivityFeature,
    DataPrivacy,
    PersonalizationLayout,
    UnifiedInstructions,
    ChromeExtensionSetup,
    PrivacyCenter,
    SigninBedrock,
    SigninBrowserHelp,
}

impl Gate {
    const fn id(self) -> &'static str {
        match self {
            Self::ActivityFeature => "4039078146",
            Self::DataPrivacy => "3157804138",
            Self::PersonalizationLayout => "890187595",
            Self::UnifiedInstructions => "2826489636",
            Self::ChromeExtensionSetup => "410065390",
            Self::PrivacyCenter => "3959961672",
            Self::SigninBedrock => "1934304769",
            Self::SigninBrowserHelp => "2502927686",
        }
    }

    pub(crate) fn enabled(self, payload: &Value) -> bool {
        payload["feature_gates"][self.id()]["value"] == true
    }
}

/// The HTTP status a failed host fetch carries in its error text (`host HTTP status N`).
pub(crate) fn status(error: &anyhow::Error) -> Option<u16> {
    let message = error.to_string();
    let rest = message
        .strip_prefix("host HTTP status ")
        .or_else(|| message.strip_prefix("host error "))?;
    rest.split([' ', ':']).next()?.parse().ok()
}

/// A request through the host's desktop fetch with the web frontend's headers
/// (`/global/search`, `/conversations`).
pub(crate) fn web_request(
    identity: &Identity,
    method: &str,
    url: &str,
    body: Option<String>,
) -> Request {
    let mut request = json!({
        "url": url,
        "method": method,
        "headers": {
            "x-openai-web-frontend": "codex_webview",
            "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron",
            "OAI-Language": "en-US",
            "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true,
    });
    if let Some(body) = body {
        request["body"] = body.into();
    }
    Request::Http(request)
}

fn bootstrap_body(stable_id: &str) -> Value {
    json!({
        "app_session_id": host::device_id(),
        "app_version": ORIGINAL_APP_VERSION,
        "brand_name": "chatgpt",
        "build_flavor": "prod",
        "locale": "en-US",
        "stable_id": stable_id,
        "system_name": if cfg!(target_os = "macos") { "macOS" } else { std::env::consts::OS },
        "window_type": "electron",
    })
}

/// A request with the desktop app's attach headers; the host adds the account's auth.
pub(crate) fn desktop_request(method: &str, url: &str, body: Option<Value>) -> Value {
    let mut request = json!({
        "url": url,
        "method": method,
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
        },
        "returnErrorResponse": true,
    });
    if let Some(body) = body {
        request["body"] = body.to_string().into();
    }
    request
}

/// The signed-in Statsig bootstrap; `identity` pins the account, `None` is the pre-login call.
pub(crate) fn bootstrap_request(stable_id: &str, identity: Option<&Identity>) -> Request {
    let mut request = desktop_request(
        "POST",
        "/wham/statsig/bootstrap",
        Some(bootstrap_body(stable_id)),
    );
    request["retry"] = false.into();
    if let Some(identity) = identity {
        request["expectedIdentity"] =
            json!({"accountId": identity.account_id, "userId": identity.user_id});
    }
    Request::Http(request)
}

/// Statsig's own endpoint, for a session without an account.
pub(crate) fn anonymous_bootstrap_request(stable_id: &str) -> Request {
    Request::Http(json!({
        "url": "https://ab.chatgpt.com/v1/initialize?k=client-sYWqzCYMRkUg4DqqiZcR5DGTNl2iD7zNJY0HoeDLzxR&st=js-mono&sv=3.34.0",
        "method": "POST",
        "body": json!({
            "user": {"customIDs": {"stableID": stable_id}},
            "hash": "djb2",
            "deltasResponseRequested": false,
            "full_checksum": null,
        }).to_string(),
        "headers": {"Content-Type": "application/json"},
        "retry": false,
        "returnErrorResponse": true,
    }))
}

/// The gate payload of a bootstrap response: `statsigPayload` as a JSON string or an object, or
/// the response itself when it already is the payload.
pub(crate) fn payload(response: Value) -> Result<Value> {
    let payload = match &response["statsigPayload"] {
        Value::String(raw) => serde_json::from_str(raw).context("invalid Statsig payload")?,
        Value::Object(_) => response["statsigPayload"].clone(),
        _ => response,
    };
    ensure!(
        payload["feature_gates"].is_object(),
        "Statsig response has no feature gates"
    );
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_decodes_string_object_and_bare_forms() -> Result<()> {
        let bare = json!({"feature_gates": {"4039078146": {"value": true}}});
        for response in [
            bare.clone(),
            json!({"statsigPayload": bare}),
            json!({"statsigPayload": bare.to_string()}),
        ] {
            let payload = payload(response)?;
            assert!(Gate::ActivityFeature.enabled(&payload));
            assert!(!Gate::DataPrivacy.enabled(&payload));
        }
        assert!(payload(json!({"statsigPayload": "{}"})).is_err());
        assert!(payload(json!({"other": 1})).is_err());
        Ok(())
    }

    #[test]
    fn status_reads_both_host_error_spellings() {
        let status = |text: &str| status(&anyhow::anyhow!("{text}"));
        assert_eq!(status("host HTTP status 404"), Some(404));
        assert_eq!(status("host error 429: slow down"), Some(429));
        assert_eq!(status("connection reset"), None);
    }
}
