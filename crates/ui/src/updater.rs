use crate::{
    dispatch, focus, icon,
    tooltip::{Side, tooltip},
};
use gpui::{
    Animation, AnimationExt as _, App, Context, Div, Global, InteractiveElement, IntoElement,
    ParentElement, Role, StatefulInteractiveElement, Styled, Transformation, Window, actions, div,
    prelude::FluentBuilder as _, px, radians,
};
use std::time::Duration;
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    Downloading(Option<u8>),
    Ready,
    Installing,
}

impl Status {
    pub fn label(self) -> Option<gpui::SharedString> {
        match self {
            Self::Idle | Self::Checking => None,
            Self::Ready => Some("Update available".into()),
            Self::Installing => Some("Installing".into()),
            Self::Downloading(None) => Some("Downloading".into()),
            Self::Downloading(Some(percent)) => Some(format!("Downloading {percent}%").into()),
        }
    }
}

/// The update Sparkle found: its display version and release notes page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub title: gpui::SharedString,
    pub notes: Option<gpui::SharedString>,
}

#[derive(Default)]
pub struct Updates {
    pub available: bool,
    pub managed: bool,
    pub automatic: bool,
    pub status: Status,
    pub release: Option<Release>,
    /// The release whose sidebar banner the user dismissed.
    pub dismissed: Option<Release>,
}

impl Global for Updates {}

actions!(updater, [InstallUpdate, ToggleAutomaticChecks]);

fn glyph(status: Status) -> gpui::AnyElement {
    match status {
        Status::Ready => crate::icon::icon("updater-download", 16., gpui::white())
            .runtime()
            .composited(Some(gpui::point(px(0.), Theme::TITLEBAR_HEIGHT)))
            .into_any_element(),
        Status::Downloading(Some(percent)) => {
            let length = std::f64::consts::TAU * 7.;
            let src = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="2" fill="none" opacity=".16"/><circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="2" fill="none" opacity="{}" stroke-linecap="round" stroke-dasharray="{length}" stroke-dashoffset="{}" transform="rotate(-90 8 8)"/></svg>"#,
                if percent == 0 { 0 } else { 1 },
                length * f64::from(100 - percent.min(100)) / 100.
            );
            gpui::svg()
                .data(src.as_bytes())
                .size(px(16.))
                .text_color(gpui::white())
                .into_any_element()
        }
        _ => icon("sidebar-cmd-spinner", 16., gpui::white())
            .with_animation(
                "updater-spinner",
                Animation::new(Duration::from_secs(2))
                    .with_easing(crate::motion::steps(60))
                    .repeat(),
                |icon, phase| {
                    icon.with_transformation(Transformation::rotate(radians(
                        phase * std::f32::consts::TAU,
                    )))
                },
            )
            .into_any_element(),
    }
}

pub fn rail<T: 'static>(
    t: &Theme,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Option<gpui::AnyElement> {
    let updates = cx.try_global::<Updates>()?;
    if !updates.available || updates.managed {
        return None;
    }
    let status = updates.status;
    let label = status.label()?;
    let ready = status == Status::Ready;
    let handle = focus::handle("rail-update", 0, window, cx);
    let button = focus::ring(div().id("rail-update"), &handle, px(12.5), px(1.), cx)
        .debug_selector(|| "rail-update".into())
        .size(px(36.))
        .mb(px(8.))
        .flex_none()
        .rounded(px(12.5))
        .flex()
        .items_center()
        .justify_center()
        .aria_label(label.clone())
        .when(ready, |d| {
            d.role(Role::Button)
                .cursor_pointer()
                .hover(|d| d.bg(t.ghost_hover))
                .on_click(dispatch::<InstallUpdate>)
                .on_key_down(install_on_key)
        })
        .when(!ready, |d| {
            d.role(Role::ProgressIndicator)
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(100.)
        })
        .when_some(
            match status {
                Status::Downloading(percent) => percent,
                _ => None,
            },
            |d, percent| d.aria_numeric_value(f64::from(percent)),
        )
        .child(
            div()
                .relative()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .child(crate::in_layer(
                    shapes::Decoration {
                        radius: gpui::Corners::all(px(9999.)),
                        fill: Some(t.info_solid),
                        ..Default::default()
                    },
                    || gpui::point(px(0.), Theme::TITLEBAR_HEIGHT),
                ))
                .child(glyph(status)),
        );
    Some(tooltip(button, label, None, Side::Right).into_any_element())
}

pub fn managed_row(t: &Theme, cx: &App) -> Option<Div> {
    cx.try_global::<Updates>()
        .filter(|updates| updates.managed)
        .map(|_| {
            crate::screens::settings::settings_row(
                "In-app updates",
                Some("Your organization has turned off in-app updates"),
                crate::text("Managed", 13., 18.571_428, 430., t.text_secondary).into_any_element(),
                t,
            )
        })
}

