//! Host-confirmed Computer use settings.
//! The `available` field is a platform and feature gate, not a permission grant.
//! The app replaces confirmed state after each successful host operation.

use super::{Layer, button, deco, on_card, probe, select, switch};
use crate::{composer::popover, focus, icon::icon, text};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gpui::{
    AnyElement, App, Context, Div, FocusHandle, Global, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Window, div, img, prelude::FluentBuilder as _, px,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

pub(crate) fn decode_app_image(url: &str) -> Option<Arc<gpui::Image>> {
    let (header, data) = url.strip_prefix("data:")?.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    let format = gpui::ImageFormat::from_mime_type(mime)?;
    let bytes = BASE64.decode(data).ok()?;
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

fn app_image(app: &AppApproval, window: &mut Window, cx: &mut App) -> Option<Arc<gpui::Image>> {
    let url = app.icon_data_url.as_deref()?;
    let key = SharedString::from(format!("computer-use-icon-{}", app.bundle_identifier));
    data_image(url, key, window, cx)
}

pub(crate) fn data_image(
    url: &str,
    key: SharedString,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<gpui::Image>> {
    let cache = window.use_keyed_state(key, cx, |_, _| (url.to_owned(), decode_app_image(url)));
    cache.update(cx, |cache, _| {
        if cache.0 != url {
            *cache = (url.to_owned(), decode_app_image(url));
        }
        cache.1.clone()
    })
}

#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppApproval {
    pub bundle_identifier: String,
    pub display_name: String,
    #[serde(rename = "iconDataURL")]
    pub icon_data_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SoundMode {
    ForegroundClicks,
    ForegroundAndBackgroundClicks,
    Off,
}

impl SoundMode {
    fn label(self) -> &'static str {
        match self {
            Self::ForegroundClicks => "Play sounds for foreground clicks",
            Self::ForegroundAndBackgroundClicks => {
                "Play sounds for foreground and background clicks"
            }
            Self::Off => "Don’t play sounds",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeApp {
    Excel,
    PowerPoint,
    Word,
}

impl OfficeApp {
    pub const ALL: [Self; 3] = [Self::Excel, Self::PowerPoint, Self::Word];

    /// Lowercase stem of the add-in plugin name the host matches on.
    pub fn plugin_name(self) -> &'static str {
        match self {
            Self::Excel => "excel",
            Self::PowerPoint => "powerpoint",
            Self::Word => "word",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Excel => "Microsoft Excel",
            Self::PowerPoint => "Microsoft PowerPoint",
            Self::Word => "Microsoft Word",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Excel => "Let ChatGPT use Microsoft Excel add-in for additional control",
            Self::PowerPoint => "Let ChatGPT use Microsoft PowerPoint add-in for additional control",
            Self::Word => "Let ChatGPT use Microsoft Word add-in for additional control",
        }
    }
}

/// An installed Office add-in plugin; `enabled` is the shared document-control connector flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfficeControl {
    pub app: OfficeApp,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub installed: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum ComputerUseAction {
    Refresh,
    Install,
    SetEnabled(bool),
    SetSoundMode(SoundMode),
    SetOfficeEnabled(bool),
    RemoveApproval(String),
    OpenPermissions,
}

pub type Dispatch = Arc<dyn Fn(ComputerUseAction, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ComputerUseState {
    pub available: bool,
    pub plugin: Load<Option<Plugin>>,
    pub approvals: Load<Vec<AppApproval>>,
    pub sound: Option<SoundMode>,
    pub office: Vec<OfficeControl>,
    pub pending: bool,
    pub error: Option<String>,
    pub permission_guidance: Option<String>,
    pub dispatch: Option<Dispatch>,
}

impl Global for ComputerUseState {}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    let page = window.use_keyed_state("computer-use-settings", cx, |_, cx| {
        cx.observe_global::<ComputerUseState>(|_, cx| cx.notify())
            .detach();
        cx.observe_global::<crate::screens::computer_use_native::NativeSettingsState>(|_, cx| {
            cx.notify()
        })
        .detach();
        ComputerUse {
            sound_open: false,
            removal: None,
            layer: Layer::default(),
            modal_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            composited: false,
        }
    });
    page.into_any_element()
}

struct ComputerUse {
    sound_open: bool,
    removal: Option<AppApproval>,
    layer: Layer,
    modal_focus: FocusHandle,
    scroll: ScrollHandle,
    composited: bool,
}

impl ComputerUse {
    fn dispatch(&mut self, action: ComputerUseAction, cx: &mut Context<Self>) {
        if cx
            .try_global::<ComputerUseState>()
            .is_some_and(|state| state.pending)
        {
            return;
        }
        let dispatch = cx
            .try_global::<ComputerUseState>()
            .and_then(|s| s.dispatch.clone());
        if let Some(dispatch) = dispatch {
            dispatch(action, cx);
        } else {
            let mut state = cx
                .try_global::<ComputerUseState>()
                .cloned()
                .unwrap_or_default();
            state.error = Some("Computer use settings are unavailable.".into());
            cx.set_global(state);
        }
        cx.notify();
    }

    fn action(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        action: ComputerUseAction,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id: SharedString = id.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            button(label, t.text_alpha_5, None, &t)
                .id(id)
                .when(disabled, |d| d.opacity(0.6)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.dispatch(action.clone(), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.dispatch(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn control(
        &self,
        state: &ComputerUseState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let t = cx.theme().clone();
        let mut rows = match &state.plugin {
            Load::Loading => vec![message("Loading plugins…", &t)],
            Load::Failed(error) => vec![row(
                error.clone(),
                None,
                None,
                self.action(
                    "retry-computer-use",
                    "Retry",
                    ComputerUseAction::Refresh,
                    state.pending,
                    window,
                    cx,
                ),
                &t,
            )],
            Load::Ready(Some(plugin)) if state.available => {
                let control = if plugin.installed {
                    let enabled = plugin.enabled;
                    let disabled = state.pending;
                    let handle = focus::handle("computer-use-enabled", 0, window, cx);
                    focus::ring(
                        switch("computer-use-enabled", enabled, disabled, &t),
                        &handle,
                        px(10.),
                        px(2.),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !disabled {
                            this.dispatch(ComputerUseAction::SetEnabled(!enabled), cx);
                        }
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.dispatch(ComputerUseAction::SetEnabled(!enabled), cx);
                            cx.stop_propagation();
                        }
                    }))
                    .into_any_element()
                } else {
                    self.action(
                        "install-computer-use",
                        "Install",
                        ComputerUseAction::Install,
                        state.pending,
                        window,
                        cx,
                    )
                };
                vec![row(
                    "Any App",
                    Some("Let ChatGPT control apps on your computer".into()),
                    Some(
                        div()
                            .size(px(40.))
                            .relative()
                            .p(px(1.))
                            .child(on_card(
                                12.5,
                                shapes::Corner::Squircle,
                                gpui::Hsla {
                                    a: 0.,
                                    ..t.background_panel
                                },
                                Some(t.border),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("settings-plugins", 38., t.text_secondary))
                            .into_any_element(),
                    ),
                    control,
                    &t,
                )]
            }
            Load::Ready(_) => Vec::new(),
        };
        if matches!(state.plugin, Load::Ready(_)) {
            rows.push(chrome_row(&t));
        }
        if state.available {
            rows.extend(
                state
                    .office
                    .iter()
                    .map(|control| self.office_row(*control, state.pending, window, cx)),
            );
        }
        rows
    }

    fn office_row(
        &self,
        control: OfficeControl,
        pending: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let id = SharedString::from(format!("office-{}", control.app.plugin_name()));
        let enabled = control.enabled;
        let handle = focus::handle(id.clone(), 0, window, cx);
        let toggle = focus::ring(
            switch(id, enabled, pending, &t),
            &handle,
            px(10.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !pending {
                this.dispatch(ComputerUseAction::SetOfficeEnabled(!enabled), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.dispatch(ComputerUseAction::SetOfficeEnabled(!enabled), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element();
        // ponytail: the original's app icons are not extracted; a letter badge stands in.
        let badge = div()
            .size(px(40.))
            .rounded(px(12.5))
            .border_1()
            .border_color(t.border)
            .flex()
            .items_center()
            .justify_center()
            .child(text(
                control.app.title()[10..11].to_owned(),
                16.,
                20.,
                600.,
                t.text_secondary,
            ))
            .into_any_element();
        row(
            control.app.title(),
            Some(control.app.description().into()),
            Some(badge),
            toggle,
            &t,
        )
    }

    fn approvals(
        &self,
        state: &ComputerUseState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let t = cx.theme().clone();
        match &state.approvals {
            Load::Loading => vec![message("Loading allowed apps…", &t)],
            Load::Failed(_) => vec![message("Unable to load allowed apps.", &t)],
            Load::Ready(apps) if apps.is_empty() => vec![message("None yet", &t)],
            Load::Ready(apps) => apps
                .iter()
                .map(|app| {
                    let image = match app_image(app, window, cx) {
                        Some(image) => img(image).size(px(36.)).rounded(px(6.)).into_any_element(),
                        None => div()
                            .size(px(36.))
                            .rounded(px(6.))
                            .bg(gpui::Hsla { a: 0.1, ..t.text })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text(
                                app.display_name
                                    .chars()
                                    .next()
                                    .unwrap_or(' ')
                                    .to_uppercase()
                                    .to_string(),
                                14.,
                                20.,
                                600.,
                                t.text_secondary,
                            ))
                            .into_any_element(),
                    };
                    let app = app.clone();
                    let id = SharedString::from(format!("remove-{}", app.bundle_identifier));
                    let handle = focus::handle(id.clone(), 0, window, cx);
                    let pending = state.pending;
                    let remove = focus::ring(
                        div()
                            .id(id)
                            .size(px(28.))
                            .rounded(px(12.5))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(pending, |d| d.opacity(0.6))
                            .child(icon("close", 16., t.text_secondary)),
                        &handle,
                        px(12.5),
                        px(2.),
                        cx,
                    )
                    .on_click(cx.listener({
                        let app = app.clone();
                        move |this, _, window, cx| {
                            if !pending {
                                this.removal = Some(app.clone());
                                this.modal_focus.focus(window, cx);
                                cx.notify();
                            }
                        }
                    }))
                    .on_key_down(cx.listener({
                        let app = app.clone();
                        move |this, event: &KeyDownEvent, window, cx| {
                            if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space")
                            {
                                this.removal = Some(app.clone());
                                this.modal_focus.focus(window, cx);
                                cx.notify();
                                cx.stop_propagation();
                            }
                        }
                    }))
                    .into_any_element();
                    row(app.display_name, None, Some(image), remove, &t)
                })
                .collect(),
        }
    }

    fn sound(
        &self,
        mode: SoundMode,
        pending: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let open = self.sound_open;
        let handle = focus::handle("computer-use-sound", 0, window, cx);
        let trigger = focus::ring(
            select(mode.label(), 430., None, pending, false, &t).id("computer-use-sound"),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !pending {
                this.sound_open = !open;
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.sound_open = !open;
                cx.notify();
                cx.stop_propagation();
            }
        }));
        div()
            .relative()
            .child(trigger)
            .when(open, |d| {
                d.child(gpui::deferred(
                    div()
                        .absolute()
                        .top(px(32.))
                        .left_0()
                        .child(popover(&t, 420., |d| {
                            d.children(
                                [
                                    SoundMode::ForegroundClicks,
                                    SoundMode::ForegroundAndBackgroundClicks,
                                    SoundMode::Off,
                                ]
                                .into_iter()
                                .map(|choice| {
                                    let handle = focus::handle(choice.label(), 0, window, cx);
                                    focus::ring(
                                        div()
                                            .id(choice.label())
                                            .px(px(8.))
                                            .py(px(6.))
                                            .rounded(px(12.))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap(px(12.))
                                            .hover(|s| s.bg(t.menu_item_hover))
                                            .child(text(choice.label(), 14., 20., 430., t.text))
                                            .when(choice == mode, |d| {
                                                d.child(icon("check", 16., t.text))
                                            }),
                                        &handle,
                                        px(12.),
                                        px(2.),
                                        cx,
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sound_open = false;
                                        this.dispatch(ComputerUseAction::SetSoundMode(choice), cx);
                                    }))
                                    .on_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, _, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                this.sound_open = false;
                                                this.dispatch(
                                                    ComputerUseAction::SetSoundMode(choice),
                                                    cx,
                                                );
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                                }),
                            )
                        })),
                ))
            })
            .into_any_element()
    }

    fn confirmation(
        &self,
        app: &AppApproval,
        state: &ComputerUseState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let pending = state.pending;
        let cancel_handle = focus::handle("cancel-remove-app", 0, window, cx)
            .read(cx)
            .handle
            .clone();
        let confirm_handle = focus::handle("confirm-remove-app", 0, window, cx)
            .read(cx)
            .handle
            .clone();
        let cancel = button("Cancel", t.text_alpha_5, None, &t)
            .id("cancel-remove-app")
            .track_focus(&cancel_handle)
            .on_click(cx.listener(move |this, _, _, cx| {
                if !pending {
                    this.removal = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.removal = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }));
        let remove = self.action(
            "confirm-remove-app",
            "Remove",
            ComputerUseAction::RemoveApproval(app.bundle_identifier.clone()),
            pending,
            window,
            cx,
        );
        gpui::deferred(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .bg(gpui::black().opacity(0.4))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(448.))
                        .p(px(24.))
                        .rounded(px(20.))
                        .bg(t.background_panel)
                        .track_focus(&self.modal_focus)
                        .on_key_down(move |event, window, cx| {
                            if event.keystroke.key == "tab" {
                                if cancel_handle.is_focused(window) {
                                    confirm_handle.focus(window, cx);
                                } else {
                                    cancel_handle.focus(window, cx);
                                }
                                cx.stop_propagation();
                            }
                        })
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(text(
                            format!("Remove “{}” from always allowed apps?", app.display_name),
                            18.,
                            24.,
                            600.,
                            t.text,
                        ).whitespace_normal())
                        .child(text(
                            format!(
                                "ChatGPT will ask to use “{}” in the next computer use session.",
                                app.display_name
                            ),
                            14.,
                            20.,
                            430.,
                            t.text_secondary,
                        ).whitespace_normal())
                        .children(
                            state
                                .error
                                .as_ref()
                                .map(|error| text(error.clone(), 13., 18., 430., t.text_secondary).whitespace_normal()),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(8.))
                                .child(cancel)
                                .child(remove),
                        ),
                ),
        )
        .into_any_element()
    }
}

impl Render for ComputerUse {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx
            .try_global::<ComputerUseState>()
            .cloned()
            .unwrap_or_default();
        if self.removal.as_ref().is_some_and(|removed| matches!(&state.approvals, Load::Ready(apps) if !apps.iter().any(|app| app.bundle_identifier == removed.bundle_identifier))) {
            self.removal = None;
        }
        let control = self.control(&state, window, cx);
        let locked = state
            .available
            .then(|| {
                crate::screens::computer_use_native::locked_use_row(window, cx).map(|row| {
                    card(vec![row], self.composited.then_some(&self.layer), &t).mt(px(6.))
                })
            })
            .flatten();
        let approvals = state.available.then(|| self.approvals(&state, window, cx));
        let sound = state
            .available
            .then_some(state.sound)
            .flatten()
            .map(|mode| self.sound(mode, state.pending, window, cx));
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
        div().size_full().relative().flex().flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.sound_open = false;
                    if !cx.try_global::<ComputerUseState>().is_some_and(|s| s.pending) { this.removal = None; }
                    cx.notify();
                }
            }))
            .child(div().h(px(46.)).flex_none())
            .child(div().id("computer-use-page").flex_1().overflow_y_scroll()
                .track_scroll(&self.scroll)
                .when(self.composited, |d| d.layer())
                .child(probe(&self.layer))
                .child(div().pt(px(16.)).pb(px(1.))
                    .child(div().w_full().max_w(px(768.)).mx_auto().px(px(20.))
                        .child(div().py(px(12.)).flex().flex_col().justify_center().gap(px(4.))
                            .child(text("Computer use", 28., 33.6, 500., t.text))
                            .child(text("Manage how ChatGPT uses other applications on your computer", 14., 24., 430., t.text_secondary)))))
                .child(div().w_full().max_w(px(768.)).mx_auto().p(px(20.))
                    .flex().flex_col().gap(px(40.))
                    .child(group("Control", control, self.composited.then_some(&self.layer), &t).children(locked))
                    .children(state.permission_guidance.as_ref().map(|guidance| row(guidance.clone(), None, None,
                        self.action("computer-use-permissions", "Open System Settings", ComputerUseAction::OpenPermissions, state.pending, window, cx), &t)))
                    .children(state.error.as_ref().map(|error| message(error.clone(), &t)))
                    .children(approvals.map(|rows| group("Always-allowed apps", rows, self.composited.then_some(&self.layer), &t)))
                    .children(sound.map(|menu| div().flex().child(menu)))))
            .children(self.removal.as_ref().map(|app| self.confirmation(app, &state, window, cx)))
    }
}

