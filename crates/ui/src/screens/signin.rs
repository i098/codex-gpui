pub mod bedrock;
pub mod policy {
    use serde_json::Value;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Policy {
        Loading,
        Error,
        Unsupported,
        Ready { api: bool, chatgpt: bool },
    }

    impl Policy {
        pub fn requirements(response: &Value) -> Self {
            let Some(requirements) = response.get("requirements") else {
                return Self::Error;
            };
            if requirements.is_null() {
                return Self::Loading;
            }
            let Some(methods) = requirements.get("allowedLoginMethods") else {
                return Self::Unsupported;
            };
            if methods.is_null() {
                return Self::Ready {
                    api: true,
                    chatgpt: true,
                };
            }
            let Some(methods) = methods.as_array() else {
                return Self::Error;
            };
            if methods
                .iter()
                .any(|value| !matches!(value.as_str(), Some("api" | "chatgpt")))
            {
                return Self::Error;
            }
            Self::Ready {
                api: methods.iter().any(|value| value == "api"),
                chatgpt: methods.iter().any(|value| value == "chatgpt"),
            }
        }

        pub fn config(response: &Value) -> Self {
            let Some(config) = response.get("config").filter(|value| value.is_object()) else {
                return Self::Error;
            };
            let method = config
                .get("forced_login_method")
                .filter(|value| !value.is_null());
            Self::Ready {
                api: method.is_none_or(|value| value == "api"),
                chatgpt: method.is_none_or(|value| value == "chatgpt"),
            }
        }

        pub fn permits_api(self) -> bool {
            matches!(self, Self::Ready { api: true, .. })
        }

        pub fn permits_chatgpt(self) -> bool {
            matches!(self, Self::Ready { chatgpt: true, .. })
        }

        pub fn message(self) -> Option<&'static str> {
            match self {
                Self::Loading => Some("Loading sign-in requirements…"),
                Self::Error => Some("Unable to load sign-in requirements"),
                Self::Unsupported => {
                    Some("Update Codex on this machine to read its managed sign-in requirements")
                }
                Self::Ready {
                    api: false,
                    chatgpt: false,
                } => Some("Your administrator has disabled all available sign-in methods"),
                Self::Ready { .. } => None,
            }
        }
    }
    pub fn gateway_provider(response: &Value) -> Option<(String, String)> {
        if !response.get("required")?.as_bool()? {
            return None;
        }
        Some((
            response.get("providerId")?.as_str()?.to_owned(),
            response.get("providerName")?.as_str()?.to_owned(),
        ))
    }
    pub(super) fn supports_access_keys(user_agent: &str) -> bool {
        let Some(version) = user_agent
            .split_whitespace()
            .next()
            .and_then(|agent| agent.rsplit('/').next())
        else {
            return false;
        };
        static MINIMUM: std::sync::LazyLock<semver::Version> = std::sync::LazyLock::new(|| {
            semver::Version::parse("0.150.0-alpha.9").expect("valid minimum version")
        });
        semver::Version::parse(version).is_ok_and(|version| version >= *MINIMUM)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn gateway_login_requires_a_complete_required_provider() {
            assert_eq!(
                gateway_provider(&json!({
                    "required": true,
                    "providerId": "corp",
                    "providerName": "Company"
                })),
                Some(("corp".into(), "Company".into()))
            );
            for response in [
                json!({"required": false, "providerId": "corp", "providerName": "Company"}),
                json!({"required": true, "providerId": "corp"}),
                json!({"required": true, "providerName": "Company"}),
            ] {
                assert_eq!(gateway_provider(&response), None);
            }
        }
        #[test]
        fn bedrock_access_keys_respect_prerelease_ordering() {
            for (version, supported) in [
                ("0.150.0-alpha.8", false),
                ("0.150.0-alpha.9", true),
                ("0.150.0-beta.1", true),
                ("0.163.0-alpha.10.2", true),
                ("unknown", false),
            ] {
                assert_eq!(
                    supports_access_keys(&format!("codex/{version} (macOS)")),
                    supported
                );
            }
        }

        #[test]
        fn managed_restrictions_fail_closed_and_override_local_config() {
            for response in [
                json!({}),
                json!({"requirements": {"allowedLoginMethods": true}}),
                json!({"requirements": {"allowedLoginMethods": ["api", "unknown"]}}),
            ] {
                assert_eq!(Policy::requirements(&response), Policy::Error);
            }
            assert_eq!(
                Policy::requirements(&json!({"requirements": {}})),
                Policy::Unsupported
            );
            assert_eq!(
                Policy::requirements(&json!({"requirements": null})),
                Policy::Loading
            );
            for (methods, api, chatgpt) in [
                (json!([]), false, false),
                (json!(["api"]), true, false),
                (json!(["chatgpt"]), false, true),
                (Value::Null, true, true),
            ] {
                assert_eq!(
                    Policy::requirements(
                        &json!({"requirements": {"allowedLoginMethods": methods}})
                    ),
                    Policy::Ready { api, chatgpt }
                );
            }
            assert_eq!(Policy::config(&json!({})), Policy::Error);
            for (method, api, chatgpt) in [
                (Value::Null, true, true),
                (json!("api"), true, false),
                (json!("chatgpt"), false, true),
                (json!("unknown"), false, false),
            ] {
                assert_eq!(
                    Policy::config(&json!({"config": {"forced_login_method": method}})),
                    Policy::Ready { api, chatgpt }
                );
            }
        }
    }
}
mod snake {
    use std::collections::VecDeque;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Direction {
        Up,
        Down,
        Left,
        Right,
    }

