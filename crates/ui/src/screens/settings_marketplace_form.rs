use super::{Action, Activate as _, State, action_button, dispatch, form::Close};
use crate::{composer::Composer, text};
use gpui::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use serde_json::{Value, json};
use theme::ActiveTheme as _;

pub struct MarketplaceForm {
    source: Entity<Composer>,
    reference: Entity<Composer>,
    sparse_paths: Entity<Composer>,
    error: Option<&'static str>,
}

impl EventEmitter<Close> for MarketplaceForm {}

impl MarketplaceForm {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut field = |placeholder| {
            let field = cx.new(|cx| Composer::settings_field(placeholder, "", false, cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            field
        };
        Self {
            source: field("openai/plugins or git@github.com:org/repo.git"),
            reference: field("main"),
            sparse_paths: field("plugins/codex"),
            error: None,
        }
    }

    pub fn controls(&self, window: &mut Window, cx: &mut App) -> Vec<FocusHandle> {
        vec![
            self.source.focus_handle(cx),
            self.reference.focus_handle(cx),
            self.sparse_paths.focus_handle(cx),
            crate::focus::handle("cancel-marketplace-add", 0, window, cx)
                .read(cx)
                .handle
                .clone(),
            crate::focus::handle("submit-marketplace-add", 0, window, cx)
                .read(cx)
                .handle
                .clone(),
        ]
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        match parameters(
            self.source.read(cx).field_text(),
            self.reference.read(cx).field_text(),
            self.sparse_paths.read(cx).field_text(),
        ) {
            Some(params) => {
                self.error = None;
                dispatch(Action::AddMarketplace(params), cx);
            }
            None => self.error = Some("Enter a marketplace source"),
        }
        cx.notify();
    }
}

fn parameters(source: &str, reference: &str, sparse_paths: &str) -> Option<Value> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let paths = sparse_paths
        .split(['\n', ','])
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    Some(
        json!({"source":source,"refName":(!reference.trim().is_empty()).then(|| reference.trim()),
        "sparsePaths":(!paths.is_empty()).then_some(paths)}),
    )
}

impl Render for MarketplaceForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.global::<State>().clone();
        let pending = state.pending;
        let field = |label, editor: Entity<Composer>| -> Div {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(text(label, 13., 18., 500., t.text))
                .child(editor)
        };
        div()
            .w(px(576.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(text("Add plugin marketplace", 20., 28., 500., t.text))
            .child(text(
                "Add from a GitHub repo, Git URL, or local folder.",
                13.,
                18.,
                430.,
                t.text_secondary,
            ))
            .child(field("Source", self.source.clone()))
            .child(field("Git ref", self.reference.clone()))
            .child(field("Sparse paths", self.sparse_paths.clone()))
            .children(
                self.error
                    .map(|error| text(error, 13., 18., 430., t.text_error)),
            )
            .children(
                state.operation_error.as_ref().map(|error| {
                    text(error.clone(), 13., 18., 430., t.text_error).whitespace_normal()
                }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        action_button("cancel-marketplace-add", "Cancel", &t)
                            .aria_disabled(pending)
                            .on_activate(window, cx, move |_, _, cx| {
                                if !pending {
                                    cx.emit(Close);
                                }
                            }),
                    )
                    .child(
                        action_button("submit-marketplace-add", "Add marketplace", &t)
                            .aria_disabled(pending)
                            .on_activate(window, cx, move |this, _, cx| {
                                if !pending {
                                    this.submit(cx);
                                }
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_options_trim_without_splitting_path_spaces() {
        super::super::test_panic_hook();
        assert!(parameters("  ", "main", "plugins").is_none());
        assert_eq!(
            parameters(
                " /tmp/team plugins ",
                " stable ",
                " plugins/a,\n plugins with spaces \n, "
            ),
            Some(
                json!({"source":"/tmp/team plugins","refName":"stable","sparsePaths":["plugins/a","plugins with spaces"]})
            )
        );
        assert_eq!(
            parameters("org/repo", "", ""),
            Some(json!({"source":"org/repo","refName":null,"sparsePaths":null}))
        );
    }
}
