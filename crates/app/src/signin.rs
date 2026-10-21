mod browser {
    use anyhow::Result;
    #[cfg(any(target_os = "macos", test))]
    use anyhow::{Context as _, bail, ensure};
    #[cfg(any(target_os = "macos", test))]
    use std::collections::HashMap;
    #[cfg(any(target_os = "macos", test))]
    use std::io::Read;
    #[cfg(target_os = "macos")]
    use std::io::Write;
    #[cfg(target_os = "macos")]
    use std::net::{TcpListener, TcpStream};
    #[cfg(any(target_os = "macos", test))]
    use std::time::{Duration, Instant};
    use ui::screens::signin::BrowserProfile;

    #[cfg(any(target_os = "macos", test))]
    const LIMIT: usize = 16_384;

    #[cfg(target_os = "macos")]
    pub(super) fn lookup(all_accounts: bool) -> Result<Vec<BrowserProfile>> {
        let app = std::env::var_os("CODEX_GPUI_LCU_APP")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/Applications/ChatGPT.app".into());
        let addon = app.join("Contents/Resources/native/sky.node");
        ensure!(
            addon.is_file(),
            "Browser lookup native helper is unavailable"
        );
        let mut command = std::process::Command::new(app.join("Contents/MacOS/ChatGPT"));
        command.env("ELECTRON_RUN_AS_NODE", "1")
            .arg("-e")
            .arg("process.stdout.write(JSON.stringify(require(process.argv[1]).getDefaultWebBrowserState()))")
            .arg(addon);
        let Some(output) = bounded_output(&mut command, Duration::from_secs(5))? else {
            return Ok(Vec::new());
        };
        let state: serde_json::Value = serde_json::from_slice(&output)?;
        if state["browserIsRunning"] != true || state["browserHasVisibleWindow"] != true {
            return Ok(Vec::new());
        }
        probe(all_accounts, |url| {
            let mut command = std::process::Command::new("/usr/bin/open");
            command.args(["-g", url]);
            ensure!(
                bounded_output(&mut command, Duration::from_secs(5))?.is_some(),
                "Browser lookup launch timed out"
            );
            Ok(())
        })
    }

    #[cfg(any(target_os = "macos", test))]
    fn bounded_output(
        command: &mut std::process::Command,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>> {
        use std::process::Stdio;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "Browser lookup native helper failed");
                let mut output = Vec::new();
                child
                    .stdout
                    .take()
                    .context("Browser helper stdout")?
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut output)?;
                ensure!(
                    output.len() <= LIMIT,
                    "Browser helper response is too large"
                );
                return Ok(Some(output));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn lookup(_: bool) -> Result<Vec<BrowserProfile>> {
        Ok(Vec::new())
    }

    #[cfg(target_os = "macos")]
    fn probe(
        all_accounts: bool,
        open: impl FnOnce(&str) -> Result<()>,
    ) -> Result<Vec<BrowserProfile>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let mut random = [0u8; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        use std::fmt::Write as _;
        let mut state = String::with_capacity(64);
        for byte in random {
            write!(&mut state, "{byte:02x}")?;
        }
        let mut url = url::Url::parse("https://chatgpt.com/codex/desktop-session")?;
        url.query_pairs_mut()
            .append_pair("callback_port", &listener.local_addr()?.port().to_string())
            .append_pair("state", &state)
            .append_pair("callback_method", "post");
        if all_accounts {
            url.query_pairs_mut()
                .append_pair("list_all_accounts", "true");
        }
        open(url.as_str())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let result = receive(
                        &mut stream,
                        &state,
                        deadline.saturating_duration_since(Instant::now()),
                    );
                    let (status, body) = if result.is_ok() {
                        ("200 OK", "<!doctype html><script>window.close()</script>")
                    } else {
                        ("400 Bad Request", "")
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}", body.len()).ok();
                    if let Ok(profiles) = result {
                        return Ok(profiles);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Vec::new())
    }

    #[cfg(target_os = "macos")]
    fn receive(
        stream: &mut TcpStream,
        state: &str,
        remaining: Duration,
    ) -> Result<Vec<BrowserProfile>> {
        let deadline = Instant::now() + remaining;
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut buffer = [0u8; 1024];
            let length = read_before_deadline(stream, &mut buffer, deadline)?;
            ensure!(
                length != 0 && bytes.len() + length <= LIMIT + 4096,
                "Invalid browser callback length"
            );
            bytes.extend_from_slice(&buffer[..length]);
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end])?;
        let mut lines = headers.split("\r\n");
        ensure!(
            lines.next() == Some("POST /browser-session HTTP/1.1"),
            "Invalid browser callback target"
        );
        let headers: HashMap<_, _> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim()))
            .collect();
        ensure!(
            headers.get("content-type").is_some_and(
                |value| value.split(';').next() == Some("application/x-www-form-urlencoded")
            ),
            "Invalid browser callback type"
        );
        let length: usize = headers
            .get("content-length")
            .context("Missing browser callback length")?
            .parse()?;
        ensure!(length <= LIMIT, "Browser callback is too large");
        while bytes.len() < header_end + length {
            let mut buffer = [0u8; 1024];
            let length = read_before_deadline(stream, &mut buffer, deadline)?;
            ensure!(length != 0, "Incomplete browser callback");
            bytes.extend_from_slice(&buffer[..length]);
        }
        parse(&bytes[header_end..header_end + length], state)
    }

    #[cfg(target_os = "macos")]
    fn read_before_deadline(
        stream: &mut TcpStream,
        buffer: &mut [u8],
        deadline: Instant,
    ) -> Result<usize> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .context("Browser callback timed out")?;
        stream.set_read_timeout(Some(remaining))?;
        Ok(stream.read(buffer)?)
    }

    #[cfg(any(target_os = "macos", test))]
    fn parse(body: &[u8], state: &str) -> Result<Vec<BrowserProfile>> {
        let mut fields = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            ensure!(
                matches!(
                    key.as_ref(),
                    "state"
                        | "email"
                        | "name"
                        | "avatar_url"
                        | "profiles"
                        | "error"
                        | "debug_auth_status"
                        | "debug_has_email"
                        | "debug_has_session_user"
                ),
                "Unknown browser callback field"
            );
            ensure!(
                fields
                    .insert(key.into_owned(), value.into_owned())
                    .is_none(),
                "Duplicate browser callback field"
            );
        }
        ensure!(
            fields.get("state").is_some_and(|value| value == state),
            "Invalid browser callback state"
        );
        if let Some(error) = fields.get("error") {
            ensure!(
                !["email", "name", "avatar_url", "profiles"]
                    .iter()
                    .any(|key| fields.contains_key(*key)),
                "Invalid browser error callback"
            );
            ensure!(
                matches!(
                    error.as_str(),
                    "browser_authentication_required"
                        | "browser_profile_missing_email"
                        | "browser_session_lookup_disabled"
                ),
                "Unknown browser callback error"
            );
            return Ok(Vec::new());
        }
        let profiles = if let Some(profiles) = fields.get("profiles") {
            ensure!(
                !["email", "name", "avatar_url"]
                    .iter()
                    .any(|key| fields.contains_key(*key)),
                "Invalid browser profile callback"
            );
            let profiles: Vec<serde_json::Value> = serde_json::from_str(profiles)?;
            profiles
                .into_iter()
                .map(|profile| {
                    let email = profile["email"]
                        .as_str()
                        .context("Missing profile email")?
                        .trim();
                    ensure!(
                        !email.is_empty() && email.len() <= 320,
                        "Invalid profile email"
                    );
                    Ok(BrowserProfile {
                        email: email.to_owned(),
                        avatar_url: avatar_url(profile["avatar_url"].as_str()),
                        name: profile["name"]
                            .as_str()
                            .map(|name| name.trim().chars().take(256).collect())
                            .filter(|name: &String| !name.is_empty()),
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            let email = fields.get("email").context("Missing browser email")?.trim();
            ensure!(
                !email.is_empty() && email.len() <= 320,
                "Invalid browser email"
            );
            vec![BrowserProfile {
                email: email.to_owned(),
                avatar_url: avatar_url(fields.get("avatar_url").map(String::as_str)),
                name: fields
                    .get("name")
                    .map(|name| name.trim().chars().take(256).collect())
                    .filter(|name: &String| !name.is_empty()),
            }]
        };
        if profiles.len() > 100 {
            bail!("Too many browser profiles");
        }
        Ok(profiles)
    }

    #[cfg(any(target_os = "macos", test))]
    fn avatar_url(value: Option<&str>) -> Option<String> {
        let value = value.filter(|value| value.len() <= 4096)?;
        let url = url::Url::parse(value).ok()?;
        (url.scheme() == "https").then(|| url.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stalled_lookup_helper_is_stopped() {
            let mut command = std::process::Command::new("/bin/sleep");
            command.arg("3600");
            assert!(
                bounded_output(&mut command, Duration::from_millis(10))
                    .unwrap()
                    .is_none()
            );
        }

        #[test]
        fn callback_rejects_wrong_state_duplicate_fields_and_mixed_payloads() {
            for body in [
                "state=other&email=a%40example.com",
                "state=token&state=token&email=a%40example.com",
                "state=token&profiles=%5B%5D&email=a%40example.com",
            ] {
                assert!(parse(body.as_bytes(), "token").is_err());
            }
            let profiles =
                parse(b"state=token&email=test%40example.com&name=Test", "token").unwrap();
            assert_eq!(profiles[0].email, "test@example.com");
            assert_eq!(profiles[0].name.as_deref(), Some("Test"));
        }

        #[test]
        fn callback_preserves_avatar_urls_for_both_profile_shapes() {
            let avatar = "https://example.com/avatar.png";
            let multiple = serde_json::json!([{
                "email": "test@example.com",
                "avatar_url": avatar
            }])
            .to_string();
            for (key, value) in [("avatar_url", avatar), ("profiles", multiple.as_str())] {
                let mut body = url::form_urlencoded::Serializer::new(String::new());
                body.append_pair("state", "token");
                if key == "avatar_url" {
                    body.append_pair("email", "test@example.com");
                }
                body.append_pair(key, value);
                let profiles = parse(body.finish().as_bytes(), "token").unwrap();
                assert_eq!(profiles[0].avatar_url.as_deref(), Some(avatar));
            }
        }
    }
}

use crate::{Root, host, host_settings};
use anyhow::{Context as _, Result};
use gpui::{AppContext as _, Context, Entity};
use serde_json::{Value, json};
use ui::screens::signin::bedrock::GovCloud;
use ui::screens::signin::{Login, LoginEvent, Options, Provider};
use url::Url;

const ORIGINAL_APP_VERSION: &str = "26.1007.30912";

impl Root {
    pub(crate) fn new_login(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Entity<Login> {
        let login = cx.new(Login::new);
        cx.subscribe_in(
            &login,
            window,
            |this, source, event: &LoginEvent, window, cx| {
                if this.login.as_ref() != Some(source) {
                    return;
                }
                match event {
                    LoginEvent::OpenBrowser {
                        url,
                        provider,
                        email,
                    } => {
                        let stable_id = cx
                            .global::<host_settings::Settings>()
                            .read_all()
                            .get("statsig-stable-id")
                            .and_then(Value::as_str);
                        match browser_url(
                            url,
                            *provider,
                            email.as_deref(),
                            source.read(cx).options(),
                            stable_id,
                        ) {
                            Ok(url) => {
                                cx.open_url(url.as_str());
                                source
                                    .update(cx, |login, cx| login.set_browser_url(url.into(), cx));
                            }
                            Err(error) => this.toast(
                                ui::toast::Toast::danger(format!("Sign-in failed: {error}")),
                                cx,
                            ),
                        }
                    }
                    LoginEvent::RestartBedrock { region } => {
                        this.restart_bedrock(source.clone(), region.clone(), cx)
                    }
                    LoginEvent::CheckGovCloud { region } => {
                        let bridge = this.bridge.clone();
                        let login = source.clone();
                        let region = region.clone();
                        cx.spawn(async move |_, cx| {
                            let result =
                                check_govcloud(&bridge, &region, cx.background_executor()).await;
                            login.update(cx, |login, cx| login.bedrock_finished(Ok(result), cx));
                        })
                        .detach();
                    }
                    LoginEvent::Authenticated => {
                        this.login = None;
                        this.read_account_email(window, cx);
                        this.refresh_threads(cx);
                        this.load_projects(cx);
                        this.load_sections(cx);
                        cx.notify();
                    }
                }
            },
        )
        .detach();
        let settings = cx.global_mut::<host_settings::Settings>();
        let stable_id = if let Some(id) = settings
            .read_all()
            .get("statsig-stable-id")
            .and_then(Value::as_str)
        {
            id.to_owned()
        } else {
            let id = host::device_id();
            if let Err(error) = settings.write("statsig-stable-id", json!(id)) {
                eprintln!("Cannot save sign-in device identity: {error}");
            }
            id
        };
        let host = self.host.clone();
        let weak = login.downgrade();
        let request = host::Request::Http(json!({
            "url": "/wham/statsig/bootstrap", "method": "POST",
            "body": json!({"app_session_id": host::device_id(), "app_version": ORIGINAL_APP_VERSION, "brand_name": "chatgpt", "build_flavor": "prod", "locale": "en-US", "stable_id": stable_id, "system_name": std::env::consts::OS, "window_type": "electron"}).to_string(),
            "headers": {"OAI-Language": "en", "X-OpenAI-Attach-Auth": "1", "X-OpenAI-Attach-Integrity-State": "1", "originator": "Codex Desktop"},
            "retry": false, "returnErrorResponse": true,
        }));
        let anonymous = host::Request::Http(json!({
            "url": "https://ab.chatgpt.com/v1/initialize?k=client-sYWqzCYMRkUg4DqqiZcR5DGTNl2iD7zNJY0HoeDLzxR&st=js-mono&sv=3.34.0",
            "method": "POST",
            "body": json!({"user":{"customIDs":{"stableID":stable_id}},"hash":"djb2","deltasResponseRequested":false,"full_checksum":null}).to_string(),
            "headers":{"Content-Type":"application/json"},
            "retry":false,"returnErrorResponse":true
        }));
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    host.fetch(&request)
                        .and_then(|response| host::json_body(&response))
                        .and_then(|response| options(&response))
                        .or_else(|_| {
                            host.fetch(&anonymous)
                                .and_then(|response| host::json_body(&response))
                                .and_then(|response| options(&response))
                        })
                })
                .await;
            if let Ok(mut options) = result {
                if options.browser_lookup {
                    weak.update(cx, |login, cx| login.set_browser_loading(true, cx))
                        .ok();
                    let all = options.all_browser_accounts;
                    options.browser_profiles = cx
                        .background_executor()
                        .spawn(async move { browser::lookup(all).unwrap_or_default() })
                        .await;
                }
                weak.update(cx, |login, cx| login.set_options(options, cx))
                    .ok();
            }
        })
        .detach();
        login
    }

    pub fn login(&self) -> Option<&Entity<Login>> {
        self.login.as_ref()
    }

    fn read_account_email(&self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            cx.background_spawn(async move { host.invalidate_identity() })
                .await;
            let account = bridge.account_read().await?;
            this.update_in(cx, |this, window, cx| {
                this.account_email = crate::chatgpt_email(&account);
                this.chatgpt_account = matches!(
                    account.account,
                    Some(bridge::types::Account::Chatgpt { .. })
                );
                if this.chatgpt_account
                    && this.host.supports_cloud_search()
                    && let Some(menu) = &this.command_menu
                {
                    menu.update(cx, |menu, cx| menu.enable_cloud_search(cx));
                }
                this.maybe_onboard(window, cx);
            })
        });
        crate::log_err(task, cx);
    }

    fn restart_bedrock(&mut self, login: Entity<Login>, region: String, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut command = bridge::default_command();
                    command.current_dir(cwd);
                    let (bridge, events) = bridge::Bridge::spawn(command)?;
                    pollster::block_on(bridge.initialize("codex-gpui", env!("CARGO_PKG_VERSION")))?;
                    Ok::<_, anyhow::Error>((std::sync::Arc::new(bridge), events))
                })
                .await;
            match result {
                Ok((bridge, events)) => {
                    let check = check_govcloud(&bridge, &region, cx.background_executor()).await;
                    this.update(cx, |this, cx| {
                        if this.login.as_ref() != Some(&login) {
                            return;
                        }
                        this.bridge = bridge.clone();
                        this.host = std::sync::Arc::new(
                            host::CloudHost::from_env(host::app_server_credentials(bridge.clone()))
                                .with_bridge(bridge.clone()),
                        );
                        cx.set_global(ui::screens::settings::AppServer(bridge));
                        this._events = cx.spawn(async move |this, cx| {
                            while let Ok(event) = events.recv().await {
                                if this
                                    .update(cx, |this, cx| this.on_event(event, cx))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                        login.update(cx, |login, cx| login.bedrock_finished(Ok(check), cx));
                    })
                    .ok();
                }
                Err(error) => {
                    login.update(cx, |login, cx| {
                        login.bedrock_finished(Err(error.to_string()), cx)
                    });
                }
            }
        })
        .detach();
    }
}