pub fn automatic_row(t: &Theme, cx: &App) -> Option<Div> {
    let updates = cx.try_global::<Updates>()?;
    if !updates.available || updates.managed {
        return None;
    }
    Some(crate::screens::settings::settings_row(
        "Automatically check for updates",
        Some("Look for a new version of Codex in the background"),
        crate::screens::settings::switch("updates.automatic", updates.automatic, false, t)
            .on_click(dispatch::<ToggleAutomaticChecks>)
            .into_any_element(),
        t,
    ))
}

/// The sidebar banner (`sidebar.updateBanner`): the new version, its download progress,
/// `Restart to update` once staged, `Read more` for the release notes, and dismiss.
pub fn banner<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Option<gpui::AnyElement> {
    let updates = cx.try_global::<Updates>()?;
    let release = updates.release.clone()?;
    let status = updates.status;
    if !updates.available
        || updates.managed
        || updates.dismissed.as_ref() == Some(&release)
        || matches!(status, Status::Idle | Status::Checking)
    {
        return None;
    }
    let progress = match status {
        Status::Downloading(Some(percent)) => Some(format!("Downloading update… {percent}%")),
        Status::Downloading(None) => Some("Downloading update…".into()),
        Status::Installing => Some("Installing…".into()),
        _ => None,
    };
    let t = &cx.theme().clone();
    let restart = focus::handle("update-banner-restart", 0, window, cx);
    let dismissed = release.clone();
    let card = div()
        .id("update-banner")
        .debug_selector(|| "update-banner".into())
        .role(Role::Group)
        .aria_label("Update")
        .mx(px(8.))
        .mb(px(8.))
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .rounded(px(16.))
        .bg(t.surface_elevated)
        .border_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    crate::text(
                        format!("New Codex update available: {}", release.title),
                        13.,
                        18.,
                        530.,
                        t.text,
                    )
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal(),
                )
                .child(
                    div()
                        .id("update-banner-dismiss")
                        .debug_selector(|| "update-banner-dismiss".into())
                        .role(Role::Button)
                        .aria_label("Dismiss release announcement")
                        .size(px(20.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|d| d.bg(t.ghost_hover))
                        .on_click(move |_, _, cx| {
                            cx.global_mut::<Updates>().dismissed = Some(dismissed.clone());
                            cx.refresh_windows();
                        })
                        .child(icon("close", 14., t.text_tertiary)),
                ),
        )
        .children(progress.map(|label| {
            let percent = match status {
                Status::Downloading(Some(percent)) => f32::from(percent.min(100)) / 100.,
                _ => 0.,
            };
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(crate::text(label, 12., 16., 430., t.text_secondary))
                .child(
                    div()
                        .id("update-banner-progress")
                        .role(Role::ProgressIndicator)
                        .aria_label("Update download progress")
                        .aria_min_numeric_value(0.)
                        .aria_max_numeric_value(100.)
                        .aria_numeric_value(f64::from(percent * 100.))
                        .h(px(4.))
                        .w_full()
                        .rounded(px(2.))
                        .bg(t.ghost_hover)
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(percent))
                                .rounded(px(2.))
                                .bg(t.info_solid),
                        ),
                )
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(status == Status::Ready, |d| {
                    d.child(
                        focus::ring(
                            div().id("update-banner-restart"),
                            &restart,
                            px(8.),
                            px(1.),
                            cx,
                        )
                        .debug_selector(|| "update-banner-restart".into())
                        .role(Role::Button)
                        .h(px(28.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(8.))
                        .bg(t.button_primary_solid)
                        .hover(|d| d.bg(t.button_primary_solid_hover))
                        .cursor_pointer()
                        .on_click(dispatch::<InstallUpdate>)
                        .on_key_down(install_on_key)
                        .child(crate::text(
                            "Restart to update",
                            13.,
                            18.,
                            530.,
                            t.surface,
                        )),
                    )
                })
                .children(release.notes.map(|url| {
                    div()
                        .id("update-banner-notes")
                        .debug_selector(|| "update-banner-notes".into())
                        .role(Role::Link)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .child(crate::text("Read more", 13., 18., 430., t.text_accent))
                })),
        );
    Some(card.into_any_element())
}

/// Enter or Space on a focused install control installs the staged update.
fn install_on_key(event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut App) {
    if matches!(event.keystroke.key.as_str(), "enter" | "space")
        && !event.keystroke.modifiers.platform
        && !event.keystroke.modifiers.control
        && !event.keystroke.modifiers.alt
    {
        window.dispatch_action(Box::new(InstallUpdate), cx);
        cx.stop_propagation();
    }
}
