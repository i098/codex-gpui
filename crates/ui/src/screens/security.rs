use crate::{EmptyMain, Shell, composer::Composer};
use bridge::{
    Bridge,
    security::{Client, FindingFilter, Findings, Gate, Repository, Scan, Setup},
};
use gpui::{AnyView, App, AppContext, Context, Entity, EventEmitter, Render, Task, Window};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

#[path = "security_view.rs"]
mod view;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Access {
    pub gate_enabled: bool,
    pub plugin_available: bool,
    pub remote_host: bool,
    pub host_connected: bool,
}

impl Access {
    pub fn from_evaluations(evaluations: &Value, remote_host: bool, host_connected: bool) -> Self {
        let payload = evaluations.get("payload").unwrap_or(evaluations);
        let decoded = payload
            .as_str()
            .and_then(|value| serde_json::from_str::<Value>(value).ok());
        let payload = decoded.as_ref().unwrap_or(payload);
        Self {
            gate_enabled: payload["feature_gates"][bridge::security::ROLLOUT_GATE]["value"] == true,
            plugin_available: false,
            remote_host,
            host_connected,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    Scans,
    Findings,
    Repositories,
    NewScan,
    Scan(String),
    Finding {
        scan_id: String,
        occurrence_id: String,
    },
    Repository(String),
}

#[derive(Clone)]
pub enum Action {
    Navigate(Location),
    Choose(&'static str, String),
    Refresh,
    Start,
    Stop,
    Recover,
    Rename,
    SaveContext,
    CloseFinding,
    ReopenFinding,
    Remediate(String),
    LoadMore,
    InspectTarget,
    PickFolder,
    ShowDialog(&'static str),
    DetailTab(&'static str),
    ToggleDeep,
    ToggleContext,
    ViewThread(String),
    OpenPath(String),
    CloseFile,
    UseLocal,
}

#[derive(Clone)]
pub struct Project {
    pub id: String,
    pub path: String,
    pub name: String,
}

pub struct SecurityPage {
    bridge: Arc<Bridge>,
    client: Option<Client>,
    access: Access,
    gate: Gate,
    location: Location,
    finding_from_global: bool,
    detail_width: Option<f32>,
    detail_drag: Option<(f32, f32)>,
    scans: Option<Vec<Scan>>,
    repositories: Option<Vec<Repository>>,
    findings: Option<Findings>,
    scan: Option<Scan>,
    history: Option<Vec<Scan>>,
    file: Option<Entity<crate::screens::diff::panel::FileView>>,
    projects: Vec<Project>,
    models: Vec<bridge::types::Model>,
    model: String,
    effort: String,
    target: String,
    target_metadata: Option<Value>,
    mode: String,
    area: String,
    diff_kind: String,
    query: Entity<Composer>,
    folder: Entity<Composer>,
    revision: Entity<Composer>,
    base_revision: Entity<Composer>,
    context: Entity<Composer>,
    name: Entity<Composer>,
    note: Entity<Composer>,
    finding_filter: FindingFilter,
    scan_status: String,
    scan_mode: String,
    repository_status: String,
    dialog: Option<&'static str>,
    rendered_dialog: Option<&'static str>,
    modal_focus: gpui::FocusHandle,
    restore_focus: Option<gpui::FocusHandle>,
    close_reason: String,
    detail_tab: &'static str,
    context_expanded: bool,
    error: Option<String>,
    busy: bool,
    loading: bool,
    generation: u64,
    request: Option<Task<()>>,
    refresh: Option<Task<()>>,
}

impl EventEmitter<Action> for SecurityPage {}

pub fn shell(
    bridge: Arc<Bridge>,
    access: Entity<Access>,
    projects: Vec<Project>,
    models: Vec<bridge::types::Model>,
    model: String,
    effort: String,
    cx: &mut App,
) -> (Entity<Shell>, Entity<SecurityPage>) {
    let initial = access.read(cx).clone();
    let page = cx.new(|cx| SecurityPage::new(bridge, initial, projects, models, model, effort, cx));
    page.update(cx, |_, cx| {
        cx.observe(&access, |this, access, cx| {
            let access = access.read(cx);
            if access != &this.access {
                this.access = access.clone();
                this.load_gate(cx);
            }
        })
        .detach();
    });
    App::observe_release(cx, &page, |page, cx| {
        if !page.busy
            && let Some(client) = page.client.clone()
        {
            cx.background_executor()
                .spawn(async move { client.close().await })
                .detach();
        }
    })
    .detach();
    let main: AnyView = page.clone().into();
    let sidebar = cx.new(|_| EmptyMain).into();
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, main);
        shell.sidebar_open = false;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.selected_rail = None;
        shell.main_backgrounds = 0;
        shell
    });
    shell.update(cx, |_, cx| {
        cx.observe(&page, |shell, page, cx| {
            let ready = page.read(cx).gate == Gate::Ready;
            let backgrounds = u8::from(ready);
            let selected = ready.then_some("explore");
            if shell.main_backgrounds != backgrounds || shell.selected_rail != selected {
                shell.main_backgrounds = backgrounds;
                shell.selected_rail = selected;
                cx.notify();
            }
        })
        .detach();
    });
    page.update(cx, |page, cx| page.load_gate(cx));
    (shell, page)
}

impl SecurityPage {
    pub fn set_detail_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if width.is_finite() {
            self.detail_width = Some(width);
            cx.notify();
        }
    }

    pub fn target_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(
            self.scan
                .as_ref()
                .map(|scan| scan.target_path.as_str())
                .unwrap_or(&self.target),
        )
    }

