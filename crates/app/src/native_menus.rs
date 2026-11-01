mod about;
pub(crate) mod context_files;
pub(crate) mod navigation;

use crate::{
    CloseWindow, FindInThread, GoBack, GoForward, Hide, HideOthers, LogOut, Minimize, NewChat,
    NextChat, OpenFolder, OpenSettings, PreviousChat, Quit, ShowAll, ShowKeyboardShortcuts,
    ToggleChangesPanel, ToggleSidebar, Zoom, features,
};
use gpui::{
    App, Context, Div, InteractiveElement as _, KeyBinding, Menu, MenuItem, OsAction,
    SystemMenuType, actions,
};
use ui::{Copy, Cut, DeleteSelection, Paste, PasteAndMatchStyle, Redo, SelectAll, Undo};

actions!(
    native_menus,
    [
        ShowAbout,
        ShowFeedback,
        CheckForUpdates,
        OpenDocumentation,
        OpenChangelog,
        OpenTroubleshooting,
        OpenSystemStatus,
        BringAllToFront,
        ToggleFullScreen,
        ShowSubstitutions,
        SmartQuotes,
        SmartDashes,
        TextReplacement,
        TogglePinnedSummary,
        ToggleNavigationPanel,
        ToggleDebugPanel,
        ToggleBrowserPanel,
    ]
);

pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &ShowAbout, cx| cx.defer(about::open));
    cx.on_action(|_: &ToggleDebugPanel, cx| cx.defer(features::debug::toggle_panel));
    cx.bind_keys([
        KeyBinding::new("cmd-shift-v", PasteAndMatchStyle, None),
        KeyBinding::new("cmd-alt-shift-v", PasteAndMatchStyle, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    cx.on_action(|_: &OpenDocumentation, cx| {
        cx.open_url("https://developers.openai.com/codex/app")
    });
    cx.on_action(|_: &OpenChangelog, cx| {
        cx.open_url("https://developers.openai.com/codex/changelog")
    });
    cx.on_action(|_: &OpenTroubleshooting, cx| {
        cx.open_url("https://developers.openai.com/codex/app/troubleshooting")
    });
    cx.on_action(|_: &OpenSystemStatus, cx| cx.open_url("https://status.openai.com"));
    cx.on_action(|_: &BringAllToFront, cx| bring_all_to_front(cx));
    cx.on_action(|_: &ToggleFullScreen, cx| {
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, _| window.toggle_fullscreen())
                    .ok();
            });
        }
    });
}

pub(crate) fn actions(root: Div, owner: &crate::Root, cx: &mut Context<crate::Root>) -> Div {
    let root = navigation::actions(root, owner, cx);
    let root = if active_thread(owner, cx).is_some() {
        root.on_action(cx.listener(|this, _: &TogglePinnedSummary, _, cx| {
            if let Some(thread) = active_thread(this, cx) {
                thread.update(cx, |view, cx| {
                    view.summary_open = !view.summary_open;
                    cx.notify();
                });
            }
        }))
    } else {
        root
    };
    let root = root.on_action(cx.listener(|this, _: &ShowFeedback, window, cx| {
        this.open_feedback(window, cx);
    }));
    let root = root.on_action(cx.listener(|this, _: &ToggleBrowserPanel, window, cx| {
        if !matches!(this.state.route, crate::Route::Thread(_)) {
            return;
        }
        if let Some((key, _)) = this.browser_tabs.last() {
            let side = crate::SidePanel::Browser(*key);
            if this.side_visible(side) {
                this.tabs
                    .retain(|tab| !matches!(tab, crate::SidePanel::Browser(_)));
                this.side = this.tabs.last().copied();
                this.sync(window, cx);
            } else {
                this.set_side(side, window, cx);
            }
        } else {
            this.open_browser_tab(None, window, cx);
        }
    }));
    if ui::native_text::is_speaking(cx) {
        root.on_action(|_: &ui::StopSpeaking, _, cx| ui::native_text::stop_speaking(cx))
    } else {
        root
    }
}

fn active_thread<'a>(
    root: &'a crate::Root,
    cx: &App,
) -> Option<&'a gpui::Entity<crate::ThreadView>> {
    if root.login.is_some() {
        return None;
    }
    let crate::Route::Thread(id) = root.route() else {
        return None;
    };
    root.thread()
        .filter(|thread| thread.read(cx).id() == id.as_str())
}

fn bring_all_to_front(cx: &mut App) {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::{class, msg_send, runtime::AnyObject};
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![application, arrangeInFront: std::ptr::null::<AnyObject>()];
    }
    #[cfg(not(target_os = "macos"))]
    cx.activate(true);
    #[cfg(target_os = "macos")]
    let _ = cx;
}

struct NativeMenuState {
    sidebar_collapsible: bool,
    browser_available: bool,
}
impl gpui::Global for NativeMenuState {}

pub(crate) fn refresh_menus(cx: &mut App) {
    let (sidebar_collapsible, browser_available) = cx
        .try_global::<NativeMenuState>()
        .map_or((true, false), |state| {
            (state.sidebar_collapsible, state.browser_available)
        });
    let updates = cx
        .try_global::<ui::updater::Updates>()
        .is_some_and(|updates| updates.available && !updates.managed);
    cx.set_menus(menus_for_route(
        sidebar_collapsible,
        updates,
        browser_available,
    ));
}

