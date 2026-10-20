//! Modal surfaces recorded in `refs/interactions/f3b.json`; the Scheduled sidebar's `New task`
//! create dialog in `refs/interactions/s036.json`.
//! The model-upgrade announcement is recorded in `refs/interactions/f3c.json`.
use crate::tooltip::{Side, tooltip};
use crate::{Copy, Cut, Paste, SelectAll, icon::icon, motion::ButtonVisual, text};
use bridge::types::Model;
use gpui::{
    App, Bounds, Context, Corners, Div, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Pixels,
    Render, RenderImage, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, canvas, div, point, prelude::FluentBuilder as _, px, rgba, size,
};
use image::{Frame, RgbaImage};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, LazyLock},
};
use theme::{ActiveTheme as _, Theme};

fn contains_ignore_ascii_case(text: &str, query: &str) -> bool {
    query.is_empty()
        || text
            .as_bytes()
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
}

type Shortcut = (&'static str, &'static str, &'static [&'static str]);

type AnnouncementArtworkCache = Rc<RefCell<Option<(Bounds<Pixels>, f32, Arc<RenderImage>, bool)>>>;

struct ModelAnnouncement {
    model: SharedString,
    reasoning_effort: SharedString,
    label: SharedString,
    title: SharedString,
    message: SharedString,
    primary_label: SharedString,
    artwork: AnnouncementArtworkCache,
}

struct ArchiveSection {
    title: String,
    description: String,
    running: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Shortcuts,
    CloseWindow,
    FullAccess,
    LogOut,
    RevertChanges,
    ModelAnnouncement,
    /// `Schedule a task`: the Scheduled sidebar's `New task` create editor (AM-005).
    ScheduleTask,
    /// `Rename chat` from a Recents row's menu (RS-091, refs/interactions/s002.json): the
    /// `Chat title` input, prefilled and selected, `Cancel`, `Save` and `Close dialog`.
    Rename,
    NewSection,
    EditSection,
    ArchiveSection,
    /// `Commit or push` (DT-139): the original's commit form for the open thread's repository.
    Commit,
    Branches,
    CreateBranch,
    BranchConflict,
}
#[derive(Clone)]
pub enum Outcome {
    Dismiss,
    FullAccess,
    LogOut,
    DismissAnnouncement(SharedString),
    UseAnnouncedModel {
        model: SharedString,
        reasoning_effort: SharedString,
    },
    CloseWindow {
        do_not_ask_again: bool,
    },
    RevertChanges {
        do_not_ask_again: bool,
    },
    /// `Save` or Enter in `Rename chat`: `thread/name/set` of `thread` to `name`.
    Rename {
        thread: String,
        name: String,
    },
    SectionName {
        section: Option<String>,
        name: String,
    },
    ArchiveSection(String),
    ScheduleTask {
        prompt: String,
        rrule: String,
    },
    /// A submitted commit form: the app creates the branch, commits and pushes.
    Commit {
        step: CommitStep,
        message: String,
        include_unstaged: bool,
        new_branch: Option<String>,
    },
    CheckoutBranch {
        name: String,
        create: bool,
    },
    CommitBeforeCheckout,
}

/// The rows of the commit form: `Commit`, `Commit and push` and `Push`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CommitStep {
    Commit,
    CommitAndPush,
    Push,
}

/// Files and lines of one change selection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Selection {
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
}

/// What the host reports about the repository when the commit form opens.
#[derive(Clone, Debug, Default)]
pub struct CommitRepository {
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
    pub branches: Vec<String>,
    pub has_commits: bool,
    pub ahead: usize,
    pub staged: Selection,
    pub all: Selection,
    pub include_unstaged: bool,
    /// The prefilled name of a new branch (`codex/` plus the chat title).
    pub new_branch_name: String,
}

struct CommitForm {
    repo: CommitRepository,
    include_unstaged: bool,
    new_branch: bool,
    branch_name: String,
    /// The highlighted `Commit to` item while that list is open.
    menu: Option<usize>,
}

struct BranchForm {
    repo: CommitRepository,
    highlighted: usize,
    target: Option<String>,
    conflicts: Vec<String>,
}

pub struct Dialog {
    kind: Kind,
    controls: [FocusHandle; 7],
    selected: usize,
    /// The text of the search, chat title, or task instructions.
    query: String,
    select_all: bool,
    instruction_cursor: usize,
    instruction_anchor: usize,
    do_not_ask_again: bool,
    scroll: ScrollHandle,
    /// `Rename`: the thread id.
    thread: String,
    /// `LogOut`: the signed-in ChatGPT account's email, from `account/read`.
    email: Option<String>,
    schedule_time: String,
    repeat: usize,
    repeat_open: bool,
    saving: bool,
    archive_section: Option<ArchiveSection>,
    error: Option<String>,
    announcement: Option<ModelAnnouncement>,
    commit: Option<CommitForm>,
    branch: Option<BranchForm>,
}
impl EventEmitter<Outcome> for Dialog {}
impl Focusable for Dialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.controls[self.selected].clone()
    }
}
impl Dialog {
    pub fn new(kind: Kind, cx: &mut Context<Self>) -> Self {
        Self {
            kind,
            controls: std::array::from_fn(|_| cx.focus_handle()),
            selected: usize::from(kind == Kind::RevertChanges),
            query: String::new(),
            select_all: false,
            instruction_cursor: 0,
            instruction_anchor: 0,
            do_not_ask_again: false,
            scroll: ScrollHandle::new(),
            thread: String::new(),
            email: None,
            schedule_time: "09:00".into(),
            repeat: 0,
            repeat_open: false,
            saving: false,
            error: None,
            announcement: None,
            archive_section: None,
            commit: None,
            branch: None,
        }
    }
    /// `Log out?` for the signed-in account `email`.
    pub fn logout(email: Option<String>, cx: &mut Context<Self>) -> Self {
        Self {
            email,
            ..Self::new(Kind::LogOut, cx)
        }
    }
    /// `Rename chat` for `thread`, its input prefilled with `title` and all of it selected.
    pub fn rename(thread: String, title: String, cx: &mut Context<Self>) -> Self {
        Self {
            query: title,
            select_all: true,
            thread,
            ..Self::new(Kind::Rename, cx)
        }
    }
    pub fn section(id: Option<String>, name: String, cx: &mut Context<Self>) -> Self {
        Self {
            kind: if id.is_some() {
                Kind::EditSection
            } else {
                Kind::NewSection
            },
            thread: id.unwrap_or_default(),
            ..Self::rename(String::new(), name, cx)
        }
    }

