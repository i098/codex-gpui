use super::BrowserTab;
use crate::composer::Composer;
use crate::screens::diff::panel::comments::browser::{
    self, CommentMetadata, Selection, SelectionKind,
};
use crate::screens::diff::panel::comments::{Comment, Position, Side};
#[cfg(target_os = "macos")]
use gpui::{AppContext as _, Window};
use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use std::num::NonZeroU32;
use theme::ActiveTheme as _;

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Pick {
    generation: u32,
    kind: SelectionKind,
    target_rect: browser::Rect,
    target_description: Option<String>,
    target_role: Option<String>,
    target_selector: Option<String>,
    target_path: Option<String>,
    target_immediate_text: Option<String>,
    nearby_text: Option<String>,
    selected_text: Option<String>,
}

#[derive(Default)]
pub(super) struct Annotation {
    pub(super) generation: u32,
    multi_select: bool,
    draft: Option<Draft>,
}

struct Draft {
    kind: SelectionKind,
    context: browser::Context,
    editor: Entity<Composer>,
    _observe: gpui::Subscription,
}

impl BrowserTab {
    pub fn set_annotation_multi_select(&mut self, enabled: bool) {
        self.annotation.multi_select = enabled;
        self.sync_annotation();
    }

    pub(super) fn reset_annotation(&mut self) {
        self.annotation.draft = None;
        self.annotation.generation = self.annotation.generation.wrapping_add(1);
        self.sync_annotation();
    }

    pub(super) fn sync_annotation(&self) {
        let mode = if self.hidden || !self.annotating || !self.can_annotate() {
            0
        } else if let Some(draft) = &self.annotation.draft {
            if self.annotation.multi_select && draft.kind != SelectionKind::Text {
                2
            } else {
                3
            }
        } else {
            1
        };
        self.page.annotation_mode(self.annotation.generation, mode);
        #[cfg(target_os = "macos")]
        self.page.site_annotation_state(
            self.site_annotation_api.filter(|_| {
                !self.site_annotation_blocked && !self.hidden && self.can_annotate() && mode < 2
            }),
            self.annotating,
        );
    }

    #[cfg(target_os = "macos")]
    pub(super) fn select_annotation(
        &mut self,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.annotating
            || self.hidden
            || !self.can_annotate()
            || self.annotation.generation != pick.generation
        {
            return;
        }
        let Some(url) = self.state.url.as_ref() else {
            return;
        };
        let page_url = url.to_string();
        let browser::Rect {
            x,
            y,
            width,
            height,
        } = pick.target_rect;
        if x < 0. || y < 0. || width <= 0. || height <= 0. {
            return;
        }
        if let Some(draft) = &mut self.annotation.draft {
            if !self.annotation.multi_select
                || draft.kind == SelectionKind::Text
                || draft.context.additional_selections.len() >= 31
            {
                return;
            }
            let selection = match pick.kind {
                SelectionKind::Element => Selection::Element {
                    page_url,
                    frame_path: vec![],
                    frame_url: None,
                    target_description: pick.target_description,
                    target_name: None,
                    target_role: pick.target_role,
                    target_selector: pick.target_selector,
                    target_path: pick.target_path,
                    target_immediate_text: pick.target_immediate_text,
                    nearby_text: pick.nearby_text,
                },
                SelectionKind::Region => Selection::Region {
                    page_url,
                    frame_path: vec![],
                    frame_url: None,
                    rect: pick.target_rect,
                    target_path: None,
                },
                _ => return,
            };
            if !draft.context.additional_selections.contains(&selection) {
                draft.context.additional_selections.push(selection);
                cx.notify();
            }
            return;
        }
        if pick.kind == SelectionKind::Virtual
            || (pick.kind == SelectionKind::Text && pick.selected_text.is_none())
        {
            return;
        }
        let weak = cx.weak_entity();
        let editor = cx.new(|cx| {
            Composer::comment_editor(
                "",
                move |_, cx| {
                    let _ = weak.update(cx, |tab, cx| {
                        tab.reset_annotation();
                        cx.notify();
                    });
                },
                window,
                cx,
            )
        });
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        self.annotation.draft = Some(Draft {
            kind: pick.kind,
            context: browser::Context {
                page_url,
                selected_text: pick.selected_text,
                target_description: pick.target_description,
                target_role: pick.target_role,
                target_selector: pick.target_selector,
                target_path: pick.target_path,
                nearby_text: pick.nearby_text,
                target_rect: Some(pick.target_rect),
                ..Default::default()
            },
            editor,
            _observe: observe,
        });
        self.annotation.generation = self.annotation.generation.wrapping_add(1);
        self.sync_annotation();
        cx.notify();
    }