fn options(response: &Value) -> Result<Options> {
    let decoded;
    let payload = if let Some(raw) = response["statsigPayload"].as_str() {
        decoded = serde_json::from_str::<Value>(raw)?;
        &decoded
    } else {
        response
    };
    anyhow::ensure!(
        payload["feature_gates"].is_object() && payload["layer_configs"].is_object(),
        "missing sign-in feature policy"
    );
    let provider = &payload["layer_configs"]["1561420571"]["value"];
    let api = &payload["layer_configs"]["1504988142"]["value"];
    let auth = &payload["layer_configs"]["3446609779"]["value"];
    Ok(Options {
        providers: provider["enabled"] == true,
        bedrock: payload["feature_gates"]["1934304769"]["value"] == true,
        browser_help: payload["feature_gates"]["2502927686"]["value"] == true,
        api_key_help: matches!(
            api["api_key_help_variant"].as_str(),
            Some("link" | "explanation" | "explanation-with-chatgpt")
        ),
        explicit_api_key: api["update_cta_for_sign_in_with_api_key"] == true,
        browser_lookup: api["enabled"] == true,
        all_browser_accounts: api["list_all_accounts"] == true,
        desktop_auth: auth["use_desktop_auth"]
            .as_bool()
            .unwrap_or(auth["enabled"] == true),
        streamlined_login: auth["use_streamlined_login_ux"]
            .as_bool()
            .unwrap_or(auth["enabled"] == true),
        ..Options::default()
    })
}

