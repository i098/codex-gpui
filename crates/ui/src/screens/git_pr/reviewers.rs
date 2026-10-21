use crate::{composer::Composer, icon::icon, text};
use gpui::{
    Anchor, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Point,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, img,
    prelude::FluentBuilder as _, px,
};
use std::{collections::HashSet, time::Duration};
use theme::ActiveTheme as _;

#[derive(Clone)]
pub struct Reviewer {
    pub login: SharedString,
    pub avatar_url: Option<SharedString>,
    pub requested: bool,
    pub team_slug: Option<SharedString>,
    pub submitted: bool,
}

pub enum Event {
    Search {
        query: String,
        epoch: u64,
    },
    Request(Vec<String>),
    Remove {
        reviewers: Vec<String>,
        teams: Vec<String>,
    },
    Dismiss,
}

pub struct ReviewerPicker {
    query: Entity<Composer>,
    existing: Vec<Reviewer>,
    selected: Vec<Reviewer>,
    results: Vec<Reviewer>,
    search: String,
    epoch: u64,
    loading: bool,
    error: bool,
    highlighted: Option<usize>,
    position: Point<Pixels>,
    focus: FocusHandle,
}

impl EventEmitter<Event> for ReviewerPicker {}
impl Focusable for ReviewerPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl ReviewerPicker {
    pub fn new(existing: Vec<Reviewer>, position: Point<Pixels>, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut field = Composer::bare_field("Request review from…", "", false, 430., cx);
            field.set_field_label("Search GitHub users");
            field
        });
        cx.observe(&query, |this, query, cx| {
            let value = query.read(cx).field_text().trim().to_owned();
            if value == this.search {
                return;
            }
            this.search = value.clone();
            this.epoch += 1;
            this.loading = !value.is_empty();
            this.error = false;
            this.highlighted = None;
            let epoch = this.epoch;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                this.update(cx, |this, cx| {
                    if this.epoch == epoch {
                        cx.emit(Event::Search {
                            query: value,
                            epoch,
                        });
                    }
                })
                .ok();
            })
            .detach();
            cx.notify();
        })
        .detach();
        Self {
            query,
            existing,
            selected: Vec::new(),
            results: Vec::new(),
            search: String::new(),
            epoch: 0,
            loading: false,
            error: false,
            highlighted: None,
            position,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_results(
        &mut self,
        epoch: u64,
        results: Result<Vec<Reviewer>, String>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch {
            return;
        }
        self.loading = false;
        self.error = results.is_err();
        self.results = results.unwrap_or_default();
        self.highlighted = None;
        cx.notify();
    }

    pub fn set_existing(&mut self, existing: Vec<Reviewer>, cx: &mut Context<Self>) {
        self.existing = existing;
        cx.notify();
    }

    fn options(&self) -> Vec<Reviewer> {
        if self.error || self.loading {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        self.existing
            .iter()
            .chain(self.selected.iter())
            .chain(self.results.iter())
            .filter(|reviewer| seen.insert(reviewer.login.to_lowercase()))
            .cloned()
            .collect()
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(reviewer) = self.options().get(index).cloned() else {
            return;
        };
        if self
            .existing
            .iter()
            .any(|existing| existing.login.eq_ignore_ascii_case(&reviewer.login))
        {
            if !reviewer.requested {
                return;
            }
            match reviewer.team_slug {
                Some(team) => cx.emit(Event::Remove {
                    reviewers: Vec::new(),
                    teams: vec![team.to_string()],
                }),
                None => cx.emit(Event::Remove {
                    reviewers: vec![reviewer.login.to_string()],
                    teams: Vec::new(),
                }),
            }
        } else if let Some(index) = self
            .selected
            .iter()
            .position(|selected| selected.login.eq_ignore_ascii_case(&reviewer.login))
        {
            self.selected.remove(index);
        } else {
            self.selected.push(reviewer);
        }
        cx.notify();
    }

    fn submit(&self, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.emit(Event::Request(
                self.selected
                    .iter()
                    .map(|reviewer| reviewer.login.to_string())
                    .collect(),
            ));
        }
    }

    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.options().len();
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.emit(Event::Dismiss);
                cx.stop_propagation();
            }
            "down" if count > 0 => {
                self.highlighted = Some(self.highlighted.map_or(0, |index| (index + 1) % count));
                cx.notify();
                cx.stop_propagation();
            }
            "up" if count > 0 => {
                self.highlighted = Some(
                    self.highlighted
                        .map_or(count - 1, |index| (index + count - 1) % count),
                );
                cx.notify();
                cx.stop_propagation();
            }
            "enter" if self.highlighted.is_some() => {
                self.choose(self.highlighted.unwrap(), cx);
                cx.stop_propagation();
            }
            "enter" if event.keystroke.modifiers.platform || event.keystroke.modifiers.control => {
                self.submit(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for ReviewerPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let options = self.options();
        let checked: HashSet<_> = self
            .existing
            .iter()
            .chain(self.selected.iter())
            .map(|reviewer| reviewer.login.to_lowercase())
            .collect();
        let rows = options
            .iter()
            .enumerate()
            .map(|(index, reviewer)| {
                let color = theme.text;
                let mut avatar = div()
                    .size(px(20.))
                    .flex_none()
                    .rounded_full()
                    .overflow_hidden()
                    .bg(gpui::white());
                if let Some(url) = &reviewer.avatar_url {
                    avatar = avatar.child(img(url.clone()).size_full());
                }
                div()
                    .id(("reviewer-choice", index))
                    .role(gpui::Role::ListBoxOption)
                    .aria_label(reviewer.login.clone())
                    .aria_selected(checked.contains(&reviewer.login.to_lowercase()))
                    .h(px(if reviewer.submitted { 48. } else { 32. }))
                    .px(px(8.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .when(self.highlighted == Some(index), |d| d.bg(theme.ghost_hover))
                    .child(avatar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(text(reviewer.login.clone(), 14., 20., 430., color))
                            .when(reviewer.submitted, |d| {
                                d.child(text(
                                    "Review submitted",
                                    12.,
                                    16.,
                                    430.,
                                    theme.text_tertiary,
                                ))
                            }),
                    )
                    .when(checked.contains(&reviewer.login.to_lowercase()), |d| {
                        d.child(icon("menu-check", 14., color))
                    })
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.highlighted = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, cx)))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let message = if self.loading {
            "Searching…"
        } else if self.error {
            "Couldn’t search GitHub users"
        } else if self.search.is_empty() {
            "Search by name or GitHub username"
        } else {
            "No users found"
        };
        let panel = div()
            .id("reviewer-picker")
            .role(gpui::Role::Dialog)
            .aria_label("Request approvals")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_action(
                cx.listener(|_, _: &crate::composer::Dismiss, _, cx| cx.emit(Event::Dismiss)),
            )
            .w(px(288.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated_secondary)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("search", 16., theme.text_tertiary))
                    .child(div().flex_1().child(self.query.clone())),
            )
            .child(
                div()
                    .id("reviewer-list")
                    .role(gpui::Role::ListBox)
                    .aria_label("GitHub users")
                    .max_h(px(264.))
                    .overflow_y_scroll()
                    .p(px(4.))
                    .children(rows)
                    .when(options.is_empty(), |d| {
                        d.child(div().px(px(8.)).py(px(12.)).child(text(
                            message,
                            14.,
                            20.,
                            430.,
                            if self.error {
                                theme.text_error
                            } else {
                                theme.text_tertiary
                            },
                        )))
                    }),
            )
            .when(self.error, |d| {
                d.child(
                    div()
                        .id("reviewer-search-retry")
                        .role(gpui::Role::Button)
                        .aria_label("Retry")
                        .px(px(12.))
                        .py(px(6.))
                        .child(text("Retry", 14., 20., 430., theme.text))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.loading = true;
                            this.error = false;
                            cx.emit(Event::Search {
                                query: this.search.clone(),
                                epoch: this.epoch,
                            });
                            cx.notify();
                        })),
                )
            })
            .when(!self.selected.is_empty(), |d| {
                d.child(
                    div().p(px(4.)).child(
                        div()
                            .id("reviewer-submit")
                            .role(gpui::Role::Button)
                            .aria_label("Request")
                            .py(px(6.))
                            .rounded(px(8.))
                            .bg(theme.ghost_hover)
                            .flex()
                            .justify_center()
                            .child(text("Request", 14., 20., 500., theme.text))
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
                )
            });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(Event::Dismiss)),
            )
            .child(deferred(
                anchored()
                    .position(self.position)
                    .anchor(Anchor::TopRight)
                    .child(panel),
            ))
    }
}
