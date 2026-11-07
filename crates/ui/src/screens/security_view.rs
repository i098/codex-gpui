use super::*;
use crate::{focus, icon::icon, text};
use gpui::{
    AnyElement, Div, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme, Theme};

#[derive(Clone, Copy)]
enum ButtonKind {
    Secondary,
    Primary,
    SmallPrimary,
    FormPrimary,
    Tab(bool),
    Filter,
}

impl SecurityPage {
    pub(super) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut page = div().relative().size_full().flex().flex_col();
        if self.gate != Gate::Ready {
            return self.gated_page(page, window, cx);
        }
        let split = !matches!(self.location, Location::Findings | Location::Repositories);
        let section = match self.location {
            Location::Repositories | Location::Repository(_) => "Repositories",
            Location::Findings => "Findings",
            Location::Finding { .. } if self.finding_from_global => "Findings",
            _ => "Scans",
        };
        let total = f32::from(window.viewport_size().width) - 56.;
        let height = f32::from(window.viewport_size().height) - 48.;
        let default_width = 320_f32
            .max((height * 1.6).min(total - 500.))
            .max(640_f32.min(total - 352.));
        let detail_width = self
            .detail_width
            .unwrap_or(default_width)
            .clamp(320., (total - 352.).max(320.));
        let main = self.workbench(section, split, window, cx);
        if split {
            page = self.split_workbench(page, main, detail_width, total, window, cx);
        } else {
            page = page.child(main);
        }
        if self.rendered_dialog != self.dialog {
            if self.dialog.is_some() {
                if self.rendered_dialog.is_none() {
                    self.restore_focus = window.focused(cx);
                }
                self.modal_focus.focus(window, cx);
            } else if let Some(focus) = self.restore_focus.take() {
                focus.focus(window, cx);
            }
            self.rendered_dialog = self.dialog;
        }
        if let Some(dialog) = self.dialog {
            page = page.child(self.dialog_view(dialog, window, cx));
        }
        page.into_any_element()
    }

    fn gated_page(&self, page: Div, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let title = if self.gate == Gate::Loading {
            "Codex Security"
        } else {
            "Codex Security is unavailable"
        };
        let mut empty = self.empty(title, self.gate.description(), &t);
        if self.access.remote_host {
            empty = empty.child(self.action(
                "security-use-local",
                "Use local",
                Action::UseLocal,
                false,
                window,
                cx,
            ));
        }
        page.child(div().h(px(46.)).flex_none())
            .child(div().flex_1().min_h_0().child(empty))
            .into_any_element()
    }

    fn dialog_content(
        &self,
        mut content: Stateful<Div>,
        dialog: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        content = match dialog {
            "rename" | "context" | "close" => self.edit_dialog(content, dialog, window, cx),
            "target" => {
                let mut content =
                    content.child(text("Select a repository", 18., 24., 500., t.text));
                for project in &self.projects {
                    content = content.child(self.option(
                        "target",
                        &project.path,
                        &project.name,
                        self.target == project.path,
                        window,
                        cx,
                    ));
                }
                content.child(self.action(
                    "security-use-folder",
                    "Use another folder",
                    Action::PickFolder,
                    false,
                    window,
                    cx,
                ))
            }
            "mode" => content
                .child(self.option(
                    "mode",
                    "standard",
                    "Codebase",
                    self.mode != "diff",
                    window,
                    cx,
                ))
                .child(self.option("mode", "diff", "Changes", self.mode == "diff", window, cx)),
            "depth" => content
                .child(self.option(
                    "depth",
                    "standard",
                    "Standard scan",
                    self.mode == "standard",
                    window,
                    cx,
                ))
                .child(self.option(
                    "depth",
                    "deep",
                    "Deep scan",
                    self.mode == "deep",
                    window,
                    cx,
                )),
            "area" if self.mode == "diff" => content
                .child(self.option(
                    "diff",
                    "working_tree",
                    "Working-tree changes",
                    self.diff_kind == "working_tree",
                    window,
                    cx,
                ))
                .child(self.option(
                    "diff",
                    "commit",
                    "Commit",
                    self.diff_kind == "commit",
                    window,
                    cx,
                ))
                .child(self.option(
                    "diff",
                    "range",
                    "Revision range",
                    self.diff_kind == "range",
                    window,
                    cx,
                )),
            "area" => content
                .child(self.option(
                    "area",
                    "entire",
                    "Entire repository",
                    self.area == "entire",
                    window,
                    cx,
                ))
                .child(self.option(
                    "area",
                    "folder",
                    "Folder",
                    self.area == "folder",
                    window,
                    cx,
                )),
            "model" => self.model_dialog(content, window, cx),
            "filters" => self.filters(content, window, cx),
            _ => content,
        };
        content
    }

    fn workbench(
        &self,
        section: &'static str,
        split: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut header = div()
            .layer()
            .h(px(46.))
            .flex_none()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(2.));
        for (label, location) in [
            ("Scans", Location::Scans),
            ("Findings", Location::Findings),
            ("Repositories", Location::Repositories),
        ] {
            header = header.child(self.button(
                format!("security-tab-{label}"),
                (label, None),
                Action::Navigate(location),
                ButtonKind::Tab(section == label),
                window,
                cx,
            ));
        }
        header = header.child(div().flex_1()).child(self.button(
            "security-start",
            ("Scan", Some("security-plus")),
            Action::Navigate(Location::NewScan),
            ButtonKind::Primary,
            window,
            cx,
        ));
        let t = cx.theme().clone();
        let gutter = if split { 30. } else { 20. };
        let mut search = div()
            .h(px(60.))
            .flex_none()
            .w_full()
            .pt(px(20.))
            .pb(px(8.))
            .px(px(gutter))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(!split, |d| d.max_w(px(768.)))
            .mx_auto()
            .child(
                div()
                    .relative()
                    .h(px(32.))
                    .flex_1()
                    .min_w_0()
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(px(9999.)),
                        corner: shapes::Corner::Round,
                        fill: Some(t.control_background),
                        border: Some((px(1.), t.border_strong)),
                        ..Default::default()
                    }))
                    .child(icon("security-search", 18., t.text.opacity(0.65)))
                    .child(div().flex_1().min_w_0().child(self.query.clone())),
            );
        if section != "Scans" {
            search = search.child(self.button(
                "security-filter",
                ("Filter", Some("security-filter")),
                Action::ShowDialog("filters"),
                ButtonKind::Filter,
                window,
                cx,
            ));
        }
        let list = match section {
            "Findings" => self.findings_list(window, cx),
            "Repositories" => self.repositories_list(window, cx),
            _ => self.scan_list(window, cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(t.surface)
                    .flex()
                    .flex_col()
                    .child(search)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .when(!split, |d| d.max_w(px(768.)))
                            .mx_auto()
                            .px(px(gutter))
                            .py(px(20.))
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    fn action(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        action: Action,
        primary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.button(
            id,
            (label, None),
            action,
            if primary {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            },
            window,
            cx,
        )
    }

    fn button(
        &self,
        id: impl Into<SharedString>,
        (label, glyph): (&'static str, Option<&'static str>),
        action: Action,
        kind: ButtonKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let id = id.into();
        let focus = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        let primary = matches!(
            kind,
            ButtonKind::Primary | ButtonKind::SmallPrimary | ButtonKind::FormPrimary
        );
        let (height, padding, radius, font) = match kind {
            ButtonKind::SmallPrimary => (24., 8., 9999., 13.),
            ButtonKind::FormPrimary => (32., 16., 12.5, 14.),
            ButtonKind::Filter => (28., 4., 12.5, 13.),
            _ => (28., 8., 12.5, 13.),
        };
        let (color, fill) = button_colors(primary, kind, &t);
        let disabled =
            self.busy || matches!(action, Action::Start) && self.validate_setup(cx).is_err();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_disabled(disabled)
                .relative()
                .h(px(height))
                .px(px(padding + 1.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(radius)),
                    corner: if radius > 100. {
                        shapes::Corner::Round
                    } else {
                        shapes::Corner::Squircle
                    },
                    fill,
                    border: primary.then_some((px(1.), t.border)),
                    ..Default::default()
                }))
                .children(glyph.map(|name| {
                    icon(
                        name,
                        if matches!(kind, ButtonKind::Filter) {
                            18.
                        } else {
                            16.
                        },
                        color,
                    )
                }))
                .when(!matches!(kind, ButtonKind::Filter), |d| {
                    d.child(text(label, font, 18., 430., color))
                })
                .when(disabled, |d| d.opacity(0.4)),
            &focus,
            px(radius),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                this.activate(action.clone(), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.activate(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
    }

    fn empty(&self, title: &str, description: Option<&str>, t: &Theme) -> Div {
        self.empty_with_action(title, description, None, t)
    }

    fn empty_with_action(
        &self,
        title: &str,
        description: Option<&str>,
        action: Option<AnyElement>,
        t: &Theme,
    ) -> Div {
        let mut content = div()
            .w_full()
            .max_w(px(576.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .child(icon(
                if self.loading {
                    "sidebar-cmd-spinner"
                } else {
                    "security"
                },
                32.,
                t.text.opacity(0.65),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(text(title.to_owned(), 16., 24., 500., t.text))
                    .children(description.map(|value| {
                        text(
                            value.to_owned(),
                            13.,
                            18.571_428,
                            430.,
                            t.text.opacity(0.65),
                        )
                    })),
            );
        if let Some(action) = action {
            content = content.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(action),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .px(px(12.))
            .py(px(24.))
            .child(content)
    }

    fn list_state(
        &self,
        loaded: bool,
        empty: bool,
        title: &str,
        description: &str,
        t: &Theme,
    ) -> Option<Div> {
        if !loaded && self.loading {
            return Some(
                div().child(
                    self.empty("Loading Security data", None, t)
                        .id("security-loading")
                        .role(gpui::Role::Status),
                ),
            );
        }
        if !loaded {
            return Some(self.empty(title, self.error.as_deref(), t));
        }
        if empty {
            return Some(self.empty(title, Some(description), t));
        }
        None
    }

    fn scan_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        let rows = self
            .scans
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|scan| scan_matches(scan, &query, &self.scan_mode, &self.scan_status))
            .collect::<Vec<_>>();
        let title = if self.scans.is_none() && !self.loading {
            "Scans unavailable"
        } else if self.scans.as_ref().is_some_and(|scans| !scans.is_empty()) {
            "No matching scans"
        } else {
            "No Security scans yet"
        };
        if self.scans.as_ref().is_some_and(Vec::is_empty) {
            let action = self
                .button(
                    "security-first-scan",
                    ("Start a scan", None),
                    Action::Navigate(Location::NewScan),
                    ButtonKind::SmallPrimary,
                    window,
                    cx,
                )
                .into_any_element();
            return self
                .empty_with_action(
                    title,
                    Some("Scan a codebase or its changes to find security issues"),
                    Some(action),
                    &t,
                )
                .into_any_element();
        }
        if let Some(state) = self.list_state(self.scans.is_some(), rows.is_empty(), title, "", &t) {
            return state.into_any_element();
        }
        let mut list = div()
            .id("security-scan-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.));
        for scan in rows {
            let id = scan.scan_id.clone();
            let selected = match &self.location {
                Location::Scan(current)
                | Location::Finding {
                    scan_id: current, ..
                } => *current == id,
                _ => false,
            };
            let row = self.row(
                format!("security-scan-{id}"),
                [
                    scan.title(),
                    &scan.target_path,
                    scan.status_label(),
                    &format!("{} findings", scan.finding_count),
                ],
                selected,
                Action::Navigate(Location::Scan(id)),
                window,
                cx,
            );
            list = list.child(row);
        }
        list.into_any_element()
    }

    fn row(
        &self,
        id: impl Into<SharedString>,
        [title, subtitle, right, bottom_right]: [&str; 4],
        selected: bool,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let id = id.into();
        let focus = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(title.to_owned())
                .aria_selected(selected)
                .relative()
                .w_full()
                .px(px(12.))
                .py(px(9.))
                .rounded(px(10.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .when(selected, |d| d.bg(t.list_hover))
                .hover(|d| d.bg(t.list_hover))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(16.))
                        .child(text(title.to_owned(), 14., 20., 500., t.text))
                        .child(text(right.to_owned(), 12., 16., 430., t.text_secondary)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(16.))
                        .child(text(subtitle.to_owned(), 12., 16., 430., t.text_secondary))
                        .child(text(
                            bottom_right.to_owned(),
                            12.,
                            16.,
                            430.,
                            t.text_secondary,
                        )),
                ),
            &focus,
            px(10.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.activate(action.clone(), cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.activate(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
    }

    fn findings_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let rows = self
            .findings
            .as_ref()
            .map(|page| page.findings.as_slice())
            .unwrap_or(&[]);
        let filtered = !self.finding_filter.query.trim().is_empty()
            || self.finding_filter.status.is_some()
            || self.finding_filter.severity.is_some()
            || self.finding_filter.target_id.is_some();
        let title = if self.findings.is_none() && !self.loading {
            "Findings unavailable"
        } else if filtered {
            "No matching findings"
        } else {
            "No findings yet"
        };
        if let Some(state) = self.list_state(
            self.findings.is_some(),
            rows.is_empty(),
            title,
            "Completed scans with findings will appear here",
            &t,
        ) {
            return state.into_any_element();
        }
        let mut list = div()
            .id("security-findings-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.));
        for finding in rows {
            let scan_id = value(finding, "scanId").to_owned();
            let occurrence_id = value(finding, "occurrenceId").to_owned();
            let severity = value(&finding["severity"], "level");
            list = list.child(self.row(
                format!("security-finding-{occurrence_id}"),
                [
                    value(finding, "title"),
                    value(finding, "locationPath"),
                    severity,
                    value(finding, "status"),
                ],
                false,
                Action::Navigate(Location::Finding {
                    scan_id,
                    occurrence_id,
                }),
                window,
                cx,
            ));
        }
        if self
            .findings
            .as_ref()
            .and_then(|page| page.next_offset)
            .is_some()
        {
            list = list.child(self.action(
                "security-more-findings",
                "Load more",
                Action::LoadMore,
                false,
                window,
                cx,
            ));
        }
        list.into_any_element()
    }

    fn repositories_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        let rows = self
            .repositories
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|repo| {
                (repo.display_name.to_lowercase().contains(&query)
                    || repo.target_path.to_lowercase().contains(&query))
                    && match self.repository_status.as_str() {
                        "open_findings" => repo.open_findings_count > 0,
                        "not_scanned" => false,
                        _ => true,
                    }
            })
            .collect::<Vec<_>>();
        if let Some(state) = self.list_state(
            self.repositories.is_some(),
            rows.is_empty() && self.projects.is_empty(),
            "No scanned repositories yet",
            "Repositories appear here after their first Security scan",
            &t,
        ) {
            return state.into_any_element();
        }
        let mut list = div()
            .id("security-repositories-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.));
        for repo in &rows {
            let id = repo.target_id.clone();
            let selected =
                matches!(&self.location, Location::Repository(current) if *current == id);
            list = list.child(self.row(
                format!("security-repository-{id}"),
                [
                    &repo.display_name,
                    if repo.checkout_available {
                        &repo.target_path
                    } else {
                        "Checkout unavailable"
                    },
                    &format!("{} open findings", repo.open_findings_count),
                    repo.latest_scan.status_label(),
                ],
                selected,
                Action::Navigate(Location::Repository(id)),
                window,
                cx,
            ));
        }
        if matches!(self.repository_status.as_str(), "all" | "not_scanned") {
            for (index, project) in self.projects.iter().enumerate().filter(|(_, project)| {
                !self
                    .repositories
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .any(|repo| repo.target_path == project.path)
                    && (project.name.to_lowercase().contains(&query)
                        || project.path.to_lowercase().contains(&query))
            }) {
                let id = format!("project:{}", project.id);
                list = list.child(self.row(
                    format!("security-unscanned-{index}"),
                    [&project.name, &project.path, "", "Not scanned"],
                    false,
                    Action::Navigate(Location::Repository(id)),
                    window,
                    cx,
                ));
            }
        }
        list.into_any_element()
    }

    fn setup_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        if self.busy {
            return self.empty("Starting scan", Some("Codex is preparing the selected folder and starting a Security scan thread. This can take a moment"), &t).into_any_element();
        }
        let mut form = div().mt(px(32.)).flex().flex_col().gap(px(16.)).child(
            div()
                .flex()
                .gap(px(2.))
                .child(self.review_kind(
                    "standard",
                    ("Codebase", "Review the whole working tree"),
                    self.mode != "diff",
                    false,
                    window,
                    cx,
                ))
                .child(
                    self.review_kind(
                        "diff",
                        ("Changes", "Review a commit or working-tree changes"),
                        self.mode == "diff",
                        self.target_metadata
                            .as_ref()
                            .is_none_or(|data| data["reviewChangesSupported"] != true),
                        window,
                        cx,
                    ),
                ),
        );
        let mut choices = div()
            .flex()
            .flex_col()
            .child(
                self.setup_choice("target", window, cx).child(
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
                self.setup_choice("area", window, cx).child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(px(16.))
                        .right(px(16.))
                        .h(px(1.))
                        .bg(t.border),
                ),
            );
        choices = choices.child(self.setup_choice("model", window, cx));
        form = form.child(setup_card(choices, &t));
        if self.mode == "diff" && self.diff_kind != "working_tree" {
            form = form.child(field("Commit", self.revision.clone(), &t));
            if self.diff_kind == "range" {
                form = form.child(field("Base revision", self.base_revision.clone(), &t));
            }
        } else if self.area == "folder" {
            form = form.child(field("Folder", self.folder.clone(), &t));
        }
        if let Some(metadata) = &self.target_metadata {
            form = form
                .child(property(
                    "Current branch",
                    metadata["branch"].as_str().unwrap_or("Detached HEAD"),
                    &t,
                ))
                .child(property(
                    "Last commit",
                    metadata["shortRevision"]
                        .as_str()
                        .unwrap_or("Unversioned folder"),
                    &t,
                ));
        }
        if self.mode != "diff" {
            form = form.child(self.setup_toggle(
                "deep",
                (
                    "Deep scan",
                    "More comprehensive, but takes substantially longer",
                ),
                self.mode == "deep",
                Action::ToggleDeep,
                window,
                cx,
            ));
        }
        form = form.child(self.setup_toggle(
            "context",
            (
                "Additional context",
                "Share areas you want Codex Security to prioritize during this review",
            ),
            self.context_expanded,
            Action::ToggleContext,
            window,
            cx,
        ));
        if self.context_expanded {
            form = form.child(field("Additional context", self.context.clone(), &t));
        }
        if let Some(error) = &self.error {
            form = form.child(text(error.clone(), 13., 20., 430., t.text_error));
        }
        form = form.child(div().flex().justify_end().child(self.button(
            "security-setup-submit",
            ("Start scan", None),
            Action::Start,
            ButtonKind::FormPrimary,
            window,
            cx,
        )));
        div()
            .id("security-setup")
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(text("New scan", 20., 26.6, 500., t.text))
                    .child(text(
                        "What should Codex review?",
                        14.,
                        21.,
                        430.,
                        t.text.opacity(0.65),
                    )),
            )
            .child(form)
            .into_any_element()
    }

    fn review_kind(
        &self,
        mode: &'static str,
        (title, detail): (&'static str, &'static str),
        selected: bool,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let focus = focus::handle(format!("security-review-{mode}"), 0, window, cx);
        let choose = move |this: &mut Self, cx: &mut Context<Self>| {
            if !disabled {
                this.choose("mode", mode.into(), cx);
                cx.notify();
            }
        };
        focus::ring(
            div()
                .id(format!("security-review-{mode}"))
                .relative()
                .flex_1()
                .min_w_0()
                .px(px(21.))
                .py(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .role(gpui::Role::Button)
                .aria_label(title)
                .aria_selected(selected)
                .aria_disabled(disabled)
                .when(selected, |d| {
                    d.child(shapes::decoration(shapes::Decoration {
                        radius: gpui::Corners::all(px(9999.)),
                        corner: shapes::Corner::Round,
                        fill: Some(t.text_alpha_5),
                        ..Default::default()
                    }))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(2.))
                        .child(text(
                            title,
                            13.,
                            18.571_428,
                            500.,
                            if selected { t.text } else { t.text_tertiary },
                        ))
                        .child(text(detail, 12., 16., 430., t.text.opacity(0.65))),
                )
                .when(disabled, |d| d.opacity(0.4)),
            &focus,
            px(9999.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| choose(this, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                choose(this, cx);
                cx.stop_propagation();
            }
        }))
    }

    fn setup_choice(&self, menu: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let (label, value, padding, muted) = self.setup_choice_value(menu);
        let focus = focus::handle(format!("security-setup-{menu}"), 0, window, cx);
        let open = move |this: &mut Self, cx: &mut Context<Self>| {
            if !this.busy {
                this.dialog = Some(menu);
                cx.notify();
            }
        };
        let trigger = focus::ring(
            div()
                .id(format!("security-setup-{menu}"))
                .role(gpui::Role::Button)
                .aria_label(label)
                .relative()
                .h(px(28.))
                .px(px(padding))
                .border_1()
                .border_color(t.text.opacity(0.))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(text(
                    value,
                    13.,
                    18.,
                    430.,
                    if muted { t.text_tertiary } else { t.text },
                ))
                .child(icon("chevron-down", 14., t.text_tertiary)),
            &focus,
            px(9999.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| open(this, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                open(this, cx);
                cx.stop_propagation();
            }
        }));
        div()
            .relative()
            .h(px(44.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                text(label, 13., 18.571_428, 430., t.text)
                    .flex_1()
                    .min_w_0(),
            )
            .child(div().flex_1().min_w_0().flex().justify_end().child(trigger))
    }

    fn setup_toggle(
        &self,
        id: &'static str,
        (label, detail): (&'static str, &'static str),
        checked: bool,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let focus = focus::handle(format!("security-switch-{id}"), 0, window, cx);
        let key_action = action.clone();
        let control = focus::ring(
            crate::screens::settings::compact_switch(format!("security-switch-{id}"), checked, &t)
                .role(gpui::Role::Switch)
                .aria_label(label)
                .aria_selected(checked)
                .aria_value(if checked { "On" } else { "Off" }),
            &focus,
            px(9999.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.activate(action.clone(), cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.activate(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }));
        setup_card(
            div()
                .px(px(16.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(label, 13., 18.571_428, 500., t.text))
                        .child(text(detail, 12., 16., 430., t.text.opacity(0.65))),
                )
                .child(
                    div()
                        .w(px(160.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .child(control),
                ),
            &t,
        )
    }

    fn detail_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let expected = match &self.location {
            Location::Scan(id) | Location::Finding { scan_id: id, .. } => id,
            _ => return div().into_any_element(),
        };
        let Some(scan) = self.scan.as_ref().filter(|scan| scan.scan_id == *expected) else {
            return self
                .empty(
                    if self.loading {
                        "Loading scan progress"
                    } else {
                        "Codex Security could not load this scan"
                    },
                    None,
                    &t,
                )
                .into_any_element();
        };
        if let Location::Finding { .. } = self.location {
            return self.finding_view(scan, window, cx);
        }
        let mut panel = div()
            .id("security-scan-detail")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text(scan.title().to_owned(), 24., 28., 600., t.text));
        panel = self.scan_properties(panel, scan, window, cx);
        if !scan.has_results() {
            return panel.into_any_element();
        }
        panel = panel.child(text("Findings", 16., 20., 500., t.text));
        if scan.additional_findings_load_failed {
            panel = panel.child(text(
                "Some findings could not be loaded",
                13.,
                20.,
                430.,
                t.text_warning,
            ));
        }
        if scan.findings.is_empty() {
            panel = panel.child(text(
                "No findings reported",
                14.,
                21.,
                430.,
                t.text_secondary,
            ));
        }
        for finding in &scan.findings {
            let scan_id = scan.scan_id.clone();
            let occurrence_id = value(finding, "occurrenceId").to_owned();
            panel = panel.child(self.row(
                format!("security-detail-finding-{occurrence_id}"),
                [
                    value(finding, "title"),
                    value(finding, "summary"),
                    value(&finding["severity"], "level"),
                    value(&finding["triage"], "status"),
                ],
                false,
                Action::Navigate(Location::Finding {
                    scan_id,
                    occurrence_id,
                }),
                window,
                cx,
            ));
        }
        if !scan.artifacts.is_empty() {
            panel = panel.child(text("Artifacts", 16., 20., 500., t.text));
        }
        for (kind, path) in &scan.artifacts {
            let path = path.clone();
            panel = panel.child(self.row(
                format!("security-artifact-{kind}"),
                [kind, &path, "", ""],
                false,
                Action::OpenPath(path.clone()),
                window,
                cx,
            ));
        }
        panel.into_any_element()
    }

    fn finding_view(&self, scan: &Scan, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let Some(finding) = self.finding() else {
            return self
                .empty("Codex Security could not find this finding", None, &t)
                .into_any_element();
        };
        let closed = finding["triage"]["status"] == "closed";
        let mut panel = div()
            .id("security-finding-detail")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .child(self.action(
                        "security-finding-back",
                        "Back",
                        Action::Navigate(Location::Scan(scan.scan_id.clone())),
                        false,
                        window,
                        cx,
                    ))
                    .child(if closed {
                        self.action(
                            "security-reopen-finding",
                            "Reopen finding",
                            Action::ReopenFinding,
                            true,
                            window,
                            cx,
                        )
                    } else {
                        self.action(
                            "security-close-finding",
                            "Close finding",
                            Action::ShowDialog("close"),
                            true,
                            window,
                            cx,
                        )
                    }),
            )
            .child(text(
                value(finding, "title").to_owned(),
                24.,
                28.,
                600.,
                t.text,
            ))
            .child(property(
                "Severity",
                value(&finding["severity"], "level"),
                &t,
            ))
            .child(property(
                "Status",
                if closed { "Closed" } else { "Open" },
                &t,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(self.action(
                        "security-finding-summary-tab",
                        "Summary",
                        Action::DetailTab("summary"),
                        false,
                        window,
                        cx,
                    ))
                    .child(self.action(
                        "security-finding-patch-tab",
                        "Patch",
                        Action::DetailTab("patch"),
                        false,
                        window,
                        cx,
                    )),
            );
        if self.detail_tab == "patch" {
            panel = self.finding_patch(panel, scan, finding, window, cx);
        } else {
            panel = self.finding_summary(panel, finding, window, cx);
        }
        panel.into_any_element()
    }

    fn repository_detail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let Location::Repository(id) = &self.location else {
            return div().into_any_element();
        };
        if let Some(project) = id
            .strip_prefix("project:")
            .and_then(|id| self.projects.iter().find(|project| project.id == id))
        {
            return div()
                .p(px(24.))
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(text(project.name.clone(), 24., 28., 600., t.text))
                .child(property("Repo", &project.path, &t))
                .child(text("Not scanned", 14., 21., 430., t.text_secondary))
                .child(self.action(
                    "security-unscanned-start",
                    "New scan",
                    Action::Choose("project", project.path.clone()),
                    true,
                    window,
                    cx,
                ))
                .into_any_element();
        }
        let Some(repo) = self
            .repositories
            .as_ref()
            .and_then(|repos| repos.iter().find(|repo| repo.target_id == *id))
        else {
            return self
                .empty("Repository unavailable", None, &t)
                .into_any_element();
        };
        let path = repo.target_path.clone();
        let mut panel = div()
            .id("security-repository-detail")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text(repo.display_name.clone(), 24., 28., 600., t.text))
            .child(self.action(
                "security-repository-new-scan",
                "New scan",
                Action::Choose("project", path),
                true,
                window,
                cx,
            ))
            .child(property("Repo", &repo.target_path, &t))
            .child(property(
                "Open findings",
                &repo.open_findings_count.to_string(),
                &t,
            ))
            .child(property("Scans", &repo.scan_count.to_string(), &t))
            .child(property(
                "Last scanned commit",
                repo.latest_scan
                    .target_revision
                    .as_deref()
                    .unwrap_or("Unavailable"),
                &t,
            ))
            .child(property("Last scan", repo.latest_scan.status_label(), &t))
            .child(text("Recent scans", 16., 20., 500., t.text));
        let query = self.query.read(cx).field_text().to_owned();
        for scan in self
            .history
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|scan| scan_matches(scan, &query, &self.scan_mode, &self.scan_status))
        {
            let id = scan.scan_id.clone();
            panel = panel.child(self.row(
                format!("security-repository-scan-{id}"),
                [
                    scan.mode_label(),
                    &scan.scope,
                    scan.status_label(),
                    &format!("{} findings", scan.finding_count),
                ],
                false,
                Action::Navigate(Location::Scan(id)),
                window,
                cx,
            ));
        }
        panel.into_any_element()
    }

    fn dialog_view(
        &self,
        dialog: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let mut content = div()
            .id("security-dialog-content")
            .w(px(460.))
            .max_h(px(600.))
            .overflow_y_scroll()
            .p(px(24.))
            .rounded(px(20.))
            .bg(t.surface_elevated)
            .border_1()
            .border_color(t.border_subtle)
            .flex()
            .flex_col()
            .gap(px(16.))
            .role(gpui::Role::Dialog)
            .aria_label(dialog)
            .track_focus(&self.modal_focus)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if !this.busy {
                    this.dialog = None;
                    cx.notify();
                }
            }));
        content = self.dialog_content(content, dialog, window, cx);
        content = content.child(self.action(
            "security-dialog-cancel",
            "Cancel",
            Action::ShowDialog(""),
            false,
            window,
            cx,
        ));
        let cancel = focus::handle("security-dialog-cancel", 0, window, cx)
            .read(cx)
            .handle
            .clone();
        div()
            .absolute()
            .inset_0()
            .bg(t.dialog_backdrop)
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !this.busy {
                    this.dialog = None;
                    cx.notify();
                    cx.stop_propagation();
                } else if event.keystroke.key == "tab" {
                    focus::tab(event, window, cx);
                    if !this.modal_focus.contains_focused(window, cx) {
                        if event.keystroke.modifiers.shift {
                            cancel.focus(window, cx);
                        } else {
                            this.modal_focus.focus(window, cx);
                        }
                    }
                }
            }))
            .child(content)
    }

    fn filters(
        &self,
        mut content: Stateful<Div>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        content = content.child(text("Status", 14., 21., 500., t.text));
        match self.location {
            Location::Findings | Location::Finding { .. } => {
                for (id, label) in [("all", "All"), ("open", "Open"), ("closed", "Closed")] {
                    content = content.child(self.option(
                        "finding-status",
                        id,
                        label,
                        self.finding_filter.status.as_deref().unwrap_or("all") == id,
                        window,
                        cx,
                    ));
                }
                content = content.child(text("Severity", 14., 21., 500., t.text));
                for (id, label) in [
                    ("all", "All severities"),
                    ("critical", "Critical"),
                    ("high", "High"),
                    ("medium", "Medium"),
                    ("low", "Low"),
                    ("informational", "Informational"),
                ] {
                    content = content.child(self.option(
                        "severity",
                        id,
                        label,
                        self.finding_filter.severity.as_deref().unwrap_or("all") == id,
                        window,
                        cx,
                    ));
                }
                content = content.child(text("Repository", 14., 21., 500., t.text));
                content = content.child(self.option(
                    "finding-repository",
                    "all",
                    "All repositories",
                    self.finding_filter.target_id.is_none(),
                    window,
                    cx,
                ));
                if let Some(repositories) = &self.repositories {
                    for repository in repositories {
                        content = content.child(self.option(
                            "finding-repository",
                            &repository.target_id,
                            &repository.display_name,
                            self.finding_filter.target_id.as_ref() == Some(&repository.target_id),
                            window,
                            cx,
                        ));
                    }
                } else {
                    content = content.child(text(
                        "Could not load repositories",
                        13.,
                        20.,
                        430.,
                        t.text_error,
                    ));
                }
            }
            Location::Repositories | Location::Repository(_) => {
                for (id, label) in [
                    ("all", "All"),
                    ("scanned", "Scanned"),
                    ("not_scanned", "Not scanned"),
                    ("open_findings", "Open findings"),
                ] {
                    content = content.child(self.option(
                        "repository-status",
                        id,
                        label,
                        self.repository_status == id,
                        window,
                        cx,
                    ));
                }
            }
            _ => {
                for (id, label) in [
                    ("all", "All statuses"),
                    ("running", "Running"),
                    ("complete", "Completed"),
                    ("failed", "Failed"),
                    ("canceled", "Canceled"),
                ] {
                    content = content.child(self.option(
                        "scan-status",
                        id,
                        label,
                        self.scan_status == id,
                        window,
                        cx,
                    ));
                }
                for (id, label) in [
                    ("all", "All scan types"),
                    ("standard", "Standard scan"),
                    ("deep", "Deep scan"),
                    ("diff", "Changes scan"),
                ] {
                    content = content.child(self.option(
                        "scan-mode",
                        id,
                        label,
                        self.scan_mode == id,
                        window,
                        cx,
                    ));
                }
            }
        }
        content
    }

    fn option(
        &self,
        kind: &'static str,
        id: &str,
        label: &str,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = id.to_owned();
        self.row(
            format!("security-option-{kind}-{id}"),
            [label, "", if selected { "✓" } else { "" }, ""],
            selected,
            Action::Choose(kind, id),
            window,
            cx,
        )
    }
    fn split_workbench(
        &self,
        mut page: Div,
        main: AnyElement,
        detail_width: f32,
        total: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let right = if let Some(file) = &self.file {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.action(
                    "security-file-back",
                    "Back",
                    Action::CloseFile,
                    false,
                    window,
                    cx,
                ))
                .child(div().flex_1().min_h_0().child(file.clone()))
                .into_any_element()
        } else {
            match &self.location {
                Location::Scans => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().h(px(46.)).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text("Select a scan to view", 16., 24., 500., t.text)),
                    )
                    .into_any_element(),
                Location::NewScan => self.setup_view(window, cx),
                Location::Repository(_) => self.repository_detail(window, cx),
                _ => self.detail_view(window, cx),
            }
        };
        let resize_focus = focus::handle("security-detail-resize", 0, window, cx);
        let resize_handle = resize_focus.read(cx).handle.clone();
        let resizer = focus::ring(
            div()
                .id("security-detail-resize")
                .role(gpui::Role::Button)
                .aria_label("Resize Security details")
                .absolute()
                .left(px(total - detail_width - 8.))
                .top_0()
                .bottom_0()
                .w(px(16.))
                .cursor(gpui::CursorStyle::ResizeLeftRight)
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        resize_handle.focus(window, cx);
                        this.detail_drag = Some((f32::from(event.position.x), detail_width));
                        cx.stop_propagation();
                    }),
                ),
            &resize_focus,
            px(0.),
            px(0.),
            cx,
        )
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            let delta = match event.keystroke.key.as_str() {
                "left" => 16.,
                "right" => -16.,
                _ => return,
            };
            this.set_detail_width(detail_width + delta, cx);
            cx.stop_propagation();
        }));
        page = page
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .child(div().flex_1().min_w_0().child(main))
                    .child(
                        div()
                            .w(px(detail_width))
                            .flex_none()
                            .bg(t.surface)
                            .border_l_1()
                            .snap_origin()
                            .border_color(crate::over(t.border, t.surface))
                            .child(div().size_full().bg(t.surface).child(right)),
                    )
                    .child(resizer),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((start, width)) = this.detail_drag {
                    this.set_detail_width(width + start - f32::from(event.position.x), cx);
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.detail_drag = None),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.detail_drag = None),
            );
        page
    }

    fn setup_choice_value(&self, menu: &str) -> (&'static str, String, f32, bool) {
        let model = self
            .models
            .iter()
            .find(|model| model.model == self.model)
            .map(|model| model.display_name.as_str())
            .unwrap_or(&self.model);
        let model = crate::composer::model_label(model);
        let effort = crate::composer::effort_label(&self.effort);
        let (label, value, padding, muted) = match menu {
            "target" => (
                "Repository",
                if self.target.is_empty() {
                    "Select a repository".to_owned()
                } else {
                    self.target.clone()
                },
                6.,
                false,
            ),
            "area" => (
                if self.mode == "diff" {
                    "Changes to review"
                } else {
                    "Scan area"
                },
                if self.mode == "diff" {
                    self.diff_kind.clone()
                } else if self.area == "folder" {
                    "Folder".into()
                } else {
                    "Entire codebase".into()
                },
                12.,
                true,
            ),
            _ => (
                "Model and reasoning",
                format!("{model} · {effort}"),
                6.,
                true,
            ),
        };
        (label, value, padding, muted)
    }

    fn scan_properties(
        &self,
        mut panel: Stateful<Div>,
        scan: &Scan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let mut actions = div().flex().gap(px(8.));
        if let Some(thread) = &scan.continuation_thread_id {
            actions = actions.child(self.action(
                "security-view-activity",
                "View activity",
                Action::ViewThread(thread.clone()),
                false,
                window,
                cx,
            ));
        }
        actions = actions.child(self.action(
            "security-rename",
            "Rename",
            Action::ShowDialog("rename"),
            false,
            window,
            cx,
        ));
        if scan.progress.status == "running" {
            actions = actions.child(self.action(
                "security-stop",
                "Stop scan",
                Action::Stop,
                false,
                window,
                cx,
            ));
        }
        panel = panel
            .child(actions)
            .child(property("Status", scan.status_label(), &t))
            .child(property("Model", scan.model.as_deref().unwrap_or(""), &t))
            .child(property(
                "Commit",
                scan.target_revision.as_deref().unwrap_or("Unavailable"),
                &t,
            ))
            .child(property("Repo", &scan.target_path, &t))
            .child(property("Findings", &scan.finding_count.to_string(), &t))
            .child(property(
                "Scope",
                if scan.scope == "." {
                    "Entire repository"
                } else {
                    &scan.scope
                },
                &t,
            ));
        if let Some(files) = scan.progress.coverage.files_total {
            panel = panel.child(property("Files in scope", &files.to_string(), &t));
        }
        panel = panel.child(self.action(
            "security-edit-context",
            "Additional context",
            Action::ShowDialog("context"),
            false,
            window,
            cx,
        ));
        if let Some(context) = &scan.user_context {
            panel = panel.child(text(context.clone(), 14., 21., 430., t.text));
        }
        if let Some(usage) = &scan.usage
            && let Some(total) = usage["totalTokens"].as_u64()
        {
            panel = panel.child(property("Token usage", &format!("{total} Tokens"), &t));
        }
        if scan.progress.status == "running" {
            for (phase, label) in [
                ("preflight", "Preparing scan"),
                ("discovery", "Reviewing code"),
                ("validation", "Validating findings"),
                ("attack_path", "Tracing attack paths"),
                ("reporting", "Finalizing scan"),
            ] {
                panel = panel.child(text(
                    label,
                    14.,
                    21.,
                    if phase == scan.progress.phase {
                        600.
                    } else {
                        430.
                    },
                    t.text_secondary,
                ));
            }
            for issue in &scan.progress.preflight_issues {
                panel = panel.child(text(
                    value(issue, "reason").to_owned(),
                    13.,
                    20.,
                    430.,
                    t.text_warning,
                ));
            }
        }
        if let Some(failure) = &scan.failure_message {
            panel = panel.child(text(failure.clone(), 14., 21., 430., t.text_error));
        }
        if scan.results_recovery_needed {
            panel = panel.child(self.action(
                "security-recover",
                "Recover results",
                Action::Recover,
                false,
                window,
                cx,
            ));
        }
        panel
    }

    fn finding_patch(
        &self,
        mut panel: Stateful<Div>,
        scan: &Scan,
        finding: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        panel = panel.child(text(
            finding["remediationState"]["verificationSummary"]
                .as_str()
                .or_else(|| finding["remediationState"]["summary"].as_str())
                .or_else(|| finding["remediation"].as_str())
                .unwrap_or("No remediation guidance is available for this finding")
                .to_owned(),
            14.,
            21.,
            430.,
            t.text,
        ));
        if let Some(patch) = finding["remediationState"]["patch"].as_str() {
            panel = panel
                .child(text(patch.to_owned(), 12., 18., 430., t.text).font_family(crate::MONO));
        }
        if let Some(reason) = &scan.remediation_unavailable_reason {
            panel = panel.child(text(reason.clone(), 14., 21., 430., t.text_secondary));
        }
        if scan.remediation_available != Some(false) {
            if let Some(action) =
                bridge::security::remediation_action(finding, chrono::Utc::now().timestamp_millis())
            {
                let label = match action.as_str() {
                    "apply" => "Apply patch",
                    "verify" => "Verify fix",
                    _ => "Generate patch",
                };
                panel = panel.child(self.action(
                    "security-remediate",
                    label,
                    Action::Remediate(action),
                    true,
                    window,
                    cx,
                ));
            } else if finding["remediationState"]["pendingAction"]
                .as_str()
                .is_some()
            {
                panel = panel.child(text(
                    "Starting remediation…",
                    14.,
                    21.,
                    430.,
                    t.text_secondary,
                ));
            }
        }
        panel
    }

    fn finding_summary(
        &self,
        mut panel: Stateful<Div>,
        finding: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        panel = panel.child(text(
            value(finding, "summary").to_owned(),
            14.,
            21.,
            430.,
            t.text,
        ));
        for (title, key) in [
            ("Root cause", "rootCause"),
            ("Validation", "validation"),
            ("Attack path", "attackPath"),
        ] {
            if let Some(content) = finding.get(key).filter(|v| !v.is_null()) {
                panel = panel.child(section(title, content, &t));
            }
        }
        if let Some(rationale) = finding["severity"]["rationale"].as_str() {
            panel = panel.child(section(
                "Severity rationale",
                &Value::String(rationale.into()),
                &t,
            ));
        }
        if let Some(locations) = finding["locations"].as_array() {
            for (index, location) in locations.iter().enumerate() {
                let path = location["absolutePath"]
                    .as_str()
                    .or_else(|| location["path"].as_str())
                    .unwrap_or("")
                    .to_owned();
                panel = panel.child(self.row(
                    format!("security-location-{index}"),
                    [value(location, "path"), "View source", "", ""],
                    false,
                    Action::OpenPath(path),
                    window,
                    cx,
                ));
            }
        }
        if let Some(evidence) = finding["codeEvidence"].as_array() {
            for (index, evidence) in evidence.iter().enumerate() {
                panel = panel.child(section(&format!("Evidence {}", index + 1), evidence, &t));
            }
        }
        for (title, key) in [
            ("Source", "sourceExcerpt"),
            ("Preventive controls", "preventiveControls"),
            ("Tests", "remediationTests"),
        ] {
            if let Some(content) = finding.get(key).filter(|v| !v.is_null()) {
                panel = panel.child(section(title, content, &t));
            }
        }
        panel
    }

    fn edit_dialog(
        &self,
        content: Stateful<Div>,
        dialog: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let (title, input, submit, label) = match dialog {
            "rename" => ("Rename scan", self.name.clone(), Action::Rename, "Save"),
            "context" => (
                "Additional context",
                self.context.clone(),
                Action::SaveContext,
                "Save context",
            ),
            _ => (
                "Close finding",
                self.note.clone(),
                Action::CloseFinding,
                "Close finding",
            ),
        };
        let mut content = content.child(text(title, 20., 26., 600., t.text));
        if dialog == "close" {
            content = content.child(text(
                "Record why this finding no longer needs active remediation",
                14.,
                21.,
                430.,
                t.text_secondary,
            ));
            for (id, label) in [
                ("already_fixed", "Already fixed"),
                ("false_positive", "False positive"),
                ("wont_fix", "Won't fix"),
            ] {
                content = content.child(self.option(
                    "close-reason",
                    id,
                    label,
                    self.close_reason == id,
                    window,
                    cx,
                ));
            }
        }
        content.child(input).child(self.action(
            "security-dialog-submit",
            label,
            submit,
            true,
            window,
            cx,
        ))
    }

    fn model_dialog(
        &self,
        content: Stateful<Div>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let mut content = content.child(text("Model and reasoning", 18., 24., 500., t.text));
        for model in self.models.iter().filter(|model| !model.hidden) {
            content = content.child(self.option(
                "model",
                &model.model,
                &model.display_name,
                self.model == model.model,
                window,
                cx,
            ));
        }
        if let Some(model) = self.models.iter().find(|model| model.model == self.model) {
            for effort in &model.supported_reasoning_efforts {
                let effort = value(effort, "reasoningEffort");
                content = content.child(self.option(
                    "effort",
                    effort,
                    effort,
                    self.effort == effort,
                    window,
                    cx,
                ));
            }
        }
        content
    }
}

