//! Artifact annotations on a PDF or image preview (bundle `artifact-annotation-batch-controls`):
//! drag a region or click a point on the current page, comment on it, and send the batch with
//! Send or Cmd+Enter (`artifact.sendAnnotations`) as one `artifact-direct-comment` turn. The
//! Changes panel's previews and an artifact tab's PDF ([`Annotate`]) take them.

use super::comment_view::comment_button;
use super::comments::{ArtifactContext, ArtifactTarget, Comment, PagePoint, PageSize};
use super::*;
use crate::composer::Composer;
use gpui::{
    AnyElement, Bounds, MouseMoveEvent, MouseUpEvent, Point, Subscription, anchored, canvas, hsla,
};
use std::{num::NonZeroU32, sync::LazyLock};

/// The annotation mark color (`#2563eb`).
fn mark() -> Hsla {
    hsla(221. / 360., 0.83, 0.53, 1.)
}

/// `--popcorn-review-accent` (`#0285ff`).
fn accent() -> Hsla {
    hsla(209. / 360., 1., 0.5, 1.)
}

static TABULAR_NUMBERS: LazyLock<gpui::FontFeatures> =
    LazyLock::new(|| gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)])));

/// A press shorter than this many window px in both directions is a point annotation.
const POINT_DRAG: f32 = 4.;
const CARD_WIDTH: f32 = 320.;

/// The page a preview shows, in the units its annotations use: the rendered raster's pixels.
/// shortcut: PDF coordinates are raster px, not PDF points; pass the page box from
/// `pdf::render` when a model needs exact PDF units.
#[derive(Clone)]
pub(super) struct Page {
    pub(super) path: String,
    pub(super) title: String,
    pub(super) kind: &'static str,
    pub(super) number: usize,
    pub(super) count: usize,
    pub(super) size: PageSize,
    /// The page's window bounds, recorded at paint.
    pub(super) bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Page {
    fn to_page(&self, position: Point<Pixels>) -> PagePoint {
        let bounds = self.bounds.get();
        let fraction = |offset: Pixels, extent: Pixels| {
            (f32::from(offset) / f32::from(extent).max(1.)).clamp(0., 1.)
        };
        PagePoint {
            x: fraction(position.x - bounds.left(), bounds.size.width) * self.size.width,
            y: fraction(position.y - bounds.top(), bounds.size.height) * self.size.height,
        }
    }

    /// Page units per window px.
    fn units(&self) -> f32 {
        self.size.width / f32::from(self.bounds.get().size.width).max(1.)
    }
}

struct Drag {
    path: String,
    start: PagePoint,
    end: PagePoint,
}

pub(super) struct Draft {
    context: ArtifactContext,
    target: ArtifactTarget,
    /// The draft edits a pending annotation (Save and Delete) rather than adds one.
    editing: bool,
    editor: Entity<Composer>,
    _observe: Subscription,
}

#[derive(Default)]
pub(super) struct ArtifactAnnotations {
    pending: Vec<Comment>,
    draft: Option<Draft>,
    drag: Option<Drag>,
    /// The artifact Cmd+Enter sends: the last one annotated.
    active: Option<String>,
    /// The annotation the review bar navigation selected.
    current: Option<String>,
}

impl ArtifactAnnotations {
    pub(super) fn pending(&self) -> &[Comment] {
        &self.pending
    }

    pub(super) fn draft_editor(&self) -> Option<&Entity<Composer>> {
        self.draft.as_ref().map(|draft| &draft.editor)
    }
}

fn annotation_id(comment: &Comment) -> Option<String> {
    comment
        .artifact_context()
        .map(|context| context.annotation_id)
}

fn on_path(comment: &Comment, path: &str) -> Option<(ArtifactContext, ArtifactTarget)> {
    let context = comment.artifact_context()?;
    (context.path == path).then_some(())?;
    Some((context, comment.artifact_target()?))
}

/// Window px per page unit for a target recorded on a page of `target`'s size.
fn scale(target: &ArtifactTarget, displayed: Pixels) -> f32 {
    f32::from(displayed) / target.page_size().width.max(1.)
}

impl DiffPanel {
    pub fn artifact_annotations(&self) -> &[Comment] {
        self.artifacts.pending()
    }

    pub fn artifact_annotation_draft(&self) -> Option<&Entity<Composer>> {
        self.artifacts.draft_editor()
    }

    /// The open chat accepted the batch of the active artifact.
    pub fn artifact_annotations_sent(&mut self, cx: &mut Context<Self>) {
        self.annotations_sent(cx);
    }
}

impl Annotate for DiffPanel {
    fn annotations(&self) -> &ArtifactAnnotations {
        &self.artifacts
    }