fn row(
    title: impl Into<SharedString>,
    detail: Option<String>,
    leading: Option<AnyElement>,
    control: AnyElement,
    t: &Theme,
) -> Div {
    div()
        .min_h(px(60.))
        .px(px(16.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(12.))
        .children(leading)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(text(title, 13., 18.571_428, 500., t.text))
                .children(detail.map(|detail| text(detail, 12., 16., 430., t.text_secondary))),
        )
        .child(control)
}

fn chrome_row(t: &Theme) -> Div {
    // ponytail: no browser host service yet; retain the original not-installed state.
    let chrome = icon("computer-use-chrome", 38., t.text);
    div()
        .min_h(px(60.))
        .px(px(16.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(24.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .size(px(40.))
                        .relative()
                        .p(px(1.))
                        .child(on_card(
                            12.5,
                            shapes::Corner::Squircle,
                            gpui::Hsla {
                                a: 0.,
                                ..t.background_panel
                            },
                            Some(t.border),
                        ))
                        .child(chrome),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text("Google Chrome", 13., 18.571_428, 500., t.text))
                        .child(
                            div().h(px(16.5)).relative().child(
                                div()
                                    .absolute()
                                    .top(px(0.5))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().size(px(8.)).relative().child(on_card(
                                        9999.,
                                        shapes::Corner::Round,
                                        t.text_error,
                                        None,
                                    )))
                                    .child(text(
                                        "Browser extension not installed",
                                        12.,
                                        16.,
                                        430.,
                                        t.text_secondary,
                                    )),
                            ),
                        ),
                ),
        )
        .child(div())
}