    fn add_annotation(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self
            .annotation
            .draft
            .take_if(|draft| !draft.editor.read(cx).field_text().trim().is_empty())
        else {
            return;
        };
        let text = draft.editor.read(cx).field_text().trim().to_owned();
        let label = draft
            .context
            .selected_text
            .as_deref()
            .or(draft.context.target_description.as_deref())
            .unwrap_or("Selected browser region");
        let mut comment = Comment::new(
            Position {
                path: format!("browser:{label}"),
                side: Side::Right,
                line: NonZeroU32::MIN,
                start_line: None,
                start_side: None,
            },
            text,
            None,
            None,
        );
        comment.local_browser_context = Some(draft.context);
        comment.local_browser_comment_metadata = Some(CommentMetadata { kind: draft.kind });
        self.reset_annotation();
        cx.emit(super::BrowserAnnotationAdded(comment));
        cx.notify();
    }

    pub(super) fn annotation_card(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let draft = self.annotation.draft.as_ref()?;
        let enabled = !draft.editor.read(cx).field_text().trim().is_empty();
        let t = cx.theme();
        let selection_label = |kind, index| {
            let label = match kind {
                SelectionKind::Element => "Selected element",
                SelectionKind::Region => "Selected area",
                _ => "Selected text",
            };
            format!("{label} {index}")
        };
        let labels =
            std::iter::once(draft.kind).chain(draft.context.additional_selections.iter().map(
                |extra| match extra {
                    Selection::Element { .. } => SelectionKind::Element,
                    Selection::Region { .. } => SelectionKind::Region,
                },
            ));
        let cancel = div()
            .id("browser-comment-cancel")
            .role(gpui::Role::Button)
            .cursor_pointer()
            .px(px(8.))
            .child(crate::text("Cancel", 13., 22., 500., t.text_secondary))
            .on_click(cx.listener(|tab, _, _, cx| {
                tab.reset_annotation();
                cx.notify();
            }));
        let add = div()
            .id("browser-comment-add")
            .role(gpui::Role::Button)
            .aria_label("Add comment")
            .w(px(28.))
            .h(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.button_primary_solid)
            .child(crate::icon("composer-arrow-up", 16., t.text_inverse))
            .map(|button| {
                if enabled {
                    button
                        .cursor_pointer()
                        .on_click(cx.listener(|tab, _, _, cx| tab.add_annotation(cx)))
                } else {
                    button.opacity(0.4)
                }
            });
        Some(
            div()
                .id("browser-comment-card")
                .flex_none()
                .border_t_1()
                .border_color(t.border_subtle)
                .p(px(12.))
                .gap(px(8.))
                .flex()
                .flex_col()
                .on_action(
                    cx.listener(|tab, _: &crate::composer::ModifiedSubmit, _, cx| {
                        tab.add_annotation(cx);
                    }),
                )
                .child(
                    div()
                        .id("browser-comment-selections")
                        .flex()
                        .gap(px(6.))
                        .overflow_x_scroll()
                        .children(labels.enumerate().map(|(index, kind)| {
                            div()
                                .flex_none()
                                .rounded(px(4.))
                                .px(px(6.))
                                .bg(t.text_alpha_5)
                                .child(crate::text(
                                    selection_label(kind, index + 1),
                                    12.,
                                    20.,
                                    400.,
                                    t.text_secondary,
                                ))
                        })),
                )
                .child(
                    div()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(t.border_subtle)
                        .p(px(8.))
                        .h(px(76.))
                        .child(draft.editor.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .items_center()
                        .justify_end()
                        .child(cancel)
                        .child(add),
                ),
        )
    }
}
