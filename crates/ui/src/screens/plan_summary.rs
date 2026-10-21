use crate::{
    icon::icon,
    icon_button, text,
    tooltip::{Side, tooltip},
};
use gpui::{
    Action, App, ClipboardItem, Context, Corners, Div, IntoElement, ParentElement, Render, Styled,
    Task, Window, canvas, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use theme::{ActiveTheme as _, Theme};

use super::thread::view::{gradient_image, render_markdown};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub conversation_id: String,
    pub host_id: String,
    pub item_id: String,
    pub content: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = plan_summary, no_json)]
pub struct OpenPlan(pub Plan);

#[derive(Clone, PartialEq, Action)]
#[action(namespace = plan_summary, no_json)]
pub struct DownloadPlan(pub String);

pub fn download(content: String, cx: &mut App) -> Task<Result<(), String>> {
    let directory = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join("Downloads");
    let prompt = cx.prompt_for_new_path(&directory, Some("PLAN.md"));
    cx.background_spawn(async move {
        match prompt
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
        {
            Some(path) => std::fs::write(path, content).map_err(|error| error.to_string()),
            None => Ok(()),
        }
    })
}

pub struct PlanSummary {
    plan: Plan,
    collapsed: bool,
    panel: bool,
    error: Option<String>,
}

impl PlanSummary {
    pub fn page(plan: Plan) -> Self {
        Self {
            plan,
            collapsed: false,
            panel: false,
            error: None,
        }
    }

    pub fn panel(plan: Plan) -> Self {
        Self {
            panel: true,
            ..Self::page(plan)
        }
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    fn download(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let task = download(self.plan.content.clone(), cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.error = result.err();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }
}

fn control(
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> crate::tooltip::Tooltip {
    tooltip(
        icon_button(id, icon(glyph, 16., t.text), t)
            .tab_index(0)
            .focus(|style| style.border_2().border_color(gpui::rgb(0x3a83f7)))
            .on_key_down(crate::focus::tab)
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                on_click(event, window, cx);
            }),
        name,
        None,
        Side::Bottom,
    )
}

fn frame(content: Div, preview: bool, t: &Theme) -> Div {
    let background = if preview {
        t.surface_elevated_secondary.opacity(0.5)
    } else {
        t.text_alpha_5
    };
    div()
        .relative()
        .w_full()
        .overflow_hidden()
        .rounded(Theme::RADIUS_LG)
        .when(preview, |d| d.p(px(1.)).max_h(px(200.)))
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(Theme::RADIUS_LG),
            fill: Some(background),
            border: preview.then_some((px(1.), t.border)),
            ..Default::default()
        }))
        .child(content)
}

pub fn preview(plan: Plan, completed: bool, open: bool, window: &Window, t: &Theme) -> Div {
    let title = if completed { "Plan" } else { "Writing plan" };
    let mut header = div()
        .h(px(40.))
        .px(px(12.))
        .py(px(8.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("plan-lightbulb", 16., t.text_tertiary))
                .child(text(title, 14., 17.5, 400., t.text_tertiary)),
        );
    if completed {
        let label = if open {
            "Close plan side panel"
        } else {
            "Open plan in side panel"
        };
        let mut controls = div().flex().items_center().gap(px(4.));
        if !open && !plan.content.trim().is_empty() {
            controls = controls
                .child(control(
                    "plan-download",
                    "Download plan",
                    "plan-download",
                    t,
                    {
                        let content = plan.content.clone();
                        move |_, window, cx| {
                            window.dispatch_action(Box::new(DownloadPlan(content.clone())), cx)
                        }
                    },
                ))
                .child(control("plan-copy", "Copy", "thread-copy", t, {
                    let content = plan.content.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(content.clone()))
                    }
                }));
        }
        header = header.child(controls.child(control(
            "plan-open",
            label,
            if open { "plan-close" } else { "plan-open" },
            t,
            {
                let plan = plan.clone();
                move |_, window, cx| window.dispatch_action(Box::new(OpenPlan(plan.clone())), cx)
            },
        )));
    }
    let mut content = div().child(header);
    if !open {
        content = content.child(
            div()
                .relative()
                .max_h(px(160.))
                .overflow_hidden()
                .layer()
                .child(div().px(px(16.)).py(px(12.)).child(render_markdown(
                    &plan.content,
                    window,
                    t,
                )))
                .child(shapes::bottom_fade(px(64.))),
        );
    }
    div().child(
        frame(content, true, t)
            .id(gpui::SharedString::from(format!("plan-{}", plan.item_id)))
            .on_click(move |_, window, cx| {
                if completed {
                    window.dispatch_action(Box::new(OpenPlan(plan.clone())), cx);
                }
            }),
    )
}

