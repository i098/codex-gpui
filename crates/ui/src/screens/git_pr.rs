pub mod reviewers;

use crate::{
    composer::Composer,
    icon::icon,
    text,
    tooltip::{Side, tooltip},
};
use gpui::{
    App, AppContext as _, Context, Corners, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
    rgba,
};
use theme::{ActiveTheme as _, Appearance};

#[derive(Clone, Default)]
pub struct Repository {
    pub name: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub branches: Vec<String>,
    pub new_branch: Option<String>,
    pub additions: usize,
    pub deletions: usize,
    pub existing_pr: Option<String>,
    pub blocker: Option<String>,
}

#[derive(Clone)]
pub struct Submission {
    pub title: String,
    pub body: String,
    pub new_branch: Option<String>,
    pub include_local_changes: bool,
    pub draft: bool,
    pub open_in_browser: bool,
}

pub enum Event {
    Dismiss,
    Submit(Submission),
    Open(String),
}

pub struct CreatePullRequest {
    title: Entity<Composer>,
    body: Entity<Composer>,
    branch: Option<Entity<Composer>>,
    repository: Option<Result<Repository, String>>,
    include_local_changes: bool,
    selected: usize,
    controls: [FocusHandle; 4],
}

impl EventEmitter<Event> for CreatePullRequest {}

impl Focusable for CreatePullRequest {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.branch
            .as_ref()
            .unwrap_or(&self.title)
            .read(cx)
            .focus_handle(cx)
    }
}

impl CreatePullRequest {
    pub fn title_field(&self) -> &Entity<Composer> {
        &self.title
    }

    pub fn body_field(&self) -> &Entity<Composer> {
        &self.body
    }