    impl Direction {
        fn delta(self) -> (i32, i32) {
            match self {
                Self::Up => (0, -1),
                Self::Down => (0, 1),
                Self::Left => (-1, 0),
                Self::Right => (1, 0),
            }
        }
    }

    pub struct Snake {
        pub cells: VecDeque<(i32, i32)>,
        pub food: (i32, i32),
        pub columns: i32,
        pub rows: i32,
        direction: Direction,
        next: Direction,
        random: u64,
    }

    impl Snake {
        pub fn new(width: f32, height: f32, seed: u64) -> Self {
            let columns = (width / 18.).floor().max(12.) as i32;
            let cell_size = width / columns as f32;
            let rows = (height / cell_size).floor().max(12.) as i32;
            let head = (columns / 2, rows / 2);
            let mut snake = Self {
                cells: VecDeque::from([head, (head.0 - 1, head.1), (head.0 - 2, head.1)]),
                food: (0, 0),
                columns,
                rows,
                direction: Direction::Right,
                next: Direction::Right,
                random: seed.max(1),
            };
            snake.place_food();
            snake
        }

        pub fn turn(&mut self, direction: Direction) {
            let current = self.direction.delta();
            let next = direction.delta();
            if current.0 != -next.0 || current.1 != -next.1 {
                self.next = direction;
            }
        }

        pub fn tick(&mut self) -> bool {
            let delta = self.next.delta();
            let head = self.cells[0];
            let head = (head.0 + delta.0, head.1 + delta.1);
            if head.0 < 0 || head.1 < 0 || head.0 >= self.columns || head.1 >= self.rows {
                return false;
            }
            let eating = head == self.food;
            let occupied = self.cells.len() - usize::from(!eating);
            if self.cells.iter().take(occupied).any(|cell| *cell == head) {
                return false;
            }
            if !eating {
                self.cells.pop_back();
            }
            self.cells.push_front(head);
            self.direction = self.next;
            if eating {
                if self.cells.len() == (self.columns * self.rows) as usize {
                    return false;
                }
                self.place_food();
            }
            true
        }

        fn place_food(&mut self) {
            loop {
                self.random ^= self.random << 13;
                self.random ^= self.random >> 7;
                self.random ^= self.random << 17;
                let cell = (self.random % (self.columns * self.rows) as u64) as i32;
                let food = (cell % self.columns, cell / self.columns);
                if !self.cells.contains(&food) {
                    self.food = food;
                    return;
                }
            }
        }
    }

    impl super::Login {
        pub(super) fn start_snake(
            &mut self,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) {
            let size = window.viewport_size();
            self.snake = Some(Snake::new(
                size.width.into(),
                size.height.into(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
            ));
            self.snake_timer = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(120))
                        .await;
                    let active = this
                        .update(cx, |this, cx| {
                            let active = this.snake.as_mut().is_some_and(Snake::tick);
                            if !active {
                                this.snake = None;
                            }
                            cx.notify();
                            active
                        })
                        .unwrap_or(false);
                    if !active {
                        break;
                    }
                }
            }));
            cx.notify();
        }

        pub(super) fn snake_view(&self, cx: &mut gpui::Context<Self>) -> gpui::Div {
            use gpui::{
                InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _,
            };
            use theme::ActiveTheme as _;
            let entity = cx.entity();
            gpui::div()
                .size_full()
                .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    let direction = match event.keystroke.key.as_str() {
                        "up" | "w" => Some(Direction::Up),
                        "down" | "s" => Some(Direction::Down),
                        "left" | "a" => Some(Direction::Left),
                        "right" | "d" => Some(Direction::Right),
                        _ => None,
                    };
                    if let Some(direction) = direction
                        && let Some(snake) = &mut this.snake
                    {
                        snake.turn(direction);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    gpui::canvas(
                        |_, _, _| {},
                        move |bounds, _, window, cx| {
                            let Some(snake) = &entity.read(cx).snake else {
                                return;
                            };
                            let cell = bounds.size.width / snake.columns as f32;
                            for &(x, y) in &snake.cells {
                                let rect = gpui::Bounds::new(
                                    bounds.origin + gpui::point(cell * x as f32, cell * y as f32),
                                    gpui::size(cell, cell),
                                );
                                window.paint_quad(gpui::fill(rect, cx.theme().text));
                            }
                            let (x, y) = snake.food;
                            let rect = gpui::Bounds::new(
                                bounds.origin + gpui::point(cell * x as f32, cell * y as f32),
                                gpui::size(cell, cell),
                            );
                            window.paint_quad(gpui::fill(rect, cx.theme().text_error));
                        },
                    )
                    .size_full()
                    .into_any_element(),
                )
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn snake_rejects_reverse_grows_on_food_and_exits_at_walls() {
            let mut snake = Snake::new(216., 216., 42);
            snake.turn(Direction::Left);
            snake.food = (7, 6);
            assert!(snake.tick());
            assert_eq!(snake.cells[0], (7, 6));
            assert_eq!(snake.cells.len(), 4);
            assert!(!snake.cells.contains(&snake.food));
            for _ in 0..4 {
                assert!(snake.tick());
            }
            assert!(!snake.tick());
        }
    }
}

