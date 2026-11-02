use super::shortcuts;
use crate::*;
use ui::screens::settings::keyboard_shortcuts::{
    self as keyboard, Change, KeyboardState, PreferenceKind, Preferences,
};

const BINDINGS: &str = "codex-command-keybindings";
const NUMBER_TARGET: &str = "primary-number-shortcut-target";
const LINK_MODIFIER: &str = "external-browser-link-shortcut";

pub(super) fn install(cx: &mut App) {
    if cx
        .try_global::<KeyboardState>()
        .is_some_and(|state| state.dispatch.is_some())
    {
        return;
    }
    let preferences = (|| -> Result<Preferences> {
        let values = cx
            .try_global::<host_settings::Settings>()
            .context("Host settings are unavailable")?
            .read_all();
        let preferences = Preferences {
            bindings: values
                .get(BINDINGS)
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default(),
            primary_number_shortcut_target: values
                .get(NUMBER_TARGET)
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default(),
            external_browser_link_shortcut: values
                .get(LINK_MODIFIER)
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default(),
        };
        preferences.validate().map_err(anyhow::Error::msg)?;
        Ok(preferences)
    })();
    let error = preferences
        .as_ref()
        .err()
        .map(|error| format!("Failed to load keyboard shortcuts: {error}"));
    let preferences = preferences.unwrap_or_default();
    cx.set_global(KeyboardState {
        loaded: error.is_none(),
        preferences: preferences.clone(),
        capture_owner: None,
        error,
        preference_error: None,
        active_commands: keyboard::commands()
            .iter()
            .filter(|command| action(command.id).is_some())
            .map(|command| command.id)
            .collect(),
        dispatch: Some(Arc::new(change)),
        home: false,
    });
    rebind(None, &preferences, cx);
}

fn change(change: Change, cx: &mut App) {
    let previous = cx.global::<KeyboardState>().preferences.clone();
    let mut preferences = previous.clone();
    let result = preferences
        .change(&change)
        .map_err(anyhow::Error::msg)
        .and_then(|()| {
            let writes = match &change {
                Change::Bindings { .. } | Change::ResetCommand(_) => {
                    vec![(BINDINGS, json!(preferences.bindings))]
                }
                Change::NumberTarget(_) => vec![(
                    NUMBER_TARGET,
                    json!(preferences.primary_number_shortcut_target),
                )],
                Change::LinkModifier(_) => vec![(
                    LINK_MODIFIER,
                    json!(preferences.external_browser_link_shortcut),
                )],
                Change::Reset => vec![
                    (BINDINGS, json!({})),
                    (
                        LINK_MODIFIER,
                        json!(preferences.external_browser_link_shortcut),
                    ),
                ],
            };
            cx.update_global::<host_settings::Settings, _>(|settings, _| {
                settings.write_batch(writes)
            })
        });
    if result.is_ok() {
        rebind(Some(&previous), &preferences, cx);
    }
    cx.update_global::<KeyboardState, _>(|state, _| {
        state.error = result.err().map(|error| error.to_string());
        state.preference_error = state.error.as_ref().and(match change {
            Change::NumberTarget(_) => Some(PreferenceKind::Number),
            Change::LinkModifier(_) => Some(PreferenceKind::Link),
            _ => None,
        });
        if state.error.is_none() {
            state.preferences = preferences;
            state.loaded = true;
        }
    });
}

