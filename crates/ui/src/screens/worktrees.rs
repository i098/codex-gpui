use super::{Layer, SettingsPage, button, deco, probe, settings_row, switch};
use crate::{
    composer::{Composer, tabular},
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, Global, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc, sync::Arc};
use theme::{ActiveTheme as _, Theme};

use super::computer_use::Load;

#[derive(Clone)]
pub enum Action {
    Refresh,
    NewChat(String),
    OpenThread(String),
    Delete(String),
}

pub type Dispatch = Arc<dyn Fn(Action, &mut Window, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub worktrees: Load<Arc<[Value]>>,
    pub default_root: String,
    pub pending_delete: Option<String>,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

pub fn render(page: Entity<SettingsPage>, window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state(
            SharedString::from(format!("worktrees-settings-{:?}", page.entity_id())),
            cx,
            |window, cx| Worktrees::new(page, window, cx),
        )
        .into_any_element()
}

struct Worktrees {
    page: Entity<SettingsPage>,
    root: Entity<Composer>,
    limit: Entity<Composer>,
    layer: Layer,
    confirmation: Option<Entity<Confirmation>>,
    environments: bool,
    scroll: gpui::ScrollHandle,
    composited: bool,
    heading_height: Rc<Cell<gpui::Pixels>>,
}

impl Worktrees {
    fn new(page: Entity<SettingsPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = page
            .read(cx)
            .choice("host.git-worktree-root", "")
            .to_owned();
        let limit = page
            .read(cx)
            .get("host.worktree-keep-count")
            .as_u64()
            .unwrap_or(15)
            .to_string();
        let placeholder = cx
            .try_global::<State>()
            .map(|state| state.default_root.clone())
            .unwrap_or_default();
        let root = cx.new(|cx| {
            let mut editor =
                Composer::settings_compact_field("", &root, cx).field_label("Worktree root");
            editor.set_placeholder(placeholder, cx);
            editor
        });
        let limit = cx.new(|cx| {
            Composer::settings_compact_field("15", &limit, cx)
                .small_input()
                .field_label("Auto-delete limit")
        });
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        cx.observe(&root, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<State>(|this, cx| {
            let placeholder = cx.global::<State>().default_root.clone();
            this.root
                .update(cx, |root, cx| root.set_placeholder(placeholder, cx));
            cx.notify();
        })
        .detach();
        cx.on_blur(&root.focus_handle(cx), window, |this, _, cx| {
            this.save_root(cx)
        })
        .detach();
        cx.on_blur(&limit.focus_handle(cx), window, |this, _, cx| {
            this.save_limit(cx)
        })
        .detach();
        if let Some(dispatch) = cx
            .try_global::<State>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(Action::Refresh, window, cx);
        }
        Self {
            page,
            root,
            limit,
            layer: Layer::default(),
            confirmation: None,
            environments: false,
            scroll: gpui::ScrollHandle::new(),
            composited: false,
            heading_height: Rc::new(Cell::new(px(0.))),
        }
    }

    fn dispatch(&self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dispatch) = cx
            .try_global::<State>()
            .and_then(|state| state.dispatch.clone())
        {
            dispatch(action, window, cx);
        }
    }

    fn save_root(&mut self, cx: &mut Context<Self>) {
        let value = self.root.read(cx).field_text().trim().to_owned();
        if !value.is_empty() && !std::path::Path::new(&value).is_absolute() {
            return;
        }
        self.root.update(cx, |root, cx| root.set_text(&value, cx));
        if self.page.read(cx).choice("host.git-worktree-root", "") != value {
            self.page.update(cx, |page, cx| {
                page.set("host.git-worktree-root", json!(value), cx)
            });
        }
    }

    fn save_limit(&mut self, cx: &mut Context<Self>) {
        if !self
            .page
            .read(cx)
            .flag("host.worktree-auto-cleanup-enabled", true)
        {
            return;
        }
        let value = self.limit.read(cx).field_text().trim().parse::<i64>();
        let value = value.map(|value| value.max(1)).unwrap_or_else(|_| {
            self.page
                .read(cx)
                .get("host.worktree-keep-count")
                .as_i64()
                .unwrap_or(15)
        });
        self.limit
            .update(cx, |limit, cx| limit.set_text(&value.to_string(), cx));
        if self
            .page
            .read(cx)
            .get("host.worktree-keep-count")
            .as_i64()
            .unwrap_or(15)
            != value
        {
            self.page.update(cx, |page, cx| {
                page.set("host.worktree-keep-count", json!(value), cx)
            });
        }
    }

    fn toggle_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .page
            .read(cx)
            .flag("host.worktree-auto-cleanup-enabled", true)
        {
            self.page.update(cx, |page, cx| {
                page.set("host.worktree-auto-cleanup-enabled", json!(true), cx)
            });
            return;
        }
        let dialog = cx.new(|cx| Confirmation::new("Disable automatic worktree deletion?", "We highly recommend keeping automatic deletion on so old worktrees do not build up and use unnecessary disk space. If you prefer to manage old worktrees yourself, you can turn this off and ChatGPT will stop deleting them automatically.", "Disable automatic deletion", window, cx).destructive().settings_style().cancel_label("Keep automatic deletion"));
        cx.subscribe(&dialog, |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.page.update(cx, |page, cx| {
                    page.set("host.worktree-auto-cleanup-enabled", json!(false), cx)
                });
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    fn preferences(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let cleanup = self
            .page
            .read(cx)
            .flag("host.worktree-auto-cleanup-enabled", true);
        let refresh = self
            .page
            .read(cx)
            .choice("host.worktree-upstream-refresh-mode", "never")
            == "best-effort";
        let root = div().w(px(288.)).child(self.root.clone());
        let limit = if cleanup {
            div()
                .w(px(96.))
                .child(self.limit.clone())
                .into_any_element()
        } else {
            div()
                .id("disabled-worktree-limit")
                .w(px(96.))
                .h(px(28.))
                .relative()
                .px(px(8.))
                .flex()
                .items_center()
                .role(gpui::Role::TextInput)
                .aria_label("Auto-delete limit")
                .aria_disabled(true)
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(10.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.text_alpha_5),
                    border: Some((px(1.), t.border_strong)),
                    ..Default::default()
                }))
                .child(tabular(text(
                    self.limit.read(cx).field_text().to_owned(),
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                )))
                .into_any_element()
        };
        let rows = vec![
            settings_row(
                "Worktree root",
                Some(
                    "Directory where ChatGPT creates managed worktrees. Leave blank to use the default location",
                ),
                root.into_any_element(),
                t,
            ),
            settings_row(
                "Always fetch upstream before creating worktrees",
                Some(
                    "Codex normally picks up branch updates during regular Git activity. This also fetches before each new worktree.",
                ),
                switch("worktree-upstream-refresh-mode", refresh, false, t)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page.update(cx, |page, cx| {
                            page.set(
                                "host.worktree-upstream-refresh-mode",
                                json!(if refresh { "never" } else { "best-effort" }),
                                cx,
                            )
                        })
                    }))
                    .into_any_element(),
                t,
            ),
            settings_row(
                "Automatically delete old worktrees",
                Some(
                    "Recommended for most users. Turn this off only if you want to manage old worktrees and disk usage yourself.",
                ),
                switch("worktree-auto-cleanup-enabled", cleanup, false, t)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_cleanup(window, cx)))
                    .into_any_element(),
                t,
            ),
            settings_row(
                "Auto-delete limit",
                Some(if cleanup {
                    "Number of managed worktrees to keep before older ones are pruned automatically. ChatGPT snapshots worktrees before deleting, so pruned worktrees should always be restorable."
                } else {
                    "Automatic deletion is disabled. ChatGPT will not prune old worktrees automatically. Re-enable it to use this saved limit again."
                }),
                limit,
                t,
            ),
        ];
        card(rows, self.composited.then_some(&self.layer), t)
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let worktrees = cx
            .try_global::<State>()
            .map(|state| state.worktrees.clone())
            .unwrap_or_default();
        if matches!(worktrees, Load::Loading) {
            return div()
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("sidebar-cmd-spinner", 16., t.text_secondary))
                .child(text(
                    "Fetching worktree details…",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ));
        }
        let mut refresh = Some(
            button("", gpui::transparent_black(), None, t)
                .id("refresh-worktrees")
                .w(px(28.))
                .aria_label("Refresh")
                .child(icon("diff-refresh", 16., t.text_secondary))
                .on_click(
                    cx.listener(|this, _, window, cx| this.dispatch(Action::Refresh, window, cx)),
                )
                .into_any_element(),
        );
        match worktrees {
            Load::Loading => unreachable!(),
            Load::Failed(error) => self.group(
                "Unable to load worktrees".into(),
                vec![
                    div()
                        .p(px(16.))
                        .child(text(error, 13., 18.571_428, 430., t.text_secondary)),
                ],
                refresh,
                t,
            ),
            Load::Ready(rows) if rows.is_empty() => self.group(
                "No worktrees yet".into(),
                vec![div().p(px(16.)).child(text(
                    "Worktrees created by ChatGPT will appear here",
                    13.,
                    18.571_428,
                    430.,
                    t.text_secondary,
                ))],
                refresh,
                t,
            ),
            Load::Ready(rows) => {
                let mut repositories = std::collections::BTreeMap::<&str, Vec<&Value>>::new();
                for row in rows.iter() {
                    let repository = row["repositoryRoot"]
                        .as_str()
                        .or_else(|| row["dir"].as_str())
                        .unwrap_or("Unknown repository");
                    repositories.entry(repository).or_default().push(row);
                }
                let mut groups = Vec::with_capacity(repositories.len());
                for (repository, mut rows) in repositories {
                    rows.sort_by_cached_key(|row| {
                        std::cmp::Reverse(
                            row["conversations"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|thread| {
                                    thread["isPinned"] == true || thread["isSubagent"] != true
                                })
                                .count(),
                        )
                    });
                    let rows = rows
                        .into_iter()
                        .map(|row| self.worktree(row, t, cx))
                        .collect();
                    groups.push(self.group(repository.to_owned().into(), rows, refresh.take(), t));
                }
                div().flex().flex_col().gap(px(40.)).children(groups)
            }
        }
    }

    fn group(
        &self,
        title: SharedString,
        rows: Vec<gpui::Div>,
        action: Option<AnyElement>,
        t: &Theme,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .min_h(px(46.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text(title, 14., 21., 500., t.text))
                    .children(action),
            )
            .child(card(rows, self.composited.then_some(&self.layer), t))
    }

    fn worktree(&self, row: &Value, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let path = row["dir"].as_str().unwrap_or_default().to_owned();
        let conversations = row["conversations"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let pinned = conversations
            .iter()
            .any(|thread| thread["isPinned"] == true);
        let deleting = cx
            .try_global::<State>()
            .is_some_and(|state| state.pending_delete.as_deref() == Some(&path));
        let new_path = path.clone();
        let remove_path = path.clone();
        let hover = t.list_hover;
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(text("Worktree", 14., 20., 500., t.text))
                            .child(text(path, 12., 16., 430., t.text_secondary))
                            .child(text(
                                if pinned {
                                    "Unpin the chats in this worktree before deleting it"
                                } else {
                                    "Starts a fresh chat using the same files and branch"
                                },
                                12.,
                                16.,
                                430.,
                                t.text_secondary,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button("New chat in this worktree", t.text_alpha_5, None, t)
                                    .id(SharedString::from(format!("new-chat-{new_path}")))
                                    .role(gpui::Role::Button)
                                    .aria_label("New chat in this worktree")
                                    .aria_disabled(deleting)
                                    .opacity(if deleting { 0.4 } else { 1. })
                                    .when(!deleting, |button| {
                                        button.on_click(cx.listener(move |this, _, window, cx| {
                                            this.dispatch(
                                                Action::NewChat(new_path.clone()),
                                                window,
                                                cx,
                                            )
                                        }))
                                    }),
                            )
                            .child(
                                button(
                                    if deleting { "Deleting…" } else { "Delete" },
                                    t.text_alpha_5,
                                    None,
                                    t,
                                )
                                .id(SharedString::from(format!("delete-{remove_path}")))
                                .role(gpui::Role::Button)
                                .aria_label("Delete")
                                .aria_disabled(pinned || deleting)
                                .opacity(if pinned { 0.4 } else { 1. })
                                .when(
                                    !pinned && !deleting,
                                    |button| {
                                        button.on_click(cx.listener(move |this, _, window, cx| {
                                            this.dispatch(
                                                Action::Delete(remove_path.clone()),
                                                window,
                                                cx,
                                            )
                                        }))
                                    },
                                ),
                            ),
                    ),
            )
            .child(text(
                "Known conversations",
                12.,
                16.,
                430.,
                t.text_secondary,
            ))
            .when(conversations.is_empty(), |row| {
                row.child(text(
                    "No known conversations linked to this worktree",
                    12.,
                    16.,
                    430.,
                    t.text_secondary,
                ))
            })
            .children(
                conversations
                    .iter()
                    .filter(|thread| thread["isPinned"] == true || thread["isSubagent"] != true)
                    .map(|thread| {
                        let id = thread["id"].as_str().unwrap_or_default().to_owned();
                        let title = thread["title"]
                            .as_str()
                            .filter(|title| !title.is_empty())
                            .unwrap_or("Untitled conversation")
                            .to_owned();
                        div()
                            .id(SharedString::from(format!("worktree-thread-{id}")))
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .hover(move |s| s.bg(hover))
                            .child(text(title, 14., 20., 430., t.text))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dispatch(Action::OpenThread(id.clone()), window, cx)
                            }))
                    }),
            )
    }
}

