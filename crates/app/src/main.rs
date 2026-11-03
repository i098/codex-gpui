//! Starts the backend and opens the primary native window.

use app::State;
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use std::path::PathBuf;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        app::crash::save(&app::crash::report(info));
        eprintln!("{info}");
        std::process::exit(101);
    }));
    if std::env::args().nth(1).as_deref() == Some("--qa-build-info") {
        println!("{}", app::qa::build_info());
        return;
    }
    app::qa::prepare().expect("prepare isolated QA profile");
    if std::env::args().nth(1).as_deref() == Some("--setup-computer-use") {
        if let Err(error) = bridge::setup_computer_use() {
            eprintln!("codex-gpui: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let launch_path = std::env::args().nth(1);
    let mut state = State::load();
    if let Some(route) = launch_path.as_deref().and_then(|path| {
        app::Route::remote_path(path)
            .or_else(|| app::Route::pull_request_path(path))
            .or_else(|| app::onboarding::route(path))
    }) {
        state.route = route;
    }
    if launch_path.as_deref() == Some("/diff") {
        let path = std::env::args_os().nth(2).unwrap_or_else(|| {
            eprintln!("usage: codex-gpui /diff <patch-file>");
            std::process::exit(2)
        });
        let patch = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            eprintln!("codex-gpui: cannot read diff: {error}");
            std::process::exit(1)
        });
        let cwd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        gpui_platform::application()
            .with_assets(theme::Assets)
            .run(move |cx: &mut App| {
                theme::load_fonts(cx);
                cx.set_global(state.theme.theme(cx.window_appearance()));
                app::init(cx);
                cx.on_action(|_: &app::CloseWindow, cx| cx.quit());
                let settings = state.path.as_ref().map_or_else(
                    || Ok(app::host_settings::Settings::default()),
                    |path| {
                        app::host_settings::Settings::load(
                            path.with_file_name("host-settings.json"),
                        )
                    },
                );
                match settings {
                    Ok(settings) => cx.set_global(settings),
                    Err(error) => eprintln!("codex-gpui: cannot load host settings: {error:#}"),
                }
                cx.on_window_closed(|cx, _| cx.quit()).detach();
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(state.width), px(state.height)),
                            cx,
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Codex".into()),
                            appears_transparent: false,
                            traffic_light_position: Some(point(px(16.), px(15.))),
                        }),
                        window_min_size: Some(size(px(480.), px(600.))),
                        ..Default::default()
                    },
                    |_, cx| {
                        let panel = cx
                            .new(|_| ui::screens::diff::panel::DiffPanel::from_patch(&cwd, &patch));
                        app::diff_content::bind_standalone_diff(
                            &panel,
                            std::sync::Arc::new(app::host::LocalHost::from_env()),
                            cx,
                        );
                        panel
                    },
                )
                .expect("open diff window");
                cx.activate(true);
            });
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let receiver = match app::single_instance::start(std::env::args().skip(1).collect()) {
        Ok(Some(receiver)) => receiver,
        Ok(None) => return,
        Err(error) => {
            eprintln!("codex-gpui: cannot initialize single-instance forwarding: {error:#}");
            std::process::exit(1);
        }
    };
    // Project new threads start in: `$CODEX_GPUI_CWD`, else the launch directory.
    let cwd = std::env::var_os("CODEX_GPUI_CWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let application = gpui_platform::application()
        .with_assets(theme::Assets)
        .with_http_client(std::sync::Arc::new(ImageHttpClient(
            ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(30)))
                .build()
                .into(),
        )));
    // Route `codex://` deep links to the primary window and registered HTTP(S) links to
    // the in-app browser.
    application.on_open_urls(|urls, cx| {
        for url in urls.iter().rev() {
            if app::deep_link::parse(url).is_some() {
                app::deep_link::open(url, cx);
                break;
            }
            if let Some(url) = web_url(url) {
                open_browser_url(url, cx);
                break;
            }
        }
    });
    application.on_reopen(|cx| {
        if let Err(error) = app::windows::reopen(cx) {
            eprintln!("codex-gpui: cannot reopen the window: {error:#}");
        }
    });
    application.run(move |cx| {
        theme::load_fonts(cx);
        cx.set_global(state.theme.theme(cx.window_appearance()));
        app::init(cx);
        cx.set_menus(app::menus());
        // Launch argv deep links (the original's `queueProcessArgs`): the last codex://
        // argument navigates the new window once it exists.
        let argv_link = std::env::args()
            .filter_map(|arg| app::deep_link::parse(&arg))
            .next_back();
        let argv_web_url = std::env::args().filter_map(|arg| web_url(&arg)).next_back();
        let handle = app::windows::open_primary(state, cwd, cx).unwrap_or_else(|error| {
            eprintln!("codex-gpui: cannot open the primary window: {error:#}");
            std::process::exit(1)
        });
        if let Some(link) = argv_link {
            let _ = handle.update(cx, |root, window, cx| {
                root.open_deep_link(&link, window, cx)
            });
        } else if let Some(url) = argv_web_url {
            open_browser_url(url, cx);
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        cx.spawn(async move |cx| {
            while let Ok(arguments) = receiver.recv().await {
                cx.update(|cx| {
                    if let Some(url) = arguments
                        .iter()
                        .rev()
                        .find(|argument| app::deep_link::parse(argument).is_some())
                    {
                        app::deep_link::open(url, cx);
                    } else if !arguments
                        .iter()
                        .rev()
                        .any(|path| app::deep_link::open_path(path, cx))
                        && let Err(error) = app::windows::reopen(cx)
                    {
                        eprintln!("codex-gpui: cannot focus the primary window: {error:#}");
                    }
                });
            }
        })
        .detach();
        app::screenshot::register(cx);
        app::qa::monitor(handle, cx);
        if let Some(path) = std::env::args()
            .nth(1)
            .filter(|path| path.starts_with("/hotkey-window") || path == "/spotlight")
        {
            match app::hotkey::Target::parse(&path)
                .and_then(|target| app::hotkey::open(handle, target, cx))
            {
                Ok(popout) => app::qa::monitor(popout, cx),
                Err(error) => eprintln!("codex-gpui: {error:#}"),
            }
        }
        if !app::qa::ENABLED {
            cx.activate(true);
        }
    });
}