    fn annotations_mut(&mut self) -> &mut ArtifactAnnotations {
        &mut self.artifacts
    }

    fn annotation_focus(&self) -> Option<FocusHandle> {
        self.focus.clone()
    }

    fn show_annotation_page(
        &mut self,
        index: usize,
        page: usize,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.show_preview_page(index, 1, page, width, window, cx);
    }

    fn send_annotation_batch(&mut self, batch: Vec<Comment>, cx: &mut Context<Self>) {
        cx.emit(DiffRequest::SendAnnotations(batch));
    }
}

impl FileView {
    pub fn artifact_annotations(&self) -> &[Comment] {
        self.artifacts.pending()
    }

    pub fn artifact_annotation_draft(&self) -> Option<&Entity<Composer>> {
        self.artifacts.draft_editor()
    }

    /// The open chat accepted the artifact tab's batch.
    pub fn artifact_annotations_sent(&mut self, cx: &mut Context<Self>) {
        self.annotations_sent(cx);
    }
}

impl Annotate for FileView {
    fn annotations(&self) -> &ArtifactAnnotations {
        &self.artifacts
    }

    fn annotations_mut(&mut self) -> &mut ArtifactAnnotations {
        &mut self.artifacts
    }

    fn annotation_focus(&self) -> Option<FocusHandle> {
        self.focus.clone()
    }

    fn show_annotation_page(
        &mut self,
        _index: usize,
        page: usize,
        _width: Pixels,
        _window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.show_media_page(page, cx);
    }

    fn send_annotation_batch(&mut self, batch: Vec<Comment>, cx: &mut Context<Self>) {
        cx.emit(FileTreeRequest::SendAnnotations(batch));
    }
}

/// A view whose artifact preview takes annotations: the Changes panel and an artifact tab.
pub(super) trait Annotate: Sized + 'static {
    fn annotations(&self) -> &ArtifactAnnotations;
    fn annotations_mut(&mut self) -> &mut ArtifactAnnotations;
    /// The view's focus, which a drag and a closed draft return to.
    fn annotation_focus(&self) -> Option<FocusHandle>;
    /// Shows `page` of preview `index`, where a selected annotation sits.
    fn show_annotation_page(
        &mut self,
        index: usize,
        page: usize,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    );
    /// Asks the open chat to send `batch` as one turn; it answers with [`Self::annotations_sent`].
    fn send_annotation_batch(&mut self, batch: Vec<Comment>, cx: &mut Context<Self>);

    /// The open chat accepted the batch of the active artifact.
    fn annotations_sent(&mut self, cx: &mut Context<Self>) {
        let artifacts = self.annotations_mut();
        if let Some(path) = artifacts.active.clone() {
            artifacts
                .pending
                .retain(|comment| on_path(comment, &path).is_none());
            artifacts.current = None;
            cx.notify();
        }
    }

    fn batch(&self, path: &str) -> Vec<Comment> {
        self.annotations()
            .pending
            .iter()
            .filter(|comment| on_path(comment, path).is_some())
            .cloned()
            .collect()
    }

