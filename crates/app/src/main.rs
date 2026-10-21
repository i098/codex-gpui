//! Starts the backend and opens the primary native window.

use app::State;
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use std::path::PathBuf;

fn main() {
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
    let state = State::load();
    let launch_path = std::env::args().nth(1);
    if std::env::args().nth(1).as_deref() == Some("/diff") {
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
                        cx.new(|_| ui::screens::diff::panel::DiffPanel::from_patch(&cwd, &patch))
                    },
                )
                .expect("open diff window");
                cx.activate(true);
            });
        return;
    }
    // Project new threads start in: `$CODEX_GPUI_CWD`, else the launch directory.
    let cwd = std::env::var_os("CODEX_GPUI_CWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let application = gpui_platform::application().with_assets(theme::Assets);
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
        let handle = app::windows::open_primary(state, cwd, cx).unwrap_or_else(|error| {
            eprintln!("codex-gpui: cannot open the primary window: {error:#}");
            std::process::exit(1)
        });
        if let Some(path) = &launch_path
            && let Err(error) = handle.update(cx, |root, window, cx| {
                root.open_browser_path(path, window, cx)
            })
        {
            eprintln!("codex-gpui: cannot open browser route: {error:#}");
        }
        app::qa::monitor(handle, cx);
        if !app::qa::ENABLED {
            cx.activate(true);
        }
    });
}
