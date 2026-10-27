use crate::host::{Host, Identity, Request, json_body};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

pub const CALLBACK_URL: &str = "codex://connector/oauth_callback";

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectRequest {
    pub connector_id: String,
    pub name: String,
    pub host_id: String,
    pub install_url: String,
    pub reauth_link_id: Option<String>,
    pub requested_scopes: Option<Vec<String>>,
    pub action_names: Option<Vec<String>>,
    pub link_params: Option<Value>,
    pub install_attempt_id: Option<String>,
}

impl ConnectRequest {
    pub fn new(connector_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            connector_id: connector_id.into(),
            name: name.into(),
            host_id: "local".into(),
            install_url: "https://chatgpt.com".into(),
            reauth_link_id: None,
            requested_scopes: None,
            action_names: None,
            link_params: None,
            install_attempt_id: None,
        }
    }

    pub(super) fn key(&self) -> (&str, &str) {
        (&self.host_id, &self.connector_id)
    }

    pub(super) fn start(&self, host: &dyn Host, identity: &Identity) -> Result<Url> {
        ensure!(!self.connector_id.trim().is_empty(), "missing connector ID");
        ensure!(!self.host_id.trim().is_empty(), "missing host ID");
        let mut post_auth_url = http_url(&self.install_url)?;
        post_auth_url.set_path("/");
        post_auth_url.set_query(None);
        let mut fragment = url::form_urlencoded::Serializer::new(String::new());
        fragment.append_pair("connector", &self.connector_id);
        fragment.append_pair("product-sku", "CODEX");
        fragment.append_pair("referrer", "codex");
        post_auth_url.set_fragment(Some(&format!("settings/Connectors?{}", fragment.finish())));
        let (path, body) = if let Some(link_id) = &self.reauth_link_id {
            ensure!(!link_id.is_empty(), "missing reauthentication link ID");
            (
                "/aip/connectors/links/oauth/reauth",
                json!({
                    "callback_url": CALLBACK_URL,
                    "link_id": link_id,
                    "post_auth_url": post_auth_url.as_str(),
                    "requested_scopes": self.requested_scopes,
                }),
            )
        } else {
            let mut body = json!({
                "connector_id": self.connector_id,
                "name": self.name,
                "action_names": self.action_names,
                "requested_scopes": self.requested_scopes,
                "callback_url": CALLBACK_URL,
                "post_auth_url": post_auth_url.as_str(),
                "link_params": self.link_params,
            });
            if let Some(id) = &self.install_attempt_id {
                body["install_attempt_id"] = id.clone().into();
            }
            ("/aip/connectors/links/oauth", body)
        };
        let response = post(host, identity, path, body)?;
        let redirect = response["redirect_url"]
            .as_str()
            .context("OAuth response has no redirect URL")?;
        let redirect = http_url(redirect.trim())?;
        ensure!(
            oauth_state(&redirect).is_some(),
            "OAuth response has no state"
        );
        Ok(redirect)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ConnectedApp {
    pub connector_id: String,
    #[serde(rename = "id")]
    pub link_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub(super) fn complete(
    host: &dyn Host,
    identity: &Identity,
    request: &ConnectRequest,
    full_redirect_url: &str,
) -> Result<ConnectedApp> {
    let response = post(
        host,
        identity,
        "/aip/connectors/links/oauth/callback",
        json!({
            "full_redirect_url": full_redirect_url,
        }),
    )?;
    let app: ConnectedApp = serde_json::from_value(response["link"].clone())?;
    ensure!(
        app.connector_id == request.connector_id,
        "callback connector changed"
    );
    ensure!(!app.link_id.is_empty(), "callback has no link ID");
    if let Some(link_id) = &request.reauth_link_id {
        ensure!(app.link_id == *link_id, "reauthentication link changed");
    }
    Ok(app)
}

fn post(host: &dyn Host, identity: &Identity, path: &str, body: Value) -> Result<Value> {
    ensure!(
        host.identity().as_ref() == Some(identity),
        "OAuth account changed"
    );
    let response = host.fetch(&Request::Http(json!({
        "url": path,
        "method": "POST",
        "body": body.to_string(),
        "headers": {
            "oai-did": identity.device_id,
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
            "OAI-Product-Sku": "CODEX",
            "X-Codex-Surface": "connector_setup",
            "ChatGPT-Account-ID": identity.account_id,
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true,
    })))?;
    ensure!(
        host.identity().as_ref() == Some(identity),
        "OAuth account changed"
    );
    json_body(&response)
}

fn http_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "https" | "http"),
        "unsupported OAuth URL scheme"
    );
    ensure!(url.host_str().is_some(), "OAuth URL has no host");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "OAuth URL contains credentials"
    );
    Ok(url)
}

