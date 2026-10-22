use super::shortcuts;
use crate::*;
use gpui::{MouseButton, NavigationDirection};
use ui::keyboard::{self, Change, KeyboardState, PreferenceKind, Preferences};

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
            let context = binding_context(command.id, &key);
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

fn binding_context(command: &str, key: &str) -> Option<&'static str> {
    // Tab navigation keeps the chat bindings outside the focused side panel.
    if matches!(command, "nextTab" | "previousTab" | "closeOtherTabs") {
        Some("SideTabs")
    } else if command.starts_with("composer.") {
        Some("Composer")
    } else if !key.contains("cmd-") && !key.contains("ctrl-") && !key.contains("alt-") {
        Some("!Composer && !Editable")
    } else {
        None
    }
}

fn action(id: &str) -> Option<Box<dyn Action>> {
    fn boxed<A: Action + Default>() -> Box<dyn Action> {
        Box::new(A::default())
    }
    type Constructor = fn() -> Box<dyn Action>;
    const ACTIONS: &[(&str, Constructor)] = &[
        ("archiveThread", boxed::<ArchiveChat>),
        ("clearAllUnreads", boxed::<shortcuts::ClearAllUnreads>),
        ("closeOtherTabs", boxed::<shortcuts::CloseOtherSideTabs>),
        ("closeTab", boxed::<CloseWindow>),
        ("closeWindow", boxed::<shortcuts::CloseWholeWindow>),
        ("composer.openProjectPicker", boxed::<OpenFolder>),
        ("composer.submit", boxed::<ui::composer::Submit>),
        (
            "composer.submitInBackground",
            boxed::<ui::composer::SubmitInBackground>,
        ),
        ("copyConversationPath", boxed::<super::CopyWorkingDirectory>),
        ("copyDeeplink", boxed::<shortcuts::CopyDeeplink>),
        ("copyWorkingDirectory", boxed::<super::CopyWorkingDirectory>),
        ("findInThread", boxed::<FindInThread>),
        (
            "keyboardShortcuts",
            boxed::<shortcuts::OpenShortcutSettings>,
        ),
        ("logOut", boxed::<LogOut>),
        ("manageTasks", boxed::<OpenScheduled>),
        ("navigateBack", boxed::<GoBack>),
        ("navigateForward", boxed::<GoForward>),
        ("newProjectlessTask", boxed::<shortcuts::NewStandaloneChat>),
        ("newTask", boxed::<NewChat>),
        ("nextRecentThread", boxed::<shortcuts::NextRecentChat>),
        ("nextTab", boxed::<shortcuts::NextSideTab>),
        ("nextThread", boxed::<NextChat>),
        (
            "nextThreadNeedingAttention",
            boxed::<shortcuts::NextAttentionChat>,
        ),
        ("openCommandMenu", boxed::<OpenCommandMenu>),
        ("openFolder", boxed::<OpenFolder>),
        ("openReviewTab", boxed::<OpenChangesPanel>),
        (
            "openThreadInNewWindow",
            boxed::<super::OpenThreadInNewWindow>,
        ),
        (
            "previousRecentThread",
            boxed::<shortcuts::PreviousRecentChat>,
        ),
        ("previousTab", boxed::<shortcuts::PreviousSideTab>),
        ("previousThread", boxed::<PreviousChat>),
        ("redoAppAction", boxed::<Redo>),
        ("renameThread", boxed::<shortcuts::RenameOpenChat>),
        (
            "searchFiles",
            boxed::<super::command_destinations::SearchFiles>,
        ),
        ("settings", boxed::<OpenSettings>),
        ("showKeyboardShortcuts", boxed::<ShowKeyboardShortcuts>),
        ("temporaryChat", boxed::<shortcuts::NewTemporaryChat>),
        ("toggleSidePanel", boxed::<ToggleChangesPanel>),
        ("toggleSidebar", boxed::<ToggleSidebar>),
        ("toggleThreadPin", boxed::<shortcuts::ToggleThreadPin>),
        ("undoAppAction", boxed::<Undo>),
    ];
    if let Some((_, construct)) = ACTIONS.iter().find(|(name, _)| *name == id) {
        return Some(construct());
    }
    let action: Box<dyn Action> = if let Some(number) = id
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
    };
    Some(action)
}

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Back),
        cx.listener(|_, _, window, cx| dispatch_mouse("MouseBack", window, cx)),
    )
    .on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Forward),
        cx.listener(|_, _, window, cx| dispatch_mouse("MouseForward", window, cx)),
    )
}

fn dispatch_mouse(key: &str, window: &mut Window, cx: &mut Context<Root>) {
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
