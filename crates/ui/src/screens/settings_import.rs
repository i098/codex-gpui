use super::computer_use::Load;
use super::{Layer, button, deco, on_card, probe, switch};
use crate::{focus, icon::icon, rounded_clip, text};
use bridge::imports::{
    CategoryPreference, DetectedItem, History, ItemType, Policy, Preferences, Provider, SyncState,
};
use gpui::{
    AnyElement, App, BorrowAppContext as _, Context, Div, FocusHandle, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone)]
pub enum ImportAction {
    Refresh,
    Import {
        items: Vec<DetectedItem>,
        selection: BTreeMap<String, bool>,
    },
    SetSync(bool),
    SavePreferences(Preferences),
    OpenItem(ItemType, String),
    AuthenticateMcp(String),
}

pub type Dispatch = Arc<dyn Fn(ImportAction, &mut Window, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ImportState {
    pub policy: Policy,
    pub detected: Load<Arc<Vec<DetectedItem>>>,
    pub histories: Load<Arc<Vec<History>>>,
    pub mcp_auth: Load<Arc<Vec<String>>>,
    pub auth_urls: BTreeMap<String, Option<String>>,
    pub sync_state: SyncState,
    pub preferences: Preferences,
    pub sync_enabled: bool,
    pub running: Option<Arc<Vec<DetectedItem>>>,
    pub pending: bool,
    pub reviewing: bool,
    pub modal_open: bool,
    pub policy_request: u64,
    pub error: Option<String>,
    pub import_completed: u64,
    pub preferences_saved: u64,
    pub dispatch: Option<Dispatch>,
}

impl Global for ImportState {}

pub fn visible(cx: &App) -> bool {
    cx.try_global::<ImportState>()
        .is_some_and(|state| state.policy.enabled)
}

pub fn deactivate(cx: &mut App) {
    if cx
        .try_global::<ImportState>()
        .is_some_and(|state| state.modal_open)
    {
        cx.update_global::<ImportState, _>(|state, _| {
            state.modal_open = false;
            state.reviewing = false;
        });
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    let page = window.use_keyed_state("import-settings", cx, |window, cx| {
        cx.observe_global::<ImportState>(|_, cx| cx.notify())
            .detach();
        let page = ImportPage {
            layer: Layer::default(),
            scroll: ScrollHandle::new(),
            modal: None,
            modal_focus: cx.focus_handle(),
            return_focus: None,
            expanded: BTreeSet::new(),
            categories: BTreeSet::new(),
            show_all_history: false,
            history_head: None,
            import_completed: 0,
            preferences_saved: 0,
            attention_tab: ItemType::McpServerConfig,
            composited: false,
        };
        if let Some(dispatch) = cx
            .try_global::<ImportState>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(ImportAction::Refresh, window, cx);
        }
        page
    });
    page.into_any_element()
}

enum Modal {
    Review {
        provider: Provider,
        items: Arc<Vec<DetectedItem>>,
        selected: BTreeMap<String, bool>,
        choices: Vec<ReviewChoice>,
        tools_expanded: bool,
    },
    Customize {
        preferences: Preferences,
        show_all: bool,
    },
}

struct ReviewChoice {
    id: String,
    keys: Vec<String>,
    title: String,
    detail: String,
    icon: &'static str,
    tooltip: Option<String>,
    nested: bool,
    expandable: bool,
}

#[derive(Clone)]
enum Control {
    Dispatch(ImportAction),
    Review(Provider),
    Select(Vec<String>),
    ToggleTools,
    Import,
    Customize,
    Category(ItemType),
    AllCategories,
    ShowCategories,
    Save,
    Close,
    History(String),
    HistoryCategory(String, ItemType),
    MoreHistory,
    Attention(ItemType),
}

struct ImportPage {
    layer: Layer,
    scroll: ScrollHandle,
    modal: Option<Modal>,
    modal_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    expanded: BTreeSet<String>,
    categories: BTreeSet<(String, ItemType)>,
    show_all_history: bool,
    history_head: Option<String>,
    import_completed: u64,
    preferences_saved: u64,
    attention_tab: ItemType,
    composited: bool,
}

impl ImportPage {
    fn state(cx: &App) -> ImportState {
        cx.try_global::<ImportState>().cloned().unwrap_or_default()
    }

    fn surface(&self, theme: &Theme) -> AnyElement {
        if self.composited {
            deco(
                &self.layer,
                20.,
                shapes::Corner::Squircle,
                theme.background_panel,
                Some(theme.border),
            )
        } else {
            on_card(
                20.,
                shapes::Corner::Squircle,
                theme.background_panel,
                Some(theme.border),
            )
        }
    }

    fn dispatch(action: ImportAction, window: &mut Window, cx: &mut App) {
        if let Some(dispatch) = cx
            .try_global::<ImportState>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(action, window, cx);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        cx.update_global::<ImportState, _>(|state, _| {
            state.reviewing = false;
            state.modal_open = false;
        });
        if let Some(focus) = self.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn activate(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        let state = Self::state(cx);
        match control {
            Control::Dispatch(action) => Self::dispatch(action, window, cx),
            Control::Review(provider) => {
                let Load::Ready(items) = &state.detected else {
                    return;
                };
                let choices = review_choices(items, provider);
                let selected = choices
                    .iter()
                    .flat_map(|choice| choice.keys.iter().cloned())
                    .map(|key| (key, true))
                    .collect();
                self.modal = Some(Modal::Review {
                    provider,
                    items: items.clone(),
                    selected,
                    choices,
                    tools_expanded: false,
                });
                cx.update_global::<ImportState, _>(|state, _| {
                    state.reviewing = true;
                    state.modal_open = true;
                });
                self.return_focus = window.focused(cx);
                self.modal_focus.focus(window, cx);
            }
            Control::Select(keys) => {
                if let Some(Modal::Review { selected, .. }) = &mut self.modal {
                    let value = !keys.iter().all(|key| selected.get(key) == Some(&true));
                    for key in keys {
                        selected.insert(key, value);
                    }
                }
            }
            Control::ToggleTools => {
                if let Some(Modal::Review { tools_expanded, .. }) = &mut self.modal {
                    *tools_expanded = !*tools_expanded;
                }
            }
            Control::Import => {
                if let Some(Modal::Review {
                    provider,
                    selected,
                    items,
                    ..
                }) = &self.modal
                {
                    let chosen: Vec<_> = items
                        .iter()
                        .filter(|item| item.provider_id == *provider && item.selected(selected))
                        .cloned()
                        .collect();
                    if !chosen.is_empty() && !state.pending {
                        Self::dispatch(
                            ImportAction::Import {
                                items: chosen,
                                selection: selected.clone(),
                            },
                            window,
                            cx,
                        );
                    }
                }
            }
            Control::Customize => {
                self.modal = Some(Modal::Customize {
                    preferences: state.preferences,
                    show_all: false,
                });
                cx.update_global::<ImportState, _>(|state, _| state.modal_open = true);
                self.return_focus = window.focused(cx);
                self.modal_focus.focus(window, cx);
            }
            Control::Category(category) => {
                if let Some(Modal::Customize { preferences, .. }) = &mut self.modal {
                    let on =
                        preferences.category_checked(category, &state.sync_state) == Some(true);
                    let mut selected = preference_values(preferences, &state);
                    selected.insert(category, CategoryPreference::Enabled(!on));
                    *preferences = Preferences::Selected(selected);
                }
            }
            Control::AllCategories => {
                if let Some(Modal::Customize { preferences, .. }) = &mut self.modal {
                    *preferences = if *preferences == Preferences::All {
                        Preferences::Selected(preference_values(preferences, &state))
                    } else {
                        Preferences::All
                    };
                }
            }
            Control::ShowCategories => {
                if let Some(Modal::Customize { show_all, .. }) = &mut self.modal {
                    *show_all = !*show_all;
                }
            }
            Control::Save => {
                if let Some(Modal::Customize { preferences, .. }) = &self.modal
                    && ItemType::ALL
                        .iter()
                        .any(|category| preferences.category_selected(*category, &state.sync_state))
                {
                    Self::dispatch(
                        ImportAction::SavePreferences(preferences.clone()),
                        window,
                        cx,
                    );
                }
            }
            Control::Close if !state.pending => self.close(window, cx),
            Control::Close => {}
            Control::History(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Control::HistoryCategory(id, category) => {
                if !self.categories.remove(&(id.clone(), category)) {
                    self.categories.insert((id, category));
                }
            }
            Control::MoreHistory => self.show_all_history = true,
            Control::Attention(category) => self.attention_tab = category,
        }
        cx.notify();
    }

    fn bind_control(
        &self,
        element: gpui::Stateful<Div>,
        control: Control,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let keyboard = control.clone();
        element
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    this.activate(control.clone(), window, cx);
                }
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate(keyboard.clone(), window, cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn action(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        control: Control,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let t = cx.theme().clone();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let element = focus::ring(
            button(
                label,
                if matches!(control, Control::Customize)
                    && matches!(self.modal, Some(Modal::Customize { .. }))
                {
                    t.text_alpha_10
                } else {
                    t.text_alpha_5
                },
                None,
                &t,
            )
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(disabled)
            .when(disabled, |d| d.opacity(0.4)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        );
        self.bind_control(element, control, disabled, cx)
            .into_any_element()
    }

    fn toggle(
        &self,
        (id, label): (&'static str, &'static str),
        on: bool,
        control: Control,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let element = focus::ring(
            switch(id, on, disabled, &t)
                .role(gpui::Role::Switch)
                .aria_label(label)
                .aria_disabled(disabled)
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                }),
            &handle,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        );
        self.bind_control(element, control, disabled, cx)
            .into_any_element()
    }

    fn disclosure(
        &self,
        id: String,
        content: Div,
        expanded: bool,
        control: Control,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let handle = focus::handle(id.clone(), 0, window, cx);
        let element = focus::ring(
            content
                .id(id)
                .role(gpui::Role::Button)
                .aria_expanded(expanded),
            &handle,
            px(0.),
            px(-2.),
            cx,
        );
        div().child(self.bind_control(element, control, false, cx))
    }

    fn checkbox(
        &self,
        (id, label): (impl Into<SharedString>, &str),
        checked: Option<bool>,
        control: Control,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = id.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let accent = matches!(control, Control::Select(_));
        let fill = if checked == Some(false) {
            gpui::transparent_black()
        } else if accent {
            t.switch_track_checked
        } else {
            gpui::Hsla {
                a: 0.96,
                ..t.menu_background
            }
        };
        let border = if accent && checked != Some(false) {
            t.switch_track_checked
        } else {
            t.border_strong
        };
        let size = if accent { 16. } else { 14. };
        // Inline controls sit 1.3 px below flex centering at the original 14 px / 21 px font size.
        let baseline_offset = (21. - size) / 2. + 1.3;
        let element = focus::ring(
            div()
                .id(id)
                .role(gpui::Role::CheckBox)
                .aria_label(label.to_owned())
                .aria_disabled(disabled)
                .aria_toggled(match checked {
                    Some(true) => gpui::Toggled::True,
                    Some(false) => gpui::Toggled::False,
                    None => gpui::Toggled::Mixed,
                })
                .size(px(size))
                .relative()
                .top(px(baseline_offset))
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(
                    if accent { 3. } else { 5. },
                    shapes::Corner::Round,
                    fill,
                    Some(border),
                ))
                .when(checked == Some(true), |d| {
                    d.child(icon("menu-check", 12., t.text))
                })
                .when(checked.is_none(), |d| {
                    d.child(div().w(px(8.)).h(px(2.)).bg(t.text))
                })
                .when(disabled, |d| d.opacity(0.4)),
            &handle,
            px(5.),
            px(2.),
            cx,
        );
        div()
            .w(px(size))
            .h(px(21.))
            .child(self.bind_control(element, control, disabled, cx))
            .into_any_element()
    }

    fn detection(&self, state: &ImportState, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let mut subtitle = None;
        let rows = if state.running.is_some() {
            vec![row("Import in progress", None, None, spinner(&t), &t)]
        } else {
            match &state.detected {
                Load::Loading => vec![row(
                    "Checking for imports",
                    Some("Looking for compatible setup, projects, and recent chats".into()),
                    None,
                    spinner(&t),
                    &t,
                )],
                Load::Failed(_) => vec![row(
                    "Couldn't check for imports",
                    Some(
                        "Try again to look for compatible setup, projects, and recent chats".into(),
                    ),
                    None,
                    self.action(
                        "retry-import-detect",
                        "Retry",
                        Control::Dispatch(ImportAction::Refresh),
                        state.pending,
                        window,
                        cx,
                    ),
                    &t,
                )],
                Load::Ready(items) if items.is_empty() => {
                    let last = if let Load::Ready(histories) = &state.histories {
                        histories
                            .iter()
                            .find(|history| !history.successes.is_empty())
                    } else {
                        None
                    };
                    vec![if let Some(last) = last {
                        row(
                            "No new items found",
                            Some(format!(
                                "Last imported {} ago",
                                relative_time(last.completed_at_ms)
                            )),
                            None,
                            self.action(
                                "recheck-import-detect",
                                "Check again",
                                Control::Dispatch(ImportAction::Refresh),
                                state.pending,
                                window,
                                cx,
                            ),
                            &t,
                        )
                    } else {
                        row(
                            "Import from other AI apps",
                            Some("No importable setup found".into()),
                            None,
                            self.action(
                                "empty-import",
                                "Import",
                                Control::Import,
                                true,
                                window,
                                cx,
                            ),
                            &t,
                        )
                    }]
                }
                Load::Ready(items) => {
                    subtitle = Some("Detected setup that can be added to ChatGPT");
                    [Provider::ClaudeCode, Provider::Cursor]
                        .into_iter()
                        .filter(|provider| items.iter().any(|item| item.provider_id == *provider))
                        .map(|provider| {
                            row(
                                provider.label(),
                                None,
                                Some(provider_icon(provider, 15., &t)),
                                self.action(
                                    provider.id(),
                                    "Import",
                                    Control::Review(provider),
                                    state.pending,
                                    window,
                                    cx,
                                ),
                                &t,
                            )
                        })
                        .collect()
                }
            }
        };
        group(
            "Import from another AI app",
            subtitle,
            rows,
            self.surface(&t),
            &t,
        )
    }

    fn sync(&self, state: &ImportState, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let connected = state.sync_state.connected();
        let description = if state.sync_enabled {
            "Automatically sync new and updated content from connected sources"
        } else {
            "Sync paused. Your content selections are saved"
        };
        group(
            "Automatic sync",
            None,
            vec![
                row(
                    "Keep imports in sync",
                    Some(description.into()),
                    None,
                    self.toggle(
                        ("keep-imports-in-sync", "Keep imports in sync"),
                        state.sync_enabled,
                        Control::Dispatch(ImportAction::SetSync(!state.sync_enabled)),
                        false,
                        window,
                        cx,
                    ),
                    &t,
                ),
                row(
                    "Content to sync",
                    Some(if connected {
                        preference_summary(state)
                    } else {
                        "Available after your first import".into()
                    }),
                    None,
                    self.action(
                        "customize-import-sync",
                        "Customize",
                        Control::Customize,
                        !connected,
                        window,
                        cx,
                    ),
                    &t,
                ),
            ],
            self.surface(&t),
            &t,
        )
    }

    fn history(
        &mut self,
        state: &ImportState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let t = cx.theme().clone();
        let histories = match &state.histories {
            Load::Loading => {
                return Some(group(
                    "Import history",
                    None,
                    vec![
                        div().child(
                            div()
                                .id("import-history-loading")
                                .role(gpui::Role::Status)
                                .px(px(16.))
                                .py(px(12.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(spinner(&t))
                                .child(text(
                                    "Loading import history",
                                    13.,
                                    18.571_428,
                                    400.,
                                    t.text_secondary,
                                )),
                        ),
                    ],
                    self.surface(&t),
                    &t,
                ));
            }
            Load::Failed(_) => {
                return Some(group(
                    "Import history",
                    None,
                    vec![row(
                        "Couldn't load import history",
                        None,
                        None,
                        self.action(
                            "retry-import-history",
                            "Retry",
                            Control::Dispatch(ImportAction::Refresh),
                            state.pending,
                            window,
                            cx,
                        ),
                        &t,
                    )],
                    self.surface(&t),
                    &t,
                ));
            }
            Load::Ready(histories) => histories,
        };
        if histories.is_empty() && state.running.is_none() {
            return None;
        }
        let newest = histories
            .first()
            .map_or(0, |history| history.completed_at_ms);
        let mut entries = Vec::new();
        if let Some(items) = &state.running {
            let mut rows = vec![row("Importing now", None, None, spinner(&t), &t)];
            for category in ItemType::ALL {
                if items.iter().any(|item| item.item.item_type == category) {
                    rows.push(nested_row(
                        category.label(),
                        None,
                        text("Importing", 12., 16., 430., t.text_secondary).into_any_element(),
                        &t,
                    ));
                }
            }
            entries.push(card(rows, self.surface(&t), &t));
        }
        let mut visible_count = 0;
        for history in histories
            .iter()
            .filter(|history| self.show_all_history || newest - history.completed_at_ms <= 60_000)
        {
            visible_count += 1;
            let mut rows = Vec::new();
            let id = &history.import_id;
            let expanded = self.expanded.contains(id);
            let title = history.provider_id.map_or_else(
                || "Import".into(),
                |provider| format!("Imported from {}", provider.label()),
            );
            let warning_count = history_warning_count(history, state);
            let mut summary = format!("{} imported", history.successes.len());
            if !history.failures.is_empty() {
                summary.push_str(&format!(" · {} did not import", history.failures.len()));
            }
            if warning_count > 0 {
                summary.push_str(&format!(" · {warning_count} need review"));
            }
            let description = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(text(
                    history_time(history.completed_at_ms),
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                ))
                .child(text(summary, 12., 16., 430., t.text_secondary));
            let arrow = history_arrow(expanded, RowKind::Standard, &t);
            rows.push(
                self.disclosure(
                    format!("history-{id}"),
                    row_content(
                        title,
                        Some(description.into_any_element()),
                        history
                            .provider_id
                            .map(|provider| provider_icon(provider, 12.5, &t)),
                        arrow,
                        &t,
                        RowKind::Standard,
                    ),
                    expanded,
                    Control::History(id.clone()),
                    window,
                    cx,
                ),
            );
            if expanded {
                let categories: BTreeSet<_> = history
                    .successes
                    .iter()
                    .chain(&history.failures)
                    .map(|item| item.item_type)
                    .collect();
                if categories.is_empty() {
                    rows.push(nested_row(
                        "No results recorded",
                        None,
                        div().into_any_element(),
                        &t,
                    ));
                }
                for category in categories {
                    let count = history
                        .successes
                        .iter()
                        .filter(|item| item.item_type == category)
                        .count();
                    let failed = history
                        .failures
                        .iter()
                        .filter(|item| item.item_type == category)
                        .count();
                    let auth = imported_auth_count(history, state, category);
                    let expanded = self.categories.contains(&(id.clone(), category));
                    let detail = match category {
                        ItemType::Commands => Some("Converted into Codex skills".into()),
                        ItemType::Hooks if count > 0 => {
                            Some("Manage imported hooks in Hooks settings".into())
                        }
                        ItemType::McpServerConfig if auth > 0 => Some(format!(
                            "{auth} {} authentication",
                            if auth == 1 {
                                "server needs"
                            } else {
                                "servers need"
                            }
                        )),
                        _ => None,
                    };
                    let status = div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(category_status(category, count, failed, auth, &t))
                        .child(history_arrow(expanded, RowKind::Nested, &t));
                    rows.push(self.disclosure(
                        format!("history-{id}-{}", category.id()),
                        nested_row(category.label(), detail, status.into_any_element(), &t),
                        expanded,
                        Control::HistoryCategory(id.clone(), category),
                        window,
                        cx,
                    ));
                    if expanded {
                        for (failed, item) in history
                            .successes
                            .iter()
                            .map(|item| (false, item))
                            .chain(history.failures.iter().map(|item| (true, item)))
                            .filter(|(_, item)| item.item_type == category)
                        {
                            let label = if category == ItemType::Sessions {
                                item.title.clone().unwrap_or_else(|| "Untitled chat".into())
                            } else {
                                item.source
                                    .clone()
                                    .unwrap_or_else(|| "Imported item".into())
                            };
                            let detail = if failed {
                                item.message.clone()
                            } else {
                                item.target
                                    .clone()
                                    .filter(|target| Some(target) != item.source.as_ref())
                            };
                            let control = if !failed
                                && matches!(
                                    category,
                                    ItemType::Sessions
                                        | ItemType::Skills
                                        | ItemType::Commands
                                        | ItemType::McpServerConfig
                                        | ItemType::Plugins
                                ) {
                                item.target.as_ref().map(|target| {
                                    self.action(
                                        format!("open-import-{id}-{target}"),
                                        match category {
                                            ItemType::Sessions => "Open chat",
                                            ItemType::Skills | ItemType::Commands => {
                                                "Open in Skills"
                                            }
                                            ItemType::McpServerConfig => "Open MCP settings",
                                            _ => "Open plugin",
                                        },
                                        Control::Dispatch(ImportAction::OpenItem(
                                            category,
                                            target.clone(),
                                        )),
                                        false,
                                        window,
                                        cx,
                                    )
                                })
                            } else {
                                None
                            };
                            rows.push(nested_row(
                                label,
                                detail,
                                control.unwrap_or_else(|| div().into_any_element()),
                                &t,
                            ));
                        }
                        if category == ItemType::Hooks {
                            rows.push(nested_row(
                                "Manage imported hooks",
                                None,
                                self.action(
                                    format!("manage-hooks-{id}"),
                                    "Open hooks settings",
                                    Control::Dispatch(ImportAction::OpenItem(
                                        ItemType::Hooks,
                                        String::new(),
                                    )),
                                    false,
                                    window,
                                    cx,
                                ),
                                &t,
                            ));
                        }
                    }
                }
            }
            entries.push(card(rows, self.surface(&t), &t));
        }
        if visible_count < histories.len() {
            entries.push(div().flex().justify_center().child(self.action(
                "more-import-history",
                "View more",
                Control::MoreHistory,
                false,
                window,
                cx,
            )));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(section_heading("Import history", None, &t))
                .child(div().flex().flex_col().gap(px(6.)).children(entries)),
        )
    }
    fn attention(
        &self,
        state: &ImportState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let Load::Ready(histories) = &state.histories else {
            return None;
        };
        let t = cx.theme().clone();
        let hooks = histories.iter().any(|history| {
            history
                .successes
                .iter()
                .any(|item| item.item_type == ItemType::Hooks)
        });
        let has_mcp = histories.iter().any(|history| {
            history
                .successes
                .iter()
                .any(|item| item.item_type == ItemType::McpServerConfig)
        });
        let names = match &state.mcp_auth {
            Load::Ready(names) => names.as_slice(),
            _ => &[],
        };
        let checking = has_mcp && !matches!(state.mcp_auth, Load::Ready(_));
        if !hooks && names.is_empty() && !checking {
            return None;
        }
        let mut rows = Vec::new();
        if checking {
            rows.push(match &state.mcp_auth {
                Load::Failed(_) => row(
                    "Couldn't check imported MCP servers",
                    None,
                    None,
                    self.action(
                        "retry-imported-mcps",
                        "Retry",
                        Control::Dispatch(ImportAction::Refresh),
                        state.pending,
                        window,
                        cx,
                    ),
                    &t,
                ),
                _ => row("Loading imported MCPs…", None, None, spinner(&t), &t),
            });
        }
        let active = if self.attention_tab == ItemType::Hooks && hooks || names.is_empty() {
            ItemType::Hooks
        } else {
            ItemType::McpServerConfig
        };
        if hooks || !names.is_empty() {
            let mut tabs = div()
                .id("import-attention-tabs")
                .px(px(16.))
                .pt(px(12.))
                .flex()
                .gap(px(8.))
                .role(gpui::Role::TabList)
                .aria_label("Imported setup categories");
            for (category, label) in [
                (ItemType::McpServerConfig, format!("MCPs ({})", names.len())),
                (ItemType::Hooks, "Hooks".into()),
            ] {
                if category == ItemType::Hooks && !hooks
                    || category == ItemType::McpServerConfig && names.is_empty()
                {
                    continue;
                }
                let handle =
                    focus::handle(format!("import-attention-{}", category.id()), 0, window, cx);
                tabs = tabs.child(
                    focus::ring(
                        div()
                            .id(SharedString::from(format!(
                                "import-attention-{}",
                                category.id()
                            )))
                            .role(gpui::Role::Tab)
                            .aria_label(label.clone())
                            .aria_selected(active == category)
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(12.))
                            .when(active == category, |d| d.bg(t.text_alpha_5))
                            .child(text(label, 13., 18., 500., t.text)),
                        &handle,
                        px(12.),
                        px(2.),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate(Control::Attention(category), window, cx)
                    }))
                    .on_key_down(cx.listener(
                        move |this, event: &KeyDownEvent, window, cx| {
                            if matches!(
                                event.keystroke.key.as_str(),
                                "enter" | "space" | "left" | "right"
                            ) {
                                this.activate(Control::Attention(category), window, cx);
                                cx.stop_propagation();
                            }
                        },
                    )),
                );
            }
            rows.push(div().child(tabs));
            if active == ItemType::Hooks {
                rows.push(row(
                    "Imported hooks",
                    Some("Review imported hooks before allowing them to run".into()),
                    Some(icon("settings-hooks", 16., t.text_secondary).into_any_element()),
                    self.action(
                        "review-imported-hooks",
                        "Open settings",
                        Control::Dispatch(ImportAction::OpenItem(ItemType::Hooks, String::new())),
                        false,
                        window,
                        cx,
                    ),
                    &t,
                ));
            } else {
                for name in names {
                    rows.push(row(
                        name.clone(),
                        Some(
                            "Authentication is required before this MCP server can be used".into(),
                        ),
                        Some(
                            icon("settings-configuration", 16., t.text_secondary)
                                .into_any_element(),
                        ),
                        self.action(
                            format!("authenticate-import-{name}"),
                            "Authenticate",
                            Control::Dispatch(ImportAction::AuthenticateMcp(name.clone())),
                            state.auth_urls.get(name).is_some_and(Option::is_none),
                            window,
                            cx,
                        ),
                        &t,
                    ));
                }
            }
        }
        Some(group(
            "Needs attention",
            Some("Finish setting up items from a previous import"),
            rows,
            self.surface(&t),
            &t,
        ))
    }

    fn review_row(
        &self,
        choice: &ReviewChoice,
        selected: &BTreeMap<String, bool>,
        tools_expanded: bool,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        let count = choice
            .keys
            .iter()
            .filter(|key| selected.get(*key) == Some(&true))
            .count();
        let checked = if count == choice.keys.len() {
            Some(true)
        } else if count == 0 {
            Some(false)
        } else {
            None
        };
        let extra = if choice.expandable {
            let handle = focus::handle("import-tools-details", 0, window, cx);
            let trigger = focus::ring(
                div()
                    .id("import-tools-details")
                    .role(gpui::Role::Button)
                    .aria_label(if tools_expanded {
                        "Hide Tools & setup details"
                    } else {
                        "Show Tools & setup details"
                    })
                    .aria_expanded(tools_expanded)
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(history_arrow(tools_expanded, RowKind::Standard, &theme)),
                &handle,
                px(4.),
                px(0.),
                cx,
            );
            Some(
                self.bind_control(trigger, Control::ToggleTools, disabled, cx)
                    .into_any_element(),
            )
        } else if let Some(tooltip) = &choice.tooltip {
            let id = format!("import-info-{}", choice.id);
            let handle = focus::handle(id.clone(), 0, window, cx)
                .read(cx)
                .handle
                .clone();
            let click_focus = handle.clone();
            let trigger = div()
                .id(id)
                .role(gpui::Role::Button)
                .track_focus(&handle)
                .aria_label(if choice.id == "projects" {
                    "What will be imported for Projects"
                } else {
                    "What will be imported for Chat sessions"
                })
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("import-info", 16., theme.text_secondary))
                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    click_focus.focus(window, cx);
                    cx.stop_propagation();
                });
            Some(
                crate::tooltip::tooltip(trigger, tooltip.clone(), None, crate::tooltip::Side::Top)
                    .delay(std::time::Duration::ZERO)
                    .side_offset(px(8.))
                    .focus_handle(&handle)
                    .into_any_element(),
            )
        } else {
            None
        };
        let label_control = Control::Select(choice.keys.clone());
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(12.))
            .when(choice.nested, |row| row.pl(px(32.)))
            .child(icon(choice.icon, 20., theme.text))
            .child(
                div()
                    .id(format!("import-label-{}", choice.id))
                    .min_w_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(text(choice.title.clone(), 14., 20., 430., theme.text))
                    .child(text(
                        choice.detail.clone(),
                        12.,
                        16.,
                        430.,
                        theme.text_secondary,
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !disabled {
                            this.activate(label_control.clone(), window, cx);
                        }
                    })),
            )
            .children(extra)
            .child(div().h(px(21.)).flex().items_center().child(self.checkbox(
                (
                    format!("import-choice-{}", choice.id),
                    choice.title.as_str(),
                ),
                checked,
                Control::Select(choice.keys.clone()),
                disabled,
                window,
                cx,
            )))
    }

    fn dialog_action(
        &self,
        (id, label): (&'static str, &'static str),
        control: Control,
        (fill, foreground, border): (gpui::Hsla, gpui::Hsla, Option<gpui::Hsla>),
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handle = focus::handle(id, 0, window, cx);
        let element = focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .h(px(32.))
                .px(px(17.))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(12.5, shapes::Corner::Squircle, fill, border))
                .child(text(label, 14., 18., 430., foreground))
                .when(disabled, |element| element.opacity(0.4)),
            &handle,
            px(12.5),
            px(0.),
            cx,
        );
        self.bind_control(element, control, disabled, cx)
            .into_any_element()
    }

    fn dialog(
        &self,
        state: &ImportState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let modal = self.modal.as_ref()?;
        let review = matches!(modal, Modal::Review { .. });
        let t = cx.theme().clone();
        let mut rows = Vec::new();
        let mut footer_rows = Vec::new();
        let mut focus_ids = Vec::new();
        let (title, subtitle, save, disabled): (SharedString, &str, Control, bool) = match modal {
            Modal::Review {
                provider,
                items,
                selected,
                choices,
                tools_expanded,
            } => {
                for choice in choices
                    .iter()
                    .filter(|choice| !choice.nested || *tools_expanded)
                {
                    rows.push(self.review_row(
                        choice,
                        selected,
                        *tools_expanded,
                        state.pending,
                        window,
                        cx,
                    ));
                    focus_ids.push(format!("import-choice-{}", choice.id));
                    if choice.expandable {
                        focus_ids.push("import-tools-details".into());
                    }
                    if choice.tooltip.is_some() {
                        focus_ids.push(format!("import-info-{}", choice.id));
                    }
                }
                (
                    "Select items to import".into(),
                    "Choose what to bring over. Your existing setup won’t be affected.",
                    Control::Import,
                    state.pending
                        || !items
                            .iter()
                            .any(|item| item.provider_id == *provider && item.selected(selected)),
                )
            }
            Modal::Customize {
                preferences,
                show_all,
            } => {
                let known = known_categories(state);
                for category in ItemType::ALL
                    .into_iter()
                    .filter(|category| *show_all || known.contains(category))
                {
                    let detail = match category {
                        ItemType::Config
                            if state
                                .sync_state
                                .provider_ids
                                .contains(&Provider::ClaudeCowork) =>
                        {
                            Some("Cowork tools sync together with Settings".into())
                        }
                        ItemType::Plugins => {
                            Some("Bundled skills and MCP servers sync with their plugin".into())
                        }
                        ItemType::Commands => Some("Imported as skills".into()),
                        _ => None,
                    };
                    rows.push(
                        div().child(
                            row(
                                category.label(),
                                detail,
                                Some(icon(category.icon(), 20., t.text).into_any_element()),
                                self.checkbox(
                                    (format!("sync-category-{}", category.id()), category.label()),
                                    preferences.category_checked(category, &state.sync_state),
                                    Control::Category(category),
                                    false,
                                    window,
                                    cx,
                                ),
                                &t,
                            )
                            .id(format!("sync-row-{}", category.id()))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.activate(Control::Category(category), window, cx);
                                },
                            )),
                        ),
                    );
                    focus_ids.push(format!("sync-category-{}", category.id()));
                }
                footer_rows.push(row(
                    "Sync all categories",
                    Some("Include categories found in the future".into()),
                    None,
                    self.toggle(
                        ("sync-all-categories", "Sync all categories"),
                        *preferences == Preferences::All,
                        Control::AllCategories,
                        false,
                        window,
                        cx,
                    ),
                    &t,
                ));
                focus_ids.push("sync-all-categories".into());
                if known.len() < ItemType::ALL.len() {
                    footer_rows.push(row(
                        "Show all categories",
                        Some("Choose what to sync when new categories become available".into()),
                        None,
                        self.toggle(
                            ("show-sync-categories", "Show all categories"),
                            *show_all,
                            Control::ShowCategories,
                            false,
                            window,
                            cx,
                        ),
                        &t,
                    ));
                    focus_ids.push("show-sync-categories".into());
                }
                footer_rows.push(
                    text(
                        "Changes apply to future syncs. Content already imported stays in ChatGPT",
                        13.,
                        16.,
                        430.,
                        t.text_secondary,
                    )
                    .whitespace_normal(),
                );
                let empty = !ItemType::ALL
                    .iter()
                    .any(|category| preferences.category_selected(*category, &state.sync_state));
                if empty {
                    footer_rows.push(text("Select at least one category. To stop syncing, turn off “Keep imports in sync”", 13., 16., 430., t.text_secondary).whitespace_normal());
                }
                (
                    "Content to sync".into(),
                    "Applies to all connected sources and projects",
                    Control::Save,
                    empty,
                )
            }
        };
        let cancel = self.dialog_action(
            ("cancel-import-dialog", "Cancel"),
            Control::Close,
            if review {
                (gpui::transparent_black(), t.text_tertiary, None)
            } else {
                (t.primary_soft_alpha, t.text, Some(t.border))
            },
            state.pending,
            window,
            cx,
        );
        let save = self.dialog_action(
            (
                "save-import-dialog",
                if matches!(modal, Modal::Review { .. }) {
                    "Import to ChatGPT"
                } else {
                    "Save"
                },
            ),
            save,
            (t.button_primary_solid, t.menu_background, Some(t.border)),
            disabled,
            window,
            cx,
        );
        focus_ids.extend(["cancel-import-dialog".into(), "save-import-dialog".into()]);
        let close = review.then(|| {
            focus_ids.push("close-import-review".into());
            let handle = focus::handle("close-import-review", 0, window, cx);
            self.bind_control(
                focus::ring(
                    div()
                        .id("close-import-review")
                        .role(gpui::Role::Button)
                        .aria_label("Close")
                        .absolute()
                        .top(px(16.))
                        .right(px(16.))
                        .size(px(24.))
                        .p(px(4.))
                        .child(icon("diff-tab-close", 16., t.text.opacity(0.8))),
                    &handle,
                    px(4.),
                    px(0.),
                    cx,
                ),
                Control::Close,
                state.pending,
                cx,
            )
            .into_any_element()
        });
        let focus_order: Vec<_> = focus_ids
            .into_iter()
            .map(|id: String| focus::handle(id, 0, window, cx).read(cx).handle.clone())
            .collect();
        let pending = state.pending;
        let viewport = window.viewport_size();
        let backdrop = shapes::backdrop_blur(
            px(24.),
            shapes::Fill::Color,
            false,
            shapes::Decoration {
                radius: gpui::Corners::all(px(25.)),
                corner: shapes::Corner::Squircle,
                fill: Some(gpui::Hsla {
                    a: 0.9,
                    ..t.surface_elevated_secondary
                }),
                shadows: vec![
                    crate::composer::shadow(t.border, 0., 0., 0.5, false),
                    crate::composer::shadow(gpui::rgba(0x0000001a).into(), 4., 8., -2., false),
                ],
                ..Default::default()
            },
        );
        let mut surface = div()
            .id("import-dialog")
            .role(gpui::Role::Dialog)
            .aria_label(title.clone())
            .track_focus(&self.modal_focus)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !pending {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
                if event.keystroke.key == "tab" {
                    let current = focus_order
                        .iter()
                        .position(|handle| handle.is_focused(window));
                    let next = if event.keystroke.modifiers.shift {
                        current
                            .filter(|index| *index > 0)
                            .map_or(focus_order.len() - 1, |index| index - 1)
                    } else {
                        current.map_or(0, |index| (index + 1) % focus_order.len())
                    };
                    focus_order[next].focus(window, cx);
                    cx.stop_propagation();
                }
            }))
            .w(px(
                (if review { 520_f32 } else { 680. }).min(f32::from(viewport.width) * 0.92)
            ))
            .max_h(px(720_f32.min(f32::from(viewport.height) - 64.)))
            .relative()
            .flex()
            .flex_col()
            .p(px(20.))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(title, 20., 28., 600., t.text))
                    .child(text(subtitle, 14., 21., 430., t.text_tertiary).whitespace_normal()),
            )
            .child(
                div()
                    .id("import-dialog-content")
                    .pt(px(if review { 16. } else { 12. }))
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(card(
                        rows,
                        on_card(
                            20.,
                            shapes::Corner::Squircle,
                            if review {
                                gpui::transparent_black()
                            } else {
                                t.background_panel
                            },
                            Some(t.border),
                        ),
                        &t,
                    )),
            )
            .child(
                div()
                    .pt(px(if review { 16. } else { 12. }))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(footer_rows)
                    .children(state.error.as_ref().map(|error| {
                        div()
                            .id("import-dialog-error")
                            .role(gpui::Role::Alert)
                            .child(
                                text(error.clone(), 14., 21., 430., t.text_error)
                                    .whitespace_normal(),
                            )
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(12.))
                            .child(cancel)
                            .child(save),
                    ),
            )
            .children(close)
            .into_any_element();
        let mut overlay = div()
            .w(viewport.width)
            .h(viewport.height)
            .occlude()
            .bg(gpui::rgba(0x00000022))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if !pending {
                        this.close(window, cx);
                    }
                }),
            )
            .into_any_element();
        Some(
            gpui::deferred(
                gpui::canvas(
                    move |_, window, cx| {
                        overlay.prepaint_as_root(
                            gpui::Point::default(),
                            viewport.into(),
                            window,
                            cx,
                        );
                        let size = surface.layout_as_root(viewport.into(), window, cx);
                        let height = size.height.round();
                        let origin = gpui::point(
                            (viewport.width - size.width) / 2.,
                            (viewport.height - height) / 2.,
                        );
                        let mut background = div()
                            .w(size.width)
                            .h(height)
                            .child(backdrop)
                            .into_any_element();
                        background.prepaint_as_root(origin, viewport.into(), window, cx);
                        surface.prepaint_at(origin, window, cx);
                        (overlay, background, surface)
                    },
                    |_, (mut overlay, mut background, mut surface), window, cx| {
                        overlay.paint(window, cx);
                        background.paint(window, cx);
                        surface.paint(window, cx);
                    },
                )
                .absolute()
                .top_0()
                .left_0(),
            )
            .into_any_element(),
        )
    }
}

