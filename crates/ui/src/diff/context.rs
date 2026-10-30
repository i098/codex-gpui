use super::*;
use gpui::{ImageSource, Point};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone)]
pub struct ContextRequest {
    pub epoch: u64,
    pub generation: u64,
    pub file: usize,
    pub position: Point<Pixels>,
    pub window: gpui::AnyWindowHandle,
    pub line: Option<(comments::Side, u32)>,
    pub path: String,
    pub relative: String,
    pub cwd: String,
    pub new_tab: bool,
    pub deleted: bool,
    pub selected: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTarget {
    pub(super) target: String,
    pub(super) label: String,
    icon: Option<String>,
    pub(super) app_path: Option<String>,
    #[serde(default)]
    pub(super) kind: String,
    #[serde(skip)]
    image: Option<ImageSource>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTargets {
    preferred_target: Option<String>,
    available_targets: Vec<String>,
    #[serde(default)]
    mode: String,
    targets: Vec<OpenTarget>,
}

impl OpenTargets {
    pub(super) fn visible(mut self) -> (Vec<OpenTarget>, Option<usize>) {
        let native = self.targets.iter().any(|target| target.app_path.is_some());
        if native {
            self.targets.retain(|target| target.app_path.is_some());
        } else if self.mode == "native" {
            self.targets
                .retain(|target| matches!(target.target.as_str(), "systemDefault" | "fileManager"));
        } else {
            let available: HashSet<_> = self.available_targets.iter().map(String::as_str).collect();
            self.targets
                .retain(|target| available.contains(target.target.as_str()));
        }
        for target in &mut self.targets {
            target.image = target.icon.take().and_then(image_source);
        }
        let primary = self
            .targets
            .iter()
            .position(|target| Some(&target.target) == self.preferred_target.as_ref())
            .or_else(|| (!self.targets.is_empty()).then_some(0));
        (self.targets, primary)
    }
}

fn image_source(path: String) -> Option<ImageSource> {
    if path.starts_with("data:") {
        return crate::screens::computer_use::decode_app_image(&path).map(ImageSource::Image);
    }
    if let Some(path) = path.strip_prefix("app://fs/@fs") {
        return url::Url::parse(&format!("file://{path}"))
            .ok()?
            .to_file_path()
            .ok()
            .map(ImageSource::from);
    }
    path.strip_prefix('/')
        .filter(|path| path.starts_with("apps/"))
        .map(ImageSource::from)
}

#[derive(Clone, Debug)]
pub struct OpenTargetRequest {
    pub path: String,
    pub cwd: String,
    pub line: Option<u32>,
    pub target: String,
    pub app_path: Option<String>,
    pub persist_preferred_target_path: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Action {
    Comment,
    Primary(usize),
    OpenWith,
    GitHub,
    CopySelection,
    CopyPath,
    CopyRelative,
    Wrap,
}

pub(super) struct Item {
    pub label: SharedString,
    pub enabled: bool,
    pub action: Action,
    pub icon: Option<ImageSource>,
    pub separator: bool,
}

impl Item {
    fn new(label: impl Into<SharedString>, action: Action, enabled: bool) -> Self {
        Self {
            label: label.into(),
            action,
            enabled,
            icon: None,
            separator: false,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub epoch: u64,
    pub pending: bool,
    pub items: Vec<Item>,
    pub targets: Vec<OpenTarget>,
    pub new_tab: bool,
    file: usize,
    line: Option<(comments::Side, u32)>,
    path: String,
    relative: String,
    cwd: String,
    github: Option<String>,
}

impl State {
    pub fn open_with_index(&self) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.action == Action::OpenWith)
    }

    pub fn target_count(&self) -> usize {
        self.targets.len() + usize::from(self.new_tab)
    }

    pub fn target_label(&self, index: usize) -> &str {
        if self.new_tab && index == 0 {
            "New tab"
        } else {
            &self.targets[index - usize::from(self.new_tab)].label
        }
    }

    pub fn target_icon(&self, index: usize) -> Option<ImageSource> {
        if self.new_tab && index == 0 {
            None
        } else {
            self.targets[index - usize::from(self.new_tab)]
                .image
                .clone()
        }
    }
}

impl DiffPanel {
    pub(super) fn cancel_context_request(&mut self) {
        self.context.pending = false;
        self.context.epoch = self.context.epoch.wrapping_add(1);
    }

    pub(super) fn request_context_menu(
        &mut self,
        file: usize,
        line: Option<(comments::Side, u32)>,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.file_path(file) else {
            return;
        };
        let Some(shown) = self.files.get(file) else {
            return;
        };
        let relative = relative(self.file_root(), &path);
        let deleted = shown.raw_kind == Some(Kind::Removed)
            || shown
                .raw
                .lines()
                .any(|line| line.starts_with("deleted file mode "));
        let new_tab = !self.standalone && !deleted;
        self.context.epoch = self.context.epoch.wrapping_add(1);
        self.context.pending = true;
        self.menu = None;
        cx.emit(DiffRequest::ContextMenu(ContextRequest {
            epoch: self.context.epoch,
            generation: self.content_generation,
            file,
            position,
            line,
            path,
            relative,
            window: window.window_handle(),
            cwd: self.working_directory().to_owned(),
            new_tab,
            deleted,
            selected: self.selected_text().trim().to_owned(),
        }));
        cx.stop_propagation();
        cx.notify();
    }

    pub fn context_menu_loaded(
        &mut self,
        request: ContextRequest,
        result: Result<OpenTargets, String>,
        github: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if request.epoch != self.context.epoch || request.generation != self.content_generation {
            return;
        }
        let catalog = result.unwrap_or_else(|error| {
            eprintln!("codex-gpui: cannot load open-in targets: {error}");
            OpenTargets::default()
        });
        let (targets, primary) = catalog.visible();
        let mut items = vec![Item::new(
            "Request changes",
            Action::Comment,
            request.line.is_some() && self.comments_enabled(),
        )];
        if let Some(index) = primary {
            let target = &targets[index];
            let mut item = Item::new(
                format!("Open in {}", target.label),
                Action::Primary(index),
                true,
            );
            item.icon = target.image.clone();
            items.push(item);
        }
        if request.new_tab || !targets.is_empty() {
            items.push(Item::new("Open with", Action::OpenWith, true));
        }
        if github.is_some() {
            items.push(Item::new("Open in GitHub", Action::GitHub, true));
        }
        let mut copy = Item::new(
            "Copy selection",
            Action::CopySelection,
            !request.selected.is_empty(),
        );
        copy.separator = items.len() > 1;
        items.push(copy);
        items.push(Item::new("Copy path", Action::CopyPath, true));
        items.push(Item::new("Copy relative path", Action::CopyRelative, true));
        items.push(Item::new("Toggle line wrap", Action::Wrap, true));
        self.context.targets = targets;
        self.context.items = items;
        self.context.new_tab = request.new_tab;
        self.context.file = request.file;
        self.context.line = request.line;
        self.context.path = request.path;
        self.context.relative = request.relative;
        self.context.cwd = request.cwd;
        self.context.github = github;
        self.open_menu(request.file, menu::Kind::Body, request.position, window, cx);
        self.menu.as_mut().unwrap().selected = request.selected;
    }

    pub(super) fn activate_context(
        &mut self,
        index: usize,
        selected: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.context.items[index].action {
            Action::Comment => {
                if let Some((side, line)) = self.context.line {
                    self.context_comment(self.context.file, side, line, window, cx);
                }
            }
            Action::Primary(index) => self.launch_target(index, false, cx),
            Action::GitHub => {
                if let Some(url) = &self.context.github {
                    cx.open_url(url);
                }
            }
            Action::CopySelection => cx.write_to_clipboard(ClipboardItem::new_string(selected)),
            Action::CopyPath => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.context.path.clone()))
            }
            Action::CopyRelative => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.context.relative.clone()))
            }
            Action::Wrap => self.toggle_wrap(cx),
            Action::OpenWith => unreachable!("submenu actions stay open"),
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn activate_target(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.context.new_tab && index == 0 {
            cx.emit(DiffRequest::OpenTab {
                path: self.context.path.clone(),
                line: self.context_line(),
            });
        } else {
            self.launch_target(index - usize::from(self.context.new_tab), true, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn context_line(&self) -> Option<u32> {
        self.context.line.map(|(_, line)| line).or_else(|| {
            self.files
                .get(self.context.file)?
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .find(|line| line.kind != Kind::Context)
                .map(|line| line.number)
        })
    }

    fn launch_target(&self, index: usize, remember: bool, cx: &mut Context<Self>) {
        let target = &self.context.targets[index];
        cx.emit(DiffRequest::OpenTarget(OpenTargetRequest {
            path: self.context.path.clone(),
            cwd: self.context.cwd.clone(),
            line: self.context_line(),
            target: target.target.clone(),
            app_path: target.app_path.clone(),
            persist_preferred_target_path: (remember
                && target.kind == "editor"
                && target.app_path.is_none())
            .then(|| self.context.cwd.clone()),
        }));
    }
}
