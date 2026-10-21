use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bridge::types::Thread;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, RenderImage,
    ScrollHandle, SharedString, Styled, Window, div, img, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use theme::ActiveTheme as _;

use super::thread::view::{Clock, ThreadView};
use crate::{
    composer::{Composer, popover},
    icon::icon,
    text,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Chat,
    SettingUp,
    SetupFailed,
    Failed,
    Cancelled,
}

#[derive(Clone)]
pub struct CloudTurn {
    pub id: String,
    pub thread: Thread,
    pub phase: Phase,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub images: Vec<String>,
}

#[derive(Clone)]
pub struct CloudTask {
    pub id: String,
    pub title: String,
    pub turns: Vec<CloudTurn>,
    pub active_turn: Option<String>,
    pub diff_turn: Option<String>,
    pub diff: String,
    pub clone_url: Option<String>,
    pub environment_label: Option<String>,
    pub working: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub status: String,
    #[serde(default)]
    pub applied_paths: Vec<String>,
    #[serde(default)]
    pub conflicted_paths: Vec<String>,
    #[serde(default)]
    pub skipped_paths: Vec<String>,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct LocalTarget {
    pub cwd: String,
    pub branch: Option<String>,
}

pub enum CloudAction {
    Retry,
    Apply { revert: bool },
    LoadImage(String),
    OpenMain,
    Cancel,
}

pub enum Asset {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

#[derive(Default)]
struct AppliedCloudDiffs(HashSet<String>);
impl gpui::Global for AppliedCloudDiffs {}

pub struct CloudTaskPage {
    pub composer: Entity<Composer>,
    pub data: Option<CloudTask>,
    pub target: Option<LocalTarget>,
    pub applying: bool,
    pub hotkey: bool,
    pub error: Option<String>,
    transcripts: HashMap<String, Entity<ThreadView>>,
    assets: HashMap<String, Asset>,
    apply_open: bool,
    results: Option<ApplyResult>,
    preview: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl gpui::EventEmitter<CloudAction> for CloudTaskPage {}
impl Focusable for CloudTaskPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CloudTaskPage {
    pub fn new(composer: Entity<Composer>, hotkey: bool, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<AppliedCloudDiffs>() {
            cx.set_global(AppliedCloudDiffs::default());
        }
        Self {
            composer,
            data: None,
            target: None,
            applying: false,
            hotkey,
            error: None,
            transcripts: HashMap::new(),
            assets: HashMap::new(),
            apply_open: false,
            results: None,
            preview: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_data(&mut self, data: CloudTask, cx: &mut Context<Self>) {
        self.error = None;
        self.transcripts
            .retain(|id, _| data.turns.iter().any(|turn| &turn.id == id));
        for turn in &data.turns {
            if let Some(view) = self.transcripts.get(&turn.id) {
                view.update(cx, |view, cx| {
                    view.set_embedded_turns(turn.thread.turns.clone(), cx)
                });
            } else {
                let thread = turn.thread.clone();
                let view = cx.new(|cx| {
                    ThreadView::embedded(
                        thread,
                        Clock {
                            utc_offset: 0,
                            pinned_now: None,
                            elapsed_since: None,
                        },
                        cx,
                    )
                });
                self.transcripts.insert(turn.id.clone(), view);
            }
            for pointer in &turn.images {
                if !self.assets.contains_key(pointer) {
                    self.assets.insert(pointer.clone(), Asset::Loading);
                    cx.emit(CloudAction::LoadImage(pointer.clone()));
                }
            }
        }
        self.composer
            .update(cx, |composer, cx| composer.set_running(data.working, cx));
        self.data = Some(data);
        cx.notify();
    }

    pub fn apply_event(&mut self, turn_id: &str, event: serde_json::Value, cx: &mut Context<Self>) {
        let Some(data) = &mut self.data else {
            return;
        };
        let Some(turn) = data.turns.iter_mut().find(|turn| turn.id == turn_id) else {
            return;
        };
        if event["item_type"] == "log"
            && event
                .pointer("/key/type")
                .and_then(serde_json::Value::as_str)
                == Some("UserSetupScript")
        {
            if let Some(line) = event["line"].as_str() {
                turn.logs.push(line.to_owned());
            }
        } else if event["item_type"] == "thread_event" {
            if let Some(method) = event
                .pointer("/event/method")
                .and_then(serde_json::Value::as_str)
            {
                let mut params = event["event"]["params"].clone();
                params["threadId"] = data.id.clone().into();
                if method.starts_with("item/") {
                    turn.phase = Phase::Chat;
                }
                if method == "turn/diff/updated" {
                    if let Some(diff) = params["diff"].as_str() {
                        data.diff = diff.to_owned();
                        data.diff_turn = Some(turn_id.to_owned());
                    }
                }
                if method == "turn/completed" {
                    data.working = false;
                    self.composer
                        .update(cx, |composer, cx| composer.set_running(false, cx));
                }
                if let Some(view) = self.transcripts.get(turn_id) {
                    let notification = bridge::Event::Notification {
                        method: method.to_owned(),
                        params,
                    };
                    view.update(cx, |view, cx| view.apply(&notification, cx));
                }
            }
        }
        cx.notify();
    }

    pub fn set_image(
        &mut self,
        pointer: String,
        image: Option<Arc<RenderImage>>,
        cx: &mut Context<Self>,
    ) {
        self.assets
            .insert(pointer, image.map_or(Asset::Failed, Asset::Ready));
        cx.notify();
    }

    pub fn finish_apply(
        &mut self,
        turn: &str,
        revert: bool,
        result: ApplyResult,
        cx: &mut Context<Self>,
    ) {
        self.applying = false;
        if result.status == "success" {
            if revert {
                cx.global_mut::<AppliedCloudDiffs>().0.remove(turn);
            } else {
                cx.global_mut::<AppliedCloudDiffs>()
                    .0
                    .insert(turn.to_owned());
            }
            self.apply_open = false;
        } else {
            self.results = Some(result);
        }
        cx.notify();
    }

    fn is_applied(&self, cx: &App) -> bool {
        self.data
            .as_ref()
            .and_then(|data| data.diff_turn.as_ref())
            .is_some_and(|id| cx.global::<AppliedCloudDiffs>().0.contains(id))
    }

    fn button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme();
        let label = label.into();
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .tab_index(0)
            .px(px(12.))
            .py(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .cursor_pointer()
            .hover(|d| d.bg(t.control_background))
            .child(text(label, 14., 20., 500., t.text))
    }

    fn request_apply(&mut self, revert: bool, cx: &mut Context<Self>) {
        if self.target.is_some() && !self.applying {
            self.applying = true;
            cx.emit(CloudAction::Apply { revert });
            cx.notify();
        }
    }

    fn open_web(&self, cx: &mut App) {
        if let Some(data) = &self.data {
            cx.open_url(&format!("https://chatgpt.com/codex/tasks/{}", data.id));
        }
    }

    fn image(&self, pointer: &str, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme();
        let content = match self.assets.get(pointer) {
            Some(Asset::Ready(image)) => img(image.clone())
                .size(px(64.))
                .rounded(px(6.))
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element(),
            Some(Asset::Failed) => return div().into_any_element(),
            _ => {
                return div()
                    .id(SharedString::from(format!("cloud-image-loading-{pointer}")))
                    .size(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.control_background)
                    .aria_label("Loading image")
                    .child(text("…", 14., 20., 430., t.text))
                    .into_any_element();
            }
        };
        let pointer = pointer.to_owned();
        div()
            .id(SharedString::from(format!("cloud-image-{pointer}")))
            .role(gpui::Role::Button)
            .aria_label("User attachment")
            .tab_index(0)
            .cursor_pointer()
            .child(content)
            .on_click(cx.listener(move |page, _, window, cx| {
                if matches!(page.assets.get(&pointer), Some(Asset::Ready(_))) {
                    page.preview = Some(pointer.clone());
                    page.focus.focus(window, cx);
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn turn(&self, turn: &CloudTurn, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        let mut column = div().flex().flex_col().gap(px(12.));
        if !turn.images.is_empty() {
            column = column.child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .children(turn.images.iter().map(|pointer| self.image(pointer, cx))),
            );
        }
        if let Some(view) = self.transcripts.get(&turn.id) {
            column = column.child(view.clone());
        }
        match turn.phase {
            Phase::SettingUp | Phase::SetupFailed => {
                let title = if turn.phase == Phase::SettingUp {
                    "Setting up environment"
                } else {
                    "Environment setup failed"
                };
                let output = if turn.logs.is_empty() {
                    "Waiting for output…".to_owned()
                } else {
                    turn.logs.join("\n")
                };
                column = column
                    .child(text(title, 14., 20., 430., t.text_secondary))
                    .child(
                        div()
                            .id(SharedString::from(format!("cloud-logs-{}", turn.id)))
                            .min_h(px(180.))
                            .max_h(px(500.))
                            .overflow_y_scroll()
                            .overflow_x_scroll()
                            .p(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.control_background)
                            .font_family(crate::MONO)
                            .child(text(output, 13., 20., 430., t.text_tertiary)),
                    );
            }
            Phase::Failed => {
                column = column.child(self.banner(
                    turn.error.clone().unwrap_or_else(|| "Chat failed".into()),
                    cx,
                ))
            }
            Phase::Cancelled => {
                column = column.child(text("Cancelled", 14., 20., 430., t.text_secondary))
            }
            Phase::Chat => {}
        }
        column
    }

    fn banner(&self, message: String, cx: &App) -> gpui::Div {
        let t = cx.theme();
        div()
            .p(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .child(text(message, 14., 20., 430., t.text))
    }

    fn apply_popup(&self, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        let applied = self.is_applied(cx);
        let target = self.target.as_ref();
        let mut card = div()
            .w(px(360.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(text(
                "Apply changes to a local branch",
                14.,
                20.,
                500.,
                t.text,
            ));
        if let Some(target) = target {
            card = card.child(text(target.cwd.clone(), 13., 18., 430., t.text_secondary));
            if let Some(branch) = &target.branch {
                card = card.child(text(branch.clone(), 13., 18., 430., t.text_secondary));
            }
        }
        if let Some(environment) = self
            .data
            .as_ref()
            .and_then(|data| data.environment_label.as_ref())
        {
            card = card.child(text(
                format!(
                    "This chat was created in {environment} so its changes may not apply cleanly."
                ),
                13.,
                18.,
                430.,
                t.text_secondary,
            ));
        }
        card = card.child(
            self.button(
                "cloud-apply-confirm",
                if applied {
                    "Revert changes"
                } else {
                    "Apply changes"
                },
                cx,
            )
            .when(target.is_none() || self.applying, |button| {
                button.opacity(0.5)
            })
            .on_click(cx.listener(move |page, _, _, cx| page.request_apply(applied, cx))),
        );
        if applied {
            card = card.child(
                self.button("cloud-reapply", "Reapply", cx)
                    .on_click(cx.listener(|page, _, _, cx| page.request_apply(false, cx))),
            );
        }
        popover(&t, 360., |content| content.child(card))
    }

    fn results_dialog(&self, result: &ApplyResult, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme();
        let mut card = div()
            .w(px(480.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text("Apply results", 18., 26., 600., t.text));
        let any = !result.applied_paths.is_empty()
            || !result.conflicted_paths.is_empty()
            || !result.skipped_paths.is_empty();
        for (title, paths) in [
            ("Applied cleanly", &result.applied_paths),
            ("Conflicted", &result.conflicted_paths),
            ("Skipped", &result.skipped_paths),
        ] {
            if !paths.is_empty() {
                card = card
                    .child(text(
                        format!("{title} ({})", paths.len()),
                        14.,
                        20.,
                        500.,
                        t.text,
                    ))
                    .children(
                        paths
                            .iter()
                            .map(|path| text(path.clone(), 14., 20., 430., t.text_secondary)),
                    );
            }
        }
        if !any {
            card = card.child(text(
                if result.error_code.as_deref() == Some("not-git-repo") {
                    "This action only works when running in a Git repository."
                } else {
                    "No file details available."
                },
                14.,
                20.,
                430.,
                t.text_secondary,
            ));
        }
        card.child(
            self.button("cloud-results-close", "Close", cx)
                .on_click(cx.listener(|page, _, _, cx| {
                    page.results = None;
                    cx.notify();
                })),
        )
    }

    fn header(&self, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        let mut header = div().h(px(28.)).flex().items_center().gap(px(8.));
        if self.hotkey {
            header = header
                .child(text(
                    self.data
                        .as_ref()
                        .map(|data| data.title.clone())
                        .unwrap_or_else(|| "Codex".into()),
                    14.,
                    20.,
                    500.,
                    t.text,
                ))
                .child(
                    self.button("cloud-open-main", "Open in Main Window", cx)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(CloudAction::OpenMain))),
                );
        }
        if self.data.as_ref().is_some_and(|data| !data.diff.is_empty()) {
            header = header.child(
                self.button(
                    "cloud-apply",
                    if self.applying {
                        "Applying…"
                    } else if self.is_applied(cx) {
                        "Revert"
                    } else {
                        "Apply"
                    },
                    cx,
                )
                .h(px(28.))
                .py(px(3.))
                .on_click(cx.listener(|page, _, _, cx| {
                    if page.target.is_some() && !page.applying {
                        page.apply_open = !page.apply_open;
                        cx.notify();
                    }
                })),
            );
        }
        header.child(crate::tooltip::tooltip(
            self.button("cloud-open-web", "Open", cx)
                .h(px(28.))
                .py(px(3.))
                .flex()
                .flex_row_reverse()
                .items_center()
                .gap(px(4.))
                .child(icon("cloud-web", 12., t.text))
                .on_click(cx.listener(|page, _, _, cx| page.open_web(cx))),
            "Open in web",
            None,
            crate::tooltip::Side::Bottom,
        ))
    }
}

pub struct CloudTaskHeader {
    page: Entity<CloudTaskPage>,
}

impl CloudTaskHeader {
    pub fn new(page: Entity<CloudTaskPage>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        Self { page }
    }
}

impl Render for CloudTaskHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.page.update(cx, |page, cx| page.header(cx))
    }
}

impl Render for CloudTaskPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut body = div()
            .id("cloud-task-page")
            .size_full()
            .relative()
            .tab_group()
            .tab_index(3)
            .flex()
            .flex_col()
            .bg(t.surface)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, window, cx| {
                crate::focus::tab(event, window, cx);
                if event.keystroke.key == "escape" {
                    page.preview = None;
                    page.results = None;
                    page.apply_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|_, _: &crate::composer::Stop, _, cx| {
                cx.emit(CloudAction::Cancel);
                cx.stop_propagation();
            }));
        let mut content = div()
            .id("cloud-task-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if let Some(error) = &self.error
            && self.data.is_none()
        {
            content = content.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .child(text(
                        "Could not load this Codex chat",
                        16.,
                        24.,
                        500.,
                        t.text,
                    ))
                    .child(text(error.clone(), 13., 18., 430., t.text_secondary))
                    .child(
                        self.button("cloud-retry", "Retry", cx)
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.error = None;
                                cx.emit(CloudAction::Retry);
                                cx.notify();
                            })),
                    ),
            );
        } else if let Some(data) = &self.data {
            content = content.child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(768.))
                    .px(px(16.))
                    .pt(px(8.))
                    .pb(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .when_some(self.error.as_ref(), |column, error| {
                        column.child(text(format!("Error: {error}"), 14., 20., 430., t.text))
                    })
                    .children(data.turns.iter().map(|turn| self.turn(turn, cx))),
            );
        } else {
            content = content.child(
                div()
                    .w_full()
                    .pt(px(24.))
                    .flex()
                    .justify_center()
                    .child(icon("sidebar-cmd-spinner", 20., t.text_secondary)),
            );
        }
        body = body.child(content);
        if let Some(data) = &self.data {
            let mut footer = div()
                .mx_auto()
                .w_full()
                .max_w(px(768.))
                .px(px(16.))
                .pb(px(16.))
                .flex()
                .flex_col()
                .gap(px(8.));
            if !data.diff.is_empty() && data.diff_turn.is_some() {
                let applied = self.is_applied(cx);
                let mut actions = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(text(
                        if applied {
                            "Revert applied changes?"
                        } else {
                            "Apply changes and continue locally?"
                        },
                        16.,
                        24.,
                        430.,
                        t.text,
                    )))
                    .child(
                        self.button(
                            "cloud-footer-apply",
                            if applied { "Revert" } else { "Apply" },
                            cx,
                        )
                        .when(self.target.is_none() || self.applying, |button| {
                            button.opacity(0.5)
                        })
                        .on_click(
                            cx.listener(move |page, _, _, cx| page.request_apply(applied, cx)),
                        ),
                    );
                if applied {
                    actions = actions.child(
                        self.button("cloud-footer-reapply", "Reapply", cx)
                            .on_click(cx.listener(|page, _, _, cx| page.request_apply(false, cx))),
                    );
                }
                footer = footer.child(
                    div()
                        .p(px(12.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(t.border)
                        .child(actions),
                );
            }
            if data
                .turns
                .last()
                .is_some_and(|turn| turn.phase == Phase::Failed || turn.phase == Phase::SetupFailed)
            {
                footer = footer
                    .child(self.banner("An error occurred during this chat".into(), cx))
                    .child(
                        self.button("cloud-error-open-web", "Open in web", cx)
                            .on_click(cx.listener(|page, _, _, cx| page.open_web(cx))),
                    );
            }
            body = body.child(footer.when(data.active_turn.is_some(), |footer| {
                footer.child(self.composer.clone())
            }));
        }
        if self.apply_open {
            body = body.child(
                div()
                    .absolute()
                    .right(px(16.))
                    .top(px(44.))
                    .child(self.apply_popup(cx)),
            );
        }
        if self.preview.is_some() || self.results.is_some() {
            let mut overlay = div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.8));
            if let Some(result) = &self.results {
                let card = self.results_dialog(result, cx);
                overlay = overlay.child(popover(&t, 480., |content| content.child(card)));
            }
            if let Some(pointer) = &self.preview {
                if let Some(Asset::Ready(image)) = self.assets.get(pointer) {
                    overlay = overlay.child(img(image.clone()).max_w_full().max_h_full());
                }
                overlay = overlay.child(
                    self.button("cloud-preview-close", "", cx)
                        .aria_label("Close image preview")
                        .child(icon("close", 16., t.text))
                        .absolute()
                        .top(px(16.))
                        .right(px(16.))
                        .on_click(cx.listener(|page, _, _, cx| {
                            page.preview = None;
                            cx.notify();
                        })),
                );
            }
            body = body.child(overlay);
        }
        body
    }
}

pub fn image_dimensions(bytes: &[u8]) -> anyhow::Result<(u32, u32)> {
    let image = image::load_from_memory(bytes)?;
    Ok((image.width(), image.height()))
}

pub fn decode_image(bytes: &[u8]) -> anyhow::Result<Arc<RenderImage>> {
    let mut image = image::load_from_memory(bytes)?.into_rgba8();
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new([image::Frame::new(image)])))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoded_attachment_uses_native_bgra_without_changing_alpha() {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 200, 128]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        pixels
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image(&encoded.into_inner()).unwrap();
        assert_eq!(decoded.as_bytes(0), Some([200, 20, 10, 128].as_slice()));
    }

    use super::*;
    use bridge::types::{ThreadStatus, Turn, TurnStatus};
    use gpui::{HeadlessAppContext, size};
    use serde_json::json;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn setup_stream_transitions_to_the_real_assistant_output() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut app =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        app.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
        });
        let holder = Rc::new(RefCell::new(None));
        let output = holder.clone();
        let _window = app
            .open_window(size(px(960.), px(640.)), move |window, cx| {
                let composer = cx.new(|cx| Composer::thread(window, cx));
                let page = cx.new(|cx| CloudTaskPage::new(composer, false, cx));
                *output.borrow_mut() = Some(page.clone());
                page
            })
            .unwrap();
        let page = holder.borrow().clone().unwrap();
        let thread = Thread {
            id: "task".into(),
            preview: String::new(),
            name: Some("Cloud".into()),
            model: None,
            reasoning_effort: None,
            model_provider: String::new(),
            cwd: String::new(),
            created_at: 0,
            updated_at: 0,
            section: None,
            status: ThreadStatus::Active,
            git_info: None,
            turns: vec![Turn {
                id: "turn".into(),
                items: vec![],
                status: TurnStatus::InProgress,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
            }],
        };
        app.update(|cx| page.update(cx, |page, cx| {
            page.set_data(CloudTask { id:"task".into(),title:"Cloud".into(),turns:vec![CloudTurn {id:"turn".into(),thread,phase:Phase::SettingUp,error:None,logs:vec![],images:vec![]}],active_turn:Some("turn".into()),diff_turn:None,diff:String::new(),clone_url:None,environment_label:None,working:true },cx);
            page.apply_event("turn",json!({"item_type":"log","key":{"type":"UserSetupScript"},"line":"ready"}),cx);
            assert_eq!(page.data.as_ref().unwrap().turns[0].logs,["ready"]);
            page.apply_event("turn",json!({"item_type":"thread_event","event":{"method":"item/completed","params":{"threadId":"remote","turnId":"turn","item":{"type":"agentMessage","id":"answer","text":"Done"}}}}),cx);
            assert_eq!(page.data.as_ref().unwrap().turns[0].phase,Phase::Chat);
            assert!(matches!(&page.transcripts["turn"].read(cx).turns()[0].items[..], [bridge::types::ThreadItem::AgentMessage {text,..}] if text == "Done"));
        }));
        app.run_until_parked();
    }
}