pub(super) fn oauth_state(url: &Url) -> Option<String> {
    let mut states = url.query_pairs().filter(|(key, _)| key == "state");
    let state = states.next()?.1.trim().to_owned();
    (!state.is_empty() && states.next().is_none()).then_some(state)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CallbackRoute {
    Connector,
    ExcludedProduct,
}

impl CallbackRoute {
    /// Classifies only the original app scheme and ChatGPT origin. Other origins remain unhandled.
    pub fn from_url(value: &str) -> Option<Self> {
        let url = Url::parse(value).ok()?;
        if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
            return None;
        }
        let path = match url.scheme() {
            "codex" if url.host_str() == Some("connector") && url.path() == "/oauth_callback" => {
                return Some(Self::Connector);
            }
            "https" if url.host_str() == Some("chatgpt.com") => url.path(),
            _ => return None,
        };
        match path {
            "/connector/oauth_callback" => Some(Self::Connector),
            "/chatgpt_health_oauth_redirect"
            | "/ledger/plaid/web/oauth_redirect"
            | "/mobile/app-specific-password"
            | "/mobile/app-specific-password/complete" => Some(Self::ExcludedProduct),
            _ => None,
        }
    }

    pub fn is_codex(self) -> bool {
        self == Self::Connector
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct SchemaHost {
        identity: Mutex<Identity>,
        response: Value,
        switch_account: AtomicBool,
    }

    impl SchemaHost {
        fn new(response: Value) -> Self {
            Self {
                identity: Mutex::new(Identity {
                    account_id: "account".into(),
                    user_id: "user".into(),
                    device_id: "device".into(),
                }),
                response,
                switch_account: AtomicBool::new(false),
            }
        }
    }

    impl Host for SchemaHost {
        fn identity(&self) -> Option<Identity> {
            Some(self.identity.lock().unwrap().clone())
        }

        fn fetch(&self, _: &Request) -> Result<Value> {
            if self.switch_account.swap(false, Ordering::SeqCst) {
                self.identity.lock().unwrap().account_id = "changed".into();
            }
            Ok(json!({"status": 200, "body": self.response.to_string()}))
        }
    }

    #[test]
    fn an_account_switch_cannot_complete_a_connection() {
        let host = SchemaHost::new(json!({
            "link": {"id": "link", "connector_id": "connector", "name": "Example"},
        }));
        let identity = host.identity().unwrap();
        host.switch_account.store(true, Ordering::SeqCst);
        let error = complete(
            &host,
            &identity,
            &ConnectRequest::new("connector", "Example"),
            "codex://connector/oauth_callback?code=code&state=state",
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "OAuth account changed");
    }

    #[test]
    fn callback_cannot_connect_another_connector_or_reauthentication_link() {
        let host = SchemaHost::new(json!({
            "link": {"id": "different-link", "connector_id": "connector", "name": "Example"},
        }));
        let identity = host.identity().unwrap();
        let mut request = ConnectRequest::new("other-connector", "Example");
        let error = complete(&host, &identity, &request, CALLBACK_URL).unwrap_err();
        assert_eq!(error.to_string(), "callback connector changed");
        request.connector_id = "connector".into();
        request.reauth_link_id = Some("expected-link".into());
        let error = complete(&host, &identity, &request, CALLBACK_URL).unwrap_err();
        assert_eq!(error.to_string(), "reauthentication link changed");
    }

    #[test]
    fn provider_redirect_requires_a_web_url_and_one_nonempty_state() {
        for redirect in [
            "file:///tmp/steal?state=state",
            "https://name:secret@example.com/?state=state",
            "https://example.com/?state=",
            "https://example.com/?state=one&state=two",
            "https://example.com/?code=code",
        ] {
            let host = SchemaHost::new(json!({"redirect_url": redirect}));
            assert!(
                ConnectRequest::new("connector", "Example")
                    .start(&host, &host.identity().unwrap())
                    .is_err(),
                "{redirect}"
            );
        }
        let host = SchemaHost::new(json!({"redirect_url": "https://example.com/?state=one"}));
        assert_eq!(
            ConnectRequest::new("connector", "Example")
                .start(&host, &host.identity().unwrap())
                .unwrap()
                .host_str(),
            Some("example.com")
        );
    }

    #[test]
    fn external_origins_cannot_enter_the_callback_route() {
        for url in [
            "https://example.com/connector/oauth_callback",
            "https://chatgpt.com.evil.test/connector/oauth_callback",
            "https://name:secret@chatgpt.com/connector/oauth_callback",
            "codex://connector:123/oauth_callback",
            "codex://other/oauth_callback",
        ] {
            assert_eq!(CallbackRoute::from_url(url), None, "{url}");
        }
        assert_eq!(
            CallbackRoute::from_url(CALLBACK_URL),
            Some(CallbackRoute::Connector)
        );
        for path in [
            "/chatgpt_health_oauth_redirect",
            "/ledger/plaid/web/oauth_redirect",
            "/mobile/app-specific-password",
            "/mobile/app-specific-password/complete",
        ] {
            assert!(
                !CallbackRoute::from_url(&format!("https://chatgpt.com{path}"))
                    .unwrap()
                    .is_codex()
            );
        }
    }
}
