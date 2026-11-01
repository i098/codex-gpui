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
use serde_json::Value;
use ui::{Copy, Cut, DeleteSelection, Paste, PasteAndMatchStyle, Redo, SelectAll, Undo};

mod locale {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/native-menu-locales/mod.rs"
    ));
}

fn messages(locale: &str) -> Option<Value> {
    locale::messages(locale).and_then(|json| serde_json::from_str(json).ok())
}

fn localized(messages: Option<&Value>, key: &str, fallback: &str) -> String {
    messages
        .and_then(|messages| messages.get(key))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .replace("{appName}", "Codex")
}

fn effective_locale(locale: &str) -> String {
    let normalized = locale.replace('_', "-");
    if locale::messages(&normalized).is_some() {
        return normalized;
    }
    let language = normalized.split('-').next().unwrap_or_default();
    locale::SUPPORTED
        .iter()
        .find(|supported| supported.split('-').next() == Some(language))
        .copied()
        .unwrap_or_default()
        .to_owned()
}

fn system_locale() -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|locale| !locale.is_empty()))
        .unwrap_or_default();
    effective_locale(locale.split(['.', '@']).next().unwrap_or_default())
}

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
        ToggleBrowserPanel,
    ]
);

pub(crate) fn init(cx: &mut App) {
    cx.on_action(|_: &ShowAbout, cx| cx.defer(about::open));
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
    locale: String,
    browser_available: bool,
}
impl gpui::Global for NativeMenuState {}

