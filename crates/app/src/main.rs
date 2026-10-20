//! Starts the backend and opens the primary native window.

use app::State;
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
        app::qa::monitor(handle, cx);
        if !app::qa::ENABLED {
            cx.activate(true);
        }
    });
}
