use crate::*;
use ui::screens::recovery::{self, Recovery, State as RecoveryState};

pub(crate) struct Connection {
    pub view: Entity<Recovery>,
    pub generation: u64,
}

impl Connection {
    pub fn new(cx: &mut Context<Root>) -> Self {
        Self { view: cx.new(|cx| Recovery::new(RecoveryState::Loading, cx)), generation: 0 }
    }
}

pub(crate) fn events(events: Receiver<Event>, generation: u64, cx: &mut Context<Root>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Ok(event) = events.recv().await {
            if this.update(cx, |root, cx| {
                if root.connection.generation == generation { root.on_event(event, cx); }
            }).is_err() { return; }
        }
        let _ = this.update(cx, |root, cx| {
            if root.connection.generation == generation {
                root.ready = false;
                root.connection.view.update(cx, |view, cx| view.set_state(RecoveryState::Crashed {
                    configuration: false, details: String::new(),
                }, cx));
                cx.notify();
            }
        });
    })
}

pub(crate) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|root, _: &recovery::Retry, window, cx| {
        if let Route::Thread(id) = &root.state.route {
            root.load_thread(id.clone(), window, cx);
        }
    })).on_action(cx.listener(|root, _: &recovery::ClosePanel, window, cx| {
        if let Some(side) = root.side { root.close_side(side, window, cx); }
    }))
}

pub(crate) fn render(root: &Root, cx: &mut Context<Root>) -> Option<AnyElement> {
    if root.ready && !matches!(root.connection.view.read(cx).state(), RecoveryState::RouteError(_)) { return None; }
    Some(div().size_full().relative().track_focus(&root.focus)
        .on_action(cx.listener(|root, _: &recovery::Retry, window, cx| {
            if matches!(root.connection.view.read(cx).state(), RecoveryState::RouteError(_)) {
                root.ready = true;
                root.navigate(Route::Home, window, cx);
            } else {
                root.retry_connection(window, cx);
            }
        }))
        .on_action(|_: &recovery::ReviewConfiguration, _, cx| open_configuration(cx))
        .on_action(cx.listener(|root, _: &CloseWindow, window, cx| root.close_window(window, cx)))
        .on_action(|_: &Minimize, window, _| window.minimize_window())
        .on_action(|_: &Zoom, window, _| window.zoom_window())
        .child(root.connection.view.clone())
        .child(root.toaster.clone())
        .children(root.dialog.clone())
        .into_any_element())
}

impl Root {
    pub fn recovery_state<'a>(&'a self, cx: &'a App) -> Option<&'a RecoveryState> {
        (!self.ready || matches!(self.connection.view.read(cx).state(), RecoveryState::RouteError(_)))
            .then(|| self.connection.view.read(cx).state())
    }

    pub(crate) fn clear_route_error(&mut self, cx: &mut Context<Self>) {
        if matches!(self.connection.view.read(cx).state(), RecoveryState::RouteError(_)) {
            self.connection.view.update(cx, |view, cx| view.set_state(RecoveryState::Loading, cx));
        }
    }

    pub(crate) fn connection_failed(&mut self, error: anyhow::Error, generation: u64, cx: &mut Context<Self>) {
        if self.connection.generation != generation { return; }
        self.ready = false;
        let state = failure(&error);
        self.connection.view.update(cx, |view, cx| view.set_state(state, cx));
        cx.notify();
    }

    fn retry_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.connection.view.read(cx).state(), RecoveryState::Loading) { return; }
        self.connection.generation += 1;
        let generation = self.connection.generation;
        self.ready = false;
        self.connection.view.update(cx, |view, cx| view.set_state(RecoveryState::Loading, cx));
        let cwd = self.cwd.clone();
        let spawn = cx.background_spawn(async move { windows::connect(std::path::Path::new(&cwd)) });
        cx.spawn_in(window, async move |this, cx| {
            let result = spawn.await;
            let _ = this.update_in(cx, |root, window, cx| match result {
                Ok((bridge, receiver)) => {
                    let bridge = Arc::new(bridge);
                    root.bridge = bridge.clone();
                    cx.set_global(AppServer(bridge.clone()));
                    root.set_host(Arc::new(host::CloudHost::from_env(host::app_server_credentials(bridge.clone())).with_bridge(bridge)), cx);
                    root._events = events(receiver, generation, cx);
                    root.loaded.clear();
                    root.approvals.clear();
                    root.show_approval(cx);
                    root.connect(window, cx);
                }
                Err(error) => root.connection_failed(error, generation, cx),
            });
        }).detach();
        cx.notify();
    }

}