impl Render for ImportPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = Self::state(cx);
        let t = cx.theme().clone();
        if !state.policy.enabled {
            return div().into_any_element();
        }
        if self.modal.is_some() && !state.modal_open {
            self.close(window, cx);
        }
        if state.import_completed != self.import_completed {
            self.import_completed = state.import_completed;
            if matches!(self.modal, Some(Modal::Review { .. })) {
                self.close(window, cx);
            }
        }
        if state.preferences_saved != self.preferences_saved {
            self.preferences_saved = state.preferences_saved;
            self.close(window, cx);
        }
        if let Load::Ready(histories) = &state.histories
            && let Some(latest) = histories.first()
            && self.history_head.as_deref() != Some(latest.import_id.as_str())
        {
            self.history_head = Some(latest.import_id.clone());
            self.expanded.extend(
                histories
                    .iter()
                    .filter(|history| latest.completed_at_ms - history.completed_at_ms <= 60_000)
                    .map(|history| history.import_id.clone()),
            );
        }
        let view = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| {
                let composited = this.scroll.max_offset().y > px(0.);
                if this.composited != composited {
                    this.composited = composited;
                    cx.notify();
                }
            });
        });
        let detection = self.detection(&state, window, cx);
        let sync = state.policy.sync.then(|| self.sync(&state, window, cx));
        let history = self.history(&state, window, cx);
        let attention = self.attention(&state, window, cx);
        let dialog = self.dialog(&state, window, cx);
        div().size_full().relative().flex().flex_col()
            .child(div().flex_none().h(px(46.)))
            .child(div().id("import-settings-page").flex_1().overflow_y_scroll().track_scroll(&self.scroll).when(self.composited, |d| d.layer()).child(probe(&self.layer))
                .child(div().pt(px(16.)).pb(px(1.)).child(div().w_full().max_w(px(768.)).mx_auto().px(px(20.))
                    .child(div().py(px(12.)).flex().flex_col().gap(px(4.))
                        .child(text("Import", 28., 33.6, 500., t.text))
                        .child(text("Bring setup, projects, and chats from other AI apps into ChatGPT", 14., 24., 430., t.text_secondary).whitespace_normal()))))
                .child(div().w_full().max_w(px(768.)).mx_auto().p(px(20.)).flex().flex_col().gap(px(40.))
                    .children(sync).child(detection).children(attention).children(history)
                    .children(state.error.as_ref().filter(|_| self.modal.is_none()).map(|error|
                        div().id("import-page-error").role(gpui::Role::Alert).child(text(error.clone(), 13., 18., 430., t.text_error).whitespace_normal())))))
            .children(dialog).into_any_element()
    }
}