impl crate::Root {
    /// Settings hides Toggle Sidebar; every other route shows it.
    pub(crate) fn sync_native_menus(&self, window: &gpui::Window, cx: &mut Context<Self>) {
        if cx.active_window().is_some() && !window.is_window_active() {
            return;
        }
        let sidebar_collapsible = self.state.route != crate::Route::Settings;
        let browser_available = matches!(self.state.route, crate::Route::Thread(_));
        if cx.try_global::<NativeMenuState>().is_some_and(|state| {
            state.sidebar_collapsible == sidebar_collapsible
                && state.browser_available == browser_available
        }) {
            return;
        }
        cx.set_global(NativeMenuState {
            sidebar_collapsible,
            browser_available,
        });
        refresh_menus(cx);
    }
}

/// Builds the native menu items backed by the app's registered actions.
pub(crate) fn menus() -> Vec<Menu> {
    menus_for_route(true, false, false)
}

fn menus_for_route(sidebar_collapsible: bool, updates: bool, browser_available: bool) -> Vec<Menu> {
    let browser = browser_available.then(|| {
        MenuItem::submenu(
            Menu::new("Browser")
                .items([MenuItem::action("Toggle browser panel", ToggleBrowserPanel)]),
        )
    });
    vec![
        Menu::new("Codex").items([
            MenuItem::action("About Codex", ShowAbout),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Check for Updates…", CheckForUpdates).hidden(!updates),
            MenuItem::action("Log Out", LogOut),
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
            MenuItem::action("New Window", crate::windows::NewWindow),
            MenuItem::action("New Chat", NewChat),
            MenuItem::action("New Temporary Chat", features::shortcuts::NewTemporaryChat),
            MenuItem::action("New Project…", ui::screens::new_project::NewProject),
            MenuItem::action(
                "New standalone chat",
                features::shortcuts::NewStandaloneChat,
            ),
            MenuItem::separator(),
            MenuItem::action("Open Folder…", OpenFolder),
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
            MenuItem::action("Paste and Match Style", PasteAndMatchStyle),
            MenuItem::action("Delete", DeleteSelection),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::submenu(Menu::new("Substitutions").items([
                MenuItem::os_action(
                    "Show Substitutions",
                    ShowSubstitutions,
                    OsAction::ShowSubstitutions,
                ),
                MenuItem::separator(),
                MenuItem::os_action("Smart Quotes", SmartQuotes, OsAction::SmartQuotes),
                MenuItem::os_action("Smart Dashes", SmartDashes, OsAction::SmartDashes),
                MenuItem::os_action(
                    "Text Replacement",
                    TextReplacement,
                    OsAction::TextReplacement,
                ),
            ])),
            MenuItem::submenu(Menu::new("Speech").items([
                MenuItem::action("Start Speaking", ui::StartSpeaking),
                MenuItem::action("Stop Speaking", ui::StopSpeaking),
            ])),
        ]),
        Menu::new("View").items(
            [
                MenuItem::action("Toggle Sidebar", ToggleSidebar).hidden(!sidebar_collapsible),
                MenuItem::action(
                    "Toggle Bottom Panel",
                    features::shortcuts::ToggleBottomPanel,
                ),
                MenuItem::action("Toggle Pinned Summary", TogglePinnedSummary),
                MenuItem::action(
                    "Open Terminal",
                    features::shortcuts::OpenInteractiveTerminal,
                ),
                MenuItem::action("Toggle navigation panel", ToggleNavigationPanel),
                MenuItem::action("Full view", ui::ShowWorkspaceTabView),
                MenuItem::action("Toggle Changes Panel", ToggleChangesPanel),
            ]
            .into_iter()
            .chain(browser)
            .chain([
                MenuItem::separator(),
                MenuItem::action("Find", FindInThread),
                MenuItem::separator(),
                MenuItem::action("Previous Chat", PreviousChat),
                MenuItem::action("Next Chat", NextChat),
                MenuItem::action("Back", GoBack),
                MenuItem::action("Forward", GoForward),
                MenuItem::separator(),
                MenuItem::action("Zoom In", features::shortcuts::ZoomIn),
                MenuItem::action("Zoom Out", features::shortcuts::ZoomOut),
                MenuItem::action("Actual Size", features::shortcuts::ActualSize),
            ]),
        ),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::separator(),
            MenuItem::action("Bring All to Front", BringAllToFront),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Documentation", OpenDocumentation),
            MenuItem::action("Keyboard Shortcuts", ShowKeyboardShortcuts),
            MenuItem::action("What's New", OpenChangelog),
            MenuItem::separator(),
            MenuItem::action(
                "Start Performance Trace",
                features::debug::ToggleTraceRecording,
            ),
            MenuItem::action("Show Local Trace", ToggleDebugPanel),
            MenuItem::separator(),
            MenuItem::action("Troubleshooting", OpenTroubleshooting),
            MenuItem::action("System Status", OpenSystemStatus),
            MenuItem::action("Send Feedback", ShowFeedback),
        ]),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_menu_is_available_only_on_thread_routes() {
        let has_browser_menu = |browser_available| {
            menus_for_route(true, false, browser_available)
                .into_iter()
                .find(|menu| menu.name == "View")
                .unwrap()
                .items
                .iter()
                .any(|item| matches!(item, MenuItem::Submenu(menu) if menu.name == "Browser"))
        };

        assert!(!has_browser_menu(false));
        assert!(has_browser_menu(true));
    }
}