impl Render for Worktrees {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let view = cx.entity().downgrade();
        let heading_height = self.heading_height.get();
        window.on_next_frame(move |_, cx| {
            let _ = view.update(cx, |this, cx| {
                let composited = this.scroll.max_offset().y > px(0.);
                if this.composited != composited || this.heading_height.get() != heading_height {
                    this.composited = composited;
                    cx.notify();
                }
            });
        });
        let heading_measure = self.heading_height.clone();
        // A relative shift sticks the tabs without growing the scroll height; the clamp ignores
        // overscroll past the end.
        let offset = (-self.scroll.offset().y).min(self.scroll.max_offset().y);
        let shift = (offset - self.heading_height.get()).max(px(0.));
        let content = if self.environments {
            div().child(super::environments::render(
                SharedString::from(format!("environment-{:?}", self.page.entity_id())),
                &self.layer,
                self.composited,
                window,
                cx,
            ))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(40.))
                .child(self.preferences(&t, cx))
                .child(self.list(&t, cx))
        };
        let heading = div().min_h(px(60.)).py(px(12.)).flex().flex_col().justify_center().gap(px(4.))
            .child(text("Worktrees", 28., 33.6, 500., t.text))
            .when(self.environments, |heading| {
                heading.child(div().flex().flex_wrap()
                    .child(text("Local environments tell ChatGPT how to set up worktrees for a project. ", 14., 24., 430., t.text_secondary))
                    .child(div().id("environments-guide")
                        .child(text("Learn more", 14., 24., 430., t.text_accent))
                        .on_click(|_, _, cx| cx.open_url("https://developers.openai.com/codex/app/local-environments"))))
            });
        let tabs = div()
            .relative()
            .top(shift)
            .bg(t.surface)
            .pt(px(20.))
            .pb(px(8.))
            .flex()
            .gap(px(2.))
            .children(
                [(false, "General"), (true, "Environments")].map(|(environments, label)| {
                    div()
                        .id(label)
                        .px(px(8.))
                        .py(px(4.))
                        .relative()
                        .when(self.environments == environments, |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: gpui::Corners::all(px(10.)),
                                corner: shapes::Corner::Squircle,
                                fill: Some(t.surface_secondary),
                                ..Default::default()
                            }))
                        })
                        .child(text(
                            label,
                            13.,
                            18.571_428,
                            500.,
                            if self.environments == environments {
                                t.text
                            } else {
                                t.text_secondary
                            },
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.environments = environments;
                            cx.notify();
                        }))
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(gpui::relative(1.))
                    .left_0()
                    .right_0()
                    .h(px(32.))
                    .bg(gpui::linear_gradient(
                        180.,
                        gpui::linear_color_stop(t.surface, 0.),
                        gpui::linear_color_stop(gpui::transparent_black(), 1.),
                    )
                    .color_space(gpui::ColorSpace::Oklab)),
            );
        let page = div()
            .id("worktrees-page")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .when(self.composited, |d| d.layer())
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(probe(&self.layer))
            .child(
                div()
                    .relative()
                    .pt(px(16.))
                    .pb(px(1.))
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| heading_measure.set(bounds.size.height),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .px(px(20.))
                            .child(heading),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .max_w(px(768.))
                    .mx_auto()
                    .px(px(20.))
                    .pb(px(20.))
                    .child(gpui::deferred(tabs))
                    .child(div().pt(px(20.)).child(content)),
            );
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter"
                    || event.keystroke.modifiers.platform && event.keystroke.key == "s"
                {
                    this.save_root(cx);
                    this.save_limit(cx);
                }
            }))
            .child(div().h(px(46.)).flex_none())
            .child(page)
            .children(self.confirmation.as_ref().map(|dialog| {
                let viewport = window.viewport_size();
                gpui::deferred(
                    gpui::anchored()
                        .position(gpui::point(px(0.), px(0.)))
                        .child(
                            div()
                                .relative()
                                .w(viewport.width)
                                .h(viewport.height)
                                .child(dialog.clone()),
                        ),
                )
            }))
    }
}

pub(super) fn card(rows: Vec<gpui::Div>, layer: Option<&Layer>, t: &Theme) -> gpui::Div {
    let count = rows.len();
    let decoration = if let Some(layer) = layer {
        deco(
            layer,
            20.,
            shapes::Corner::Squircle,
            t.background_panel,
            Some(t.border),
        )
    } else {
        shapes::decoration(shapes::Decoration {
            radius: gpui::Corners::all(px(20.)),
            corner: shapes::Corner::Squircle,
            fill: Some(t.background_panel),
            border: Some((px(1.), t.border)),
            ..Default::default()
        })
        .into_any_element()
    };
    div()
        .relative()
        .flex()
        .flex_col()
        .p(px(1.))
        .child(decoration)
        .children(rows.into_iter().enumerate().map(|(index, row)| {
            row.relative().when(index + 1 < count, |row| {
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
        }))
}