fn review_choices(items: &[DetectedItem], provider: Provider) -> Vec<ReviewChoice> {
    let mut home = Vec::new();
    let mut projects = BTreeSet::new();
    let mut chats = 0;
    for (index, item) in items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.provider_id == provider)
    {
        if item.item.item_type == ItemType::Sessions {
            if let Some(sessions) = item
                .item
                .details
                .as_ref()
                .and_then(|details| details["sessions"].as_array())
            {
                chats += sessions.len();
                projects.extend(
                    sessions
                        .iter()
                        .filter(|session| session["workspaceKind"] != "projectless")
                        .filter_map(|session| session["cwd"].as_str())
                        .filter(|cwd| !cwd.is_empty()),
                );
            }
        } else if let Some(project) = item.project() {
            projects.insert(project);
        } else {
            let count = match item.item.item_type {
                ItemType::Skills => item
                    .item
                    .details
                    .as_ref()
                    .and_then(|details| details["skills"].as_array())
                    .map(Vec::len)
                    .unwrap_or(0),
                ItemType::Plugins => item
                    .item
                    .details
                    .as_ref()
                    .and_then(|details| details["plugins"].as_array())
                    .map(|plugins| {
                        plugins
                            .iter()
                            .filter_map(|plugin| plugin["pluginNames"].as_array())
                            .map(Vec::len)
                            .sum()
                    })
                    .unwrap_or(0),
                _ => 0,
            };
            let title = if count > 0 {
                format!("{} ({count})", item.item.item_type.label())
            } else {
                item.item.item_type.label().into()
            };
            home.push(ReviewChoice {
                id: format!("item-{index}"),
                keys: vec![item.key()],
                title,
                detail: item
                    .item
                    .description
                    .strip_prefix("Migrate ")
                    .unwrap_or(&item.item.description)
                    .replace(" into ", " to "),
                icon: item.item.item_type.icon(),
                tooltip: None,
                nested: true,
                expandable: false,
            });
        }
    }
    let mut choices = Vec::new();
    if !home.is_empty() {
        choices.push(ReviewChoice {
            id: "tools".into(),
            keys: home
                .iter()
                .flat_map(|choice| choice.keys.iter().cloned())
                .collect(),
            title: "Tools & setup".into(),
            detail: "Settings, instructions, plugins, skills".into(),
            icon: "import-category-braces",
            tooltip: None,
            nested: false,
            expandable: true,
        });
        choices.extend(home);
    }
    if !projects.is_empty() {
        choices.push(ReviewChoice {
            id: "projects".into(),
            keys: vec!["projects".into()],
            title: format!("Projects ({})", projects.len()),
            detail: "Use your existing project folders".into(),
            icon: "import-projects",
            tooltip: Some(
                projects
                    .iter()
                    .map(|path| {
                        std::path::Path::new(path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or(path)
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            nested: false,
            expandable: false,
        });
    }
    if chats > 0 {
        choices.push(ReviewChoice {
            id: "chats".into(),
            keys: vec!["chats".into()],
            title: format!("Chat sessions ({chats})"),
            detail: "Recent chats".into(),
            icon: "import-category-chats",
            tooltip: Some(format!(
                "{chats} recent {} from {} will be imported",
                if chats == 1 { "chat" } else { "chats" },
                provider.label()
            )),
            nested: false,
            expandable: false,
        });
    }
    choices
}

#[derive(Clone, Copy)]
enum RowKind {
    Standard,
    Nested,
}

fn imported_auth_count(history: &History, state: &ImportState, category: ItemType) -> usize {
    if category != ItemType::McpServerConfig {
        return 0;
    }
    match &state.mcp_auth {
        Load::Ready(names) => names
            .iter()
            .filter(|name| {
                history.successes.iter().any(|item| {
                    item.item_type == category && item.target.as_deref() == Some(name.as_str())
                })
            })
            .count(),
        _ => 0,
    }
}

fn history_warning_count(history: &History, state: &ImportState) -> usize {
    history
        .successes
        .iter()
        .filter(|item| item.item_type == ItemType::Hooks)
        .count()
        + imported_auth_count(history, state, ItemType::McpServerConfig)
}

fn category_status(
    category: ItemType,
    imported: usize,
    failed: usize,
    auth: usize,
    theme: &Theme,
) -> AnyElement {
    let review = category == ItemType::Hooks && imported > 0;
    let (label, color, dot) = if failed > 0 {
        (
            format!("{imported} imported · {failed} did not import"),
            theme.text_error,
            theme.text_error,
        )
    } else if auth > 0 {
        (
            format!(
                "{imported} imported · {auth} {} authentication",
                if auth == 1 { "needs" } else { "need" }
            ),
            theme.text_warning,
            theme.text_warning,
        )
    } else if review {
        (
            format!("{imported} imported · Needs review"),
            theme.text_warning,
            theme.text_warning,
        )
    } else {
        (
            format!("{imported} imported"),
            theme.text,
            theme.text_success,
        )
    };
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(8.))
        .when(failed > 0 || auth > 0 || review, |badge| {
            badge.px(px(8.)).py(px(4.))
        })
        .when(failed > 0, |badge| {
            badge.child(on_card(
                8.,
                shapes::Corner::Squircle,
                theme.text_error.opacity(0.1),
                None,
            ))
        })
        .child(div().size(px(8.)).relative().child(on_card(
            9999.,
            shapes::Corner::Round,
            dot,
            None,
        )))
        .child(text(label, 12., 16., 500., color))
        .into_any_element()
}

fn history_arrow(expanded: bool, kind: RowKind, theme: &Theme) -> AnyElement {
    let name = match (kind, expanded) {
        (RowKind::Standard, false) => "chevron-down",
        (RowKind::Standard, true) => "import-history-up",
        (RowKind::Nested, false) => "import-history-right",
        (RowKind::Nested, true) => "import-history-down",
    };
    icon(name, 14., theme.text_tertiary).into_any_element()
}

fn row(
    label: impl Into<SharedString>,
    detail: Option<String>,
    image: Option<AnyElement>,
    control: AnyElement,
    theme: &Theme,
) -> Div {
    row_content(
        label,
        detail.map(|detail| {
            text(detail, 12., 16., 430., theme.text_secondary)
                .whitespace_normal()
                .into_any_element()
        }),
        image,
        control,
        theme,
        RowKind::Standard,
    )
}

fn nested_row(
    label: impl Into<SharedString>,
    detail: Option<String>,
    control: AnyElement,
    theme: &Theme,
) -> Div {
    row_content(
        label,
        detail.map(|detail| {
            text(detail, 12., 16., 430., theme.text_secondary)
                .whitespace_normal()
                .into_any_element()
        }),
        None,
        control,
        theme,
        RowKind::Nested,
    )
}

fn row_content(
    label: impl Into<SharedString>,
    detail: Option<AnyElement>,
    image: Option<AnyElement>,
    control: AnyElement,
    theme: &Theme,
    kind: RowKind,
) -> Div {
    let nested = matches!(kind, RowKind::Nested);
    div()
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(if nested { 12. } else { 24. }))
        .px(px(16.))
        .py(px(if nested { 8. } else { 12. }))
        .when(nested, |row| row.min_h(px(40.)))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(12.))
                .children(image)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            text(
                                label,
                                13.,
                                18.571_428,
                                if nested { 430. } else { 500. },
                                theme.text,
                            )
                            .whitespace_normal(),
                        )
                        .children(detail),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .when(nested, |control| control.min_w_0().flex_1())
                .when(!nested, |control| control.flex_none().min_w(px(160.)))
                .child(control),
        )
}