use self::policy::Policy;
use super::settings::AppServer;
use crate::{SANS, composer::Composer, focus, icon::icon, text};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Theme};

fn login_theme(cx: &App) -> Theme {
    let mut theme = cx.theme().clone();
    if theme.appearance == theme::Appearance::Dark {
        theme.text = gpui::rgb(0xdfdfdf).into();
        theme.button_primary_solid = theme.text;
        theme.text_inverse = gpui::rgb(0x2d2d2d).into();
        let white = gpui::Hsla::from(gpui::rgb(0xffffff));
        theme.text_description = white.opacity(0.498);
        theme.border = white.opacity(0.084);
    }
    theme
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    SignIn,
    SignUp,
    Google,
    Microsoft,
    OtherAccount,
}

pub enum LoginEvent {
    OpenBrowser {
        url: String,
        provider: Provider,
        email: Option<String>,
    },
    OpenUrl(String),
    /// The sign-in requirements are read and the screen shows its methods.
    MethodsLoaded,
    Authenticated,
    RestartBedrock {
        region: String,
    },
    CheckGovCloud {
        region: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub providers: bool,
    pub bedrock: bool,
    pub api_key_help: bool,
    pub explicit_api_key: bool,
    pub browser_lookup: bool,
    pub all_browser_accounts: bool,
    pub desktop_auth: bool,
    pub streamlined_login: bool,
    pub browser_help: bool,
    pub browser_profiles: Vec<BrowserProfile>,
}

#[derive(Clone, Debug)]
pub struct BrowserProfile {
    pub email: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Welcome,
    OtherMethods,
    ApiKey,
}

#[derive(Clone, Copy)]
enum Command {
    Start(Provider),
    GatewayOAuth,
    Recognized(usize),
    CreateKey,
    ShowApiKey,
    ShowOtherMethods,
    Back,
    Submit,
    Cancel,
    CopyLink,
    Retry,
    PlaySnake,
    ShowBedrock,
    BedrockMethod(bedrock::Method),
    BedrockChoices,
    BedrockSubmit,
    BedrockBack,
    BedrockFinish,
    BedrockRetry,
}

#[derive(Clone, Copy)]
enum Style {
    Outline,
    Primary,
    Link,
}

pub struct Login {
    bridge: Option<std::sync::Arc<bridge::Bridge>>,
    stage: Stage,
    policy: Policy,
    policy_generation: u64,
    options: Options,
    api_key: Entity<Composer>,
    error: Option<String>,
    pending: bool,
    attempt: u64,
    login_id: Option<String>,
    login_url: Option<String>,
    restart_blocked: bool,
    snake: Option<snake::Snake>,
    snake_timer: Option<gpui::Task<()>>,
    bedrock: Option<bedrock::Bedrock>,
    supports_access_keys: bool,
    browser_loading: bool,
    gateway_pending: bool,
    hovered_button: Option<&'static str>,
    gateway_provider: Option<(String, String)>,
}

impl EventEmitter<LoginEvent> for Login {}

async fn gateway_oauth_request<R>(
    request: impl Future<Output = anyhow::Result<R>>,
    timeout: impl Future<Output = ()>,
) -> (anyhow::Result<R>, bool) {
    let mut request = std::pin::pin!(request);
    let mut timeout = std::pin::pin!(timeout);
    std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(result) = request.as_mut().poll(cx) {
            return std::task::Poll::Ready((result, false));
        }
        if timeout.as_mut().poll(cx).is_ready() {
            return std::task::Poll::Ready((
                Err(anyhow::anyhow!("Gateway sign-in timed out")),
                true,
            ));
        }
        std::task::Poll::Pending
    })
    .await
}

impl Login {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let text_color = login_theme(cx).text;
        let api_key = cx.new(|cx| {
            let mut field = Composer::settings_field("sk-…", "", false, cx);
            field.set_compact_field(text_color, cx);
            field.set_field_label("OpenAI API key", cx);
            field
        });
        cx.observe(&api_key, |_, _, cx| cx.notify()).detach();
        let mut this = Self {
            bridge: cx.try_global::<AppServer>().map(|server| server.0.clone()),
            stage: Stage::Welcome,
            policy: Policy::Loading,
            policy_generation: 0,
            options: Options::default(),
            api_key,
            error: None,
            pending: false,
            attempt: 0,
            login_id: None,
            login_url: None,
            restart_blocked: false,
            snake: None,
            snake_timer: None,
            bedrock: None,
            supports_access_keys: false,
            browser_loading: false,
            hovered_button: None,
            gateway_provider: None,
            gateway_pending: false,
        };
        this.load_policy(cx);
        this
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn api_key_field(&self) -> &Entity<Composer> {
        &self.api_key
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn set_browser_url(&mut self, url: String, cx: &mut Context<Self>) {
        self.login_url = Some(url);
        cx.notify();
    }

    pub fn set_browser_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.browser_loading = loading;
        cx.notify();
    }