fn web_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

fn open_browser_url(url: url::Url, cx: &mut App) {
    if app::windows::primary(cx).is_none()
        && let Err(error) = app::windows::reopen(cx)
    {
        eprintln!("codex-gpui: cannot reopen the window for {url}: {error:#}");
        return;
    }
    let Some(handle) = app::windows::primary(cx).and_then(|window| window.downcast::<app::Root>())
    else {
        return;
    };
    let in_thread = handle
        .read(cx)
        .is_ok_and(|root| matches!(root.route(), app::Route::Thread(_)));
    if in_thread {
        if let Err(error) = handle.update(cx, |root, window, cx| {
            root.open_browser_tab(Some(url), window, cx);
        }) {
            eprintln!("codex-gpui: cannot open browser tab: {error:#}");
        }
        if let Err(error) =
            app::windows::show(handle.into(), app::windows::Presentation::Foreground, cx)
        {
            eprintln!("codex-gpui: cannot show browser window: {error:#}");
        }
    } else {
        let title = url.host_str().unwrap_or("Browser").to_owned();
        if let Err(error) = app::windows::open(
            app::windows::Spec::new(app::windows::Kind::Page, title),
            cx,
            move |window, cx| cx.new(|cx| ui::browser::BrowserTab::new(Some(url), window, cx)),
        ) {
            eprintln!("codex-gpui: cannot open browser window: {error:#}");
        }
    }
}

/// Fetches GPUI `img(url)` sources: link favicons and account avatars, as the original's
/// `<img src>` does.
struct ImageHttpClient(ureq::Agent);

impl http_client::HttpClient for ImageHttpClient {
    fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&http_client::Url> {
        None
    }

    fn send(
        &self,
        request: http_client::Request<http_client::AsyncBody>,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = anyhow::Result<http_client::Response<http_client::AsyncBody>>>
                + Send,
        >,
    > {
        // shortcut: GET only, the request body is dropped; upgrade when a caller sends one.
        let agent = self.0.clone();
        let uri = request.uri().to_string();
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let mut response = agent.get(&uri).call()?;
                let body = response.body_mut().read_to_vec()?;
                Ok(http_client::Response::builder()
                    .status(response.status().as_u16())
                    .body(body.into())?)
            })();
            let _ = sender.send_blocking(result);
        });
        Box::pin(async move { receiver.recv().await? })
    }
}

#[cfg(test)]
mod tests {
    use super::web_url;

    #[test]
    fn browser_links_accept_only_http_and_https() {
        assert_eq!(
            web_url("https://example.com/path?q=1")
                .as_ref()
                .map(url::Url::as_str),
            Some("https://example.com/path?q=1")
        );
        assert_eq!(
            web_url("http://localhost:8080/")
                .as_ref()
                .map(url::Url::as_str),
            Some("http://localhost:8080/")
        );
        assert!(web_url("codex://threads/new").is_none());
        assert!(web_url("file:///tmp/page.html").is_none());
    }
}
