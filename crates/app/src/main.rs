//! `codex-gpui`: the main window, menu bar, routes and persisted window state.

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    KeyBinding, Menu, MenuItem, OsAction, ParentElement as _, Render, Styled as _, SystemMenuType,
    TitlebarOptions, Window, WindowAppearance, WindowBounds, WindowOptions, actions, div, point,
    px, size,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use theme::Theme;
use ui::{
    Copy, Cut, GoBack, GoForward, NewChat, OpenSettings, Paste, Redo, SelectAll, Shell,
    ToggleSidebar, Undo,
};

actions!(
    app,
    [
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        CloseWindow,
        Minimize,
        Zoom,
        OpenDocumentation
    ]
);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
enum Route {
    #[default]
    Home,
    Thread(String),
    Settings,
}

/// `~/Library/Application Support/codex-gpui/state.json`
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct State {
    width: f32,
    height: f32,
    sidebar_open: bool,
    route: Route,
}

impl Default for State {
    fn default() -> Self {
        Self {
            width: 1280.,
            height: 832.,
            sidebar_open: true,
            route: Route::Home,
        }
    }
}

impl State {
    fn path() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/codex-gpui/state.json")
    }

    fn load() -> Self {
        fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        let result = fs::create_dir_all(path.parent().expect("state path has a parent"))
            .and_then(|()| fs::write(&path, serde_json::to_vec_pretty(self)?));
        if let Err(err) = result {
            eprintln!("codex-gpui: cannot save {}: {err}", path.display());
        }
    }
}

fn theme_for(appearance: WindowAppearance) -> Theme {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
        WindowAppearance::Light | WindowAppearance::VibrantLight => Theme::light(),
    }
}

struct Root {
    route: Route,
    back: Vec<Route>,
    forward: Vec<Route>,
    shell: Entity<Shell>,
}

impl Root {
    fn new(state: &State, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|_| ui::EmptySidebar).into();
        let main = cx.new(|_| ui::EmptyMain).into();
        let sidebar_open = state.sidebar_open;
        let shell = cx.new(|_| {
            let mut shell = Shell::new(sidebar, main);
            shell.sidebar_open = sidebar_open;
            shell
        });
        cx.observe_in(&shell, window, |this, _, window, cx| this.save(window, cx))
            .detach();
        cx.observe_window_bounds(window, |this, window, cx| this.save(window, cx))
            .detach();
        cx.observe_window_appearance(window, |_, window, cx| {
            cx.set_global(theme_for(window.appearance()));
            cx.notify();
        })
        .detach();
        let mut root = Self {
            route: state.route.clone(),
            back: Vec::new(),
            forward: Vec::new(),
            shell,
        };
        root.sync(window, cx);
        root
    }

    fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route != self.route {
            self.back.push(std::mem::replace(&mut self.route, route));
            self.forward.clear();
            self.sync(window, cx);
        }
    }

    fn step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        if let Some(route) = from.pop() {
            to.push(std::mem::replace(&mut self.route, route));
            self.sync(window, cx);
        }
    }

    /// Pushes the route and history into the shell and persists the state.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // ponytail: every route shows an empty main panel until the screen slices land.
        let main = cx.new(|_| ui::EmptyMain).into();
        let (home, settings, can_back, can_forward) = (
            self.route == Route::Home,
            self.route == Route::Settings,
            !self.back.is_empty(),
            !self.forward.is_empty(),
        );
        self.shell.update(cx, |shell, cx| {
            shell.selected_rail = home.then_some("home");
            shell.chat_chrome = !settings;
            shell.new_tab = !settings;
            shell.can_go_back = can_back;
            shell.can_go_forward = can_forward;
            shell.set_main(main, cx);
        });
        self.save(window, cx);
    }

    fn save(&self, window: &Window, cx: &App) {
        let size = window.window_bounds().get_bounds().size;
        State {
            width: size.width.into(),
            height: size.height.into(),
            sidebar_open: self.shell.read(cx).sidebar_open,
            route: self.route.clone(),
        }
        .save();
    }
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(
                cx.listener(|this, _: &NewChat, window, cx| this.navigate(Route::Home, window, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.navigate(Route::Settings, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.step(false, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.shell.update(cx, |shell, cx| shell.toggle_sidebar(cx))
            }))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .child(self.shell.clone())
    }
}

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
                |window, cx| cx.new(|cx| Root::new(&state, window, cx)),
            )
            .expect("open main window");
            cx.activate(true);
        });
}