    pub fn set_options(&mut self, options: Options, cx: &mut Context<Self>) {
        self.options = options;
        self.browser_loading = false;
        cx.notify();
    }

    pub fn restart_ready(&self) -> bool {
        !self.restart_blocked && self.stage == Stage::Welcome && !self.pending
    }

    fn load_policy(&mut self, cx: &mut Context<Self>) {
        self.policy = Policy::Loading;
        self.policy_generation += 1;
        let generation = self.policy_generation;
        let Some(bridge) = self.bridge.clone() else {
            self.policy = Policy::Error;
            return;
        };
        cx.spawn(async move |this, cx| {
            let supports_access_keys = bridge
                .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                .await
                .is_ok_and(|server| policy::supports_access_keys(&server.user_agent));
            let policy = match bridge
                .request::<_, Value>("configRequirements/read", json!({}))
                .await
            {
                Ok(value) => {
                    let policy = Policy::requirements(&value);
                    if policy == Policy::Loading {
                        match bridge
                            .request::<_, Value>(
                                "config/read",
                                json!({"includeLayers": false, "cwd": null}),
                            )
                            .await
                        {
                            Ok(config) => Policy::config(&config),
                            Err(_) => Policy::Error,
                        }
                    } else {
                        policy
                    }
                }
                Err(_) => Policy::Error,
            };
            let gateway_provider = bridge
                .request::<_, Value>("account/gatewayOAuth/read", Option::<()>::None)
                .await
                .ok()
                .and_then(|value| policy::gateway_provider(&value));
            this.update(cx, |this, cx| {
                if generation != this.policy_generation {
                    return;
                }
                this.policy = policy;
                this.supports_access_keys = supports_access_keys;
                this.gateway_provider = gateway_provider;
                if !policy.permits_api() {
                    this.stage = Stage::Welcome;
                }
                cx.emit(LoginEvent::MethodsLoaded);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub fn apply(&mut self, event: &bridge::Event, cx: &mut Context<Self>) {
        let bridge::Event::Notification { method, params } = event else {
            return;
        };
        if method == "account/gatewayOAuth/changed" {
            if !self.pending
                || params["providerId"].as_str()
                    != self.gateway_provider.as_ref().map(|(id, _)| id.as_str())
            {
                return;
            }
            match params["status"].as_str() {
                Some("started") => {
                    if let Some(url) = params["authUrl"].as_str() {
                        cx.emit(LoginEvent::OpenUrl(url.to_owned()));
                    }
                }
                Some("failed") => {
                    self.gateway_pending = false;
                    self.attempt += 1;
                    self.pending = false;
                    self.error = Some(format!(
                        "Sign-in failed: {}",
                        params["error"].as_str().unwrap_or("Gateway sign-in failed")
                    ));
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        if method != "account/login/completed"
            || !self.pending
            || params["loginId"].as_str() != self.login_id.as_deref()
            || self.login_id.is_none()
        {
            return;
        }
        self.pending = false;
        self.login_id = None;
        self.login_url = None;
        if params["success"] == true {
            cx.emit(LoginEvent::Authenticated);
        } else {
            self.error = Some(format!(
                "Sign-in failed: {}",
                params["error"].as_str().unwrap_or_default()
            ));
        }
        cx.notify();
    }

    fn start_gateway_oauth(&mut self, cx: &mut Context<Self>) {
        if self.pending || !self.policy.permits_chatgpt() || self.gateway_provider.is_none() {
            return;
        }
        let Some(bridge) = self.bridge.clone() else {
            return;
        };
        self.gateway_pending = true;
        self.pending = true;
        self.error = None;
        self.restart_blocked = true;
        self.attempt += 1;
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let (result, timed_out) = gateway_oauth_request(
                bridge.request::<_, Value>("account/gatewayOAuth/login", Option::<()>::None),
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(240)),
            )
            .await;
            if timed_out {
                let _ = bridge
                    .request::<_, Value>("account/gatewayOAuth/cancel", Option::<()>::None)
                    .await;
            }
            this.update(cx, |this, cx| {
                if this.attempt != attempt {
                    return;
                }
                this.gateway_pending = false;
                this.pending = false;
                match result {
                    Ok(_) => cx.emit(LoginEvent::Authenticated),
                    Err(error) => this.error = Some(format!("Sign-in failed: {error}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn start(&mut self, provider: Provider, email: Option<String>, cx: &mut Context<Self>) {
        if self.pending || !self.policy.permits_chatgpt() {
            return;
        }
        let Some(bridge) = self.bridge.clone() else {
            return;
        };
        self.pending = true;
        self.login_url = None;
        self.restart_blocked = true;
        self.error = None;
        self.attempt += 1;
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let result = bridge.request::<_, Value>("account/login/start", json!({"type": "chatgpt", "appBrand": "chatgpt", "useHostedLoginSuccessPage": true})).await;
            this.update(cx, |this, cx| {
                if this.attempt != attempt {
                    if let Ok(value) = result && let Some(id) = value["loginId"].as_str() {
                        cancel(bridge.clone(), id.to_owned(), cx);
                    }
                    return;
                }
                match result {
                    Ok(value) => match (value["loginId"].as_str(), value["authUrl"].as_str()) {
                        (Some(id), Some(url)) => {
                            this.login_id = Some(id.to_owned());
                            cx.emit(LoginEvent::OpenBrowser { url: url.to_owned(), provider, email });
                        }
                        _ => { this.pending = false; this.error = Some("Sign-in failed: invalid sign-in response".into()); }
                    },
                    Err(error) => { this.pending = false; this.error = Some(format!("Sign-in failed: {error}")); }
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending || !self.policy.permits_api() {
            return;
        }
        let key = self.api_key.read(cx).field_text().trim();
        if key.is_empty() {
            return;
        }
        let params = json!({"type": "apiKey", "apiKey": key});
        let Some(bridge) = self.bridge.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        self.restart_blocked = true;
        self.attempt += 1;
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let result = bridge
                .request::<_, Value>("account/login/start", params)
                .await;
            this.update(cx, |this, cx| {
                if attempt != this.attempt {
                    return;
                }
                this.pending = false;
                match result {
                    Ok(_) => cx.emit(LoginEvent::Authenticated),
                    Err(error) => this.error = Some(format!("Sign-in failed: {error}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Abandons the browser sign-in in progress (`account/login/cancel`).
    pub fn cancel_pending(&mut self, cx: &mut Context<Self>) {
        self.attempt += 1;
        if self.gateway_pending {
            self.gateway_pending = false;
            if let Some(bridge) = self.bridge.clone() {
                cx.spawn(async move |_, _| {
                    let _ = bridge
                        .request::<_, Value>("account/gatewayOAuth/cancel", Option::<()>::None)
                        .await;
                })
                .detach();
            }
        }
        self.pending = false;
        self.login_url = None;
        if let Some(id) = self.login_id.take()
            && let Some(bridge) = self.bridge.clone()
        {
            cancel(bridge, id, cx);
        }
    }

    fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::GatewayOAuth => self.start_gateway_oauth(cx),
            Command::Start(provider) => self.start(provider, None, cx),
            Command::Recognized(index) => {
                if let Some(profile) = self.options.browser_profiles.get(index) {
                    self.start(Provider::SignIn, Some(profile.email.clone()), cx);
                }
            }
            Command::CreateKey => cx.open_url("https://platform.openai.com/api-keys"),
            Command::PlaySnake => self.start_snake(window, cx),
            Command::ShowBedrock => self.open_bedrock(cx),
            Command::ShowOtherMethods => {
                self.stage = Stage::OtherMethods;
                self.error = None;
            }
            Command::BedrockMethod(_)
            | Command::BedrockChoices
            | Command::BedrockSubmit
            | Command::BedrockBack
            | Command::BedrockRetry
            | Command::BedrockFinish => self.bedrock_command(command, window, cx),
            _ => self.setup_command(command, window, cx),
        }
        cx.notify();
    }

    fn setup_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::ShowApiKey if self.policy.permits_api() => {
                self.stage = Stage::ApiKey;
                self.error = None;
                self.api_key.focus_handle(cx).focus(window, cx);
            }
            Command::Back if !self.pending => {
                let was_api = self.stage == Stage::ApiKey;
                self.stage = if was_api && self.options.bedrock {
                    Stage::OtherMethods
                } else {
                    Stage::Welcome
                };
                if was_api {
                    self.api_key.update(cx, |field, cx| field.set_text("", cx));
                }
                self.error = None;
            }
            Command::Submit => self.submit(cx),
            Command::Cancel => self.cancel_pending(cx),
            Command::CopyLink => {
                if let Some(url) = &self.login_url {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()));
                }
            }
            Command::Retry => self.load_policy(cx),
            _ => {}
        }
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        command: Command,
        style: Style,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = login_theme(cx);
        let label = label.into();
        let api_form =
            self.stage == Stage::ApiKey && matches!(command, Command::Submit | Command::Back);
        let hovered = self.hovered_button == Some(id);
        let color = style.color(hovered, &t);
        let mut button = div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .relative()
            .w_full()
            .h(px(if api_form {
                36.
            } else if matches!(style, Style::Primary) {
                48.
            } else {
                46.
            }))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .rounded_full()
            .cursor_pointer()
            .text_color(color)
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.hover_button(id, *hovered, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if matches!(key.key.as_str(), "enter" | "space")
                    && !key.modifiers.platform
                    && !key.modifiers.control
                {
                    this.command(command, window, cx);
                    cx.stop_propagation();
                }
            }));
        if matches!(style, Style::Link) {
            button = button.h(px(36.)).w_auto().px(px(8.));
        } else {
            button = button.child(shapes::decoration(style.decoration(api_form, hovered, &t)));
        }
        let label = div()
            .text_size(px(14.))
            .line_height(px(if api_form { 18. } else { 20. }))
            .font_weight(gpui::FontWeight(if api_form { 430. } else { 500. }))
            .whitespace_nowrap()
            .when(matches!(style, Style::Link), |link| {
                link.relative().child(underline(label.clone()))
            })
            .child(label);
        let button = button.children(command.icon(style, color));
        let focus = focus::handle(id, 0, window, cx);
        focus::ring(button.child(label), &focus, Theme::RADIUS_FULL, px(2.), cx)
    }
}

/// Blink's auto `text-decoration: underline` under one line of `text`, painted before the text:
/// `max(1, font size / 10)` device px thick, truncated to whole px, its top
/// `ceil(thickness / 2)` device px below the baseline. Ink skipping is not drawn.
fn underline(text: SharedString) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let style = window.text_style();
            let size = style.font_size.to_pixels(window.rem_size());
            let run = style.to_run(text.len());
            let line = window
                .text_system()
                .shape_line(text.clone(), size, &[run], None);
            let scale = window.scale_factor();
            let thickness = (f32::from(size) * scale / 10.).max(1.);
            let gap = (thickness / 2.).ceil();
            // The text's baseline: Blink floors the half leading to whole device px.
            let leading = f32::from(bounds.size.height - line.ascent - line.descent) * scale;
            let below = px(((leading / 2.).floor() + gap) / scale);
            let top = bounds.top() + line.ascent + below;
            window.paint_underline(
                gpui::point(bounds.left(), top),
                line.width,
                &gpui::UnderlineStyle {
                    color: Some(style.color),
                    thickness: px(thickness.trunc() / scale),
                    wavy: false,
                },
            );
        },
    )
    .absolute()
    .inset_0()
}

fn cancel(bridge: std::sync::Arc<bridge::Bridge>, id: String, cx: &App) {
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = bridge
                .request::<_, Value>("account/login/cancel", json!({"loginId": id}))
                .await
            {
                eprintln!("account/login/cancel: {error}");
            }
        })
        .detach();
}

impl Render for Login {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = login_theme(cx);
        let frame = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family(SANS)
            .bg(t.surface)
            .on_key_down(focus::tab);
        if self.snake.is_some() {
            return frame.child(self.snake_view(cx));
        }
        if let Some(message) = self.policy.message() {
            return self.requirements_view(frame, message, window, cx);
        }
        if self.bedrock.is_some() {
            return frame.child(self.bedrock_view(window, cx));
        }
        if self.pending && self.stage == Stage::Welcome {
            return self.browser_pending_view(frame, window, cx);
        }
        if self.browser_loading && self.stage == Stage::Welcome {
            return frame.child(text("Loading…", 14., 20., 400., t.text_description));
        }
        let header = self.login_header(window, cx);
        let body = self.signin_controls(window, cx);
        frame.pb(px(24.)).child(
            // The original composites this column: its text, icons and pill edges blend
            // into a transparent tile that is then drawn over the surface.
            div()
                .w(px(340.))
                .layer()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(32.))
                .child(header)
                .child(body)
                .children(
                    self.error
                        .clone()
                        .map(|error| text(error, 14., 20., 400., t.text_error).whitespace_normal()),
                ),
        )
    }
}

impl Style {
    fn decoration(self, api_form: bool, hovered: bool, t: &Theme) -> shapes::Decoration {
        let style = self;
        let (fill, border) = match style {
            Style::Primary => (
                Some(if hovered {
                    t.text.opacity(0.8)
                } else {
                    t.button_primary_solid
                }),
                Some((
                    px(1.),
                    if api_form {
                        t.border
                    } else {
                        gpui::transparent_black()
                    },
                )),
            ),
            Style::Outline if api_form => {
                (Some(t.text.opacity(if hovered { 0.1 } else { 0.05 })), None)
            }
            Style::Outline => (
                Some(if hovered { t.ghost_hover } else { t.surface }),
                Some((px(1.), t.border)),
            ),
            Style::Link => (None, None),
        };
        shapes::Decoration {
            radius: gpui::Corners::all(Theme::RADIUS_FULL),
            corner: shapes::Corner::Round,
            fill,
            border,
            ..Default::default()
        }
    }
}

impl Login {
    fn account_choices(&self, mut body: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = login_theme(cx);
        for (index, profile) in self.options.browser_profiles.iter().take(3).enumerate() {
            let email = profile.email.clone();
            let name = profile
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&email);
            let avatar = if let Some(url) = &profile.avatar_url {
                gpui::img(SharedString::from(url.clone()))
                    .size(px(40.))
                    .rounded_full()
                    .into_any_element()
            } else {
                div()
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(t.button_secondary)
                    .child(text(
                        name.chars()
                            .next()
                            .map(|letter| letter.to_uppercase().to_string())
                            .unwrap_or_default(),
                        16.,
                        24.,
                        500.,
                        t.text,
                    ))
                    .into_any_element()
            };
            let control = div()
                .id(("browser-account", index))
                .role(gpui::Role::Button)
                .aria_label(if profile.name.is_some() {
                    format!("Continue as {name}, {email}")
                } else {
                    format!("Continue as {name}")
                })
                .cursor_pointer()
                .relative()
                .w_full()
                .h(px(64.))
                .border_1()
                .border_color(t.border)
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .child(avatar)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(text(name.to_owned(), 14., 20., 500., t.text))
                        .when(profile.name.is_some(), |label| {
                            label.child(text(email.clone(), 13., 20., 400., t.text_description))
                        }),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.command(Command::Recognized(index), window, cx)
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.command(Command::Recognized(index), window, cx);
                        cx.stop_propagation();
                    }
                }));
            let handle = focus::handle(("browser-account", index), 0, window, cx);
            body = body.child(focus::ring(
                control,
                &handle,
                Theme::RADIUS_FULL,
                px(2.),
                cx,
            ));
        }
        body
    }

    fn signin_controls(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut body = div().w_full().flex().flex_col().items_center().gap(px(12.));
        if self.stage == Stage::ApiKey {
            body = self.api_controls(body, window, cx);
        } else if self.stage == Stage::OtherMethods {
            body = body
                .child(self.button(
                    "api-key",
                    "OpenAI API Key",
                    Command::ShowApiKey,
                    Style::Outline,
                    window,
                    cx,
                ))
                .child(self.button(
                    "amazon-bedrock",
                    "Use Amazon Bedrock",
                    Command::ShowBedrock,
                    Style::Outline,
                    window,
                    cx,
                ))
                .child(self.button("other-back", "Back", Command::Back, Style::Link, window, cx));
        } else {
            if self.policy.permits_chatgpt() {
                body = self.chatgpt_controls(body, window, cx);
            }
            if self.policy.permits_api() {
                body = body.child(self.button(
                    "another-way",
                    if !self.options.bedrock
                        && (self.options.explicit_api_key || !self.policy.permits_chatgpt())
                    {
                        "Sign in with an API key"
                    } else {
                        "Sign in another way"
                    },
                    if self.options.bedrock {
                        Command::ShowOtherMethods
                    } else {
                        Command::ShowApiKey
                    },
                    Style::Outline,
                    window,
                    cx,
                ));
            }
            if self.policy.permits_chatgpt() {
                body = body.child(self.button(
                    "sign-up",
                    "Sign up",
                    Command::Start(Provider::SignUp),
                    Style::Link,
                    window,
                    cx,
                ));
            }
        }
        body
    }

    fn chatgpt_controls(&self, mut body: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if let Some((_, provider_name)) = &self.gateway_provider {
            return body.child(self.button(
                "gateway-oauth",
                format!("Sign in with {provider_name}"),
                Command::GatewayOAuth,
                Style::Primary,
                window,
                cx,
            ));
        }
        let t = login_theme(cx);
        body = self.account_choices(body, window, cx);
        let recognized = !self.options.browser_profiles.is_empty();
        if recognized {
            body = body.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .py(px(8.))
                    .child(div().flex_1().h(px(1.)).bg(t.border))
                    .child(text("OR", 12., 16., 500., t.text_description))
                    .child(div().flex_1().h(px(1.)).bg(t.border)),
            );
        }
        body = body.child(self.button(
            "continue",
            if recognized {
                "Use another account"
            } else {
                "Continue to sign in"
            },
            Command::Start(if recognized {
                Provider::OtherAccount
            } else {
                Provider::SignIn
            }),
            if recognized {
                Style::Outline
            } else {
                Style::Primary
            },
            window,
            cx,
        ));
        if self.options.providers {
            body = body
                .child(self.button(
                    "google",
                    "Continue with Google",
                    Command::Start(Provider::Google),
                    Style::Outline,
                    window,
                    cx,
                ))
                .child(self.button(
                    "microsoft",
                    "Continue with Microsoft",
                    Command::Start(Provider::Microsoft),
                    Style::Outline,
                    window,
                    cx,
                ));
        }
        body
    }
}