fn rebind(previous: Option<&Preferences>, next: &Preferences, cx: &mut App) {
    if previous.is_some_and(|previous| {
        previous.bindings == next.bindings
            && previous.primary_number_shortcut_target == next.primary_number_shortcut_target
    }) {
        return;
    }
    let previous = previous.unwrap_or(next);
    let mapper = gpui::DummyKeyboardMapper;
    let mut bindings = Vec::new();
    let mut enabled = Vec::new();
    let close_tab_keys: Vec<String> = keyboard::commands()
        .iter()
        .find(|command| command.id == "closeTab")
        .map(|command| next.keys(command).map(keyboard::canonical).collect())
        .unwrap_or_default();
    for command in keyboard::commands() {
        let Some(action) = action(command.id) else {
            continue;
        };
        for accelerator in command.keys.iter().copied().chain(previous.keys(command)) {
            let key = keyboard::canonical(accelerator);
            if key.starts_with("mouse") {
                continue;
            }
            let unbind = Box::new(gpui::Unbind(action.name().into()));
            if let Ok(binding) = KeyBinding::load(&key, unbind, None, false, None, &mapper) {
                bindings.push(binding);
            }
        }
        for accelerator in next.keys(command) {
            let key = keyboard::canonical(accelerator);
            if key.starts_with("mouse") {
                continue;
            }
            // An accelerator of `closeTab` routes to the shell tab first (`CloseWindow`); only
            // one that `closeWindow` alone has closes the window outright.
            if command.id == "closeWindow" && close_tab_keys.contains(&key) {
                continue;
            }
            // Tab navigation keeps the chat bindings outside the focused side panel.
            let context = if matches!(command.id, "nextTab" | "previousTab" | "closeOtherTabs") {
                Some("SideTabs")
            } else if matches!(command.id, "navigateBrowserBack" | "navigateBrowserForward") {
                // `⌘←`/`⌘→` keep their line-start/end meaning in the address bar.
                Some("BrowserTab && !Composer")
            } else if command.id.starts_with("composer.") {
                Some("Composer")
            } else if !key.contains("cmd-") && !key.contains("ctrl-") && !key.contains("alt-") {
                Some("!Composer && !Editable")
            } else {
                None
            };
            let predicate = context.map(|context| {
                std::rc::Rc::new(
                    gpui::KeyBindingContextPredicate::parse(context).expect("literal key context"),
                )
            });
            if let Ok(binding) =
                KeyBinding::load(&key, action.boxed_clone(), predicate, false, None, &mapper)
            {
                enabled.push(binding);
            }
        }
    }
    bindings.extend(enabled);
    cx.bind_keys(bindings);
    crate::native_menus::refresh_menus(cx);
}