    /// Cmd+Enter: add the open draft, else `artifact.sendAnnotations`. False when there is
    /// nothing to add or send.
    fn artifact_modified_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.annotations().draft.is_some() {
            self.save_artifact_draft(window, cx);
            return true;
        }
        self.send_artifact_annotations(cx)
    }

    /// `artifact.sendAnnotations`: the batch of the artifact last annotated.
    fn send_artifact_annotations(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.annotations().active.clone() else {
            return false;
        };
        let batch = self.batch(&path);
        if batch.is_empty() {
            return false;
        }
        self.send_annotation_batch(batch, cx);
        true
    }

    fn cancel_artifact_batch(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let artifacts = self.annotations_mut();
        artifacts
            .pending
            .retain(|comment| on_path(comment, path).is_none());
        artifacts.current = None;
        self.close_artifact_draft(window, cx);
    }

    fn start_artifact_drag(
        &mut self,
        page: &Page,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = page.to_page(position);
        let artifacts = self.annotations_mut();
        artifacts.draft = None;
        artifacts.drag = Some(Drag {
            path: page.path.clone(),
            start: at,
            end: at,
        });
        artifacts.active = Some(page.path.clone());
        if let Some(focus) = self.annotation_focus() {
            window.focus(&focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn move_artifact_drag(&mut self, page: &Page, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(drag) = &mut self.annotations_mut().drag
            && drag.path == page.path
        {
            drag.end = page.to_page(position);
            cx.notify();
        }
    }

    fn finish_artifact_drag(&mut self, page: &Page, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self
            .annotations_mut()
            .drag
            .take_if(|drag| drag.path == page.path)
        else {
            return;
        };
        let target = ArtifactTarget::new(
            drag.start,
            drag.end,
            POINT_DRAG * page.units(),
            page.number,
            page.count,
            page.size,
        );
        let context = ArtifactContext {
            title: page.title.clone(),
            path: page.path.clone(),
            annotation_id: uuid::Uuid::new_v4().to_string(),
            artifact_kind: page.kind.to_owned(),
            label: target.label(),
        };
        self.open_artifact_draft(context, target, false, String::new(), window, cx);
    }

    fn open_artifact_draft(
        &mut self,
        context: ArtifactContext,
        target: ArtifactTarget,
        editing: bool,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let editor = cx.new(|cx| {
            Composer::comment_editor(
                &text,
                move |window, cx| {
                    let _ = weak.update(cx, |panel, cx| panel.close_artifact_draft(window, cx));
                },
                window,
                cx,
            )
        });
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        let artifacts = self.annotations_mut();
        artifacts.active = Some(context.path.clone());
        artifacts.current = editing.then(|| context.annotation_id.clone());
        artifacts.draft = Some(Draft {
            context,
            target,
            editing,
            editor,
            _observe: observe,
        });
        cx.notify();
    }

    fn close_artifact_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.annotations_mut().draft = None;
        if let Some(focus) = self.annotation_focus() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn save_artifact_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self
            .annotations_mut()
            .draft
            .take_if(|draft| !draft.editor.read(cx).field_text().trim().is_empty())
        else {
            return;
        };
        let text = draft.editor.read(cx).field_text().trim().to_owned();
        let id = draft.context.annotation_id.clone();
        let pending = &mut self.annotations_mut().pending;
        let existing = pending
            .iter()
            .position(|comment| annotation_id(comment).as_ref() == Some(&id));
        let line = match existing {
            Some(index) => pending[index].position.line.get(),
            None => {
                pending
                    .iter()
                    .map(|comment| comment.position.line.get())
                    .max()
                    .unwrap_or(0)
                    + 1
            }
        };
        let comment = Comment::artifact(
            draft.context,
            draft.target,
            NonZeroU32::new(line).unwrap(),
            text,
        );
        match existing {
            Some(index) => pending[index] = comment,
            None => pending.push(comment),
        }
        self.annotations_mut().current = None;
        self.close_artifact_draft(window, cx);
    }

    fn delete_artifact_annotation(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let artifacts = self.annotations_mut();
        artifacts
            .pending
            .retain(|comment| annotation_id(comment).as_deref() != Some(id));
        artifacts.current = None;
        self.close_artifact_draft(window, cx);
    }

    fn edit_artifact_annotation(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self
            .annotations()
            .pending
            .iter()
            .find(|comment| annotation_id(comment).as_deref() == Some(id))
        else {
            return;
        };
        let (Some(context), Some(target)) = (comment.artifact_context(), comment.artifact_target())
        else {
            return;
        };
        let text = comment.text();
        self.open_artifact_draft(context, target, true, text, window, cx);
    }

    /// Previous/Next request: select the neighbour annotation and turn to its page.
    fn step_artifact_annotation(
        &mut self,
        path: &str,
        step: isize,
        index: usize,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let batch = self.batch(path);
        if batch.is_empty() {
            return;
        }
        let at = self.annotations().current.as_ref().and_then(|id| {
            batch
                .iter()
                .position(|comment| annotation_id(comment).as_ref() == Some(id))
        });
        let next = match at {
            Some(at) => (at as isize + step).rem_euclid(batch.len() as isize) as usize,
            None if step < 0 => batch.len() - 1,
            None => 0,
        };
        let artifacts = self.annotations_mut();
        artifacts.current = annotation_id(&batch[next]);
        artifacts.active = Some(path.to_owned());
        if let Some(target) = batch[next].artifact_target() {
            self.show_annotation_page(index, target.page_number(), width, window, cx);
        }
        cx.notify();
    }

    /// The annotation layer over `page`, displayed `displayed` px wide.
    fn artifact_layer(&self, page: Page, displayed: Pixels, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut layer = Vec::new();
        let recorder = page.bounds.clone();
        layer.push(
            canvas(move |bounds, _, _| recorder.set(bounds), |_, _, _, _| {})
                .absolute()
                .inset_0()
                .into_any_element(),
        );
        let (down, moved, up, up_out) = (page.clone(), page.clone(), page.clone(), page.clone());
        layer.push(
            div()
                .id(SharedString::from(format!(
                    "artifact-annotation-layer-{}",
                    page.path
                )))
                .absolute()
                .inset_0()
                .cursor(CursorStyle::Crosshair)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.start_artifact_drag(&down, event.position, window, cx)
                    }),
                )
                .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.move_artifact_drag(&moved, event.position, cx);
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                        this.move_artifact_drag(&up, event.position, cx);
                        this.finish_artifact_drag(&up, window, cx);
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseUpEvent, window, cx| {
                        this.move_artifact_drag(&up_out, event.position, cx);
                        this.finish_artifact_drag(&up_out, window, cx);
                    }),
                )
                .into_any_element(),
        );
        let region = |target: &ArtifactTarget, strong: bool| -> Option<AnyElement> {
            let ArtifactTarget::DocumentPageRegion { rect, .. } = target else {
                return None;
            };
            let s = scale(target, displayed);
            Some(
                div()
                    .absolute()
                    .left(px(rect.x * s))
                    .top(px(rect.y * s))
                    .w(px(rect.width * s))
                    .h(px(rect.height * s))
                    .border_2()
                    .border_color(mark())
                    .rounded(px(4.))
                    .bg(mark().opacity(if strong { 0.2 } else { 0.1 }))
                    .into_any_element(),
            )
        };
        let draft = self
            .annotations()
            .draft
            .as_ref()
            .filter(|draft| draft.context.path == page.path);
        let editing = draft
            .filter(|draft| draft.editing)
            .map(|draft| draft.context.annotation_id.clone());
        let mut number = 0;
        for comment in &self.annotations().pending {
            let Some((context, target)) = on_path(comment, &page.path) else {
                continue;
            };
            number += 1;
            if target.page_number() != page.number {
                continue;
            }
            let selected = self.annotations().current.as_ref() == Some(&context.annotation_id);
            layer.extend(region(&target, selected));
            if editing.as_ref() == Some(&context.annotation_id) {
                continue;
            }
            let (x, y) = target.anchor();
            let s = scale(&target, displayed);
            let id = context.annotation_id.clone();
            let keyboard = id.clone();
            layer.push(
                div()
                    .id(SharedString::from(format!("artifact-annotation-{id}")))
                    .absolute()
                    .left(px(x * s) - px(10.))
                    .top(px(y * s) - px(10.))
                    .size(px(20.))
                    .rounded_full()
                    .bg(mark())
                    .when(selected, |d| d.border_2().border_color(gpui::white()))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .aria_label(format!("Edit annotation {number}: {}", context.label))
                    .tab_index(0)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_artifact_annotation(&id, window, cx)
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.edit_artifact_annotation(&keyboard, window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        text(number.to_string(), 11., 14., 600., gpui::white())
                            .font_features(TABULAR_NUMBERS.clone()),
                    )
                    .into_any_element(),
            );
        }
        if let Some(drag) = &self.annotations().drag
            && drag.path == page.path
        {
            let target = ArtifactTarget::new(
                drag.start,
                drag.end,
                POINT_DRAG * page.units(),
                page.number,
                page.count,
                page.size,
            );
            layer.extend(region(&target, true));
        }
        if let Some(draft) = draft.filter(|draft| draft.target.page_number() == page.number) {
            let s = scale(&draft.target, displayed);
            if !draft.editing {
                layer.extend(region(&draft.target, true));
                if let ArtifactTarget::DocumentPagePoint { point, .. } = draft.target {
                    layer.push(
                        div()
                            .absolute()
                            .left(px(point.x * s) - px(6.))
                            .top(px(point.y * s) - px(6.))
                            .size(px(12.))
                            .rounded_full()
                            .bg(mark())
                            .border_2()
                            .border_color(gpui::white())
                            .into_any_element(),
                    );
                }
            }
            // The card opens under the mark, kept inside the window.
            let (x, y) = draft.target.bottom_left();
            layer.push(
                div()
                    .absolute()
                    .left(px(x * s))
                    .top(px(y * s) + px(8.))
                    .child(
                        deferred(
                            anchored()
                                .snap_to_window_with_margin(px(8.))
                                .child(self.artifact_card(draft, cx)),
                        )
                        .with_priority(1),
                    )
                    .into_any_element(),
            );
        }
        layer
    }

    /// `annotation-comment-editor-card`: Cancel, Add (Save when editing), Delete.
    fn artifact_card(&self, draft: &Draft, cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let enabled = !draft.editor.read(cx).field_text().trim().is_empty();
        let id = draft.context.annotation_id.clone();
        div()
            .id("artifact-annotation-card")
            .occlude()
            .w(px(CARD_WIDTH))
            .font_family(SANS)
            .flex()
            .flex_col()
            .rounded(px(12.5))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_elevated_secondary)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().px(px(12.)).pt(px(8.)).child(text(
                draft.target.label(),
                12.,
                16.,
                430.,
                t.text_tertiary,
            )))
            .child(div().p(px(12.)).child(draft.editor.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .px(px(12.))
                    .pb(px(8.))
                    .child(div().when(draft.editing, |d| {
                        d.child(comment_button(
                            "Delete",
                            false,
                            t,
                            cx,
                            move |this, window, cx| {
                                this.delete_artifact_annotation(&id, window, cx)
                            },
                        ))
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(comment_button(
                                "Cancel",
                                false,
                                t,
                                cx,
                                |this, window, cx| this.close_artifact_draft(window, cx),
                            ))
                            .child(
                                comment_button(
                                    if draft.editing { "Save" } else { "Add" },
                                    true,
                                    t,
                                    cx,
                                    |this, window, cx| this.save_artifact_draft(window, cx),
                                )
                                .aria_disabled(!enabled)
                                .when(!enabled, |d| d.opacity(0.5)),
                            ),
                    ),
            )
    }

    /// The batch review bar of `path`'s preview (`popcorn-annotation-batch`, `data-variant=batch`).
    fn artifact_review_bar(
        &self,
        path: &str,
        index: usize,
        width: Pixels,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let count = self.batch(path).len();
        let drafting = self
            .annotations()
            .draft
            .as_ref()
            .is_some_and(|draft| draft.context.path == path)
            || self
                .annotations()
                .drag
                .as_ref()
                .is_some_and(|drag| drag.path == path);
        if count == 0 && !drafting {
            return None;
        }
        let foreground = gpui::white();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label)
                .tab_index(0)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let navigate = |id: &'static str, label: &'static str, name: &'static str, step: isize| {
            let path = path.to_owned();
            let keyboard = path.clone();
            button(id, label)
                .w(px(28.))
                .px_0()
                .justify_center()
                .hover(|s| s.bg(foreground.opacity(0.15)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.step_artifact_annotation(&path, step, index, width, window, cx)
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.step_artifact_annotation(&keyboard, step, index, width, window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(icon(name, 12., foreground))
        };
        let bar = div()
            .id(SharedString::from(format!(
                "artifact-annotation-batch-{path}"
            )))
            .occlude()
            .role(gpui::Role::Toolbar)
            .aria_label("Artifact review")
            .min_h(px(40.))
            .max_w_full()
            .p(px(6.))
            .gap(px(8.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .bg(black().opacity(0.8))
            .shadow_md()
            .font_family(SANS)
            .text_size(px(14.))
            .line_height(px(20.))
            .font_weight(FontWeight(500.))
            .text_color(foreground)
            .whitespace_nowrap()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let bar = if count == 0 {
            bar.child(div().px(px(8.)).child("Select content and ask for changes"))
        } else {
            let (cancel_path, send_path) = (path.to_owned(), path.to_owned());
            bar.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .child(
                        div()
                            .font_features(TABULAR_NUMBERS.clone())
                            .child(if count == 1 {
                                "1 annotation".to_owned()
                            } else {
                                format!("{count} annotations")
                            }),
                    )
                    .when(count > 1, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(navigate(
                                    "artifact-annotation-previous",
                                    "Previous request",
                                    "request-chevron-left",
                                    -1,
                                ))
                                .child(navigate(
                                    "artifact-annotation-next",
                                    "Next request",
                                    "request-chevron-right",
                                    1,
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(1.))
                    .h(px(16.))
                    .flex_none()
                    .bg(foreground.opacity(0.2)),
            )
            .child(
                button("artifact-annotation-cancel", "Cancel")
                    .hover(|s| s.bg(foreground.opacity(0.1)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.cancel_artifact_batch(&cancel_path, window, cx)
                    }))
                    .child("Cancel"),
            )
            .child(
                button("artifact-annotation-send", "Send")
                    .bg(accent())
                    .hover(|s| s.bg(accent().opacity(0.9)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.annotations_mut().active = Some(send_path.clone());
                        this.send_artifact_annotations(cx);
                    }))
                    .child("Send"),
            )
        };
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(12.))
                .flex()
                .justify_center()
                .child(bar)
                .into_any_element(),
        )
    }
}