pub(crate) fn refresh_menus(cx: &mut App) {
    let state = cx.try_global::<NativeMenuState>();
    let sidebar_collapsible = state.is_none_or(|state| state.sidebar_collapsible);
    let browser_available = state.is_some_and(|state| state.browser_available);
    let locale = state.map_or_else(system_locale, |state| state.locale.clone());
    let updates = cx
        .try_global::<ui::updater::Updates>()
        .is_some_and(|updates| updates.available && !updates.managed);
    cx.set_menus(menus_for_route(
        sidebar_collapsible,
        updates,
        browser_available,
        &locale,
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
        let locale = self.config["desktop"]["localeOverride"]
            .as_str()
            .map(effective_locale)
            .filter(|locale| !locale.is_empty())
            .unwrap_or_else(system_locale);
        if cx.try_global::<NativeMenuState>().is_some_and(|state| {
            state.sidebar_collapsible == sidebar_collapsible
                && state.browser_available == browser_available
                && state.locale == locale
        }) {
            return;
        }
        cx.set_global(NativeMenuState {
            sidebar_collapsible,
            browser_available,
            locale,
        });
        refresh_menus(cx);
    }

    pub(crate) fn refresh_native_menu_locale(&self, cx: &mut Context<Self>) {
        let sidebar_collapsible = cx
            .try_global::<NativeMenuState>()
            .is_none_or(|state| state.sidebar_collapsible);
        let browser_available = cx
            .try_global::<NativeMenuState>()
            .is_some_and(|state| state.browser_available);
        let locale = self.config["desktop"]["localeOverride"]
            .as_str()
            .map(effective_locale)
            .filter(|locale| !locale.is_empty())
            .unwrap_or_else(system_locale);
        cx.set_global(NativeMenuState {
            sidebar_collapsible,
            browser_available,
            locale,
        });
        refresh_menus(cx);
    }
}

/// Builds the native menu items backed by the app's registered actions.
pub(crate) fn menus() -> Vec<Menu> {
    menus_for_route(true, false, false, &system_locale())
}

fn menus_for_route(
    sidebar_collapsible: bool,
    updates: bool,
    browser_available: bool,
    locale: &str,
) -> Vec<Menu> {
    let messages = messages(locale);
    let label = |key: &str, fallback: &str| localized(messages.as_ref(), key, fallback);
    let browser = browser_available.then(|| {
        MenuItem::submenu(
            Menu::new("Browser")
                .items([MenuItem::action("Toggle browser panel", ToggleBrowserPanel)]),
        )
    });
    vec![
        Menu::new(label("windowsMenuBar.app", "Codex")).items([
            MenuItem::action(
                label("codex.aboutDialog.title", "About {appName}"),
                ShowAbout,
            ),
            MenuItem::separator(),
            MenuItem::action(label("codex.command.settings", "Settings…"), OpenSettings),
            MenuItem::action(
                label("electron.appMenu.app.checkForUpdates", "Check for Updates…"),
                CheckForUpdates,
            )
            .hidden(!updates),
            MenuItem::action(label("codex.command.logOut", "Log Out"), LogOut),
            MenuItem::separator(),
            MenuItem::os_submenu(
                label("electron.appMenu.app.services", "Services"),
                SystemMenuType::Services,
            ),
            MenuItem::separator(),
            MenuItem::action(label("electron.appMenu.app.hide", "Hide Codex"), Hide),
            MenuItem::action(
                label("electron.appMenu.app.hideOthers", "Hide Others"),
                HideOthers,
            ),
            MenuItem::action(label("electron.appMenu.app.showAll", "Show All"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(label("electron.appMenu.app.quit", "Quit Codex"), Quit),
        ]),
        Menu::new(label("windowsMenuBar.file", "File")).items([
            MenuItem::action("New Window", crate::windows::NewWindow),
            MenuItem::action(label("codex.command.newThread", "New Chat"), NewChat),
            MenuItem::action(
                label("codex.command.temporaryChat", "New Temporary Chat"),
                features::shortcuts::NewTemporaryChat,
            ),
            MenuItem::action("New Project…", ui::screens::new_project::NewProject),
            MenuItem::action(
                label("codex.command.newProjectlessTask", "New standalone chat"),
                features::shortcuts::NewStandaloneChat,
            ),
            MenuItem::separator(),
            MenuItem::action(
                label("codex.command.openFolder", "Open Folder…"),
                OpenFolder,
            ),
            MenuItem::separator(),
            MenuItem::action(
                label("codex.commandMenuTitle.closeWindow", "Close"),
                CloseWindow,
            ),
        ]),
        Menu::new(label("windowsMenuBar.edit", "Edit")).items([
            MenuItem::os_action(
                label("electron.appMenu.edit.undo", "Undo"),
                Undo,
                OsAction::Undo,
            ),
            MenuItem::os_action(
                label("electron.appMenu.edit.redo", "Redo"),
                Redo,
                OsAction::Redo,
            ),
            MenuItem::separator(),
            MenuItem::os_action(
                label("electron.appMenu.edit.cut", "Cut"),
                Cut,
                OsAction::Cut,
            ),
            MenuItem::os_action(
                label("electron.appMenu.edit.copy", "Copy"),
                Copy,
                OsAction::Copy,
            ),
            MenuItem::os_action(
                label("electron.appMenu.edit.paste", "Paste"),
                Paste,
                OsAction::Paste,
            ),
            MenuItem::action(
                label(
                    "electron.appMenu.edit.pasteAndMatchStyle",
                    "Paste and Match Style",
                ),
                PasteAndMatchStyle,
            ),
            MenuItem::action(
                label("electron.appMenu.edit.delete", "Delete"),
                DeleteSelection,
            ),
            MenuItem::separator(),
            MenuItem::os_action(
                label("electron.appMenu.edit.selectAll", "Select All"),
                SelectAll,
                OsAction::SelectAll,
            ),
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::submenu(
                Menu::new(label(
                    "electron.appMenu.edit.substitutions",
                    "Substitutions",
                ))
                .items([
                    MenuItem::os_action(
                        label(
                            "electron.appMenu.edit.showSubstitutions",
                            "Show Substitutions",
                        ),
                        ShowSubstitutions,
                        OsAction::ShowSubstitutions,
                    ),
                    MenuItem::separator(),
                    MenuItem::os_action(
                        label("electron.appMenu.edit.smartQuotes", "Smart Quotes"),
                        SmartQuotes,
                        OsAction::SmartQuotes,
                    ),
                    MenuItem::os_action(
                        label("electron.appMenu.edit.smartDashes", "Smart Dashes"),
                        SmartDashes,
                        OsAction::SmartDashes,
                    ),
                    MenuItem::os_action(
                        label("electron.appMenu.edit.textReplacement", "Text Replacement"),
                        TextReplacement,
                        OsAction::TextReplacement,
                    ),
                ]),
            ),
            MenuItem::submenu(
                Menu::new(label("electron.appMenu.edit.speech", "Speech")).items([
                    MenuItem::action(
                        label("electron.appMenu.edit.startSpeaking", "Start Speaking"),
                        ui::StartSpeaking,
                    ),
                    MenuItem::action(
                        label("electron.appMenu.edit.stopSpeaking", "Stop Speaking"),
                        ui::StopSpeaking,
                    ),
                ]),
            ),
        ]),
        Menu::new(label("windowsMenuBar.view", "View")).items(
            [
                MenuItem::action(
                    label("codex.commandMenuTitle.toggleSidebar", "Toggle Sidebar"),
                    ToggleSidebar,
                )
                .hidden(!sidebar_collapsible),
                MenuItem::action(
                    label(
                        "codex.commandMenuTitle.toggleBottomPanel",
                        "Toggle Bottom Panel",
                    ),
                    features::shortcuts::ToggleBottomPanel,
                ),
                MenuItem::action(
                    label(
                        "codex.commandMenuTitle.togglePinnedSummary",
                        "Toggle Pinned Summary",
                    ),
                    TogglePinnedSummary,
                ),
                MenuItem::action(
                    label("codex.command.toggleTerminal", "Open Terminal"),
                    features::shortcuts::OpenInteractiveTerminal,
                ),
                MenuItem::action(
                    label(
                        "codex.commandMenuTitle.toggleFileTreePanel",
                        "Toggle navigation panel",
                    ),
                    ToggleNavigationPanel,
                ),
                MenuItem::action("Full view", ui::ShowWorkspaceTabView),
                MenuItem::action(
                    label(
                        "codex.commandMenuTitle.toggleReviewPanel",
                        "Toggle Changes Panel",
                    ),
                    ToggleChangesPanel,
                ),
            ]
            .into_iter()
            .chain(browser)
            .chain([
                MenuItem::separator(),
                MenuItem::action(label("codex.command.findInThread", "Find"), FindInThread),
                MenuItem::separator(),
                MenuItem::action(
                    label("codex.commandMenuTitle.previousThread", "Previous Chat"),
                    PreviousChat,
                ),
                MenuItem::action(
                    label("codex.commandMenuTitle.nextThread", "Next Chat"),
                    NextChat,
                ),
                MenuItem::action(label("codex.command.navigateBack", "Back"), GoBack),
                MenuItem::action(label("codex.command.navigateForward", "Forward"), GoForward),
                MenuItem::separator(),
                MenuItem::action(
                    label("browserSidebar.zoomBanner.zoomIn", "Zoom In"),
                    features::shortcuts::ZoomIn,
                ),
                MenuItem::action(
                    label("browserSidebar.zoomBanner.zoomOut", "Zoom Out"),
                    features::shortcuts::ZoomOut,
                ),
                MenuItem::action(
                    label("electron.appMenu.view.actualSize", "Actual Size"),
                    features::shortcuts::ActualSize,
                ),
            ]),
        ),
        Menu::new(label("electron.appMenu.window", "Window")).items([
            MenuItem::action(
                label("electron.appMenu.window.minimize", "Minimize"),
                Minimize,
            ),
            MenuItem::action(label("electron.appMenu.window.zoom", "Zoom"), Zoom),
            MenuItem::separator(),
            MenuItem::action(
                label(
                    "electron.appMenu.window.bringAllToFront",
                    "Bring All to Front",
                ),
                BringAllToFront,
            ),
        ]),
        Menu::new(label("windowsMenuBar.help", "Help")).items([
            MenuItem::action(
                label("loadingPage.documentationLink", "Documentation"),
                OpenDocumentation,
            ),
            MenuItem::action(
                label("codex.command.showKeyboardShortcuts", "Keyboard Shortcuts"),
                ShowKeyboardShortcuts,
            ),
            MenuItem::action(label("sidebarHelp.whatsNew", "What's New"), OpenChangelog),
            MenuItem::separator(),
            MenuItem::action(
                label("electron.appMenu.help.troubleshooting", "Troubleshooting"),
                OpenTroubleshooting,
            ),
            MenuItem::action(
                label("electron.appMenu.help.systemStatus", "System Status"),
                OpenSystemStatus,
            ),
            MenuItem::action("Send Feedback", ShowFeedback),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_uses_a_supported_regional_locale() {
        assert_eq!(effective_locale("ja_JP"), "ja-JP");
        assert_eq!(effective_locale("zh-HK"), "zh-HK");
        assert_eq!(effective_locale("en_US"), "");
    }

    #[test]
    fn native_menu_labels_use_the_selected_locale() {
        let messages = messages("de-DE").expect("German menu messages");
        assert_eq!(
            localized(Some(&messages), "electron.appMenu.app.hide", "Hide Codex"),
            "Codex ausblenden"
        );
        assert_eq!(
            localized(Some(&messages), "missing.key", "Fallback"),
            "Fallback"
        );
    }

    #[test]
    fn every_shipped_locale_is_valid_json() {
        for locale in locale::SUPPORTED {
            assert!(messages(locale).is_some(), "invalid locale file: {locale}");
        }
    }

    #[test]
    fn browser_menu_is_available_only_on_thread_routes() {
        let has_browser_menu = |browser_available| {
            menus_for_route(true, false, browser_available, "")
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
