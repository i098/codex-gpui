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
use theme::Theme;

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

#[derive(Default)]
pub struct Updates {
    pub available: bool,
    pub managed: bool,
    pub status: Status,
}

impl Global for Updates {}

actions!(updater, [InstallUpdate]);

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
                .on_key_down(|event, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space")
                        && !event.keystroke.modifiers.platform
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt
                    {
                        window.dispatch_action(Box::new(InstallUpdate), cx);
                        cx.stop_propagation();
                    }
                })
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