impl Command {
    fn icon(self, style: Style, color: gpui::Hsla) -> Option<gpui::AnyElement> {
        match self {
            Command::Start(Provider::SignIn) if matches!(style, Style::Primary) => {
                Some(icon("signin-provider", 24., color).into_any_element())
            }
            Command::Start(Provider::Google) => Some(
                gpui::img("icons/signin-google.svg")
                    .size(px(20.))
                    .flex_none()
                    .into_any_element(),
            ),
            Command::Start(Provider::Microsoft) => Some(
                gpui::img("icons/signin-microsoft.svg")
                    .size(px(20.))
                    .flex_none()
                    .into_any_element(),
            ),
            _ => None,
        }
    }
}

impl Style {
    fn color(self, hovered: bool, t: &Theme) -> gpui::Hsla {
        match self {
            Style::Primary => t.text_inverse,
            Style::Link if !hovered => t.text_description,
            Style::Link | Style::Outline => t.text,
        }
    }
}

impl Login {
    fn bedrock_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::BedrockMethod(method) => self.choose_bedrock(method, window, cx),
            Command::BedrockChoices => self.bedrock_choices(cx),
            Command::BedrockSubmit => self.submit_bedrock(cx),
            Command::BedrockBack => self.bedrock_back(cx),
            Command::BedrockRetry => self.bedrock_retry(cx),
            Command::BedrockFinish if !self.pending => cx.emit(LoginEvent::Authenticated),
            _ => {}
        }
    }

    fn hover_button(&mut self, id: &'static str, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(id)
        } else if self.hovered_button == Some(id) {
            None
        } else {
            return;
        };
        if self.hovered_button != next {
            self.hovered_button = next;
            cx.notify();
        }
    }

    fn requirements_view(
        &self,
        frame: Div,
        message: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = login_theme(cx);
        let retry = self.policy != Policy::Loading;
        frame.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .when(!retry, |row| {
                            row.child(icon("sidebar-cmd-spinner", 16., t.text))
                        })
                        .child(text(message, 14., 20., 400., t.text_description)),
                )
                .when(retry, |body| {
                    body.child(self.button(
                        "signin-retry",
                        "Retry",
                        Command::Retry,
                        Style::Outline,
                        window,
                        cx,
                    ))
                }),
        )
    }

    fn browser_pending_view(&self, frame: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = login_theme(cx);
        let controls = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(24.))
            .child(
                self.button(
                    "cancel",
                    "Cancel sign-in",
                    Command::Cancel,
                    Style::Outline,
                    window,
                    cx,
                )
                .h(px(42.))
                .text_color(t.text_description),
            )
            .when(self.options.browser_help, |controls| {
                controls.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(text(
                            "Browser didn't open?",
                            14.,
                            20.,
                            400.,
                            t.text_description,
                        ))
                        .child(
                            self.button(
                                "copy-signin-link",
                                "Copy sign-in link",
                                Command::CopyLink,
                                Style::Link,
                                window,
                                cx,
                            )
                            .when(self.login_url.is_none(), |button| button.opacity(0.5)),
                        ),
                )
            });
        frame.pb(px(48.)).child(
            div()
                .w(px(340.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(32.))
                .child(icon("signin-chatgpt", 40., t.text))
                .child(text(
                    "Continue signing in with your browser",
                    14.,
                    20.,
                    400.,
                    t.text_description,
                ))
                .child(controls),
        )
    }

    fn login_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = login_theme(cx);
        let heading = if self.stage == Stage::OtherMethods {
            "Sign in another way"
        } else if self.gateway_provider.is_some() {
            "Sign in to your organization"
        } else if !self.policy.permits_chatgpt() {
            "Sign in with an API key"
        } else if self.stage == Stage::Welcome && !self.options.browser_profiles.is_empty() {
            "Welcome back"
        } else {
            "Sign in to ChatGPT"
        };
        let logo_focus = focus::handle("play-snake", 0, window, cx);
        let logo = focus::ring(
            div()
                .id("play-snake")
                .role(gpui::Role::Button)
                .aria_label("Play Snake")
                .relative()
                .size(px(36.))
                .cursor_pointer()
                .child(icon("signin-chatgpt", 36., t.text))
                .on_click(
                    cx.listener(|this, _, window, cx| this.command(Command::PlaySnake, window, cx)),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.command(Command::PlaySnake, window, cx);
                        cx.stop_propagation();
                    }
                })),
            &logo_focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.))
            .child(logo)
            .child(
                text(heading, 28., 36., 400., t.text)
                    .w(px(316.))
                    .whitespace_normal()
                    .text_center(),
            )
            .when(
                self.stage == Stage::Welcome && !self.options.browser_profiles.is_empty(),
                |header| {
                    header.child(text(
                        "Choose an account to continue to ChatGPT",
                        14.,
                        20.,
                        400.,
                        t.text_description,
                    ))
                },
            )
    }

    fn api_controls(&self, mut body: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = login_theme(cx);
        let field = self.api_key.clone();
        body = body
            .items_start()
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text("OpenAI API key", 14., 21., 500., t.text))
                    .child(
                        div()
                            .w_full()
                            .capture_action(cx.listener(
                                |this, _: &crate::composer::Submit, _, cx| {
                                    this.submit(cx);
                                    cx.stop_propagation();
                                },
                            ))
                            .child(field),
                    ),
            )
            .when(self.options.api_key_help, |body| {
                body.child(self.button(
                    "create-api-key",
                    "Don’t have an API key? Create one here.",
                    Command::CreateKey,
                    Style::Link,
                    window,
                    cx,
                ))
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(
                        self.button(
                            "api-cancel",
                            "Cancel",
                            Command::Back,
                            Style::Outline,
                            window,
                            cx,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.button(
                            "api-continue",
                            "Continue",
                            Command::Submit,
                            Style::Primary,
                            window,
                            cx,
                        )
                        .flex_1()
                        .when(
                            self.pending || self.api_key.read(cx).field_text().trim().is_empty(),
                            |button| button.opacity(0.4),
                        ),
                    ),
            );
        body
    }
}

#[cfg(test)]
mod gateway_oauth_tests {
    use super::*;
    use std::task::{Context as TaskContext, Poll, Waker};

    #[test]
    fn gateway_oauth_request_races_completion_and_timeout() {
        let mut completed = std::pin::pin!(gateway_oauth_request(
            async { Ok::<_, anyhow::Error>(()) },
            async {},
        ));
        assert!(matches!(
            completed
                .as_mut()
                .poll(&mut TaskContext::from_waker(Waker::noop())),
            Poll::Ready((Ok(()), false))
        ));

        let mut timed_out = std::pin::pin!(gateway_oauth_request(
            std::future::pending::<anyhow::Result<()>>(),
            async {},
        ));
        assert!(matches!(
            timed_out
                .as_mut()
                .poll(&mut TaskContext::from_waker(Waker::noop())),
            Poll::Ready((Err(_), true))
        ));
    }
}