fn button_colors(primary: bool, kind: ButtonKind, t: &Theme) -> (gpui::Hsla, Option<gpui::Hsla>) {
    let color = if primary {
        t.text_primary_solid
    } else if matches!(kind, ButtonKind::Tab(false)) {
        t.text_tertiary
    } else {
        t.text
    };
    let fill = if primary {
        Some(t.button_primary_solid)
    } else if matches!(kind, ButtonKind::Tab(false)) {
        None
    } else {
        Some(t.text_alpha_5)
    };
    (color, fill)
}

fn setup_card(content: Div, t: &Theme) -> Div {
    div()
        .relative()
        .child(shapes::decoration(shapes::Decoration {
            radius: gpui::Corners::all(px(20.)),
            corner: shapes::Corner::Squircle,
            fill: Some(t.background_panel),
            border: Some((px(1.), t.border)),
            ..Default::default()
        }))
        .child(crate::rounded_clip(
            px(20.),
            shapes::Corner::Squircle,
            div().p(px(1.)).flex().flex_col().child(content),
        ))
}

fn field(label: &str, input: Entity<Composer>, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(text(label.to_owned(), 14., 21., 430., t.text))
        .child(input)
}

fn property(label: &str, value: &str, t: &Theme) -> Div {
    div()
        .flex()
        .gap(px(16.))
        .child(text(label.to_owned(), 14., 21., 430., t.text_secondary).w(px(160.)))
        .child(text(value.to_owned(), 14., 21., 430., t.text))
}

fn section(title: &str, value: &Value, t: &Theme) -> Div {
    let mut section =
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(title.to_owned(), 16., 20., 500., t.text));
    match value {
        Value::String(value) => {
            section = section.child(text(value.clone(), 14., 21., 430., t.text))
        }
        Value::Array(values) => {
            for value in values {
                section = section.child(self::section("", value, t));
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "evidenceRefs" | "evidence_refs" | "id") {
                    continue;
                }
                section = section.child(self::section(key, value, t));
            }
        }
        _ => {}
    }
    section
}

fn value<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn scan_matches(scan: &Scan, query: &str, mode: &str, status: &str) -> bool {
    (mode == "all" || scan.mode == mode)
        && (status == "all" || scan.progress.status == status)
        && [
            scan.title(),
            &scan.target_path,
            &scan.scope,
            scan.target_summary.as_deref().unwrap_or(""),
        ]
        .iter()
        .any(|value| value.to_lowercase().contains(&query.to_lowercase()))
}