fn group(
    title: &'static str,
    subtitle: Option<&'static str>,
    rows: Vec<Div>,
    surface: AnyElement,
    t: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .child(section_heading(title, subtitle, t))
        .child(card(rows, surface, t))
}

fn section_heading(title: &'static str, subtitle: Option<&'static str>, theme: &Theme) -> Div {
    if let Some(subtitle) = subtitle {
        div()
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(text(title, 16., 24.888_89, 500., theme.text))
            .child(text(subtitle, 13., 18., 400., theme.text_secondary))
    } else {
        div()
            .h(px(46.))
            .pb(px(6.))
            .flex()
            .items_center()
            .child(text(title, 14., 21., 500., theme.text))
    }
}

fn card(rows: Vec<Div>, surface: AnyElement, t: &Theme) -> Div {
    let last = rows.len().saturating_sub(1);
    div()
        .relative()
        .p(px(1.))
        .child(surface)
        .child(rounded_clip(
            px(19.),
            shapes::Corner::Squircle,
            div()
                .flex()
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    row.when(index < last, |row| {
                        row.child(
                            div()
                                .absolute()
                                .left(px(16.))
                                .right(px(16.))
                                .bottom_0()
                                .h(px(1.))
                                .bg(t.border),
                        )
                    })
                })),
        ))
}