fn message(message: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .p(px(16.))
        .child(text(message, 13., 18.571_428, 430., t.text_secondary))
}

pub(super) fn group(title: &'static str, rows: Vec<Div>, layer: Option<&Layer>, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(46.))
                .pb(px(6.))
                .flex()
                .items_center()
                .child(text(title, 14., 21., 500., t.text)),
        )
        .child(card(rows, layer, t))
}

pub(super) fn card(rows: Vec<Div>, layer: Option<&Layer>, t: &Theme) -> Div {
    let last = rows.len().saturating_sub(1);
    let surface = match layer {
        Some(layer) => deco(
            layer,
            20.,
            shapes::Corner::Squircle,
            t.background_panel,
            Some(t.border),
        ),
        None => on_card(
            20.,
            shapes::Corner::Squircle,
            t.background_panel,
            Some(t.border),
        ),
    };
    div()
        .relative()
        .p(px(1.))
        .child(surface)
        .child(crate::rounded_clip(
            px(19.),
            shapes::Corner::Squircle,
            div()
                .flex()
                .flex_col()
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    row.relative().when(index < last, |row| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, BorrowAppContext as _, HeadlessAppContext, size};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn native_icons_decode_in_memory_and_reject_network_urls() {
        let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let uri = format!("data:image/png;base64,{}", BASE64.encode(png.into_inner()));
        let icon = decode_app_image(&uri).unwrap();
        let decoded = image::load_from_memory(&icon.bytes).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(1, 1), &image::Rgba([255, 0, 0, 255]));
        assert!(decode_app_image("data:image/png;base64,invalid!").is_none());
        assert!(decode_app_image("https://example.com/icon.png").is_none());
    }

    #[test]
    fn failed_writes_keep_confirmed_settings_and_removal_dialog() {
        let mut cx = if std::env::var_os("CGX_COMPUTER_USE_SHOTS").is_some() {
            HeadlessAppContext::with_platform(
                textsys::text_system(),
                Arc::new(theme::Assets),
                gpui_platform::current_headless_renderer,
            )
        } else {
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets))
        };
        let requests = Arc::new(AtomicUsize::new(0));
        let sent = requests.clone();
        let app = AppApproval {
            bundle_identifier: "com.example.Fixture".into(),
            display_name: "Fixture".into(),
            icon_data_url: None,
        };
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            cx.set_global(ComputerUseState {
                available: true,
                plugin: Load::Ready(Some(Plugin {
                    installed: true,
                    enabled: true,
                })),
                approvals: Load::Ready(vec![app.clone()]),
                sound: Some(SoundMode::ForegroundClicks),
                dispatch: Some(Arc::new(move |_, cx| {
                    sent.fetch_add(1, Ordering::Relaxed);
                    cx.update_global::<ComputerUseState, _>(|state, _| {
                        state.error = Some("The host rejected this change.".into());
                    });
                })),
                ..Default::default()
            });
        });
        let page = cx.update(|cx| {
            cx.new(|cx| ComputerUse {
                sound_open: false,
                removal: None,
                layer: Layer::default(),
                modal_focus: cx.focus_handle(),
                scroll: ScrollHandle::new(),
                composited: false,
            })
        });
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, _| page.clone())
            .unwrap();
        cx.run_until_parked();
        let capture = |cx: &mut HeadlessAppContext, name: &str| {
            if let Some(path) = std::env::var_os("CGX_COMPUTER_USE_SHOTS") {
                let path = std::path::PathBuf::from(path);
                std::fs::create_dir_all(&path).unwrap();
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(path.join(name))
                    .unwrap();
            }
        };
        capture(&mut cx, "native-dark.png");
        let click_toggle = |cx: &mut HeadlessAppContext| {
            cx.update_window(window.into(), |_, window, cx| {
                let position = gpui::point(px(750.), px(248.));
                window.dispatch_event(
                    gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                        button: gpui::MouseButton::Left,
                        position,
                        modifiers: Default::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                        button: gpui::MouseButton::Left,
                        position,
                        modifiers: Default::default(),
                        click_count: 1,
                    }),
                    cx,
                );
            })
            .unwrap();
            cx.run_until_parked();
        };
        click_toggle(&mut cx);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        cx.update(|cx| page.update(cx, |view, cx| {
            view.dispatch(ComputerUseAction::SetSoundMode(SoundMode::Off), cx);
            view.removal = Some(app.clone());
            view.dispatch(ComputerUseAction::RemoveApproval(app.bundle_identifier.clone()), cx);
            let state = cx.global::<ComputerUseState>();
            assert!(matches!(state.plugin, Load::Ready(Some(Plugin { enabled: true, .. }))));
            assert_eq!(state.sound, Some(SoundMode::ForegroundClicks));
            assert!(matches!(&state.approvals, Load::Ready(apps) if apps[0].bundle_identifier == app.bundle_identifier));
            assert_eq!(view.removal.as_ref().unwrap().bundle_identifier, app.bundle_identifier);
            assert_eq!(state.error.as_deref(), Some("The host rejected this change."));
            cx.notify();
        }));
        cx.run_until_parked();
        capture(&mut cx, "native-error-dark.png");
        click_toggle(&mut cx);
        assert_eq!(
            requests.load(Ordering::Relaxed),
            3,
            "the modal must block the underlying switch"
        );
    }
}