    fn name_title(&self) -> &'static str {
        match self.kind {
            Kind::NewSection => "New section",
            Kind::EditSection => "Edit section",
            _ => "Rename chat",
        }
    }

    fn name_label(&self) -> &'static str {
        if self.kind == Kind::Rename {
            "Chat title"
        } else {
            "Section name"
        }
    }

    fn name_submit(&self) -> &'static str {
        if self.kind == Kind::NewSection {
            "Create section"
        } else {
            "Save"
        }
    }

    pub fn archive_section(
        id: String,
        name: String,
        count: usize,
        running: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let noun = if count == 1 { "chat" } else { "chats" };
        let title = format!(
            "{}{count} {noun}?",
            if running {
                "Stop and archive "
            } else {
                "Archive "
            }
        );
        let description = if running {
            format!(
                "Archiving will stop any ongoing work. You can restore {} later in settings.",
                if count == 1 { "the chat" } else { "the chats" }
            )
        } else {
            format!(
                "This will archive the chats in {name}. You can find them later in your archived chats"
            )
        };
        Self {
            thread: id,
            archive_section: Some(ArchiveSection {
                title,
                description,
                running,
            }),
            ..Self::new(Kind::ArchiveSection, cx)
        }
    }

    fn archive_section_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let form = self.archive_section.as_ref().expect("archive section");
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text(form.title.clone(), 20., 28., 600., t.text))
            .child(
                text(form.description.clone(), 14., 21., 430., t.text_secondary)
                    .whitespace_normal()
                    .mt(px(4.)),
            )
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button(0, "Cancel", 80., t, cx))
                    .child(self.button(
                        1,
                        if form.running {
                            "Stop and archive"
                        } else {
                            "Archive all"
                        },
                        if form.running { 151. } else { 102. },
                        t,
                        cx,
                    )),
            )
    }
    /// `Commit or push` for the repository `repo` describes.
    pub fn commit(repo: CommitRepository, cx: &mut Context<Self>) -> Self {
        let form = CommitForm {
            include_unstaged: repo.include_unstaged,
            new_branch: repo.branch.is_none(),
            branch_name: repo.new_branch_name.clone(),
            menu: None,
            repo,
        };
        Self {
            selected: if form.repo.all.files == 0 { 6 } else { 0 },
            commit: Some(form),
            ..Self::new(Kind::Commit, cx)
        }
    }

    pub fn branches(repo: CommitRepository, kind: Kind, cx: &mut Context<Self>) -> Self {
        Self {
            query: if kind == Kind::CreateBranch {
                repo.new_branch_name.clone()
            } else {
                String::new()
            },
            branch: Some(BranchForm {
                repo,
                highlighted: 0,
                target: None,
                conflicts: Vec::new(),
            }),
            ..Self::new(kind, cx)
        }
    }

    pub fn checkout_target(&self) -> Option<String> {
        self.branch.as_ref().and_then(|form| form.target.clone())
    }

    pub fn begin_checkout(&mut self, name: String, cx: &mut Context<Self>) {
        self.branch.as_mut().unwrap().target = Some(name);
        self.saving = true;
        self.error = None;
        cx.notify();
    }

    pub fn checkout_error(
        &mut self,
        error: String,
        conflicts: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if !conflicts.is_empty() {
            self.kind = Kind::BranchConflict;
            self.branch.as_mut().unwrap().conflicts = conflicts;
            self.selected = 1;
        }
        self.schedule_result(error, cx);
    }

    pub fn prepare_branch_commit(&mut self, mut repo: CommitRepository, cx: &mut Context<Self>) {
        let branch = self.branch.take();
        repo.new_branch_name = branch.as_ref().unwrap().repo.new_branch_name.clone();
        *self = Self::commit(repo, cx);
        self.branch = branch;
    }

    pub fn resume_checkout(&mut self, cx: &mut Context<Self>) {
        self.kind = Kind::Branches;
        self.query = self.checkout_target().unwrap();
        self.selected = 0;
        self.saving = false;
        cx.notify();
    }

    pub fn set_branch_repository(&mut self, mut repo: CommitRepository, cx: &mut Context<Self>) {
        let form = self.branch.as_mut().unwrap();
        repo.new_branch_name = std::mem::take(&mut form.repo.new_branch_name);
        form.repo = repo;
        cx.notify();
    }

    pub fn branch_names(&self) -> Vec<&str> {
        let repo = &self.branch.as_ref().unwrap().repo;
        let query = self.query.trim();
        let mut names: Vec<_> = repo
            .branch
            .iter()
            .chain(
                repo.branches
                    .iter()
                    .filter(|name| Some(*name) != repo.branch.as_ref()),
            )
            .map(String::as_str)
            .filter(|name| contains_ignore_ascii_case(name, query))
            .collect();
        if !query.is_empty() && !names.contains(&query) {
            names.push(query);
        }
        names
    }

    fn branch_name_error(&self) -> Option<&'static str> {
        let repo = &self.branch.as_ref().unwrap().repo;
        let name = self.query.trim();
        if !repo.has_commits {
            Some("Commit changes to create and checkout a new branch")
        } else if name.is_empty() {
            Some("Enter a branch name")
        } else if name.ends_with('/') {
            Some("Branch name cannot end with “/”.")
        } else if repo.branches.iter().any(|branch| branch == name) {
            Some("Branch already exists.")
        } else {
            None
        }
    }

    fn choose_branch(&mut self, cx: &mut Context<Self>) {
        let form = self.branch.as_ref().unwrap();
        let Some(name) = self.branch_names().get(form.highlighted).copied() else {
            return;
        };
        if form.repo.branch.as_deref() == Some(name) {
            self.dismiss(cx);
        } else {
            cx.emit(Outcome::CheckoutBranch {
                name: name.to_owned(),
                create: false,
            });
        }
    }

    fn branch_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let form = self.branch.as_ref().unwrap();
        let selecting = self.kind == Kind::Branches;
        let conflict = self.kind == Kind::BranchConflict;
        let title = if selecting {
            "Switch branch"
        } else if conflict {
            "Commit changes to switch branch"
        } else {
            "Create and checkout branch"
        };
        let mut content = div()
            .p(px(if selecting { 8. } else { 20. }))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(text(
                title,
                if selecting { 13. } else { 18. },
                24.,
                600.,
                t.text,
            ));
        if !conflict {
            let label = if selecting {
                "Search branches"
            } else {
                "Branch name"
            };
            content = content.child(
                div()
                    .id("branch-input")
                    .role(gpui::Role::TextInput)
                    .aria_label(label)
                    .aria_value(self.query.clone())
                    .aria_placeholder(label)
                    .track_focus(&self.controls[0])
                    .h(px(36.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if self.selected == 0 {
                        rgba(0x3a83f7ff).into()
                    } else {
                        t.border_strong
                    })
                    .bg(t.surface_elevated_secondary)
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .cursor_text()
                    .child(
                        text(
                            if self.query.is_empty() {
                                label.to_owned()
                            } else {
                                self.query.clone()
                            },
                            13.,
                            20.,
                            430.,
                            if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(self.select_all && !self.query.is_empty(), |d| {
                            d.bg(rgba(0x3a83f766))
                        }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            );
        }
        if selecting {
            content =
                content.child(
                    div()
                        .id("branch-list")
                        .max_h(px(272.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .children(self.branch_names().into_iter().enumerate().map(
                            |(index, name)| {
                                let current = form.repo.branch.as_deref() == Some(name);
                                div()
                                    .id(("branch-choice", index))
                                    .role(gpui::Role::MenuItem)
                                    .aria_label(name.to_owned())
                                    .aria_disabled(self.saving)
                                    .min_h(px(34.))
                                    .px(px(8.))
                                    .rounded(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .when(index == form.highlighted, |d| d.bg(t.menu_item_hover))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                text(
                                                    if !current
                                                        && !form
                                                            .repo
                                                            .branches
                                                            .iter()
                                                            .any(|branch| branch == name)
                                                    {
                                                        format!("Use {name}")
                                                    } else {
                                                        name.to_owned()
                                                    },
                                                    13.,
                                                    18.,
                                                    430.,
                                                    t.text,
                                                )
                                                .text_ellipsis(),
                                            )
                                            .when(current && form.repo.all.files > 0, |d| {
                                                d.child(text(
                                                    format!(
                                                        "Uncommitted: {} {}",
                                                        form.repo.all.files,
                                                        if form.repo.all.files == 1 {
                                                            "file"
                                                        } else {
                                                            "files"
                                                        }
                                                    ),
                                                    11.,
                                                    16.,
                                                    430.,
                                                    t.text_tertiary,
                                                ))
                                            }),
                                    )
                                    .when(current, |d| d.child(icon("menu-check", 16., t.text)))
                                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                                        if *hovered {
                                            this.branch.as_mut().unwrap().highlighted = index;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.saving {
                                            this.branch.as_mut().unwrap().highlighted = index;
                                            this.choose_branch(cx);
                                        }
                                    }))
                            },
                        )),
                );
            content = content.child(
                self.button(1, "Create and checkout new branch…", 284., t, cx)
                    .aria_disabled(!form.repo.has_commits || self.saving)
                    .opacity(if form.repo.has_commits { 1. } else { 0.4 }),
            );
        } else if conflict {
            content = content
                .child(
                    text(
                        "Your changes to the following files would be overwritten by checkout:",
                        14.,
                        20.,
                        430.,
                        t.text_tertiary,
                    )
                    .whitespace_normal(),
                )
                .child(
                    div()
                        .id("branch-conflicts")
                        .max_h(px(110.))
                        .overflow_y_scroll()
                        .children(form.conflicts.iter().map(|path| {
                            text(path.clone(), 13., 22., 430., t.text).whitespace_normal()
                        })),
                )
                .child(text(
                    "Please commit your changes to continue",
                    14.,
                    20.,
                    430.,
                    t.text_tertiary,
                ));
        } else {
            content = content.child(text(
                self.branch_name_error().unwrap_or(""),
                12.,
                18.,
                430.,
                t.text_error,
            ));
        }
        if let Some(error) = &self.error
            && !conflict
        {
            content = content.child(
                div()
                    .id("branch-error")
                    .max_h(px(65.))
                    .overflow_y_scroll()
                    .child(text(error.clone(), 12., 18., 430., t.text_error).whitespace_normal()),
            );
        }
        if !selecting {
            let label = if conflict {
                "Commit and switch branch…"
            } else {
                "Create and checkout"
            };
            content = content.child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.button(2, if conflict { "Cancel" } else { "Close" }, 70., t, cx))
                    .child(
                        self.button(1, label, if conflict { 225. } else { 180. }, t, cx)
                            .aria_disabled(
                                self.saving || !conflict && self.branch_name_error().is_some(),
                            )
                            .opacity(
                                if self.saving || !conflict && self.branch_name_error().is_some() {
                                    0.4
                                } else {
                                    1.
                                },
                            ),
                    ),
            );
        }
        content
    }
    /// Builds the recorded model-upgrade surface from original model metadata.
    pub fn model_announcement(model: &Model, message: String, cx: &mut Context<Self>) -> Self {
        let label = crate::composer::model_label(&model.display_name);
        Self {
            announcement: Some(ModelAnnouncement {
                model: model.model.clone().into(),
                reasoning_effort: model.default_reasoning_effort.clone().into(),
                title: format!("Introducing {label}").into(),
                primary_label: format!("Try {label} now").into(),
                label: label.into(),
                message: message.into(),
                artwork: Rc::new(RefCell::new(None)),
            }),
            ..Self::new(Kind::ModelAnnouncement, cx)
        }
    }
    /// Evicts the announcement image when its dialog leaves the window.
    pub fn release_artwork(&self, window: &mut Window) {
        if let Some(announcement) = &self.announcement
            && let Some((_, _, image, _)) = announcement.artwork.borrow_mut().take()
        {
            window.drop_image(image).ok();
        }
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn do_not_ask_again(&self) -> bool {
        self.do_not_ask_again
    }
    pub fn focused_control(&self) -> &str {
        match (self.kind, self.selected) {
            (Kind::Shortcuts, 0) => "Search shortcuts",
            (Kind::Shortcuts, _) => "Close dialog",
            (Kind::CloseWindow, 0) => "Cancel",
            (Kind::CloseWindow, 1) => "Close Window",
            (Kind::CloseWindow, _) => "Do not ask me again",
            (Kind::RevertChanges, 0) => "Cancel",
            (Kind::RevertChanges, 1) => "Confirm",
            (Kind::RevertChanges, _) => "Don't ask again",
            (Kind::FullAccess, 0) => "Cancel",
            (Kind::FullAccess, _) => "Confirm",
            (Kind::LogOut, 0) => "Cancel",
            (Kind::LogOut, _) => "Log out",
            (Kind::ArchiveSection, 0) => "Cancel",
            (Kind::ArchiveSection, _) => {
                if self.archive_section.as_ref().is_some_and(|f| f.running) {
                    "Stop and archive"
                } else {
                    "Archive all"
                }
            }
            (Kind::ModelAnnouncement, 0) => "Close",
            (Kind::ModelAnnouncement, 1) => "Continue with current model",
            (Kind::ModelAnnouncement, _) => {
                &self
                    .announcement
                    .as_ref()
                    .expect("model announcement")
                    .primary_label
            }
            (Kind::ScheduleTask, 0) => "Instructions…",
            (Kind::ScheduleTask, 1) => "Cancel",
            (Kind::ScheduleTask, 2) => "Close dialog",
            (Kind::ScheduleTask, 3) => "Repeat",
            (Kind::ScheduleTask, 4) => "Time",
            (Kind::ScheduleTask, _) => "Create",
            (Kind::Rename | Kind::NewSection | Kind::EditSection, 0) => self.name_label(),
            (Kind::Rename | Kind::NewSection | Kind::EditSection, 1) => self.name_submit(),
            (Kind::Rename | Kind::NewSection | Kind::EditSection, _) => "Cancel",
            (Kind::Commit, 0) => "Commit message",
            (Kind::Commit, 1) => "Include unstaged changes",
            (Kind::Commit, 2) => "Commit to",
            (Kind::Commit, 3) => "Branch name",
            (Kind::Commit, 4) => "Commit",
            (Kind::Commit, 5) => "Commit and push",
            (Kind::Commit, _) => "Push",
            (Kind::Branches, 0) => "Search branches",
            (Kind::CreateBranch, 0) => "Branch name",
            (Kind::Branches | Kind::CreateBranch, 2) => "Close",
            (Kind::BranchConflict, 2) => "Cancel",
            (Kind::Branches, _) => "Create and checkout new branch…",
            (Kind::CreateBranch, _) => "Create and checkout",
            (Kind::BranchConflict, _) => "Commit and switch branch…",
        }
    }
    /// The whole query is selected (typing replaces it).
    pub fn all_selected(&self) -> bool {
        self.select_all
    }
    fn editing(&self) -> bool {
        (matches!(
            self.kind,
            Kind::Shortcuts
                | Kind::Rename
                | Kind::NewSection
                | Kind::EditSection
                | Kind::ScheduleTask
                | Kind::Commit
                | Kind::Branches
                | Kind::CreateBranch
        ) && self.selected == 0)
            || (self.kind == Kind::ScheduleTask && self.selected == 4)
            || (self.kind == Kind::Commit && self.selected == 3)
    }

    /// The focused input takes `Enter` as a new line and keeps a caret and selection.
    fn multiline(&self) -> bool {
        matches!(self.kind, Kind::ScheduleTask | Kind::Commit) && self.selected == 0
    }

    fn input(&mut self) -> &mut String {
        match (self.kind, self.selected) {
            (Kind::ScheduleTask, 4) => &mut self.schedule_time,
            (Kind::Commit, 3) => &mut self.form_mut().branch_name,
            _ => &mut self.query,
        }
    }

    fn schedule_rule(&self) -> Option<String> {
        let (hour, minute) = self.schedule_time.split_once(':')?;
        if hour.len() != 2 || minute.len() != 2 {
            return None;
        }
        let hour: u8 = hour.parse().ok()?;
        let minute: u8 = minute.parse().ok()?;
        if hour > 23 || minute > 59 {
            return None;
        }
        let frequency = if self.repeat == 0 {
            "FREQ=DAILY".to_owned()
        } else {
            format!("FREQ=WEEKLY;BYDAY={}", REPEAT[self.repeat].1)
        };
        Some(format!("{frequency};BYHOUR={hour};BYMINUTE={minute}"))
    }

    pub fn schedule_result(&mut self, error: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(error);
        cx.notify();
    }

    pub fn saving(&self) -> bool {
        self.saving
    }
    fn visible(&self) -> impl Iterator<Item = &'static Shortcut> + '_ {
        let query = self.query.trim();
        SHORTCUTS
            .iter()
            .filter(move |(_, title, _)| contains_ignore_ascii_case(title, query))
    }
    pub fn visible_titles(&self) -> Vec<&'static str> {
        self.visible().map(|(_, title, _)| *title).collect()
    }
    fn dismiss(&self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if let Some(announcement) = &self.announcement {
            cx.emit(Outcome::DismissAnnouncement(announcement.model.clone()));
        } else {
            cx.emit(Outcome::Dismiss);
        }
    }
    fn activate(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        match (self.kind, self.selected) {
            (Kind::Branches, 0) => self.choose_branch(cx),
            (Kind::Branches, 1) => {
                if self.branch.as_ref().unwrap().repo.has_commits {
                    self.kind = Kind::CreateBranch;
                    self.query = self.branch.as_ref().unwrap().repo.new_branch_name.clone();
                    self.selected = 0;
                    self.error = None;
                    cx.notify();
                }
            }
            (Kind::CreateBranch, 0 | 1) => {
                if self.branch_name_error().is_none() {
                    cx.emit(Outcome::CheckoutBranch {
                        name: self.query.trim().to_owned(),
                        create: true,
                    });
                }
            }
            (Kind::BranchConflict, 0 | 1) => cx.emit(Outcome::CommitBeforeCheckout),
            (Kind::ScheduleTask, 3) => {
                self.repeat_open = !self.repeat_open;
                cx.notify();
            }
            (Kind::ScheduleTask, 5) => {
                if let Some(rrule) = self
                    .schedule_rule()
                    .filter(|_| !self.query.trim().is_empty())
                {
                    self.saving = true;
                    self.error = None;
                    cx.emit(Outcome::ScheduleTask {
                        prompt: self.query.trim().to_owned(),
                        rrule,
                    });
                    cx.notify();
                }
            }
            (Kind::ScheduleTask, 0 | 4) => {}
            (Kind::FullAccess, 1) => cx.emit(Outcome::FullAccess),
            (Kind::LogOut, 1) => cx.emit(Outcome::LogOut),
            (Kind::ArchiveSection, 1) => cx.emit(Outcome::ArchiveSection(self.thread.clone())),
            (Kind::ModelAnnouncement, 2) => {
                let announcement = self.announcement.as_ref().expect("model announcement");
                cx.emit(Outcome::UseAnnouncedModel {
                    model: announcement.model.clone(),
                    reasoning_effort: announcement.reasoning_effort.clone(),
                });
            }
            (Kind::CloseWindow, 1) => cx.emit(Outcome::CloseWindow {
                do_not_ask_again: self.do_not_ask_again,
            }),
            (Kind::RevertChanges, 1) => cx.emit(Outcome::RevertChanges {
                do_not_ask_again: self.do_not_ask_again,
            }),
            (Kind::CloseWindow | Kind::RevertChanges, 2) => {
                self.do_not_ask_again = !self.do_not_ask_again;
                cx.notify();
            }
            (Kind::Rename, 0 | 1) => cx.emit(Outcome::Rename {
                thread: self.thread.clone(),
                name: self.query.clone(),
            }),
            (Kind::NewSection | Kind::EditSection, 0 | 1) => {
                let name = self.query.trim();
                if self.kind == Kind::EditSection && name.is_empty() {
                    return;
                }
                cx.emit(Outcome::SectionName {
                    section: (self.kind == Kind::EditSection).then(|| self.thread.clone()),
                    name: if name.is_empty() {
                        "New section".to_owned()
                    } else {
                        name.to_owned()
                    },
                });
            }
            (Kind::Commit, 1) => {
                let form = self.form_mut();
                form.include_unstaged = !form.include_unstaged;
                cx.notify();
            }
            (Kind::Commit, 2) => {
                let highlighted = self.menu_selection();
                self.form_mut().menu = Some(highlighted);
                cx.notify();
            }
            (Kind::Commit, 4) => self.submit_commit(CommitStep::Commit, cx),
            (Kind::Commit, 5) => self.submit_commit(CommitStep::CommitAndPush, cx),
            (Kind::Commit, 6) => self.submit_commit(CommitStep::Push, cx),
            (Kind::Commit, 0 | 3) => {}
            _ => self.dismiss(cx),
        }
    }
    fn instruction_selection(&self) -> std::ops::Range<usize> {
        if self.select_all {
            0..self.query.len()
        } else {
            self.instruction_anchor.min(self.instruction_cursor)
                ..self.instruction_anchor.max(self.instruction_cursor)
        }
    }

    fn replace_instructions(&mut self, value: &str, cx: &mut Context<Self>) {
        let range = self.instruction_selection();
        self.instruction_cursor = range.start + value.len();
        self.query.replace_range(range, value);
        self.instruction_anchor = self.instruction_cursor;
        self.select_all = false;
        self.reveal_instruction_cursor();
        cx.notify();
    }

    fn reveal_instruction_cursor(&self) {
        self.scroll.scroll_to_item(
            self.query[..self.instruction_cursor]
                .bytes()
                .filter(|b| *b == b'\n')
                .count(),
        );
    }
    fn insert(&mut self, value: &str, cx: &mut Context<Self>) {
        if !self.editing() || self.saving {
            return;
        }
        if self.multiline() {
            let value: String = value
                .chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .collect();
            self.replace_instructions(&value, cx);
            self.error = None;
            return;
        }
        if self.select_all {
            self.input().clear();
            self.select_all = false;
        }
        let multiline = self.kind == Kind::ScheduleTask && self.selected == 0;
        let branch_name = self.kind == Kind::CreateBranch;
        self.input().extend(value.chars().filter(|c| {
            (!c.is_control() || (multiline && *c == '\n'))
                && (!branch_name || !c.is_whitespace() && !"~^:?*[]\\".contains(*c))
        }));
        self.error = None;
        if let Some(form) = &mut self.branch {
            form.highlighted = 0;
        }
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if self.saving {
            cx.stop_propagation();
            return;
        }
        if self.repeat_open {
            match k.key.as_str() {
                "up" => self.repeat = (self.repeat + REPEAT.len() - 1) % REPEAT.len(),
                "down" => self.repeat = (self.repeat + 1) % REPEAT.len(),
                "enter" | "space" | "escape" | "tab" => self.repeat_open = false,
                _ => {}
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.kind == Kind::Commit && self.form().menu.is_some() {
            let count = 1 + usize::from(self.form().repo.branch.is_some());
            let highlighted = self.form().menu.unwrap_or_default();
            match k.key.as_str() {
                "up" => self.form_mut().menu = Some((highlighted + count - 1) % count),
                "down" => self.form_mut().menu = Some((highlighted + 1) % count),
                "enter" | "space" => self.choose_target(highlighted),
                "escape" | "tab" => self.form_mut().menu = None,
                _ => {}
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.kind == Kind::Branches && self.selected == 0 {
            let count = self.branch_names().len();
            match k.key.as_str() {
                "up" | "down" if count > 0 => {
                    let form = self.branch.as_mut().unwrap();
                    form.highlighted = if k.key == "down" {
                        (form.highlighted + 1) % count
                    } else {
                        (form.highlighted + count - 1) % count
                    };
                }
                "enter" => self.choose_branch(cx),
                _ => {
                    self.branch_key(e, window, cx);
                    return;
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.branch_key(e, window, cx);
    }

    fn branch_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "left" | "right" | "home" | "end" | "backspace" | "delete" if self.multiline() => {
                let range = self.instruction_selection();
                let cursor = self.instruction_cursor;
                let next = match k.key.as_str() {
                    "home" => 0,
                    "end" => self.query.len(),
                    "left" | "backspace" => self.query[..cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i),
                    _ => {
                        cursor
                            + self.query[cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                    }
                };
                if matches!(k.key.as_str(), "backspace" | "delete") {
                    if range.is_empty() {
                        self.instruction_anchor = next;
                    }
                    self.replace_instructions("", cx);
                } else {
                    self.instruction_cursor = if !k.modifiers.shift && !range.is_empty() {
                        if k.key == "left" {
                            range.start
                        } else {
                            range.end
                        }
                    } else {
                        next
                    };
                    if !k.modifiers.shift {
                        self.instruction_anchor = self.instruction_cursor;
                    }
                    self.select_all = false;
                    self.reveal_instruction_cursor();
                    cx.notify();
                }
            }
            "escape" => self.dismiss(cx),
            "tab" => {
                let order: Vec<usize> = match self.kind {
                    Kind::Shortcuts | Kind::FullAccess | Kind::LogOut | Kind::ArchiveSection => {
                        vec![0, 1]
                    }
                    Kind::ScheduleTask => vec![0, 3, 4, 5, 1, 2],
                    Kind::Branches => vec![0, 1],
                    Kind::BranchConflict => vec![1, 2],
                    Kind::Commit => self.commit_tab_order(),
                    _ => vec![0, 1, 2],
                };
                let count = order.len();
                let current = order.iter().position(|i| *i == self.selected).unwrap_or(0);
                self.selected =
                    order[(current + if k.modifiers.shift { count - 1 } else { 1 }) % count];
                self.controls[self.selected].focus(window, cx);
                cx.notify();
                self.select_all = false;
            }
            // `Cmd+Enter` runs the focused row of the form, else `Commit`.
            "enter"
                if self.kind == Kind::Commit && (k.modifiers.platform || k.modifiers.control) =>
            {
                let step = match self.selected {
                    5 => CommitStep::CommitAndPush,
                    6 => CommitStep::Push,
                    _ => CommitStep::Commit,
                };
                self.submit_commit(step, cx);
            }
            // Enter in the title input submits the form; Space types.
            "enter"
                if matches!(
                    self.kind,
                    Kind::Rename | Kind::NewSection | Kind::EditSection | Kind::CreateBranch
                ) && self.selected == 0 =>
            {
                self.activate(cx)
            }
            "enter" if self.multiline() => self.insert("\n", cx),
            "enter" | "space"
                if matches!(
                    self.kind,
                    Kind::CloseWindow
                        | Kind::FullAccess
                        | Kind::LogOut
                        | Kind::ArchiveSection
                        | Kind::ModelAnnouncement
                        | Kind::RevertChanges
                ) || (self.selected != 0 && !self.editing()) =>
            {
                self.activate(cx);
                if self.kind == Kind::CreateBranch && self.selected == 0 {
                    self.controls[0].focus(window, cx);
                }
            }
            "backspace" if self.editing() && !self.saving => {
                if self.select_all {
                    self.input().clear();
                    self.select_all = false;
                } else {
                    self.input().pop();
                }
                self.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
            _ if !k.modifiers.platform && !k.modifiers.control => {
                if let Some(value) = &k.key_char {
                    self.insert(value, cx);
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        width: f32,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let primary = index == 1 && self.kind != Kind::Branches;
        div()
            .id(("dialog-button", index))
            .role(gpui::Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[index])
            .w(px(width))
            .h(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(12.5))
            .bg(if primary { t.text } else { t.list_hover })
            .when(
                matches!(
                    self.kind,
                    Kind::FullAccess | Kind::RevertChanges | Kind::ArchiveSection
                ),
                |d| {
                    d.h(px(36.)).rounded(px(18.)).bg(if primary {
                        t.text_error.opacity(0.1)
                    } else {
                        t.list_hover
                    })
                },
            )
            .border_2()
            .border_color(if self.selected == index {
                rgba(0x3a83f7ff)
            } else {
                rgba(0)
            })
            .when(
                matches!(self.kind, Kind::NewSection | Kind::EditSection),
                |d| {
                    d.w_auto()
                        .px(px(16.))
                        .border_1()
                        .border_color(if self.selected == index {
                            rgba(0x3a83f7ff).into()
                        } else {
                            t.border
                        })
                        .bg(if primary {
                            t.text
                        } else {
                            t.text.opacity(0.032)
                        })
                },
            )
            .hover(|s| s.opacity(0.85))
            .child(text(
                label,
                14.,
                18.,
                430.,
                if matches!(
                    self.kind,
                    Kind::FullAccess | Kind::RevertChanges | Kind::ArchiveSection
                ) && primary
                {
                    t.text_error
                } else if primary {
                    t.menu_background
                } else {
                    t.text
                },
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.controls[index].focus(window, cx);
                this.activate(cx);
                if this.kind == Kind::CreateBranch && this.selected == 0 {
                    this.controls[0].focus(window, cx);
                }
            }))
    }
    fn announcement_button(
        &self,
        index: usize,
        label: SharedString,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let primary = index == 2;
        div()
            .id(("announcement-button", index))
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .track_focus(&self.controls[index])
            .h(px(36.))
            .px(px(20.))
            .flex_none()
            .rounded(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(18.)),
                fill: Some(if primary { t.text } else { t.text_alpha_5 }),
                shadows: if self.selected == index {
                    vec![crate::composer::shadow(
                        rgba(0x3a83f7ff).into(),
                        0.,
                        0.,
                        2.,
                        false,
                    )]
                } else {
                    Vec::new()
                },
                ..Default::default()
            }))
            .child(text(
                label,
                14.,
                18.,
                430.,
                if primary {
                    t.surface_elevated_secondary
                } else {
                    t.text
                },
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.controls[index].focus(window, cx);
                this.activate(cx);
            }))
    }

    fn announcement_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let announcement = self.announcement.as_ref().expect("model announcement");
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .gap(px(40.))
            .pb(px(40.))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(214.))
                    .flex_none()
                    .child(
                        div()
                            .id("announcement-artwork")
                            .role(gpui::Role::Image)
                            .aria_label(announcement.label.clone())
                            .size_full()
                            .child(announcement_artwork(announcement.artwork.clone())),
                    )
                    .child(
                        div()
                            .id("announcement-close")
                            .role(gpui::Role::Button)
                            .aria_label("Close")
                            .track_focus(&self.controls[0])
                            .absolute()
                            .right(px(14.))
                            .top(px(14.))
                            .size(px(20.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(self.selected == 0, |d| {
                                d.child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(px(4.)),
                                    shadows: vec![crate::composer::shadow(
                                        rgba(0x3a83f7ff).into(),
                                        0.,
                                        0.,
                                        2.,
                                        false,
                                    )],
                                    ..Default::default()
                                }))
                            })
                            .child(icon("close", 16., rgba(0xffffffff).into()))
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .px(px(32.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(24.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.))
                            .child(text(announcement.title.clone(), 20., 28., 600., t.text))
                            .child(
                                text(
                                    announcement.message.clone(),
                                    14.,
                                    21.,
                                    430.,
                                    t.text_tertiary,
                                )
                                .w_full()
                                .whitespace_normal()
                                .text_center(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(12.))
                            .child(self.announcement_button(
                                1,
                                "Continue with current model".into(),
                                t,
                                cx,
                            ))
                            .child(self.announcement_button(
                                2,
                                announcement.primary_label.clone(),
                                t,
                                cx,
                            )),
                    ),
            )
    }

    /// The original `Log out?` dialog (`refs/interactions/logout.json`, 1280×832, scale 1): a
    /// 380 px card centered in the window, 20 px padding, 12 px between the title, the body and
    /// the button row, 32 px buttons sized to their text plus 16 px each side, 12 px apart,
    /// 20 px from the right and bottom edges, and a 24 px close button 16 px from the corner.
    /// The `Signed in as <email>` line sits 4 px under the title and adds 25 px.
    fn logout_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        div()
            .relative()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text("Log out?", 20., 28., 600., t.text))
                    .children(self.email.as_ref().map(|email| {
                        div().w(px(340.)).overflow_hidden().child(text(
                            format!("Signed in as {email}"),
                            14.,
                            21.,
                            430.,
                            t.text_tertiary,
                        ))
                    })),
            )
            .child(text(
                "You’ll need to sign in again to keep using ChatGPT",
                14.,
                21.,
                430.,
                t.text,
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.logout_button(0, "Cancel", t, cx))
                    .child(self.logout_button(1, "Log out", t, cx)),
            )
            .child(
                div()
                    .id("logout-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .absolute()
                    .right(px(16.))
                    .top(px(16.))
                    .size(px(24.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(t.ghost_hover))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("close", 16., t.text.opacity(0.8)))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
    }

    /// `Cancel` is the ghost button and `Log out` the danger button: 32 px high, `px-4` and a
    /// 1 px transparent border, radius 12.5, the 2 px ring outside the box when focused.
    fn logout_button(
        &self,
        index: usize,
        label: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let danger = index == 1;
        div()
            .id(("dialog-button", index))
            .role(gpui::Role::Button)
            .aria_label(label)
            .track_focus(&self.controls[index])
            .h(px(32.))
            // `px-4` plus the original's 1 px transparent border, so the painted box is the border box.
            .px(px(17.))
            .flex()
            .items_center()
            .justify_center()
            .relative()
            .when(danger, |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.text_error.opacity(0.1)),
                    ..Default::default()
                }))
            })
            .when(self.selected == index, |d| {
                d.child(div().absolute().inset(px(-2.)).child(shapes::decoration(
                    shapes::Decoration {
                        radius: Corners::all(px(14.5)),
                        corner: shapes::Corner::Squircle,
                        border: Some((px(2.), rgba(0x3a83f7ff).into())),
                        ..Default::default()
                    },
                )))
            })
            .hover(|s| s.opacity(0.85))
            .child(text(
                label,
                14.,
                18.,
                430.,
                if danger {
                    t.text_error
                } else {
                    t.text_tertiary
                },
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = index;
                this.controls[index].focus(window, cx);
                this.activate(cx);
            }))
    }

    fn full_access_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let mut capabilities = div()
            .p(px(12.))
            .rounded(px(16.))
            .bg(t.list_hover)
            .flex()
            .flex_col()
            .gap(px(12.));
        for (title, description) in [
            (
                "Files and folders",
                "Read, create, modify, upload, or delete files anywhere on this computer",
            ),
            (
                "Terminal commands",
                "Run commands, install software, and change system settings",
            ),
            (
                "Internet and connected apps",
                "Access websites, send data, and use enabled plugins",
            ),
        ] {
            capabilities = capabilities.child(
                div()
                    .flex()
                    .flex_col()
                    .child(text(title, 13., 18., 500., t.text))
                    .child(text(description, 13., 18., 400., t.text_secondary).whitespace_normal()),
            );
        }
        div().relative().size_full().p(px(20.)).flex().flex_col().gap(px(12.))
            .child(text("Turn on Full Access?", 20., 28., 600., t.text))
            .child(text("Codex will be able to run commands, use the internet, and create and edit files anywhere on this computer without your permission. This includes but is not limited to:", 14., 20., 400., t.text_secondary).whitespace_normal())
            .child(capabilities)
            .child(text("This comes with risks like loss or exposure of sensitive data and prompt injection. You can turn this off.", 14., 20., 400., t.text_secondary).whitespace_normal())
            .child(div().absolute().bottom(px(20.)).right(px(20.)).flex().gap(px(12.))
                .child(self.button(0, "Cancel", 88., t, cx))
                .child(self.button(1, "Confirm", 115., t, cx)))
    }

    fn close_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let revert = self.kind == Kind::RevertChanges;
        let checkbox = if revert {
            "Don't ask again"
        } else {
            "Do not ask me again"
        };
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(
                text(
                    if revert {
                        "Revert changes?"
                    } else {
                        "Are you sure you want to close the window?"
                    },
                    20.,
                    28.,
                    600.,
                    t.text,
                )
                .whitespace_normal()
                .h(px(if revert { 28. } else { 56. })),
            )
            .when(revert, |d| {
                d.child(
                    text(
                        "This action removes all of these changes.",
                        14.,
                        20.,
                        400.,
                        t.text_secondary,
                    )
                    .mt(px(8.))
                    .whitespace_normal(),
                )
            })
            .child(
                div()
                    .id("close-do-not-ask")
                    .role(gpui::Role::CheckBox)
                    .aria_label(checkbox)
                    .aria_toggled(if self.do_not_ask_again {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .track_focus(&self.controls[2])
                    .mt(px(12.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(t.border)
                            .when(self.selected == 2, |d| d.border_color(rgba(0x3a83f7ff)))
                            .when(self.do_not_ask_again, |d| {
                                d.bg(t.text)
                                    .child(text("✓", 12., 12., 600., t.menu_background))
                            }),
                    )
                    .child(text(checkbox, 13., 19., 430., t.text))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 2;
                        this.controls[2].focus(window, cx);
                        this.activate(cx);
                    })),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button(0, "Cancel", 80., t, cx))
                    .child(self.button(
                        1,
                        if revert { "Confirm" } else { "Close Window" },
                        128.,
                        t,
                        cx,
                    )),
            )
    }
    /// The multi-line text input of `Instructions…` and `Commit message`: lines with the
    /// selection and caret, scrolling with the caret. The caller sizes and frames it.
    fn editor(
        &self,
        id: &'static str,
        label: &'static str,
        placeholder: &'static str,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .role(gpui::Role::TextInput)
            .aria_label(label)
            .aria_value(self.query.clone())
            .track_focus(&self.controls[0])
            .cursor_text()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(
                self.query
                    .split('\n')
                    .scan(0usize, |start, line| {
                        let offset = *start;
                        *start += line.len() + 1;
                        Some((offset, line))
                    })
                    .map(|(start, line)| {
                        let selection = self.instruction_selection();
                        let selected =
                            selection.start.max(start)..selection.end.min(start + line.len());
                        let input = gpui::StyledText::new(
                            if self.query.is_empty() {
                                placeholder
                            } else if line.is_empty() {
                                " "
                            } else {
                                line
                            }
                            .to_owned(),
                        )
                        .with_highlights(
                            (selected.start < selected.end).then(|| {
                                (
                                    (selected.start - start)..(selected.end - start),
                                    gpui::HighlightStyle {
                                        background_color: Some(t.text_selection),
                                        ..Default::default()
                                    },
                                )
                            }),
                        );
                        let layout = input.layout().clone();
                        let caret = (self.selected == 0
                            && selection.is_empty()
                            && self.instruction_cursor >= start
                            && self.instruction_cursor <= start + line.len())
                        .then_some(self.instruction_cursor.saturating_sub(start));
                        let color = t.text;
                        div()
                            .relative()
                            .min_h(px(if self.kind == Kind::Commit { 21. } else { 20. }))
                            .text_size(px(14.))
                            .line_height(px(if self.kind == Kind::Commit { 21. } else { 20. }))
                            .when(self.kind == Kind::Commit, |d| {
                                d.font_weight(gpui::FontWeight(430.))
                            })
                            .text_color(if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            })
                            .whitespace_normal()
                            .child(input)
                            .child(
                                gpui::canvas(
                                    |_, _, _| (),
                                    move |_, (), window, _| {
                                        if let Some(cursor) = caret
                                            && let Some(position) =
                                                layout.position_for_index(cursor)
                                        {
                                            window.paint_quad(gpui::fill(
                                                gpui::Bounds::new(
                                                    position,
                                                    gpui::size(px(1.), px(18.)),
                                                ),
                                                color,
                                            ));
                                        }
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            )
                    }),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.selected = 0;
                this.select_all = false;
                this.controls[0].focus(window, cx);
                cx.notify();
            }))
    }
    /// The local task editor keeps the draft until the host confirms the write.
    fn schedule_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let label = t.text.opacity(0.65);
        let valid = !self.query.trim().is_empty() && self.schedule_rule().is_some() && !self.saving;
        let field = |name: &'static str, value: SharedString, width: f32| {
            div()
                .h(px(48.))
                .px(px(20.))
                .flex()
                .items_center()
                .justify_between()
                .child(text(name, 14., 20., 400., t.text))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            text(value, 14., 20., 400., label)
                                .w(px(width))
                                .flex()
                                .justify_end(),
                        )
                        .child(icon("chevron-down", 20., label).ml(px(8.))),
                )
        };
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text("Schedule a task", 16., 26., 590., t.text))
            .child(
                self.editor(
                    "schedule-instructions",
                    "Instructions…",
                    "Instructions…",
                    t,
                    cx,
                )
                .mt(px(20.))
                .h(px(122.))
                .rounded(px(20.))
                .border_1()
                .border_color(t.border)
                .px(px(18.))
                .py(px(16.)),
            )
            .child(
                div()
                    .mt(px(16.))
                    .h(px(98.))
                    .rounded(px(20.))
                    .border_1()
                    .border_color(t.border_strong)
                    .bg(t.surface_elevated_secondary.opacity(0.96))
                    .flex()
                    .flex_col()
                    .child(
                        field("Repeat", REPEAT[self.repeat].0.into(), 120.)
                            .id("schedule-repeat")
                            .role(gpui::Role::Button)
                            .aria_label("Repeat")
                            .track_focus(&self.controls[3])
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected = 3;
                                this.controls[3].focus(window, cx);
                                this.activate(cx);
                            }))
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .h(px(1.))
                                    .bg(t.border),
                            ),
                    )
                    .child(
                        field(
                            "Time",
                            if self.selected == 4 || self.schedule_time != "09:00" {
                                format!("{} (local)", self.schedule_time).into()
                            } else {
                                schedule_time().into()
                            },
                            150.,
                        )
                        .id("schedule-time")
                        .role(gpui::Role::TextInput)
                        .aria_label("Time (24-hour HH:MM, local)")
                        .aria_value(self.schedule_time.clone())
                        .track_focus(&self.controls[4])
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.selected = 4;
                            this.select_all = true;
                            this.controls[4].focus(window, cx);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div().mt(px(16.)).h(px(28.)).flex().child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .child(text(
                            self.error
                                .as_deref()
                                .unwrap_or(if self.schedule_rule().is_none() {
                                    "Enter a local time in HH:MM format."
                                } else {
                                    "Local schedule"
                                })
                                .to_owned(),
                            14.,
                            18.,
                            400.,
                            if self.error.is_some() || self.schedule_rule().is_none() {
                                t.text_error
                            } else {
                                label
                            },
                        )),
                ),
            )
            .child(
                div()
                    .mt(px(20.))
                    .h(px(36.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("schedule-cancel")
                            .role(gpui::Role::Button)
                            .aria_label("Cancel")
                            .track_focus(&self.controls[1])
                            .w(px(80.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .bg(t.text_alpha_5)
                            .hover(|s| s.bg(t.text_alpha_10))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text("Cancel", 14., 20., 500., t.text))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
                    )
                    .child(
                        div()
                            .id("schedule-create")
                            .role(gpui::Role::Button)
                            .aria_label("Create")
                            .aria_disabled(!valid)
                            .track_focus(&self.controls[5])
                            .w(px(79.))
                            .h(px(36.))
                            .rounded(px(18.))
                            .bg(t.text)
                            .border_1()
                            .border_color(t.border)
                            .opacity(if valid { 1. } else { 0.4 })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text(
                                if self.saving { "Creating…" } else { "Create" },
                                14.,
                                20.,
                                500.,
                                t.surface_elevated_secondary,
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected = 5;
                                this.controls[5].focus(window, cx);
                                this.activate(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("schedule-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .track_focus(&self.controls[2])
                    .absolute()
                    .right(px(14.))
                    .top(px(16.))
                    .size(px(32.))
                    .child(ButtonVisual {
                        glyph: icon("close", 20., t.text_tertiary),
                        radius: px(16.),
                        hover: t.ghost_hover,
                        foreground: t.text,
                        selected: false,
                    })
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
            .when(self.repeat_open, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(20.))
                        .top(px(145.))
                        .w(px(190.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.menu_background)
                        .py(px(4.))
                        .children(REPEAT.iter().enumerate().map(|(index, (label, _))| {
                            div()
                                .id(("schedule-repeat-option", index))
                                .role(gpui::Role::MenuItem)
                                .aria_label(*label)
                                .h(px(28.))
                                .px(px(12.))
                                .flex()
                                .items_center()
                                .when(self.repeat == index, |d| d.bg(t.list_hover))
                                .hover(|s| s.bg(t.list_hover))
                                .child(text(*label, 14., 20., 400., t.text))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.repeat = index;
                                    this.repeat_open = false;
                                    cx.notify();
                                }))
                        })),
                )
            })
    }
    /// `Rename chat` (420 x 185, p-20): heading 20/28, hint 14/21 at 50 %, the 36 px
    /// `Chat title` input (13 px, 1 px ring, blue when focused), `Cancel` and `Save`, and
    /// `Close dialog` 24 px at the top right.
    fn rename_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let focused = self.selected == 0;
        div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text(self.name_title(), 20., 28., 600., t.text))
            .child(
                text(
                    if self.kind == Kind::Rename {
                        "Keep it short and recognizable"
                    } else {
                        "Group chats and projects however you like"
                    },
                    14.,
                    21.,
                    430.,
                    t.text.opacity(0.498),
                )
                .mt(px(4.)),
            )
            .child(
                div()
                    .id("rename-title")
                    .role(gpui::Role::TextInput)
                    .aria_label(self.name_label())
                    .aria_value(self.query.clone())
                    .aria_placeholder(if self.kind == Kind::Rename {
                        "Add a title…"
                    } else {
                        "Section name"
                    })
                    .track_focus(&self.controls[0])
                    .mt(px(12.))
                    .h(px(36.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if focused {
                        rgba(0x3a83f7ff).into()
                    } else {
                        t.border_strong
                    })
                    .bg(t.surface_elevated_secondary.opacity(0.96))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .cursor_text()
                    .child(
                        text(
                            if self.query.is_empty() {
                                if self.kind == Kind::Rename {
                                    "Add a title…"
                                } else {
                                    "Section name"
                                }
                                .into()
                            } else {
                                self.query.clone()
                            },
                            13.,
                            18.5714,
                            430.,
                            if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(
                            self.select_all && !self.query.is_empty(),
                            |d| {
                                d.bg(if self.kind == Kind::Rename {
                                    rgba(0x3a83f766).into()
                                } else {
                                    t.text_selection
                                })
                            },
                        ),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button(2, "Cancel", 80., t, cx))
                    .child(self.button(
                        1,
                        self.name_submit(),
                        if self.kind == Kind::NewSection {
                            126.
                        } else {
                            66.
                        },
                        t,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("rename-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .absolute()
                    .right(px(16.))
                    .top(px(16.))
                    .size(px(24.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(t.ghost_hover))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("close", 16., t.text.opacity(0.8)))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
    }
    fn shortcuts_content(&self, visible: &[&Shortcut], t: &Theme, cx: &Context<Self>) -> Div {
        let mut list = div().flex().flex_col().pr(px(22.));
        let mut heading = "";
        for (index, &&(section, title, keys)) in visible.iter().enumerate() {
            if section != heading {
                list = list.child(
                    div()
                        .h(px(46.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .child(text(section, 14., 20., 500., t.text)),
                );
                heading = section;
            }
            list = list.child(
                div()
                    .h(px(30.))
                    .when(
                        visible.get(index + 1).is_some_and(|next| next.0 == section),
                        |d| d.mb(px(6.)),
                    )
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .opacity(0.75)
                    .child(text(title, 14., 18., 400., t.text))
                    .child(div().flex().gap(px(8.)).children(keys.iter().map(|key| {
                        div()
                            .rounded(px(6.))
                            .px(px(6.))
                            .py(px(2.))
                            .bg(t.list_hover)
                            .child(text(*key, 12., 12., 400., t.text))
                    }))),
            );
        }
        if visible.is_empty() {
            list = list
                .child(text("No matching shortcuts", 14., 20., 400., t.text_secondary).py(px(24.)));
        }
        div()
            .p(px(20.))
            .size_full()
            .flex()
            .flex_col()
            .child(text("Keyboard shortcuts", 20., 28., 600., t.text).flex_shrink_0())
            .child(
                div()
                    .id("shortcuts-search")
                    .role(gpui::Role::TextInput)
                    .aria_label("Search keyboard shortcuts")
                    .aria_value(self.query.clone())
                    .aria_placeholder("Search shortcuts")
                    .track_focus(&self.controls[0])
                    .mt(px(12.))
                    .h(px(32.))
                    .flex_shrink_0()
                    .rounded(px(999.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.menu_background)
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .overflow_hidden()
                    .child(icon("search", 16., t.text_secondary))
                    .child(
                        text(
                            if self.query.is_empty() {
                                "Search shortcuts".into()
                            } else {
                                self.query.clone()
                            },
                            14.,
                            18.,
                            400.,
                            if self.query.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .when(self.select_all, |d| d.bg(rgba(0x3a83f766))),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.selected = 0;
                        this.controls[0].focus(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("shortcuts-list")
                    .mt(px(12.))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(list),
            )
            .child(
                div()
                    .id("shortcuts-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .track_focus(&self.controls[1])
                    .absolute()
                    .right(px(16.))
                    .top(px(16.))
                    .size(px(24.))
                    .rounded(px(6.))
                    .when(self.selected == 1, |d| {
                        d.border_2().border_color(rgba(0x3a83f7ff))
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(text("×", 20., 24., 400., t.text_secondary))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Outcome::Dismiss))),
            )
    }
}

impl Dialog {
    fn form(&self) -> &CommitForm {
        self.commit.as_ref().expect("commit form")
    }

    fn form_mut(&mut self) -> &mut CommitForm {
        self.commit.as_mut().expect("commit form")
    }

    fn commit_tab_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        order.push(2);
        order.extend(self.form().new_branch.then_some(3));
        if self.form().repo.all.files > 0 {
            order.extend([0, 1]);
        }
        order.extend([4, 5, 6]);
        order
    }

    /// The `Commit to` item that is chosen: the current branch comes first.
    fn menu_selection(&self) -> usize {
        usize::from(self.form().repo.branch.is_some() && self.form().new_branch)
    }

    fn choose_target(&mut self, item: usize) {
        let current = self.form().repo.branch.is_some();
        let form = self.form_mut();
        form.new_branch = !(current && item == 0);
        form.menu = None;
    }

    /// `(enabled, why not)` of `Commit`, `Commit and push` and `Push` (the original's
    /// `review.commit.disabled.*` and `localConversationPage.push*` tooltips).
    fn commit_state(&self) -> [(bool, Option<&'static str>); 3] {
        if self.saving {
            return [(false, Some("Working…")); 3];
        }
        let form = self.form();
        let name = form.branch_name.trim();
        let new = form.new_branch;
        let blocked = new
            && (name.is_empty()
                || name.ends_with('/')
                || form.repo.branches.iter().any(|branch| branch == name));
        let selection = if form.include_unstaged {
            form.repo.all
        } else {
            form.repo.staged
        };
        let no_changes = (selection.files == 0).then_some("No changes to commit");
        let commit = (!blocked && no_changes.is_none(), no_changes);
        let nothing_to_push = (form.repo.ahead == 0).then_some("No new commits to push");
        let push = if new {
            nothing_to_push
        } else if form.repo.branch.is_none() {
            Some("Branch information unavailable")
        } else {
            nothing_to_push
        };
        [commit, commit, (!blocked && push.is_none(), push)]
    }

    fn submit_commit(&mut self, step: CommitStep, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if !self.commit_state()[step as usize].0 {
            return;
        }
        let form = self.form();
        cx.emit(Outcome::Commit {
            step,
            message: self.query.trim().to_owned(),
            include_unstaged: form.include_unstaged,
            new_branch: form.new_branch.then(|| form.branch_name.trim().to_owned()),
        });
    }

    /// The original uses 4 px gaps between its fields and between its 31 px action rows.
    fn commit_height(&self) -> f32 {
        10. + 36.
            + if self.form().new_branch { 33. } else { 0. }
            + if self.form().repo.all.files > 0 {
                80. + 41. + 8.
            } else {
                0.
            }
            + 4.
            + 9.
            + 101.
            + if self.error.is_some() { 65. } else { 0. }
    }

    fn commit_content(&self, t: &Theme, cx: &Context<Self>) -> Div {
        let form = self.form();
        let new = form.new_branch;
        let state = self.commit_state();
        let target: SharedString = if new {
            "New branch".into()
        } else {
            form.repo
                .branch
                .clone()
                .unwrap_or_else(|| "-".into())
                .into()
        };
        let selection = if form.include_unstaged {
            form.repo.all
        } else {
            form.repo.staged
        };
        let recorded_icon = |key: &'static str, src: &'static str, color: gpui::Hsla| {
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    if !shapes::paint_icon(
                        src.as_bytes(),
                        key,
                        bounds,
                        false,
                        color,
                        None,
                        window,
                        cx,
                    ) {
                        window
                            .paint_svg(
                                bounds,
                                key.into(),
                                Some(src.as_bytes()),
                                gpui::TransformationMatrix::unit(),
                                color,
                                cx,
                            )
                            .ok();
                    }
                },
            )
            .size(px(16.))
            .flex_none()
        };
        let branch_icon = r#"<svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><circle cx="5.4165" cy="5" r="1.875" stroke="currentColor" stroke-width="1.33"/><circle cx="5.4165" cy="15" r="1.875" stroke="currentColor" stroke-width="1.33"/><circle cx="14.5833" cy="5" r="1.875" stroke="currentColor" stroke-width="1.33"/><path d="M5.4165 6.66664V13.3333" stroke="currentColor" stroke-width="1.33" stroke-linejoin="round"/><path d="M5.41658 12.5V11.6667C5.41658 10.7462 6.16278 10 7.08325 10H12.9166C13.8371 10 14.5833 9.25381 14.5833 8.33333V7.5" stroke="currentColor" stroke-width="1.33" stroke-linejoin="round"/></svg>"#;
        let commit_icon = r#"<svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M13.5013 10.0003C13.5013 8.06653 11.9341 6.49856 10.0003 6.49838C8.06641 6.49838 6.49837 8.06642 6.49837 10.0003C6.49855 11.9341 8.06652 13.5013 10.0003 13.5013C11.934 13.5011 13.5011 11.934 13.5013 10.0003ZM14.8314 10.0003C14.8312 12.6685 12.6685 14.8312 10.0003 14.8314C7.33198 14.8314 5.16847 12.6686 5.16829 10.0003C5.16829 7.33188 7.33187 5.1683 10.0003 5.1683C12.6686 5.16848 14.8314 7.33199 14.8314 10.0003Z" fill="currentColor"/><path d="M5 9.33497C5.36727 9.33497 5.66504 9.63274 5.66504 10C5.66504 10.3673 5.36727 10.665 5 10.665H1.25C0.882731 10.665 0.584961 10.3673 0.584961 10C0.584961 9.63274 0.882731 9.33497 1.25 9.33497H5Z" fill="currentColor"/><path d="M18.75 9.33497C19.1173 9.33497 19.415 9.63274 19.415 10C19.415 10.3673 19.1173 10.665 18.75 10.665H15C14.6327 10.665 14.335 10.3673 14.335 10C14.335 9.63274 14.6327 9.33497 15 9.33497H18.75Z" fill="currentColor"/></svg>"#;
        let upload_icon = r#"<svg viewBox="0 0 20 20" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M15.0001 14.9967C16.841 14.9967 18.3334 13.5044 18.3334 11.6634C18.3334 9.82246 16.841 8.33008 15.0001 8.33008C15.0001 5.56865 12.7615 3.33008 10.0001 3.33008C7.80904 3.33008 5.94715 4.73939 5.27148 6.70098C3.23605 6.97537 1.66675 8.71946 1.66675 10.8301C1.66675 12.8458 3.09817 14.5273 5 14.9134" stroke="currentColor" stroke-width="1.33" stroke-linecap="round" stroke-linejoin="round"/><path d="M7.5 12.5L10 10L12.5 12.5" stroke="currentColor" stroke-width="1.33" stroke-linecap="round" stroke-linejoin="round"/><path d="M10 10.5V17" stroke="currentColor" stroke-width="1.33" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
        let selector = div()
            .id("commit-target")
            .role(gpui::Role::Button)
            .aria_label("Commit to")
            .track_focus(&self.controls[2])
            .h(px(36.))
            .px(px(8.))
            .border_1()
            .border_color(gpui::transparent_black())
            .rounded(px(12.5))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(recorded_icon("commit-branch", branch_icon, t.text))
            .when(self.selected == 2, |d| d.bg(t.ghost_hover))
            .child(
                text(target, 14., 18., 430., t.text)
                    .text_ellipsis()
                    .min_w_0(),
            )
            .child(icon("chevron-down-small", 14., t.text))
            .hover(|s| s.bg(t.ghost_hover))
            .on_click(cx.listener(|this, _, window, cx| {
                this.selected = 2;
                this.controls[2].focus(window, cx);
                this.activate(cx);
            }));
        let menu = form.menu.map(|highlighted| {
            let current = form.repo.branch.clone();
            let mut items: Vec<(SharedString, bool)> = Vec::new();
            if let Some(branch) = &current {
                items.push((branch.clone().into(), !new));
            }
            items.push(("New branch".into(), new));
            crate::composer::popover(t, 160., |d| {
                d.child(
                    text("Commit to", 13., 18.5625, 430., t.text_tertiary)
                        .h(px(26.5625))
                        .px(px(8.))
                        .flex()
                        .items_center(),
                )
                .children(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, (label, chosen))| {
                            let new_item = label.as_ref() == "New branch";
                            div()
                                .id(("commit-target-item", index))
                                .h(px(28.5625))
                                .px(px(8.))
                                .rounded(px(15.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .when(index == highlighted, |d| d.bg(t.menu_item_hover))
                                .child(if new_item {
                                    icon("plus", 16., t.text).into_any_element()
                                } else {
                                    recorded_icon("commit-branch", branch_icon, t.text)
                                        .into_any_element()
                                })
                                .child(
                                    text(label, 13., 18.5625, 430., t.text)
                                        .text_ellipsis()
                                        .flex_1()
                                        .min_w_0(),
                                )
                                .when(chosen, |d| d.child(icon("menu-check", 16., t.text)))
                                .on_hover(cx.listener(move |this, hovered, _, cx| {
                                    if *hovered {
                                        this.form_mut().menu = Some(index);
                                        cx.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose_target(index);
                                    this.selected = 2;
                                    this.controls[2].focus(window, cx);
                                    cx.notify();
                                }))
                        }),
                )
            })
            .id("commit-target-menu")
            .absolute()
            .top(px(43.5))
            .left(px(6.))
        });
        let name_taken = new
            && form
                .repo
                .branches
                .iter()
                .any(|branch| branch == form.branch_name.trim());
        let branch_name = div()
            .id("commit-branch-name")
            .role(gpui::Role::TextInput)
            .aria_label("Branch name")
            .aria_value(form.branch_name.clone())
            .track_focus(&self.controls[3])
            .mt(px(4.))
            .h(px(29.))
            .px(px(12.))
            .pt(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .cursor_text()
            .child(
                div()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .child(
                        text(
                            if form.branch_name.is_empty() {
                                "codex/".to_owned()
                            } else {
                                form.branch_name.clone()
                            },
                            14.,
                            21.,
                            430.,
                            if form.branch_name.is_empty() {
                                t.text_tertiary
                            } else {
                                t.text
                            },
                        )
                        .text_ellipsis()
                        .min_w_0()
                        .when(self.select_all && !form.branch_name.is_empty(), |d| {
                            d.bg(t.text_selection)
                        }),
                    )
                    .when(self.selected == 3 && !self.select_all, |d| {
                        d.child(div().w(px(1.)).h(px(18.)).bg(t.text))
                    }),
            )
            .when(name_taken, |d| {
                d.child(text("Branch already exists", 12., 16., 430., t.text_error))
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.selected = 3;
                this.select_all = true;
                this.controls[3].focus(window, cx);
                cx.notify();
            }));
        let include = div()
            .id("commit-include")
            .role(gpui::Role::CheckBox)
            .aria_label("Include unstaged changes")
            .aria_toggled(if form.include_unstaged {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .track_focus(&self.controls[1])
            .h(px(41.))
            .mt(px(4.))
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
                    .border_color(if self.selected == 1 {
                        rgba(0x3a83f7ff).into()
                    } else {
                        t.border_strong
                    })
                    .bg(t.surface_elevated_secondary.opacity(0.96))
                    .when(form.include_unstaged, |d| {
                        d.child(icon("menu-check", 12., t.text))
                    }),
            )
            .child(text("Include unstaged changes", 14., 21., 430., t.text).flex_1())
            .child(if selection.files == 0 {
                text("No changes", 12., 16., 430., t.text_tertiary).into_any_element()
            } else {
                crate::screens::diff::panel::stats(selection.additions, selection.deletions, 14., t)
                    .into_any_element()
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.selected = 1;
                this.controls[1].focus(window, cx);
                this.activate(cx);
            }));
        let labels = [(4, "Commit"), (5, "Commit and push"), (6, "Push")];
        // cmdk keeps one row selected whatever has the key focus: the focused row, else the
        // first that runs.
        let highlighted = if (4..=6).contains(&self.selected) {
            Some(self.selected)
        } else {
            (0..3).find(|i| state[*i].0).map(|i| i + 4)
        };
        let rows = labels.into_iter().map(|(index, label)| {
            let (enabled, reason) = state[index - 4];
            let on = enabled && highlighted == Some(index);
            let color = match (enabled, on) {
                (false, _) => t.text.opacity(0.25),
                (true, true) => t.text,
                (true, false) => t.text.opacity(0.75),
            };
            let row = div()
                .id(("commit-row", index))
                .role(gpui::Role::Button)
                .aria_label(label)
                .track_focus(&self.controls[index])
                .relative()
                .py(px(5.))
                .px(px(8.))
                .rounded(px(12.5))
                .flex()
                .items_center()
                .gap(px(8.))
                .when(on, |d| d.bg(t.ghost_hover))
                .child(recorded_icon(
                    if index == 4 {
                        "commit-action"
                    } else {
                        "commit-upload"
                    },
                    if index == 4 { commit_icon } else { upload_icon },
                    color,
                ))
                .child(text(label, 14., 21., 430., color).flex_1().min_w_0())
                .when(on, |d| {
                    d.child(
                        div().flex_none().h(px(21.)).pt(px(3.)).child(
                            div()
                                .h(px(16.))
                                .px(px(6.))
                                .rounded(px(10.))
                                .bg(t.text_alpha_10)
                                .flex()
                                .items_center()
                                .child(text("⌘↵", 12., 12., 430., t.text).opacity(0.8)),
                        ),
                    )
                })
                .when(enabled, |d| {
                    d.hover(|s| s.bg(t.ghost_hover)).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.selected = index;
                            this.controls[index].focus(window, cx);
                            this.activate(cx);
                        },
                    ))
                });
            match reason.filter(|_| !enabled) {
                Some(reason) => tooltip(row, reason, None, Side::Top).into_any_element(),
                None => row.into_any_element(),
            }
        });
        div()
            .relative()
            .p(px(5.))
            .flex()
            .flex_col()
            .child(selector)
            .when(new, |d| d.child(branch_name))
            .when(form.repo.all.files > 0, |d| {
                d.child(
                    self.editor(
                        "commit-message",
                        "Commit message",
                        "Commit message (leave blank to generate)…",
                        t,
                        cx,
                    )
                    .h(px(80.))
                    .mt(px(4.))
                    .px(px(12.))
                    .py(px(8.)),
                )
                .child(include)
            })
            .child(div().mt(px(4.)).h(px(1.)).bg(t.border))
            .child(
                div()
                    .py(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(rows),
            )
            .when_some(self.error.as_ref(), |d, error| {
                d.child(
                    div()
                        .id("commit-error")
                        .max_h(px(65.))
                        .overflow_y_scroll()
                        .child(
                            text(error.clone(), 12., 18., 430., t.text_error)
                                .px(px(12.))
                                .whitespace_normal(),
                        ),
                )
            })
            .children(menu)
    }
}

/// What a test reads from the open commit form.
impl Dialog {
    /// The visible rows with whether each runs and its tooltip when it does not.
    pub fn commit_rows(&self) -> Vec<(&'static str, bool, Option<&'static str>)> {
        let state = self.commit_state();
        vec![
            ("Commit", state[0].0, state[0].1),
            ("Commit and push", state[1].0, state[1].1),
            ("Push", state[2].0, state[2].1),
        ]
    }

    /// `New branch` with its name, or the branch the commit lands on.
    pub fn commit_target(&self) -> (bool, String) {
        let form = self.form();
        if form.new_branch {
            (true, form.branch_name.clone())
        } else {
            (false, form.repo.branch.clone().unwrap_or_default())
        }
    }

    pub fn commit_includes_unstaged(&self) -> bool {
        self.form().include_unstaged
    }
}

const REPEAT: [(&str, &str); 8] = [
    ("Daily", ""),
    ("Mondays", "MO"),
    ("Tuesdays", "TU"),
    ("Wednesdays", "WE"),
    ("Thursdays", "TH"),
    ("Fridays", "FR"),
    ("Saturdays", "SA"),
    ("Sundays", "SU"),
];
// Original gradient-e8375b62d61e.png, retained unchanged for SH-142.
static ANNOUNCEMENT_ARTWORK: LazyLock<RgbaImage> = LazyLock::new(|| {
    let mut image = image::load_from_memory(include_bytes!(
        "../../../assets/ui/announcement/gradient.png"
    ))
    .expect("original announcement PNG")
    .to_rgba8();
    shapes::premultiply(&mut image);
    image
});

fn announcement_artwork(cache: AnnouncementArtworkCache) -> impl IntoElement {
    let radius = Corners {
        top_left: px(16.),
        top_right: px(16.),
        ..Default::default()
    };
    canvas(
        move |bounds, window, _| {
            let scale = window.scale_factor();
            let mut cached = cache.borrow_mut();
            if let Some((previous, previous_scale, image, rounded)) = &*cached
                && *previous == bounds
                && *previous_scale == scale
            {
                return (image.clone(), *rounded);
            }
            if let Some((_, _, image, _)) = cached.take() {
                window.drop_image(image).ok();
            }
            let image = &*ANNOUNCEMENT_ARTWORK;
            let device = |value: Pixels| (f32::from(value) * scale).round() as u32;
            let (width, height) = (device(bounds.size.width), device(bounds.size.height));
            let factor =
                (width as f32 / image.width() as f32).max(height as f32 / image.height() as f32);
            let axis = |boxed: f32, length: f32| {
                let start = ((boxed - length) / 2. + 0.5).floor();
                (start, ((boxed + length) / 2. + 0.5).floor() - start)
            };
            let (x, image_width) = axis(width as f32, image.width() as f32 * factor);
            let (y, image_height) = axis(height as f32, image.height() as f32 * factor);
            let logical = |value: f32| px(value / scale);
            let destination = Bounds::new(
                bounds.origin + point(logical(x), logical(y)),
                size(logical(image_width), logical(image_height)),
            );
            let raster = shapes::raster_images(
                &[(image, destination)],
                bounds,
                radius,
                shapes::Corner::Round,
                shapes::ImageComposition::Direct(rgba(0x00000000).into()),
                window,
            );
            let rounded = raster.is_none();
            let raster = Arc::new(raster.unwrap_or_else(|| {
                let scaled = image::imageops::resize(
                    image,
                    image_width as u32,
                    image_height as u32,
                    image::imageops::FilterType::Triangle,
                );
                let pixels =
                    image::imageops::crop_imm(&scaled, (-x) as u32, (-y) as u32, width, height)
                        .to_image();
                RenderImage::new([Frame::new(pixels)])
            }));
            *cached = Some((bounds, scale, raster.clone(), rounded));
            (raster, rounded)
        },
        move |bounds, (image, rounded), window, _| {
            let corners = if rounded { radius } else { Corners::default() };
            window
                .paint_image(bounds, bounds, corners, image, 0, false)
                .ok();
        },
    )
    .size_full()
}

/// The editor's default `Time`: 9:00 AM in the local zone, named like the original's
/// `Intl` short zone name (`PT`, `GMT+2`).
/// ponytail: US zones by offset, else `GMT±N`; a tz database lookup if other names matter.
fn schedule_time() -> String {
    let hours = chrono::Local::now().offset().local_minus_utc() / 3600;
    let zone = match hours {
        -8 | -7 => "PT".into(),
        -6 => "MT".into(),
        -5 => "CT".into(),
        -4 => "ET".into(),
        0 => "GMT".into(),
        h => format!("GMT{h:+}"),
    };
    format!("9:00 AM {zone}")
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let visible: Vec<_> = if self.kind == Kind::Shortcuts {
            self.visible().collect()
        } else {
            Vec::new()
        };
        let mut previous = "";
        let mut sections = 0;
        for &&(section, _, _) in &visible {
            if previous != section {
                sections += 1;
                previous = section;
            }
        }
        let shortcuts_height =
            (124. + visible.len() as f32 * 36. + sections as f32 * 40.).clamp(520., 560.);
        let schedule = self.kind == Kind::ScheduleTask;
        let announcement = self.kind == Kind::ModelAnnouncement;
        let commit = self.kind == Kind::Commit;
        let (width, height): (f32, f32) = match self.kind {
            Kind::Shortcuts => (600., shortcuts_height),
            Kind::CloseWindow => (420., 171.),
            Kind::RevertChanges => (420., 199.),
            Kind::ScheduleTask => (680., 422.),
            Kind::Rename | Kind::NewSection | Kind::EditSection => (420., 185.),
            Kind::ArchiveSection => (420., 181.),
            Kind::FullAccess => (520., 420.),
            Kind::LogOut => (380., 145. + if self.email.is_some() { 25. } else { 0. }),
            Kind::ModelAnnouncement => (480., 432.),
            Kind::Commit => (420., self.commit_height()),
            Kind::Branches => (
                300.,
                152. + self.branch_names().len().min(8) as f32 * 34.
                    + if self.error.is_some() { 80. } else { 0. },
            ),
            Kind::CreateBranch => (420., 190. + if self.error.is_some() { 65. } else { 0. }),
            Kind::BranchConflict => (
                420.,
                260. + self.branch.as_ref().unwrap().conflicts.len().min(5) as f32 * 22.,
            ),
        };
        let width = width.min(f32::from(viewport.width) * 0.92);
        let height = height.min(f32::from(viewport.height) * 0.92);
        let content = match self.kind {
            Kind::Shortcuts => self.shortcuts_content(&visible, t, cx),
            Kind::CloseWindow | Kind::RevertChanges => self.close_content(t, cx),
            Kind::ScheduleTask => self.schedule_content(t, cx),
            Kind::Rename | Kind::NewSection | Kind::EditSection => self.rename_content(t, cx),
            Kind::ArchiveSection => self.archive_section_content(t, cx),
            Kind::FullAccess => self.full_access_content(t, cx),
            Kind::LogOut => self.logout_content(t, cx),
            Kind::ModelAnnouncement => self.announcement_content(t, cx),
            Kind::Commit => self.commit_content(t, cx),
            Kind::Branches | Kind::CreateBranch | Kind::BranchConflict => {
                self.branch_content(t, cx)
            }
        };
        // `codex-dialog-overlay`: the schedule dialog's `_dimmedOverlay` is rgba(0,0,0,.5) with
        // blur(1px); the blur is not drawn (flat fills are unchanged by it).
        let top = if self.kind == Kind::Branches {
            126.
        } else if schedule {
            83.
        } else {
            (f32::from(viewport.height) - height) / 2.
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(if self.kind == Kind::Branches {
                gpui::transparent_black()
            } else if schedule {
                t.dialog_backdrop
            } else {
                rgba(0x00000022).into()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.dismiss(cx)),
            )
            .child(
                div()
                    .id("modal-dialog")
                    .role(gpui::Role::Dialog)
                    .aria_label(match self.kind {
                        Kind::Shortcuts => "Keyboard shortcuts".into(),
                        Kind::CloseWindow => "Are you sure you want to close the window?".into(),
                        Kind::RevertChanges => "Revert changes?".into(),
                        Kind::ScheduleTask => "Schedule a task".into(),
                        Kind::Rename | Kind::NewSection | Kind::EditSection => {
                            self.name_title().into()
                        }
                        Kind::ArchiveSection => self
                            .archive_section
                            .as_ref()
                            .expect("archive section")
                            .title
                            .clone()
                            .into(),
                        Kind::FullAccess => "Turn on Full Access?".into(),
                        Kind::LogOut => "Log out?".into(),
                        Kind::Commit => "Commit or push".into(),
                        Kind::Branches => "Switch branch".into(),
                        Kind::CreateBranch => "Create and checkout branch".into(),
                        Kind::BranchConflict => "Commit changes to switch branch".into(),
                        Kind::ModelAnnouncement => self
                            .announcement
                            .as_ref()
                            .expect("model announcement")
                            .title
                            .clone(),
                    })
                    .on_key_down(cx.listener(Self::key))
                    .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                        this.select_all = true;
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Paste, _, cx| {
                        if let Some(s) = cx.read_from_clipboard().and_then(|c| c.text()) {
                            this.insert(&s, cx);
                        }
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Copy, _, cx| {
                        if this.multiline() {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.query[this.instruction_selection()].to_owned(),
                            ));
                            cx.stop_propagation();
                            return;
                        }
                        if this.select_all && this.editing() {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.input().clone(),
                            ));
                        }
                        cx.stop_propagation();
                    }))
                    .on_action(cx.listener(|this, _: &Cut, _, cx| {
                        if this.multiline() && !this.saving {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.query[this.instruction_selection()].to_owned(),
                            ));
                            this.replace_instructions("", cx);
                            cx.stop_propagation();
                            return;
                        }
                        if this.select_all && this.editing() && !this.saving {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(std::mem::take(
                                this.input(),
                            )));
                            this.select_all = false;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(px(if self.kind == Kind::Branches {
                        (f32::from(viewport.width) - width - 20.).max(0.)
                    } else {
                        (f32::from(viewport.width) - width) / 2.
                    }))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(if announcement {
                        16.
                    } else if commit {
                        20.
                    } else if schedule {
                        30.
                    } else if self.kind == Kind::LogOut {
                        25.
                    } else {
                        24.
                    }))
                    .when(!commit && self.kind != Kind::LogOut, |d| {
                        d.overflow_hidden()
                    })
                    .map(|d| {
                        if announcement {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(16.)),
                                fill: Some(t.surface_elevated_secondary),
                                shadows: vec![crate::composer::shadow(
                                    rgba(0x0000001a).into(),
                                    4.,
                                    8.,
                                    -2.,
                                    false,
                                )],
                                ..Default::default()
                            }))
                        } else if commit {
                            // The command menu's surface, which the original's commit form uses.
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(20.)),
                                fill: Some(t.surface_elevated_secondary),
                                border: Some((
                                    px(1.),
                                    if t.appearance == theme::Appearance::Dark {
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
                        } else if !schedule {
                            d.child(shapes::backdrop_blur(
                                px(24.),
                                shapes::Fill::Color,
                                false,
                                shapes::Decoration {
                                    radius: Corners::all(px(if self.kind == Kind::LogOut {
                                        25.
                                    } else {
                                        24.
                                    })),
                                    corner: if self.kind == Kind::LogOut {
                                        shapes::Corner::Squircle
                                    } else {
                                        shapes::Corner::default()
                                    },
                                    fill: Some(gpui::Hsla {
                                        a: 0.9,
                                        ..t.menu_background
                                    }),
                                    shadows: vec![
                                        crate::composer::shadow(t.border, 0., 0., 0.5, false),
                                        crate::composer::shadow(
                                            rgba(0x0000001a).into(),
                                            4.,
                                            8.,
                                            -2.,
                                            false,
                                        ),
                                    ],
                                    ..Default::default()
                                },
                            ))
                        } else {
                            d.bg(t.surface_elevated_secondary)
                        }
                    })
                    .child(content),
            )
    }
}

