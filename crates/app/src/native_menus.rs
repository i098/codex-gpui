use crate::{
    CloseWindow, FindInThread, GoBack, GoForward, Hide, HideOthers, LogOut, Minimize, NewChat,
    NextChat, OpenFolder, OpenSettings, PreviousChat, Quit, ShowAll, ShowKeyboardShortcuts,
    ToggleChangesPanel, ToggleSidebar, Zoom,
};
use gpui::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, actions};
use ui::{
    Copy, Cut, DeleteSelection, OpenBackgroundTerminal, Paste, PasteAndMatchStyle, Redo, SelectAll,
    Undo,
};

actions!(
    native_menus,
    [
        OpenDocumentation,
        OpenChangelog,
        OpenTroubleshooting,
        OpenSystemStatus
    ]
);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-shift-v", PasteAndMatchStyle, None),
        KeyBinding::new("cmd-alt-shift-v", PasteAndMatchStyle, None),
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
}

/// Builds the native menu items backed by the app's registered actions.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Codex").items([
            MenuItem::action("Settings…", OpenSettings),
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
            MenuItem::action("New Chat", NewChat),
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
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Open Terminal", OpenBackgroundTerminal),
            MenuItem::action("Toggle Changes Panel", ToggleChangesPanel),
            MenuItem::separator(),
            MenuItem::action("Find", FindInThread),
            MenuItem::separator(),
            MenuItem::action("Previous Chat", PreviousChat),
            MenuItem::action("Next Chat", NextChat),
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Documentation", OpenDocumentation),
            MenuItem::action("Keyboard Shortcuts", ShowKeyboardShortcuts),
            MenuItem::action("What's New", OpenChangelog),
            MenuItem::separator(),
            MenuItem::action("Troubleshooting", OpenTroubleshooting),
            MenuItem::action("System Status", OpenSystemStatus),
        ]),
    ]
}
