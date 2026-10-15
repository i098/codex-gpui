//! `codex-gpui`: starts `codex app-server`, opens the main window on [`app::Root`] and sets
//! up the menu bar.

use app::{Root, State};
use bridge::Bridge;
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size,
};
use std::path::PathBuf;

fn main() {
    let state = State::load();
    // Project new threads start in: `$CODEX_GPUI_CWD`, else the launch directory.
    let cwd = std::env::var_os("CODEX_GPUI_CWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let mut cmd = bridge::default_command();
    cmd.current_dir(&cwd);
    let (bridge, events) = Bridge::spawn(cmd).unwrap_or_else(|err| {
        eprintln!("codex-gpui: cannot start codex app-server: {err:#}");
        std::process::exit(1)
    });
    let cwd = cwd.to_string_lossy().into_owned();
    gpui_platform::application()
        .with_assets(theme::Assets)
        .run(move |cx: &mut App| {
            theme::load_fonts(cx);
            cx.set_global(state.theme.theme(cx.window_appearance()));
            app::init(cx);
            cx.set_menus(app::menus());
            cx.on_window_closed(|cx, _| cx.quit()).detach();

            let bounds = Bounds::centered(None, size(px(state.width), px(state.height)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Codex".into()),
                        appears_transparent: true,
                        // Electron `trafficLightPosition`: x 16, y (44 - 14) / 2.
                        traffic_light_position: Some(point(px(16.), px(15.))),
                    }),
                    app_owns_titlebar_drag: true,
                    // `getPrimaryMinimumSize()` of the original main process.
                    window_min_size: Some(size(px(480.), px(600.))),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Root::new(state, bridge, events, cwd, window, cx)),
            )
            .expect("open main window");
            cx.activate(true);
        });
}