const SHORTCUTS: &[(&str, &str, &[&str])] = &[
    ("Chat", "New chat", &["⌘N", "⇧⌘O"]),
    ("Chat", "Archive chat", &["⇧⌘A"]),
    ("Chat", "New standalone chat", &["⌥⌘O"]),
    ("Chat", "Toggle pin", &["⌥⌘P"]),
    ("Navigation", "Find", &["⌘F"]),
    ("Navigation", "Focus tab 1", &["⌘1"]),
    ("Navigation", "Focus tab 2", &["⌘2"]),
    ("Navigation", "Focus tab 3", &["⌘3"]),
    ("Navigation", "Focus tab 4", &["⌘4"]),
    ("Navigation", "Focus tab 5", &["⌘5"]),
    ("Navigation", "Focus tab 6", &["⌘6"]),
    ("Navigation", "Focus tab 7", &["⌘7"]),
    ("Navigation", "Focus tab 8", &["⌘8"]),
    ("Navigation", "Focus tab 9", &["⌘9"]),
    ("Navigation", "Back", &["⌘[", "Mouse Back"]),
    ("Navigation", "Forward", &["⌘]", "Mouse Forward"]),
    ("Navigation", "Next recently viewed chat", &["⌃Tab"]),
    ("Navigation", "Next tab", &["⌃Tab", "⇧⌘]", "⌥⌘→"]),
    ("Navigation", "Next chat", &["⇧⌘]", "⌥⌘→"]),
    ("Navigation", "Next chat needing attention", &["⌥⌘A"]),
    ("Navigation", "Open your dot", &["⌘."]),
    ("Navigation", "Previous recently viewed chat", &["⌃⇧Tab"]),
    ("Navigation", "Previous tab", &["⌃⇧Tab", "⇧⌘[", "⌥⌘←"]),
    ("Navigation", "Previous chat", &["⇧⌘[", "⌥⌘←"]),
    ("Navigation", "Go to recent chat 1", &["⌥⌘1"]),
    ("Navigation", "Go to recent chat 2", &["⌥⌘2"]),
    ("Navigation", "Go to recent chat 3", &["⌥⌘3"]),
    ("Navigation", "Go to recent chat 4", &["⌥⌘4", "⇧⌘9"]),
    ("Navigation", "Go to recent chat 5", &["⌥⌘5", "⇧⌘8"]),
    ("Navigation", "Go to recent chat 6", &["⌥⌘6"]),
    ("Navigation", "Toggle activity view", &["⌥⌘U"]),
    ("Panels", "Open browser tab", &["⌘T"]),
    ("Panels", "Reopen closed tab", &["⇧⌘T"]),
    ("Panels", "New tab in full view", &["⇧⌘F"]),
    ("Panels", "Cycle workspace layout", &["⇧⌘B"]),
    ("Panels", "Toggle bottom panel", &["⌘J"]),
    ("Panels", "Toggle navigation panel", &["⇧⌘E"]),
    ("Panels", "Toggle sidebar", &["⇧⌘S", "⌘B"]),
    ("Panels", "New tab", &["⌥⌘B"]),
    ("Panels", "Open terminal", &["⌃`"]),
    ("Project", "Open folder", &["⌘O"]),
    ("App", "Clear all unreads", &["⇧Esc"]),
    ("App", "Show Mini", &["⌥Space"]),
    ("App", "Settings", &["⌘,"]),
    ("General", "Close other tabs", &["⌥⌘W"]),
    ("General", "Close Tab", &["⌘W"]),
    ("General", "Close", &["⌘W"]),
    ("General", "Select effort", &["⌃⇧M"]),
    ("General", "Open project picker", &["⌥⇧⌘O"]),
    ("General", "Open recent models", &["⌃⌘M"]),
    ("General", "Toggle voice chat", &["⌃⇧V"]),
    ("General", "Send message in background", &["⌘⏎"]),
    ("General", "Copy conversation path", &["⌥⇧⌘C"]),
    ("General", "Copy deeplink", &["⌥⌘L"]),
    ("General", "Copy working directory", &["⇧⌘C"]),
    ("General", "Force Reload Browser Page", &["⇧⌘R"]),
    ("General", "Voice in Mini", &["⌥V"]),
    ("General", "Open command menu", &["⌘K", "⇧⌘P"]),
    ("General", "Reload Browser Page", &["⌘R"]),
    ("General", "Rename chat", &["⌥⌘R"]),
    ("General", "Screenshot selection to current chat", &["⌃⇧4"]),
    ("General", "Screenshot selection to new chat", &["⌃⌥⇧4"]),
    ("General", "Search Files…", &["⌘P"]),
    ("General", "Show keyboard shortcuts", &["⌘/"]),
    ("General", "Go to chat 1", &["⌃1"]),
    ("General", "Go to chat 2", &["⌃2"]),
    ("General", "Go to chat 3", &["⌃3"]),
    ("General", "Go to chat 4", &["⌃4"]),
    ("General", "Go to chat 5", &["⌃5"]),
    ("General", "Go to chat 6", &["⌃6"]),
    ("General", "Go to chat 7", &["⌃7"]),
    ("General", "Go to chat 8", &["⌃8"]),
    ("General", "Go to chat 9", &["⌃9"]),
    ("General", "Toggle Notch", &["⌥C"]),
];