async fn check_govcloud(
    bridge: &bridge::Bridge,
    region: &str,
    executor: &gpui::BackgroundExecutor,
) -> GovCloud {
    use std::task::Poll;
    let mut request = std::pin::pin!(
        bridge.request::<_, Value>("account/bedrock/checkGovCloudRequirements", json!({}))
    );
    let mut timeout = std::pin::pin!(executor.timer(std::time::Duration::from_secs(10)));
    let result = std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = request.as_mut().poll(cx) {
            return Poll::Ready(Some(result));
        }
        timeout.as_mut().poll(cx).map(|_| None)
    })
    .await;
    match result {
        Some(Ok(value)) if value["isGovCloud"] == true => GovCloud::Warning,
        Some(Ok(value)) if value["isGovCloud"] == false => GovCloud::Clear,
        Some(Err(error))
            if error
                .downcast_ref::<bridge::RpcError>()
                .is_some_and(|error| error.code == -32601)
                && !region.starts_with("us-gov-") =>
        {
            GovCloud::Clear
        }
        _ => GovCloud::Unverified,
    }
}

fn browser_url(
    raw: &str,
    provider: Provider,
    email: Option<&str>,
    options: &Options,
    stable_id: Option<&str>,
) -> Result<Url> {
    let mut url = Url::parse(raw).context("invalid sign-in URL")?;
    anyhow::ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "invalid sign-in URL scheme"
    );
    let mut desktop = if url.path() == "/codex/desktop-auth" {
        let nested = url
            .query_pairs()
            .find(|(key, _)| key == "authorize_url")
            .map(|(_, value)| value.into_owned());
        match nested {
            Some(nested) => {
                let inner = Url::parse(&nested).context("invalid nested sign-in URL")?;
                let outer = std::mem::replace(&mut url, inner);
                Some(outer)
            }
            None => return Ok(url),
        }
    } else {
        None
    };
    let query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| match provider {
            Provider::SignUp => key != "screen_hint",
            Provider::Google | Provider::Microsoft => key != "screen_hint" && key != "connection",
            Provider::OtherAccount => key != "prompt",
            Provider::SignIn => true,
        })
        .filter(|(key, _)| email.is_none() || key != "login_hint")
        .filter(|(key, _)| {
            !matches!(
                key.as_ref(),
                "codex_app_version"
                    | "source_surface_stable_id"
                    | "codex_origin_stable_id"
                    | "codex_streamlined_login"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut().clear().extend_pairs(query);
    match provider {
        Provider::SignIn => {}
        Provider::SignUp => {
            url.query_pairs_mut().append_pair("screen_hint", "signup");
        }
        Provider::Google | Provider::Microsoft => {
            url.query_pairs_mut()
                .append_pair("screen_hint", "login_or_signup")
                .append_pair(
                    "connection",
                    if provider == Provider::Google {
                        "google-oauth2"
                    } else {
                        "windowslive"
                    },
                );
        }
        Provider::OtherAccount => {
            url.query_pairs_mut()
                .append_pair("prompt", "select_account");
        }
    }
    if let Some(email) = email {
        url.query_pairs_mut().append_pair("login_hint", email);
    }
    url.query_pairs_mut()
        .append_pair("codex_app_version", ORIGINAL_APP_VERSION);
    if let Some(id) = stable_id {
        url.query_pairs_mut()
            .append_pair("source_surface_stable_id", id)
            .append_pair("codex_origin_stable_id", id);
    }
    if options.streamlined_login {
        url.query_pairs_mut()
            .append_pair("codex_streamlined_login", "true");
    }
    if desktop.is_some() || options.desktop_auth {
        let mut desktop = match desktop.take() {
            Some(desktop) => desktop,
            None => Url::parse("https://chatgpt.com/codex/desktop-auth")?,
        };
        let query: Vec<_> = desktop
            .query_pairs()
            .filter(|(key, _)| key != "authorize_url" && key != "codex_streamlined_login")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        desktop
            .query_pairs_mut()
            .clear()
            .extend_pairs(query)
            .append_pair("authorize_url", url.as_str());
        if options.streamlined_login {
            desktop
                .query_pairs_mut()
                .append_pair("codex_streamlined_login", "true");
        }
        return Ok(desktop);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_handoff_preserves_oauth_state_without_duplicate_hints() {
        let raw = "https://auth.openai.com/authorize?state=fixture&screen_hint=old&connection=old";
        let url = browser_url(
            raw,
            Provider::Google,
            Some("test+fixture@example.com"),
            &Options::default(),
            None,
        )
        .unwrap();
        let pairs: Vec<_> = url.query_pairs().collect();
        assert_eq!(
            pairs.iter().filter(|(key, _)| key == "screen_hint").count(),
            1
        );
        assert!(pairs.contains(&("state".into(), "fixture".into())));
        assert!(pairs.contains(&("connection".into(), "google-oauth2".into())));
        assert!(pairs.contains(&("login_hint".into(), "test+fixture@example.com".into())));
        assert!(
            browser_url(
                "javascript:alert(1)",
                Provider::SignIn,
                None,
                &Options::default(),
                None
            )
            .is_err()
        );
    }
}
