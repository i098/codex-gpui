//! `codex-gpui`: starts `codex app-server`, opens the main window on [`app::Root`] and sets
//! up the menu bar.

use app::{CloseWindow, Minimize, Root, State, Zoom, theme_for};
use bridge::Bridge;
use gpui::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType,
    TitlebarOptions, WindowBounds, WindowOptions, actions, point, px, size,
};
use std::path::PathBuf;
use ui::{
    Copy, Cut, GoBack, GoForward, NewChat, OpenSettings, Paste, Redo, SelectAll, ToggleSidebar,
    Undo,
};

actions!(app, [Quit, Hide, HideOthers, ShowAll, OpenDocumentation]);

/// Menu bar of the original (docs/inventory.md §13), limited to commands with a handler.
fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Codex").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Codex", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Codex", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Chat", NewChat),
            MenuItem::separator(),
            MenuItem::action("Close", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
        Menu::new("Help").items([MenuItem::action("Documentation", OpenDocumentation)]),
    ]
}

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
            cx.set_global(theme_for(cx.window_appearance()));
            cx.bind_keys([
                KeyBinding::new("cmd-,", OpenSettings, None),
                KeyBinding::new("cmd-h", Hide, None),
                KeyBinding::new("alt-cmd-h", HideOthers, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-n", NewChat, None),
                KeyBinding::new("cmd-w", CloseWindow, None),
                KeyBinding::new("cmd-z", Undo, None),
                KeyBinding::new("cmd-shift-z", Redo, None),
                KeyBinding::new("cmd-x", Cut, None),
                KeyBinding::new("cmd-c", Copy, None),
                KeyBinding::new("cmd-v", Paste, None),
                KeyBinding::new("cmd-a", SelectAll, None),
                KeyBinding::new("cmd-shift-s", ToggleSidebar, None),
                KeyBinding::new("cmd-b", ToggleSidebar, None),
                KeyBinding::new("cmd-[", GoBack, None),
                KeyBinding::new("cmd-]", GoForward, None),
                KeyBinding::new("cmd-m", Minimize, None),
            ]);
            ui::composer::bind_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &Hide, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
            cx.on_action(|_: &OpenDocumentation, cx| {
                cx.open_url("https://developers.openai.com/codex/app")
            });
            cx.set_menus(menus());
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