fn provider_icon(provider: Provider, radius: f32, t: &Theme) -> AnyElement {
    let name = match provider {
        Provider::ClaudeCode => "import-claude-code",
        Provider::ClaudeCowork => "import-claude-cowork",
        Provider::Cursor => "import-cursor",
    };
    let orange = if t.appearance == theme::Appearance::Dark {
        0xfb6a22
    } else {
        0xe25507
    };
    div()
        .size(px(40.))
        .flex_none()
        .relative()
        .when(provider != Provider::Cursor, |d| {
            d.child(on_card(
                radius,
                shapes::Corner::Squircle,
                gpui::rgb(orange).into(),
                None,
            ))
        })
        .child(icon(
            name,
            40.,
            if provider == Provider::Cursor {
                t.text
            } else {
                t.text_inverse
            },
        ))
        .into_any_element()
}

fn spinner(t: &Theme) -> AnyElement {
    icon("sidebar-cmd-spinner", 16., t.text_secondary).into_any_element()
}

fn known_categories(state: &ImportState) -> BTreeSet<ItemType> {
    let mut categories = BTreeSet::new();
    if let Load::Ready(items) = &state.detected {
        categories.extend(
            items
                .iter()
                .filter(|item| state.sync_state.provider_ids.contains(&item.provider_id))
                .map(|item| item.item.item_type),
        );
    }
    if let Load::Ready(histories) = &state.histories {
        for history in histories.iter().filter(|history| {
            history
                .provider_id
                .is_some_and(|provider| state.sync_state.provider_ids.contains(&provider))
        }) {
            categories.extend(
                history
                    .successes
                    .iter()
                    .chain(&history.failures)
                    .map(|item| item.item_type),
            );
        }
    }
    categories.extend(ItemType::ALL.into_iter().filter(|category| {
        if *category == ItemType::Sessions {
            return state.sync_state.selection.contains_key("chats");
        }
        state.sync_state.selection.keys().any(|key| {
            let mut parts = key.split(':');
            let first = parts.next();
            let item_type = if first == Some("projects") {
                parts.next()
            } else {
                first
            };
            let provider = parts.next();
            item_type == Some(category.id())
                && state
                    .sync_state
                    .provider_ids
                    .iter()
                    .any(|id| Some(id.id()) == provider)
        })
    }));
    categories
}