fn action(id: &str) -> Option<Box<dyn Action>> {
    let action: Box<dyn Action> = match id {
        "newTask" => Box::new(NewChat),
        "newProjectlessTask" => Box::new(shortcuts::NewStandaloneChat),
        "temporaryChat" => Box::new(shortcuts::NewTemporaryChat),
        "settings" => Box::new(OpenSettings),
        "keyboardShortcuts" => Box::new(shortcuts::OpenShortcutSettings),
        "showKeyboardShortcuts" => Box::new(ShowKeyboardShortcuts),
        "toggleSidebar" => Box::new(ToggleSidebar),
        "togglePriorityFilter" => Box::new(ToggleActivity),
        "switchToMode1" => Box::new(ui::SwitchMode1),
        "switchToMode2" => Box::new(ui::SwitchMode2),
        "switchToMode3" => Box::new(ui::SwitchMode3),
        "navigateBack" => Box::new(GoBack),
        "navigateForward" => Box::new(GoForward),
        "previousThread" => Box::new(PreviousChat),
        "nextThread" => Box::new(NextChat),
        "previousRecentThread" => Box::new(shortcuts::PreviousRecentChat),
        "nextRecentThread" => Box::new(shortcuts::NextRecentChat),
        "toggleBottomPanel" => Box::new(shortcuts::ToggleBottomPanel),
        "toggleTerminal" => Box::new(shortcuts::OpenInteractiveTerminal),
        "environmentAction1" => Box::new(shortcuts::EnvironmentAction1),
        "environmentAction2" => Box::new(shortcuts::EnvironmentAction2),
        "environmentAction3" => Box::new(shortcuts::EnvironmentAction3),
        "environmentAction4" => Box::new(shortcuts::EnvironmentAction4),
        "environmentAction5" => Box::new(shortcuts::EnvironmentAction5),
        "environmentAction6" => Box::new(shortcuts::EnvironmentAction6),
        "environmentAction7" => Box::new(shortcuts::EnvironmentAction7),
        "environmentAction8" => Box::new(shortcuts::EnvironmentAction8),
        "environmentAction9" => Box::new(shortcuts::EnvironmentAction9),
        "openReviewTab" => Box::new(OpenChangesPanel),
        "openBrowserTab" => Box::new(ui::browser::OpenBrowserTab::default()),
        "navigateBrowserBack" => Box::new(ui::browser::BrowserBack),
        "navigateBrowserForward" => Box::new(ui::browser::BrowserForward),
        "reloadBrowserPage" => Box::new(ui::browser::ReloadBrowserPage),
        "hardReloadBrowserPage" => Box::new(ui::browser::HardReloadBrowserPage),
        "focusBrowserAddressBar" => Box::new(ui::browser::FocusBrowserAddressBar),
        "toggleSidePanel" => Box::new(ToggleChangesPanel),
        "toggleFileTreePanel" => Box::new(crate::native_menus::ToggleNavigationPanel),
        "showWorkspaceTabView" => Box::new(ui::ShowWorkspaceTabView),
        "stepWorkspaceLayout" => Box::new(ui::StepWorkspaceLayout),
        "undoAppAction" => Box::new(Undo),
        "redoAppAction" => Box::new(Redo),
        "openFolder" | "composer.openProjectPicker" => Box::new(OpenFolder),
        "newProject" => Box::new(ui::screens::new_project::NewProject),
        "openCommandMenu" => Box::new(OpenCommandMenu),
        "searchFiles" => Box::new(super::command_destinations::SearchFiles),
        "archiveThread" => Box::new(ArchiveChat),
        "logOut" => Box::new(LogOut),
        "clearAllUnreads" => Box::new(shortcuts::ClearAllUnreads),
        "nextThreadNeedingAttention" => Box::new(shortcuts::NextAttentionChat),
        "markThreadUnread" => Box::new(shortcuts::MarkOpenChatUnread),
        "manageTasks" => Box::new(OpenScheduled),
        "openThreadInNewWindow" => Box::new(super::OpenThreadInNewWindow),
        "newWindow" => Box::new(crate::windows::NewWindow),
        "closeTab" => Box::new(CloseWindow),
        "closeWindow" => Box::new(shortcuts::CloseWholeWindow),
        "closeOtherTabs" => Box::new(shortcuts::CloseOtherSideTabs),
        "nextTab" => Box::new(shortcuts::NextSideTab),
        "previousTab" => Box::new(shortcuts::PreviousSideTab),
        "toggleThreadPin" => Box::new(shortcuts::ToggleThreadPin),
        "renameThread" => Box::new(shortcuts::RenameOpenChat),
        "findInThread" => Box::new(FindInThread),
        "copyDeeplink" => Box::new(shortcuts::CopyDeeplink),
        "copyLastCodeBlock" => Box::new(shortcuts::CopyLastCodeBlock),
        "copyWorkingDirectory" | "copyConversationPath" => {
            Box::new(super::copy_directory::CopyWorkingDirectory)
        }
        "composer.submit" => Box::new(ui::composer::Submit),
        "composer.submitInBackground" => Box::new(ui::composer::SubmitInBackground),
        id => {
            if let Some(number) = id
                .strip_prefix("recentThread")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=6).contains(n))
            {
                Box::new(GoToRecentChat(number - 1))
            } else if let Some(number) = id
                .strip_prefix("thread")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=9).contains(n))
            {
                Box::new(GoToChat(number - 1))
            } else {
                let number = id
                    .strip_prefix("focusTab")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| (1..=9).contains(n))?;
                Box::new(ActivateSideTab(number - 1))
            }
        }
    };
    Some(action)
}

pub(super) fn dispatch_mouse(key: &str, window: &mut Window, cx: &mut Context<Root>) {
    let state = cx.global::<KeyboardState>();
    if state.capture_owner.is_some() {
        return;
    }
    let command = keyboard::commands().iter().find(|command| {
        state
            .preferences
            .keys(command)
            .any(|binding| binding == key)
    });
    if let Some(action) = command.and_then(|command| action(command.id)) {
        window.dispatch_action(action, cx);
    }
}