    pub fn show_error(&mut self, error: &str, cx: &mut Context<Self>) {
        self.error = Some(error.into());
        cx.notify();
    }

    pub fn set_projects(&mut self, projects: Vec<Project>, cx: &mut Context<Self>) {
        let initial = self
            .target
            .is_empty()
            .then(|| projects.first().map(|project| project.path.clone()))
            .flatten();
        self.projects = projects;
        if let Some(path) = initial {
            self.set_target(path, cx);
        }
        cx.notify();
    }

    pub fn show_file(
        &mut self,
        file: Entity<crate::screens::diff::panel::FileView>,
        cx: &mut Context<Self>,
    ) {
        self.file = Some(file);
        cx.notify();
    }

    fn new(
        bridge: Arc<Bridge>,
        access: Access,
        projects: Vec<Project>,
        models: Vec<bridge::types::Model>,
        model: String,
        effort: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| Composer::inline_field("Search scans", "", cx));
        cx.observe(&query, |this, _, cx| {
            if this.location == Location::Findings {
                let query = this.query.read(cx).field_text().to_owned();
                if this.finding_filter.query != query {
                    this.finding_filter.query = query;
                    this.findings = None;
                    this.reload(cx);
                }
            }
            cx.notify();
        })
        .detach();
        let target = projects
            .first()
            .map(|project| project.path.clone())
            .unwrap_or_default();
        Self {
            bridge,
            client: None,
            access,
            gate: Gate::Loading,
            location: Location::Scans,
            finding_from_global: false,
            detail_width: None,
            detail_drag: None,
            scans: None,
            repositories: None,
            findings: None,
            scan: None,
            history: None,
            file: None,
            projects,
            models,
            model,
            effort,
            target,
            target_metadata: None,
            mode: "standard".into(),
            area: "entire".into(),
            diff_kind: "working_tree".into(),
            query,
            folder: cx.new(|cx| Composer::settings_field("src/server", "", false, cx)),
            revision: cx.new(|cx| Composer::settings_field("Commit or branch", "HEAD", false, cx)),
            base_revision: cx.new(|cx| Composer::settings_field("Base revision", "", false, cx)),
            context: cx.new(|cx| {
                Composer::settings_field(
                    "Tell Codex what to prioritize during this scan",
                    "",
                    true,
                    cx,
                )
            }),
            name: cx.new(|cx| Composer::settings_field("Scan name", "", false, cx)),
            note: cx.new(|cx| Composer::settings_field("Add a note", "", true, cx)),
            finding_filter: FindingFilter::default(),
            scan_status: "all".into(),
            scan_mode: "all".into(),
            repository_status: "all".into(),
            dialog: None,
            rendered_dialog: None,
            modal_focus: cx.focus_handle(),
            restore_focus: None,
            close_reason: "already_fixed".into(),
            detail_tab: "summary",
            context_expanded: false,
            error: None,
            busy: false,
            loading: true,
            generation: 0,
            request: None,
            refresh: None,
        }
    }

    pub fn set_target(&mut self, path: String, cx: &mut Context<Self>) {
        self.target = path;
        self.target_metadata = None;
        self.inspect_target(cx);
    }

    fn load_gate(&mut self, cx: &mut Context<Self>) {
        if !self.busy
            && let Some(client) = self.client.take()
        {
            cx.background_executor()
                .spawn(async move { client.close().await })
                .detach();
        }
        self.client = None;
        self.request = None;
        self.refresh = None;
        self.busy = false;
        self.dialog = None;
        self.gate = Gate::resolve(self.access.gate_enabled, self.access.remote_host, true, &[]);
        self.loading = self.gate == Gate::Loading;
        self.generation += 1;
        let generation = self.generation;
        if !self.loading {
            cx.notify();
            return;
        }
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let installed = bridge
                .request::<_, Value>("plugin/installed", json!({"cwds": []}))
                .await;
            let plugins: Vec<Value> = installed
                .as_ref()
                .ok()
                .and_then(|reply| reply["marketplaces"].as_array())
                .into_iter()
                .flatten()
                .flat_map(|marketplace| marketplace["plugins"].as_array().into_iter().flatten())
                .map(|plugin| json!({"plugin": plugin}))
                .collect();
            let gate = Gate::resolve(true, false, false, &plugins);
            let client = if gate == Gate::Ready {
                Client::connect(bridge).await.ok()
            } else {
                None
            };
            let accepted = this
                .update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    this.gate = if gate == Gate::Ready && client.is_none() {
                        Gate::PluginUnavailable
                    } else {
                        gate
                    };
                    this.client = client.clone();
                    this.loading = false;
                    this.request = None;
                    if this.gate == Gate::Ready {
                        this.reload(cx);
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !accepted && let Some(client) = client {
                client.close().await;
            }
        })
        .detach();
    }

    pub fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.accept_action(&action, cx) {
            return;
        }
        match action {
            Action::Navigate(location) => self.navigate(location, cx),
            Action::Refresh => self.reload(cx),
            Action::ShowDialog(dialog) => self.show_dialog(dialog, cx),
            Action::DetailTab(tab) => self.detail_tab = tab,
            Action::ToggleDeep | Action::ToggleContext => self.toggle(action),
            Action::Choose(kind, id) => self.choose(kind, id, cx),
            Action::InspectTarget => self.inspect_target(cx),
            Action::CloseFile => self.file = None,
            Action::LoadMore => self.load_more(cx),
            Action::PickFolder | Action::ViewThread(_) | Action::OpenPath(_) => cx.emit(action),
            Action::UseLocal => {
                self.access.remote_host = false;
                self.access.host_connected = true;
                self.load_gate(cx);
            }
            action => self.mutate(action, cx),
        }
        cx.notify();
    }

    fn accept_action(&mut self, action: &Action, cx: &mut Context<Self>) -> bool {
        if self.gate == Gate::Ready || matches!(action, Action::UseLocal) {
            return true;
        }
        if matches!(action, Action::Refresh) {
            self.load_gate(cx);
        }
        false
    }

    fn navigate(&mut self, location: Location, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.location != location {
            self.history = None;
        }
        if matches!(location, Location::Finding { .. }) {
            self.finding_from_global = matches!(self.location, Location::Findings)
                || self.finding_from_global && matches!(self.location, Location::Finding { .. });
        }
        self.location = location;
        self.error = None;
        self.dialog = None;
        self.file = None;
        self.detail_tab = "summary";
        self.query.update(cx, |input, cx| input.set_text("", cx));
        self.finding_filter.query.clear();
        let placeholder = match &self.location {
            Location::Findings => "Search title, repository, or path",
            Location::Repositories | Location::Repository(_) => "Search name or path",
            Location::Finding { .. } if self.finding_from_global => {
                "Search title, repository, or path"
            }
            _ => "Search scans",
        };
        self.query
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        self.reload(cx);
    }

    fn show_dialog(&mut self, dialog: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(scan) = &self.scan {
            if dialog == "rename" {
                self.name
                    .update(cx, |input, cx| input.set_text(scan.title(), cx));
            }
            if dialog == "context" {
                self.context.update(cx, |input, cx| {
                    input.set_text(scan.user_context.as_deref().unwrap_or(""), cx)
                });
            }
        }
        self.dialog = (!dialog.is_empty()).then_some(dialog);
    }

    fn toggle(&mut self, action: Action) {
        if self.busy {
            return;
        }
        match action {
            Action::ToggleDeep if self.mode != "diff" => {
                self.mode = if self.mode == "deep" {
                    "standard"
                } else {
                    "deep"
                }
                .into();
            }
            Action::ToggleContext => self.context_expanded = !self.context_expanded,
            _ => {}
        }
    }

    fn choose(&mut self, kind: &'static str, id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match kind {
            "mode" => self.mode = id,
            "depth" if self.mode != "diff" => self.mode = id,
            "area" => self.area = id,
            "diff" => self.diff_kind = id,
            "model" => self.choose_model(id),
            "effort" => self.effort = id,
            "target" => self.set_target(id, cx),
            "project" => {
                self.location = Location::NewScan;
                self.set_target(id, cx);
            }
            "close-reason" => self.close_reason = id,
            _ => self.choose_filter(kind, id, cx),
        }
        if kind != "close-reason" && kind != "model" {
            self.dialog = None;
        }
    }

    fn choose_model(&mut self, id: String) {
        if let Some(model) = self.models.iter().find(|model| model.model == id) {
            self.effort = model.default_reasoning_effort.clone();
            self.model = id;
        }
    }

    fn choose_filter(&mut self, kind: &str, id: String, cx: &mut Context<Self>) {
        match kind {
            "scan-status" => self.scan_status = id,
            "scan-mode" => self.scan_mode = id,
            "repository-status" => self.repository_status = id,
            "finding-status" => self.finding_filter.status = (id != "all").then_some(id),
            "severity" => self.finding_filter.severity = (id != "all").then_some(id),
            "finding-repository" => self.finding_filter.target_id = (id != "all").then_some(id),
            _ => return,
        }
        if matches!(kind, "finding-status" | "severity" | "finding-repository") {
            self.findings = None;
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.location == Location::NewScan {
            self.inspect_target(cx);
            return;
        }
        if matches!(&self.location, Location::Repository(id) if id.starts_with("project:")) {
            self.loading = false;
            cx.notify();
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let location = self.location.clone();
        let filter = self.finding_filter.clone();
        let loaded_findings = self
            .findings
            .as_ref()
            .map_or(20, |page| page.findings.len().max(20));
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = Loaded::read(&client, &location, &filter, loaded_findings).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.request = None;
                match result {
                    Ok(Loaded::Scans(scans)) => {
                        this.scans = Some(scans);
                        this.error = None;
                    }
                    Ok(Loaded::Findings(findings, repositories)) => {
                        this.findings = Some(findings);
                        if let Some(repositories) = repositories {
                            this.repositories = Some(repositories);
                        }
                        this.error = None;
                    }
                    Ok(Loaded::Repositories(repositories)) => {
                        this.repositories = Some(repositories);
                        this.error = None;
                    }
                    Ok(Loaded::Scan(scan, scans)) => {
                        this.scan = Some(*scan);
                        if let Some(scans) = scans {
                            this.scans = Some(scans);
                        }
                        this.error = None;
                    }
                    Ok(Loaded::Repository(repositories, history)) => {
                        this.repositories = Some(repositories);
                        this.history = Some(history);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                this.schedule_refresh(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let running = self
            .scans
            .as_ref()
            .is_some_and(|scans| scans.iter().any(|scan| scan.progress.status == "running"))
            || self
                .scan
                .as_ref()
                .is_some_and(|scan| scan.progress.status == "running");
        self.refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(if running { 5 } else { 30 }))
                .await;
            this.update(cx, |this, cx| this.reload(cx)).ok();
        }));
    }

    fn inspect_target(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        if self.target.is_empty() {
            self.loading = false;
            cx.notify();
            return;
        }
        let target = self.target.clone();
        self.loading = true;
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = client.inspect(&target).await;
            this.update(cx, |this, cx| {
                if this.target != target {
                    return;
                }
                this.loading = false;
                this.request = None;
                match result {
                    Ok(metadata) => {
                        this.target_metadata = Some(metadata);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(offset) = self
            .findings
            .as_ref()
            .and_then(|findings| findings.next_offset)
        else {
            return;
        };
        let Some(client) = self.client.clone() else {
            return;
        };
        let filter = self.finding_filter.clone();
        let generation = self.generation;
        self.loading = true;
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = client.findings(&filter, offset).await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                this.request = None;
                match result {
                    Ok(page) => {
                        if let Some(findings) = &mut this.findings {
                            findings.findings.extend(page.findings);
                            findings.next_offset = page.next_offset;
                            this.error = None;
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn validate_setup(&self, cx: &App) -> Result<(), &'static str> {
        if self.target.is_empty() {
            return Err("Select a repository");
        }
        if self.model.is_empty() || self.models.is_empty() {
            return Err("Could not load models");
        }
        let metadata = self
            .target_metadata
            .as_ref()
            .ok_or("Codex Security could not inspect this repository")?;
        if self.mode == "diff" && metadata["reviewChangesSupported"] != true {
            return Err("This repository does not support a Changes scan");
        }
        if self.area == "folder" && self.folder.read(cx).field_text().trim().is_empty() {
            return Err("Enter a repository-relative folder");
        }
        if self.mode == "diff"
            && self.diff_kind != "working_tree"
            && self.revision.read(cx).field_text().trim().is_empty()
        {
            return Err("Enter a commit or revision");
        }
        if self.mode == "diff"
            && self.diff_kind == "range"
            && self.base_revision.read(cx).field_text().trim().is_empty()
        {
            return Err("Enter a base revision");
        }
        Ok(())
    }

    fn setup(&self, cx: &App) -> Result<Setup, &'static str> {
        self.validate_setup(cx)?;
        let scope = if self.area == "folder" {
            self.folder.read(cx).field_text().trim()
        } else {
            "."
        };
        let diff_target = if self.mode == "diff" {
            let mut target = json!({"kind": self.diff_kind});
            if self.diff_kind != "working_tree" {
                target["headRevision"] = self.revision.read(cx).field_text().trim().into();
            }
            if self.diff_kind == "range" {
                target["baseRevision"] = self.base_revision.read(cx).field_text().trim().into();
            }
            Some(target)
        } else {
            None
        };
        Ok(Setup {
            target_path: self.target.clone(),
            scope: scope.into(),
            mode: self.mode.clone(),
            diff_target,
            model: self.model.clone(),
            reasoning_effort: self.effort.clone(),
            user_context: if self.context_expanded {
                self.context.read(cx).field_text().into()
            } else {
                String::new()
            },
        })
    }

    fn finding(&self) -> Option<&Value> {
        let Location::Finding { occurrence_id, .. } = &self.location else {
            return None;
        };
        self.scan
            .as_ref()?
            .findings
            .iter()
            .find(|finding| finding["occurrenceId"] == *occurrence_id)
    }

    fn mutate(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let mutation = match self.prepare_mutation(action, cx) {
            Ok(mutation) => mutation,
            Err(error) => {
                self.error = Some(error.into());
                return;
            }
        };
        self.busy = true;
        self.refresh = None;
        self.request = None;
        self.generation += 1;
        let generation = self.generation;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = mutation.run(&client).await;
            let accepted = this
                .update(cx, |this, cx| {
                    if this.generation != generation {
                        return false;
                    }
                    this.busy = false;
                    match result {
                        Ok(location) => {
                            if let Some(location) = location {
                                this.location = location;
                            }
                            this.dialog = None;
                            this.reload(cx);
                        }
                        Err(error) => {
                            this.error = Some(error.to_string());
                            this.schedule_refresh(cx);
                        }
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !accepted {
                client.close().await;
            }
        })
        .detach();
    }

    fn prepare_mutation(&self, action: Action, cx: &App) -> Result<Mutation, &'static str> {
        if matches!(action, Action::Start) {
            return self.setup(cx).map(Mutation::Start);
        }
        let scan = self
            .scan
            .as_ref()
            .ok_or("Codex Security could not find the selected scan or finding")?;
        match action {
            Action::Stop => Ok(Mutation::Stop(scan.clone())),
            Action::Recover | Action::Rename | Action::SaveContext => {
                self.edit_scan(action, scan, cx)
            }
            Action::CloseFinding | Action::ReopenFinding => self.triage(action, cx),
            Action::Remediate(action) => Ok(Mutation::Remediate {
                scan: scan.clone(),
                finding: self
                    .finding()
                    .cloned()
                    .ok_or("Codex Security could not find the selected scan or finding")?,
                action,
                model: self.model.clone(),
                effort: self.effort.clone(),
            }),
            _ => unreachable!("only mutation actions reach prepare_mutation"),
        }
    }

    fn edit_scan(&self, action: Action, scan: &Scan, cx: &App) -> Result<Mutation, &'static str> {
        let (tool, args) = match action {
            Action::Recover => (
                "recover_codex_security_scan_results",
                json!({"scanId": scan.scan_id}),
            ),
            Action::Rename => {
                let name = self.name.read(cx).field_text().trim();
                if name.is_empty() {
                    return Err("Enter a scan name");
                }
                (
                    "rename_codex_security_scan",
                    json!({"scanId": scan.scan_id, "name": name}),
                )
            }
            Action::SaveContext => (
                "update_codex_security_scan_context_from_app",
                json!({"scanId": scan.scan_id, "userContext": self.context.read(cx).field_text().trim()}),
            ),
            _ => unreachable!("only scan edits reach edit_scan"),
        };
        Ok(Mutation::Tool(tool, args))
    }

    fn triage(&self, action: Action, cx: &App) -> Result<Mutation, &'static str> {
        let finding = self
            .finding()
            .ok_or("Codex Security could not find the selected scan or finding")?;
        let mut args = json!({"occurrenceId": finding["occurrenceId"], "status": "open"});
        if matches!(action, Action::CloseFinding) {
            let note = self.note.read(cx).field_text().trim();
            if self.close_reason == "wont_fix" && note.is_empty() {
                return Err("Add a note explaining why this finding will not be fixed");
            }
            args["status"] = "closed".into();
            args["closeReason"] = self.close_reason.clone().into();
            if !note.is_empty() {
                args["note"] = note.into();
            }
        }
        Ok(Mutation::Tool("set_codex_security_finding_triage", args))
    }
}

enum Mutation {
    Start(Setup),
    Stop(Scan),
    Tool(&'static str, Value),
    Remediate {
        scan: Scan,
        finding: Value,
        action: String,
        model: String,
        effort: String,
    },
}

impl Mutation {
    async fn run(self, client: &Client) -> Result<Option<Location>, bridge::security::Error> {
        match self {
            Self::Start(setup) => client
                .start(&setup)
                .await
                .map(|id| Some(Location::Scan(id))),
            Self::Stop(scan) => client.stop(&scan).await.map(|_| None),
            Self::Tool(tool, args) => client.call(tool, args).await.map(|_| None),
            Self::Remediate {
                scan,
                finding,
                action,
                model,
                effort,
            } => client
                .remediate(&scan, &finding, &action, &model, &effort)
                .await
                .map(|_| None),
        }
    }
}

enum Loaded {
    Scans(Vec<Scan>),
    Findings(Findings, Option<Vec<Repository>>),
    Repositories(Vec<Repository>),
    Scan(Box<Scan>, Option<Vec<Scan>>),
    Repository(Vec<Repository>, Vec<Scan>),
}

impl Loaded {
    async fn read(
        client: &Client,
        location: &Location,
        filter: &FindingFilter,
        count: usize,
    ) -> Result<Self, bridge::security::Error> {
        Ok(match location {
            Location::Scans => Self::Scans(client.scans(None).await?),
            Location::Findings => Self::Findings(
                client.findings_through(filter, count).await?,
                client.repositories().await.ok(),
            ),
            Location::Repositories => Self::Repositories(client.repositories().await?),
            Location::Scan(id) => Self::Scan(
                Box::new(client.scan(id, None).await?),
                client.scans(None).await.ok(),
            ),
            Location::Finding {
                scan_id,
                occurrence_id,
            } => Self::Scan(
                Box::new(client.scan(scan_id, Some(occurrence_id)).await?),
                client.scans(None).await.ok(),
            ),
            Location::Repository(id) => {
                Self::Repository(client.repositories().await?, client.scans(Some(id)).await?)
            }
            Location::NewScan => unreachable!("scan setup uses target inspection"),
        })
    }
}

impl Render for SecurityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        self.render_page(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{HeadlessAppContext, px, size};

    fn page() -> (HeadlessAppContext, Entity<SecurityPage>) {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let (bridge, _) = Bridge::replay(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl"),
        )
        .unwrap();
        let mut cx =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let page = cx.update(|cx| {
            cx.new(|cx| {
                SecurityPage::new(
                    Arc::new(bridge),
                    Access::default(),
                    Vec::new(),
                    Vec::new(),
                    String::new(),
                    String::new(),
                    cx,
                )
            })
        });
        (cx, page)
    }

    #[test]
    fn closed_rollout_gate_blocks_privileged_actions() {
        let (mut cx, page) = page();
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, _| page.clone())
            .unwrap();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.load_gate(cx);
                page.activate(Action::Navigate(Location::NewScan), cx);
                page.activate(Action::Start, cx);
                assert_eq!(page.gate, Gate::Disabled);
                assert_eq!(page.location, Location::Scans);
                assert!(!page.busy);
                assert!(page.client.is_none());
            })
        });
        cx.run_until_parked();
        cx.update_window(window.into(), |_, _, _| {}).unwrap();
    }

    #[test]
    fn access_changes_stay_with_their_own_window() {
        let (mut cx, seed) = page();
        let (access, first_shell, first, second) = cx.update(|cx| {
            let bridge = seed.read(cx).bridge.clone();
            let access = cx.new(|_| Access::default());
            let other = cx.new(|_| Access::default());
            let (first_shell, first) = shell(
                bridge.clone(),
                access.clone(),
                Vec::new(),
                Vec::new(),
                String::new(),
                String::new(),
                cx,
            );
            let (_, second) = shell(
                bridge,
                other,
                Vec::new(),
                Vec::new(),
                String::new(),
                String::new(),
                cx,
            );
            (access, first_shell, first, second)
        });
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, _| first_shell)
            .unwrap();
        cx.update(|cx| {
            access.update(cx, |access, cx| {
                access.gate_enabled = true;
                access.remote_host = true;
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(first.read(cx).gate, Gate::RemoteHost);
            assert_eq!(second.read(cx).gate, Gate::Disabled);
            first.update(cx, |page, cx| {
                page.activate(Action::Start, cx);
                assert!(!page.busy);
                assert!(page.client.is_none());
            });
        });
        cx.update_window(window.into(), |_, _, _| {}).unwrap();
    }

    #[test]
    fn selecting_depth_cannot_change_a_changes_scan_scope() {
        let (mut cx, page) = page();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.mode = "diff".into();
                page.diff_kind = "range".into();
                page.target = "/repo".into();
                page.target_metadata = Some(json!({"reviewChangesSupported":true}));
                page.model = "model".into();
                page.models.push(bridge::types::Model {
                    id: "model".into(),
                    model: "model".into(),
                    display_name: "Model".into(),
                    description: String::new(),
                    hidden: false,
                    default_reasoning_effort: "medium".into(),
                    supported_reasoning_efforts: Vec::new(),
                    is_default: true,
                });
                page.base_revision
                    .update(cx, |input, cx| input.set_text("base", cx));
                page.choose("depth", "standard".into(), cx);
                let setup = page.setup(cx).unwrap();
                assert_eq!(setup.mode, "diff");
                assert_eq!(
                    setup.diff_target,
                    Some(json!({"kind":"range","headRevision":"HEAD","baseRevision":"base"}))
                );
            })
        });
    }

    #[test]
    fn repository_navigation_clears_the_previous_history_before_loading() {
        let (mut cx, page) = page();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.gate = Gate::Ready;
                page.location = Location::Repository("A".into());
                page.history = Some(Vec::new());
                page.activate(Action::Navigate(Location::Repository("B".into())), cx);
                assert!(page.history.is_none());
            })
        });
    }

    #[test]
    fn a_late_default_repository_invalidates_unrelated_target_metadata() {
        let (mut cx, page) = page();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.location = Location::NewScan;
                page.target_metadata = Some(json!({"reviewChangesSupported":true}));
                page.set_projects(
                    vec![Project {
                        id: "repo".into(),
                        name: "Repo".into(),
                        path: "/repo".into(),
                    }],
                    cx,
                );
                assert_eq!(page.target, "/repo");
                assert!(page.target_metadata.is_none());
            })
        });
    }

    #[test]
    fn only_an_explicit_original_gate_value_enables_the_workbench() {
        let enabled = json!({"feature_gates":{"1598376":{"value":true}}});
        assert!(Access::from_evaluations(&enabled, false, true).gate_enabled);
        assert!(
            Access::from_evaluations(&json!({"payload":enabled.to_string()}), false, true)
                .gate_enabled
        );
        for value in [
            json!({}),
            json!({"payload":"invalid"}),
            json!({"feature_gates":{"1598376":{"value":"true"}}}),
        ] {
            assert!(!Access::from_evaluations(&value, false, true).gate_enabled);
        }
    }
}