impl Render for PlanSummary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        if !self.panel
            && (self.plan.conversation_id.is_empty()
                || self.plan.host_id.is_empty()
                || self.plan.content.is_empty())
        {
            return div()
                .id("plan-summary-page")
                .size_full()
                .p(px(20.))
                .bg(t.surface)
                .child(
                    div()
                        .rounded(Theme::RADIUS_2XL)
                        .border_1()
                        .border_color(t.border)
                        .bg(t.surface_tertiary.opacity(0.5))
                        .child(
                            div()
                                .h(px(56.))
                                .px(px(16.))
                                .py(px(12.))
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(
                                    div()
                                        .size(px(32.))
                                        .rounded(Theme::RADIUS_MD)
                                        .bg(t.text_alpha_10),
                                )
                                .child(
                                    div()
                                        .w(px(96.))
                                        .h(px(16.))
                                        .rounded(Theme::RADIUS_XS)
                                        .bg(t.text.opacity(0.2)),
                                ),
                        )
                        .child(
                            div()
                                .px(px(16.))
                                .py(px(16.))
                                .flex()
                                .flex_col()
                                .gap(px(12.))
                                .children([5. / 6., 4. / 6., 0.5].map(|width| {
                                    div()
                                        .w(gpui::relative(width))
                                        .h(px(12.))
                                        .rounded(Theme::RADIUS_XS)
                                        .bg(t.text_alpha_10)
                                })),
                        ),
                );
        }
        let body =
            div()
                .px(px(16.))
                .py(px(12.))
                .child(render_markdown(&self.plan.content, window, t));
        if self.panel {
            return div()
                .id("plan-panel")
                .size_full()
                .overflow_y_scroll()
                .px(px(4.))
                .child(body);
        }
        let mut controls = div().flex().items_center().gap(px(4.));
        if !self.plan.content.trim().is_empty() {
            controls = controls
                .child(control(
                    "plan-download",
                    "Download plan",
                    "plan-download",
                    t,
                    cx.listener(Self::download),
                ))
                .child(control("plan-copy", "Copy", "thread-copy", t, {
                    let content = self.plan.content.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(content.clone()))
                    }
                }));
        }
        controls = controls.child(control(
            "plan-collapse",
            if self.collapsed {
                "Expand plan summary"
            } else {
                "Collapse plan summary"
            },
            if self.collapsed {
                "plan-expand"
            } else {
                "chevron-down"
            },
            t,
            cx.listener(Self::toggle),
        ));
        let header = div()
            .h(px(40.))
            .px(px(12.))
            .py(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(text("Plan", 14., 17.5, 600., t.text))
            .child(controls);
        let mut content = div().relative().child(header).child(if self.collapsed {
            body.h(px(320.)).overflow_hidden()
        } else {
            body
        });
        if self.collapsed {
            let base = crate::over(t.text_alpha_5, t.surface);
            let color = t.primary_soft_alpha;
            let fade = canvas(
                move |bounds, window, _| {
                    gradient_image(
                        &[(color, 0.), (color.opacity(0.), 1.)],
                        base,
                        (bounds.size.width.into(), 160.),
                        (bounds.left().into(), bounds.top().into()),
                        true,
                        window.scale_factor(),
                    )
                },
                |bounds, image, window, _| {
                    window
                        .paint_image(bounds, bounds, Corners::default(), image, 0, false)
                        .ok();
                },
            )
            .absolute()
            .bottom_0()
            .w_full()
            .h(px(160.))
            .top_layer();
            content = content.child(fade);
            content = content.child(
                div()
                    .absolute()
                    .bottom(px(12.))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .id("plan-expand")
                            .tab_index(0)
                            .on_key_down(crate::focus::tab)
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(Theme::RADIUS_LG)
                            .bg(t.text)
                            .child(text("Expand plan", 14., 20., 500., t.surface))
                            .on_click(cx.listener(Self::toggle)),
                    ),
            );
        }
        div()
            .id("plan-summary-page")
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .font_family(crate::SANS)
            .bg(t.surface)
            .child(frame(content, false, t))
            .children(self.error.as_ref().map(|error| {
                text(
                    format!("Could not download plan: {error}"),
                    14.,
                    21.,
                    430.,
                    t.text,
                )
            }))
    }
}