fn open_configuration(cx: &mut App) {
    let home = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
    });
    match url::Url::from_file_path(home.join("config.toml")) {
        Ok(url) => cx.open_url(url.as_str()),
        Err(()) => eprintln!("codex-gpui: cannot resolve the configuration file"),
    }
}

struct StartupError {
    state: State,
    cwd: String,
    view: Entity<Recovery>,
}

impl Render for StartupError {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
            .on_action(cx.listener(|this, _: &recovery::Retry, window, cx| {
                match windows::open_primary(this.state.clone(), this.cwd.clone(), cx) {
                    Ok(handle) => {
                        qa::monitor(handle, cx);
                        window.remove_window();
                    }
                    Err(error) => this.view.update(cx, |view, cx| view.set_state(failure(&error), cx)),
                }
            }))
            .on_action(|_: &recovery::ReviewConfiguration, _, cx| open_configuration(cx))
            .on_action(|_: &CloseWindow, window, cx| windows::close(window, cx))
            .child(self.view.clone())
    }
}

pub fn open_startup_error(state: State, cwd: String, error: anyhow::Error, cx: &mut App) -> Result<gpui::AnyWindowHandle> {
    let mut spec = windows::Spec::new(windows::Kind::Primary, "Codex");
    spec.bounds = Some(windows::primary_bounds(&state, cx));
    windows::open(spec, cx, |_, cx| cx.new(|cx| StartupError {
        state, cwd, view: cx.new(|cx| Recovery::new(failure(&error), cx)),
    })).map(Into::into)
}

fn failure(error: &anyhow::Error) -> RecoveryState {
    if let Some(error) = error.downcast_ref::<bridge::RpcError>() {
        let data = &error.data;
        let message = match data["code"].as_str() {
            Some("login-required") => Some("You are currently logged out.".to_owned()),
            Some("remote-codex-not-found") => Some("Codex CLI is not installed on this remote machine".to_owned()),
            Some("restart-required") => Some(match (data["currentVersion"].as_str(), data["installedVersion"].as_str()) {
                (Some(current), Some(installed)) => format!("Restart now to update to {installed}. Currently running {current}"),
                _ => "Something went wrong connecting to the Codex CLI. Try restarting".into(),
            }),
            Some("update-required") => match (data["minRequiredVersion"].as_str(), data["currentVersion"].as_str()) {
                (Some(minimum), Some(current)) => Some(format!("Codex CLI on this environment is out of date. Update to {minimum} or newer. Current version: {current}")),
                _ => None,
            },
            Some("connection-failed") => Some(error.message.clone()),
            _ => None,
        };
        if let Some(message) = message { return RecoveryState::ConnectionError(message); }
    }
    let details = error.to_string();
    let lower = details.to_ascii_lowercase();
    let configuration = error.downcast_ref::<bridge::RpcError>().is_some_and(|error| error.data["reason"] == "configLoad")
        || lower.starts_with("failed to load configuration")
        || lower.starts_with("error loading config")
        || lower.starts_with("error loading default config")
        || lower.split("error:").skip(1).any(|message| {
            let message = message.trim_start();
            message.starts_with("error loading config") || message.starts_with("error loading default config")
        });
    RecoveryState::Crashed { configuration, details }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_failures_keep_server_version_and_login_state() {
        let error = bridge::RpcError { code: -32000, message: "unavailable".into(), data: json!({"code":"update-required", "currentVersion":"1.0", "minRequiredVersion":"2.0"}) };
        assert_eq!(failure(&error.into()), RecoveryState::ConnectionError("Codex CLI on this environment is out of date. Update to 2.0 or newer. Current version: 1.0".into()));
        let error = bridge::RpcError { code: -32000, message: "unavailable".into(), data: json!({"code":"login-required"}) };
        assert_eq!(failure(&error.into()), RecoveryState::ConnectionError("You are currently logged out.".into()));
    }
}