    pub fn new(draft: bool, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| Composer::bare_field("Title", "", false, 600., cx));
        let body = cx.new(|cx| {
            let mut field =
                Composer::bare_field("Description (leave empty to generate)", "", true, 430., cx);
            field.set_field_label("Message");
            field
        });
        for field in [&title, &body] {
            cx.observe(field, |_, _, cx| cx.notify()).detach();
        }
        Self {
            title,
            body,
            branch: None,
            repository: None,
            include_local_changes: true,
            selected: usize::from(!draft),
            controls: std::array::from_fn(|_| cx.focus_handle()),
        }
    }

    pub fn set_repository(
        &mut self,
        repository: Result<Repository, String>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(repository) = &repository
            && let Some(name) = &repository.new_branch
        {
            let field = cx.new(|cx| Composer::bare_field("Branch name", name, false, 430., cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            self.branch = Some(field);
        }
        if repository
            .as_ref()
            .is_ok_and(|repository| repository.existing_pr.is_some())
        {
            self.selected = 2;
        }
        self.repository = Some(repository);
        cx.notify();
    }

    pub fn blocker(&self, cx: &App) -> Option<String> {
        let repository = match &self.repository {
            None => return Some("Loading pull request provider…".into()),
            Some(Err(error)) => return Some(error.clone()),
            Some(Ok(repository)) => repository,
        };
        if let Some(field) = &self.branch {
            let name = field.read(cx).field_text().trim();
            if name.is_empty() {
                return Some("Enter a branch name".into());
            }
            if name.ends_with('/') {
                return Some("Branch name cannot end with “/”".into());
            }
            if repository.branches.iter().any(|branch| branch == name) {
                return Some("Branch already exists".into());
            }
        }
        repository.blocker.clone()
    }

    fn activate(&self, index: usize, cx: &mut Context<Self>) {
        if let Some(Ok(repository)) = &self.repository
            && let Some(url) = &repository.existing_pr
        {
            cx.emit(Event::Open(url.clone()));
        } else if self.blocker(cx).is_none() {
            cx.emit(Event::Submit(Submission {
                title: self.title.read(cx).field_text().to_owned(),
                body: self.body.read(cx).field_text().to_owned(),
                new_branch: self
                    .branch
                    .as_ref()
                    .map(|field| field.read(cx).field_text().trim().to_owned()),
                include_local_changes: self.include_local_changes,
                draft: index == 0,
                open_in_browser: index == 2,
            }));
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => {
                cx.emit(Event::Dismiss);
                cx.stop_propagation();
            }
            "enter" if key.modifiers.platform || key.modifiers.control => {
                self.activate(self.selected, cx);
                cx.stop_propagation();
            }
            "tab" => {
                let mut handles = Vec::new();
                if let Some(branch) = &self.branch {
                    handles.push(branch.read(cx).focus_handle(cx));
                }
                handles.push(self.title.read(cx).focus_handle(cx));
                handles.push(self.body.read(cx).focus_handle(cx));
                handles.extend(self.controls.iter().cloned());
                let at = handles
                    .iter()
                    .position(|handle| handle.is_focused(window))
                    .unwrap_or(0);
                let next = if key.modifiers.shift {
                    (at + handles.len() - 1) % handles.len()
                } else {
                    (at + 1) % handles.len()
                };
                handles[next].focus(window, cx);
                cx.stop_propagation();
            }
            "up" | "down"
                if self
                    .controls
                    .iter()
                    .skip(1)
                    .any(|handle| handle.is_focused(window)) =>
            {
                self.selected = if key.key == "up" {
                    (self.selected + 2) % 3
                } else {
                    (self.selected + 1) % 3
                };
                self.controls[self.selected + 1].focus(window, cx);
                cx.notify();
                cx.stop_propagation();
            }
            "enter" | "space" if self.controls[0].is_focused(window) => {
                self.include_local_changes = !self.include_local_changes;
                cx.notify();
                cx.stop_propagation();
            }
            "enter" | "space"
                if self
                    .controls
                    .iter()
                    .skip(1)
                    .any(|handle| handle.is_focused(window)) =>
            {
                self.activate(self.selected, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Render for CreatePullRequest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let repository = self
            .repository
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let existing = repository
            .and_then(|repository| repository.existing_pr.as_ref())
            .is_some();
        let blocker = self.blocker(cx);
        let width = 420_f32.min(f32::from(window.viewport_size().width) * 0.92);
        let header = repository.map_or_else(
            || "- → -".to_owned(),
            |repository| {
                let branch = if self.branch.is_some() {
                    "New branch"
                } else {
                    repository.branch.as_deref().unwrap_or("-")
                };
                let base = repository.base.as_deref().unwrap_or("-");
                if repository.name.is_empty() {
                    format!("{branch} → {base}")
                } else {
                    format!("{} · {branch} → {base}", repository.name)
                }
            },
        );
        let mut content = div().flex().flex_col().min_h(px(128.)).gap(px(8.));
        if existing {
            content = content.px(px(12.)).py(px(8.)).child(text(
                "A pull request already exists for this branch",
                14.,
                20.,
                430.,
                t.text_tertiary,
            ));
        } else {
            if let Some(branch) = &self.branch {
                content = content.child(div().px(px(12.)).pt(px(8.)).child(branch.clone()));
            }
            content = content
                .child(div().px(px(12.)).pt(px(8.)).child(self.title.clone()))
                .child(
                    div()
                        .px(px(12.))
                        .pb(px(8.))
                        .min_h(px(60.))
                        .max_h(px(180.))
                        .id("create-pr-body-scroll")
                        .overflow_y_scroll()
                        .child(self.body.clone()),
                )
                .child(
                    div()
                        .id("create-pr-include-local-changes")
                        .role(gpui::Role::CheckBox)
                        .aria_label("Commit and push local changes")
                        .aria_toggled(if self.include_local_changes {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .track_focus(&self.controls[0])
                        .px(px(12.))
                        .pt(px(8.))
                        .pb(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .size(px(14.))
                                .flex_none()
                                .rounded(px(5.))
                                .border_1()
                                .border_color(t.border_strong)
                                .bg(t.surface_elevated_secondary.opacity(0.96))
                                .when(self.include_local_changes, |d| {
                                    d.child(icon("menu-check", 12., t.text))
                                }),
                        )
                        .child(
                            text("Commit and push local changes", 14., 20., 430., t.text).flex_1(),
                        )
                        .when_some(
                            repository.filter(|_| self.include_local_changes),
                            |d, repository| {
                                d.child(crate::screens::diff::panel::stats(
                                    repository.additions,
                                    repository.deletions,
                                    14.,
                                    &t,
                                ))
                            },
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.include_local_changes = !this.include_local_changes;
                            this.controls[0].focus(window, cx);
                            cx.notify();
                        })),
                );
        }
        let labels = if existing {
            vec![(2, "Open PR in browser", "pr-open-action")]
        } else {
            vec![
                (0, "Create draft PR", "pr-draft-action"),
                (1, "Create PR", "pr-create-action"),
                (2, "Open PR in browser", "pr-open-action"),
            ]
        };
        let rows = labels
            .into_iter()
            .map(|(index, label, glyph)| {
                let enabled = existing || blocker.is_none();
                let selected = enabled && self.selected == index;
                let color = if !enabled {
                    t.text.opacity(0.25)
                } else if selected {
                    t.text
                } else {
                    t.text.opacity(0.75)
                };
                let row = div()
                    .id(("create-pr-action", index))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .aria_disabled(!enabled)
                    .track_focus(&self.controls[index + 1])
                    .relative()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(12.5))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(selected, |d| d.bg(t.ghost_hover))
                    .child(icon(glyph, 16., color))
                    .child(text(label, 14., 21., 430., color).flex_1())
                    .when(selected, |d| {
                        d.child(
                            div()
                                .h(px(16.))
                                .px(px(6.))
                                .rounded(px(10.))
                                .bg(t.text_alpha_10)
                                .child(text("⌘↵", 12., 12., 430., t.text).opacity(0.8)),
                        )
                    })
                    .when(enabled, |d| {
                        d.on_hover(cx.listener(move |this, hovered, _, cx| {
                            if *hovered {
                                this.selected = index;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.activate(index, cx)))
                    });
                if let Some(reason) = blocker.clone().filter(|_| !existing) {
                    tooltip(row, SharedString::from(reason), None, Side::Right).into_any_element()
                } else {
                    row.into_any_element()
                }
            })
            .collect::<Vec<_>>();
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x00000022))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(Event::Dismiss)),
            )
            .child(
                div()
                    .id("create-pr-dialog")
                    .role(gpui::Role::Dialog)
                    .aria_label("Create PR")
                    .w(px(width))
                    .max_h(window.viewport_size().height * 0.92)
                    .rounded(px(20.))
                    .relative()
                    .on_key_down(cx.listener(Self::key))
                    .on_action(cx.listener(|_, _: &crate::composer::Dismiss, _, cx| {
                        cx.emit(Event::Dismiss);
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(20.)),
                        fill: Some(t.surface_elevated_secondary),
                        border: Some((
                            px(1.),
                            if t.appearance == Appearance::Dark {
                                t.border
                            } else {
                                gpui::transparent_black()
                            },
                        )),
                        shadows: vec![crate::composer::shadow(
                            rgba(0x00000030).into(),
                            16.,
                            32.,
                            -8.,
                            false,
                        )],
                        ..Default::default()
                    }))
                    .child(
                        div()
                            .h(px(36.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .child(text(header, 14., 20., 430., t.text_tertiary).text_ellipsis()),
                    )
                    .child(content)
                    .child(
                        div()
                            .border_t_1()
                            .border_color(t.border)
                            .p(px(4.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .children(rows),
                    ),
            )
    }
}