fn preference_values(
    preferences: &Preferences,
    state: &ImportState,
) -> BTreeMap<ItemType, CategoryPreference> {
    match preferences {
        Preferences::Selected(selected) => selected.clone(),
        _ => known_categories(state)
            .into_iter()
            .map(|category| {
                (
                    category,
                    if *preferences == Preferences::All {
                        CategoryPreference::Enabled(true)
                    } else if state.sync_state.category_selected(category) {
                        CategoryPreference::Original
                    } else {
                        CategoryPreference::Enabled(false)
                    },
                )
            })
            .collect(),
    }
}

fn preference_summary(state: &ImportState) -> String {
    if state.preferences == Preferences::All {
        return "All categories, including new ones".into();
    }
    let selected: Vec<_> = ItemType::ALL
        .into_iter()
        .filter(|category| {
            state
                .preferences
                .category_selected(*category, &state.sync_state)
        })
        .collect();
    match selected.as_slice() {
        [] => "No content selected".into(),
        [ItemType::Sessions] => "Chats only".into(),
        items if items.len() > 3 => format!("{} categories selected", items.len()),
        items => items
            .iter()
            .map(|category| category.label())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn relative_time(timestamp: i64) -> String {
    let seconds = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        - timestamp / 1000)
        .max(0);
    if seconds < 60 {
        "less than a minute".into()
    } else if seconds < 3600 {
        format!("{} minutes", seconds / 60)
    } else if seconds < 86400 {
        format!("{} hours", seconds / 3600)
    } else {
        format!("{} days", seconds / 86400)
    }
}

fn history_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%b %-d, %Y, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, HeadlessAppContext, Keystroke, size};

    #[test]
    fn project_and_chat_choices_select_their_own_import_items() {
        let items = vec![
            DetectedItem {
                provider_id: Provider::ClaudeCode,
                item: bridge::imports::MigrationItem {
                    item_type: ItemType::AgentsMd,
                    description: "Project instructions".into(),
                    cwd: Some("/owned/project".into()),
                    details: None,
                },
            },
            DetectedItem {
                provider_id: Provider::ClaudeCode,
                item: bridge::imports::MigrationItem {
                    item_type: ItemType::Sessions,
                    description: "Recent chats".into(),
                    cwd: None,
                    details: Some(
                        serde_json::json!({"sessions":[{"cwd":"/owned/project","title":"Recorded chat"}]}),
                    ),
                },
            },
        ];
        let choices = review_choices(&items, Provider::ClaudeCode);
        assert_eq!(
            choices
                .iter()
                .map(|choice| choice.id.as_str())
                .collect::<Vec<_>>(),
            ["projects", "chats"]
        );
        let selected = BTreeMap::from([("projects".into(), false), ("chats".into(), true)]);
        let imported: Vec<_> = items
            .iter()
            .filter(|item| item.selected(&selected))
            .map(|item| item.item.item_type)
            .collect();
        assert_eq!(imported, [ItemType::Sessions]);
    }

    #[test]
    fn legacy_project_selection_does_not_invent_available_categories() {
        let item = DetectedItem {
            provider_id: Provider::ClaudeCode,
            item: bridge::imports::MigrationItem {
                item_type: ItemType::AgentsMd,
                description: "Recorded instructions".into(),
                cwd: Some("/owned/project".into()),
                details: None,
            },
        };
        let state = ImportState {
            detected: Load::Ready(Arc::new(vec![item])),
            sync_state: SyncState {
                version: 1,
                provider_ids: vec![Provider::ClaudeCode],
                selection: BTreeMap::from([("projects".into(), true)]),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            known_categories(&state),
            BTreeSet::from([ItemType::AgentsMd])
        );
    }

    #[test]
    fn customization_cancels_drafts_and_rejects_an_empty_selection() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let render = std::env::var_os("CGX_IMPORT_SHOTS").is_some();
        let mut cx = if render {
            HeadlessAppContext::with_platform(
                textsys::text_system(),
                Arc::new(theme::Assets),
                gpui_platform::current_headless_renderer,
            )
        } else {
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets))
        };
        let item = DetectedItem {
            provider_id: Provider::ClaudeCode,
            item: bridge::imports::MigrationItem {
                item_type: ItemType::Skills,
                description: "Fixture skill".into(),
                cwd: None,
                details: None,
            },
        };
        let mut sync_state = SyncState::default();
        sync_state.merge_import(
            std::slice::from_ref(&item),
            BTreeMap::from([(item.key(), true)]),
            1_000,
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            cx.set_global(ImportState {
                policy: Policy {
                    enabled: true,
                    sync: true,
                    cursor: true,
                },
                detected: Load::Ready(Arc::new(vec![item])),
                histories: Load::Ready(Arc::new(Vec::new())),
                mcp_auth: Load::Ready(Arc::new(Vec::new())),
                sync_state,
                preferences: Preferences::Original,
                ..Default::default()
            });
        });
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, cx| {
                cx.new(|cx| ImportPage {
                    layer: Layer::default(),
                    scroll: ScrollHandle::new(),
                    modal: None,
                    modal_focus: cx.focus_handle(),
                    return_focus: None,
                    expanded: BTreeSet::new(),
                    categories: BTreeSet::new(),
                    show_all_history: false,
                    history_head: None,
                    import_completed: 0,
                    preferences_saved: 0,
                    attention_tab: ItemType::McpServerConfig,
                    composited: false,
                })
            })
            .unwrap();
        cx.run_until_parked();
        let capture = |cx: &mut HeadlessAppContext, name: &str| {
            if let Some(directory) = std::env::var_os("CGX_IMPORT_SHOTS") {
                let path = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&path).unwrap();
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(path.join(name))
                    .unwrap();
            }
        };
        capture(&mut cx, "native-import.png");
        window
            .update(&mut cx, |page, window, cx| {
                page.activate(Control::Customize, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        capture(&mut cx, "native-customize.png");
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse("tab").unwrap(), cx);
            window.dispatch_keystroke(Keystroke::parse("space").unwrap(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| cx.update_global::<ImportState, _>(|state, _| state.import_completed += 1));
        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |page, window, cx| {
                let Some(Modal::Customize { preferences, .. }) = &page.modal else {
                    panic!("Customize must stay open")
                };
                assert!(
                    !preferences.category_selected(
                        ItemType::Skills,
                        &cx.global::<ImportState>().sync_state
                    )
                );
                page.activate(Control::Save, window, cx);
                assert!(page.modal.is_some());
                assert_eq!(
                    cx.global::<ImportState>().preferences,
                    Preferences::Original
                );
            })
            .unwrap();
        capture(&mut cx, "native-empty-selection.png");
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |page, window, cx| {
                assert!(page.modal.is_none());
                page.activate(Control::Customize, window, cx);
                let Some(Modal::Customize { preferences, .. }) = &page.modal else {
                    panic!("Customize must reopen")
                };
                assert!(
                    preferences.category_selected(
                        ItemType::Skills,
                        &cx.global::<ImportState>().sync_state
                    )
                );
            })
            .unwrap();
    }
}
