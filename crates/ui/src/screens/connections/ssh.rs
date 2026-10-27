use super::{Action, Connection, ConnectionsPage, ConnectionsState, SshDraft, dispatch};
use crate::{composer::Composer, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Corners, Div, Entity,
    FocusHandle, Focusable as _, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, point, prelude::FluentBuilder as _, px,
};
use std::collections::BTreeSet;
use theme::{ActiveTheme as _, Appearance, Theme};

pub(super) struct Editor {
    draft: SshDraft,
    fields: [Entity<Composer>; 4],
    errors: Vec<&'static str>,
    remote_environment: Option<String>,
    saving: bool,
    saved_revision: u64,
    editing: bool,
    details: bool,
    version: Option<String>,
    initial_color: Option<String>,
    color_open: bool,
    focus: FocusHandle,
    previous: Option<FocusHandle>,
}

pub(super) struct Discovery {
    selected: BTreeSet<String>,
    saved_revision: u64,
    saving: bool,
    focus: FocusHandle,
    previous: Option<FocusHandle>,
}

impl ConnectionsPage {
    pub(super) fn open_ssh_discovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.global::<ConnectionsState>();
        if state.pending {
            return;
        }
        self.discovery = Some(Discovery {
            selected: BTreeSet::new(),
            saved_revision: state.saved_revision,
            saving: false,
            focus: cx.focus_handle().tab_stop(true),
            previous: window.focused(cx),
        });
        self.editor = None;
        self.discovery
            .as_ref()
            .expect("new discovery")
            .focus
            .focus(window, cx);
        cx.update_global::<ConnectionsState, _>(|state, _| state.aliases_loaded = false);
        dispatch(Action::DiscoverSsh, cx);
        cx.notify();
    }

    pub(super) fn open_editor(
        &mut self,
        connection: Option<&Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = connection
            .map(|connection| SshDraft {
                host_id: Some(connection.host_id.clone()),
                display_name: connection.display_name.clone(),
                hostname: connection
                    .alias
                    .clone()
                    .or(connection.hostname.clone())
                    .unwrap_or_default(),
                port: connection
                    .ssh_port
                    .map(|port| port.to_string())
                    .unwrap_or_default(),
                identity: connection.identity.clone().unwrap_or_default(),
                use_identity: connection.identity.is_some(),
                alias: connection.alias.is_some(),
                color: None,
            })
            .unwrap_or_else(|| SshDraft {
                host_id: Some(format!("remote-ssh-codex-managed:{}", uuid::Uuid::new_v4())),
                ..Default::default()
            });
        let fields = [
            (("Display name", ""), draft.display_name.as_str()),
            (
                ("Hostname", "host.com or user@host.com"),
                draft.hostname.as_str(),
            ),
            (("SSH port", ""), draft.port.as_str()),
            (("Identity file path", ""), draft.identity.as_str()),
        ]
        .map(|((label, placeholder), value)| {
            cx.new(|cx| {
                let mut field = Composer::inline_field(placeholder, value, cx);
                field.set_field_label(label, cx);
                field
            })
        });
        let previous = self
            .discovery
            .take()
            .and_then(|dialog| dialog.previous)
            .or_else(|| window.focused(cx));
        fields[0].read(cx).focus_handle(cx).focus(window, cx);
        self.editor = Some(Editor {
            draft,
            fields,
            errors: Vec::new(),
            remote_environment: connection.and_then(|connection| connection.environment_id.clone()),
            saving: false,
            saved_revision: cx.global::<ConnectionsState>().saved_revision,
            editing: connection.is_some(),
            details: false,
            version: connection.and_then(|connection| connection.app_server_version.clone()),
            initial_color: connection.and_then(|connection| connection.color.clone()),
            color_open: false,
            focus: cx.focus_handle().tab_stop(true),
            previous,
        });
        cx.notify();
    }

    pub(super) fn open_details(
        &mut self,
        connection: Option<&Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_editor(connection, window, cx);
        if let Some(editor) = &mut self.editor {
            editor.details = true;
            editor.focus.focus(window, cx);
        }
    }

    fn close_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<ConnectionsState>().pending {
            return;
        }
        self.finish_ssh(window, cx);
    }

    fn finish_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous = self
            .editor
            .take()
            .and_then(|editor| editor.previous)
            .or_else(|| self.discovery.take().and_then(|dialog| dialog.previous));
        if let Some(previous) = previous {
            previous.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn sync_ssh_dialogs(
        &mut self,
        state: &ConnectionsState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved =
            self.editor.as_ref().is_some_and(|editor| {
                editor.saving && editor.saved_revision != state.saved_revision
            }) || self.discovery.as_ref().is_some_and(|dialog| {
                dialog.saving && dialog.saved_revision != state.saved_revision
            });
        if saved {
            self.finish_ssh(window, cx);
        } else if !state.pending {
            if let Some(editor) = &mut self.editor {
                editor.saving = false;
            }
            if let Some(dialog) = &mut self.discovery {
                dialog.saving = false;
            }
        }
    }

    pub(super) fn save_editor(&mut self, cx: &mut Context<Self>) {
        if cx.global::<ConnectionsState>().pending {
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        if editor.details {
            return;
        }
        editor.draft.display_name = editor.fields[0].read(cx).field_text().into();
        editor.draft.hostname = editor.fields[1].read(cx).field_text().into();
        editor.draft.port = editor.fields[2].read(cx).field_text().into();
        editor.draft.identity = editor.fields[3].read(cx).field_text().into();
        editor.errors.clear();
        if let Some(environment_id) = editor.remote_environment.clone() {
            let name = editor.draft.display_name.trim();
            if name.is_empty() {
                editor
                    .errors
                    .push("Remote control display name cannot be empty.");
            } else {
                dispatch(
                    Action::RenameRemote {
                        environment_id,
                        name: name.into(),
                    },
                    cx,
                );
                editor.saving = true;
            }
        } else {
            let state = cx.global::<ConnectionsState>();
            editor.errors.extend(
                editor
                    .draft
                    .errors(state.connections.as_deref().unwrap_or_default()),
            );
            if editor.errors.is_empty() {
                dispatch(Action::Save(editor.draft.clone()), cx);
                editor.saving = true;
            }
        }
        cx.notify();
    }

    fn save_aliases(&mut self, cx: &mut Context<Self>) {
        if cx.global::<ConnectionsState>().pending {
            return;
        }
        let Some(dialog) = &mut self.discovery else {
            return;
        };
        if dialog.selected.is_empty() {
            return;
        }
        dialog.saving = true;
        dispatch(
            Action::SaveAliases(dialog.selected.iter().cloned().collect()),
            cx,
        );
        cx.notify();
    }

    fn toggle_alias(&mut self, alias: &str, cx: &mut Context<Self>) {
        if let Some(dialog) = &mut self.discovery {
            if !dialog.selected.remove(alias) {
                dialog.selected.insert(alias.into());
            }
            cx.notify();
        }
    }

    fn toggle_color(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor {
            editor.color_open = !editor.color_open;
            cx.notify();
        }
    }

    fn select_color(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor {
            editor.draft.color = Some((index != 0).then(|| PRESETS[index].1.into()));
            editor.color_open = false;
            editor.fields[0].read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn select_identity(&mut self, identity: bool, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor {
            editor.draft.use_identity = identity;
        }
        cx.notify();
    }

    fn ssh_modal(
        &self,
        title: &'static str,
        height: f32,
        body: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let viewport = window.viewport_size();
        let focus = self
            .editor
            .as_ref()
            .map(|editor| &editor.focus)
            .or_else(|| self.discovery.as_ref().map(|dialog| &dialog.focus))
            .expect("open SSH dialog");
        let frame = div()
            .id("ssh-dialog")
            .relative()
            .w(px(520.))
            .h(px(height.ceil()))
            .max_w(viewport.width * 0.92)
            .max_h(viewport.height - px(32.))
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .track_focus(focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let root = this
                    .editor
                    .as_ref()
                    .map(|editor| &editor.focus)
                    .or_else(|| this.discovery.as_ref().map(|dialog| &dialog.focus));
                if let Some(root) = root {
                    trap_focus(root, event, window, cx);
                }
            }))
            .child(shapes::backdrop_blur(
                px(24.),
                shapes::Fill::Color,
                false,
                shapes::Decoration {
                    radius: Corners::all(px(25.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(Hsla {
                        a: 0.9,
                        ..t.menu_background
                    }),
                    shadows: vec![
                        crate::composer::shadow(t.border, 0., 0., 0.5, false),
                        crate::composer::shadow(gpui::rgba(0x0000001a).into(), 4., 8., -2., false),
                    ],
                    ..Default::default()
                },
            ))
            .child(body)
            .child(
                div()
                    .id("close-ssh-dialog")
                    .absolute()
                    .top(px(16.))
                    .right(px(16.))
                    .size(px(24.))
                    .p(px(4.))
                    .role(gpui::Role::Button)
                    .aria_label("Close")
                    .focusable()
                    .tab_index(0)
                    .child(icon("close", 16., Hsla { a: 0.8, ..t.text }))
                    .on_click(cx.listener(|this, _, window, cx| this.close_ssh(window, cx)))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.close_ssh(window, cx);
                            cx.stop_propagation();
                        }
                    })),
            );
        gpui::deferred(
            gpui::anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(viewport.width)
                    .h(viewport.height)
                    .occlude()
                    .bg(gpui::rgba(0x00000022))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                if let Some(editor) = &mut this.editor
                                    && std::mem::take(&mut editor.color_open)
                                {
                                    cx.notify();
                                } else {
                                    this.close_ssh(window, cx);
                                }
                            }
                            "enter" if this.discovery.is_some() => this.save_aliases(cx),
                            "enter" => this.save_editor(cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_ssh(window, cx)),
                    )
                    .child(frame),
            ),
        )
        .with_priority(10)
        .into_any_element()
    }

    pub(super) fn discovery_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.discovery.as_ref()?;
        let state = cx.global::<ConnectionsState>();
        if !state.aliases_loaded {
            return None;
        }
        let t = cx.theme().clone();
        let list = div()
            .relative()
            .p(px(1.))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(20.)),
                corner: shapes::Corner::Squircle,
                fill: Some(t.background_panel),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(
                div()
                    .id("ssh-alias-list")
                    .max_h(px(298.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(state.aliases.iter().map(|alias| {
                        let selected = dialog.selected.contains(alias);
                        let click_alias = alias.clone();
                        let key_alias = alias.clone();
                        let mut checkbox = div().relative().size(px(14.)).child(
                            shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(5.)),
                                corner: shapes::Corner::Squircle,
                                fill: Some(if selected {
                                    Hsla {
                                        a: 0.96,
                                        ..t.menu_background
                                    }
                                } else {
                                    t.text_alpha_5
                                }),
                                border: Some((px(1.), t.border_strong)),
                                shadows: vec![crate::composer::shadow(
                                    gpui::rgba(0x00000014).into(),
                                    1.,
                                    2.,
                                    -1.,
                                    false,
                                )],
                                ..Default::default()
                            }),
                        );
                        if selected {
                            checkbox = checkbox.child(icon("menu-check", 12., t.text));
                        }
                        div()
                            .id(SharedString::from(format!("ssh-alias-{alias}")))
                            .role(gpui::Role::CheckBox)
                            .aria_label(alias.clone())
                            .aria_toggled(if selected {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .focusable()
                            .tab_index(0)
                            .px(px(16.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(24.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(12.))
                                    .child(icon("laptop", 18., t.text))
                                    .child(text(alias.clone(), 13., 18.571_428, 500., t.text)),
                            )
                            .child(div().w(px(160.)).flex().justify_end().child(checkbox))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_alias(&click_alias, cx)
                            }))
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.toggle_alias(&key_alias, cx);
                                    cx.stop_propagation();
                                }
                            }))
                    })),
            );
        let disabled = dialog.selected.is_empty() || state.pending;
        let add = div()
            .id("add-selected-ssh")
            .h(px(36.))
            .px(px(21.))
            .relative()
            .role(gpui::Role::Button)
            .aria_label("Add")
            .aria_disabled(disabled)
            .focusable()
            .tab_index(0)
            .flex()
            .items_center()
            .justify_center()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(9999.)),
                fill: Some(t.button_primary_solid),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(text("Add", 14., 18., 430., primary_text(&t)))
            .when(disabled, |button| button.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.save_aliases(cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.save_aliases(cx);
                    cx.stop_propagation();
                }
            }));
        let body = div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text("Add SSH Connection", 20., 28., 600., t.text))
            .child(
                div()
                    .pt(px(12.))
                    .h(px(312.))
                    .child(if state.aliases.is_empty() {
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(text(
                                "No SSH connections found",
                                13.,
                                18.571_428,
                                430.,
                                t.text_tertiary,
                            ))
                    } else {
                        list
                    }),
            )
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("refresh-ssh-aliases")
                            .size(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .role(gpui::Role::Button)
                            .aria_label("Refresh")
                            .focusable()
                            .tab_index(0)
                            .child(icon("diff-refresh", 20., t.text_tertiary))
                            .on_click(cx.listener(|_, _, _, cx| dispatch(Action::DiscoverSsh, cx)))
                            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    dispatch(Action::DiscoverSsh, cx);
                                    cx.stop_propagation();
                                }
                            })),
                    )
                    .child(
                        div()
                            .id("add-ssh-manually")
                            .flex_1()
                            .role(gpui::Role::Button)
                            .aria_label("Add manually")
                            .focusable()
                            .tab_index(0)
                            .child(text("Add manually", 13., 18.571_428, 430., t.text))
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_editor(None, window, cx)
                                }),
                            )
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.open_editor(None, window, cx);
                                    cx.stop_propagation();
                                }
                            })),
                    )
                    .child(add),
            );
        Some(self.ssh_modal("Add SSH Connection", 428., body, window, cx))
    }

    fn editor_field(
        &self,
        editor: &Editor,
        index: usize,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let label = match index {
            0 => "Display name",
            1 if editor.draft.alias => "Alias",
            1 => "Hostname",
            2 => "SSH port",
            _ => "Identity file path",
        };
        let mut row = div().flex().flex_col().gap(px(4.));
        if index != 0 || !editor.editing {
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(text(label, 12., 16., 500., t.text))
                    .when(index == 2, |row| {
                        row.child(text(
                            "(optional)",
                            12.,
                            16.,
                            400.,
                            Hsla { a: 0.65, ..t.text },
                        ))
                    }),
            );
        }
        if editor.details {
            return row.child(text(
                editor.fields[index].read(cx).field_text().to_owned(),
                13.,
                20.,
                430.,
                t.text,
            ));
        }
        let name_field = index == 0;
        let focused = editor.fields[index]
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let mut input = div()
            .relative()
            .h(px(if name_field { 40. } else { 36. }))
            .flex()
            .items_center()
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(if name_field { 15. } else { 10. })),
                corner: shapes::Corner::Squircle,
                fill: Some(Hsla {
                    a: 0.96,
                    ..t.menu_background
                }),
                border: Some((
                    px(1.),
                    if focused {
                        t.focus_ring
                    } else if name_field {
                        t.border
                    } else {
                        t.border_strong
                    },
                )),
                ..Default::default()
            }));
        if name_field {
            let state = cx.global::<ConnectionsState>();
            let color = editor.draft.color.as_ref().unwrap_or(&editor.initial_color);
            let color = resolved_color(
                color.as_deref(),
                editor.draft.host_id.as_deref().unwrap_or_default(),
                state,
                t,
            );
            input = input.child(
                div()
                    .id("ssh-connection-color")
                    .w(px(40.))
                    .h(px(38.))
                    .ml(px(1.))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .aria_label("Change connection color")
                    .focusable()
                    .tab_index(0)
                    .when(editor.color_open, |button| button.bg(t.text_alpha_5))
                    .child(div().absolute().right_0().w(px(1.)).h_full().bg(t.border))
                    .child(icon("settings-connections", 16., color))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .children(self.color_palette(cx)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_color(cx)))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.toggle_color(cx);
                            cx.stop_propagation();
                        }
                    })),
            );
        }
        input = input.child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(if name_field { 8. } else { 11. }))
                .child(editor.fields[index].clone()),
        );
        row.child(input)
    }

    pub(super) fn editor(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        let t = cx.theme().clone();
        let title = if editor.details {
            "Details"
        } else if editor.editing {
            "Edit connection"
        } else {
            "Add SSH connection"
        };
        let mut fields = div()
            .pt(px(12.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(self.editor_field(editor, 0, &t, window, cx));
        if editor.remote_environment.is_none() {
            fields = fields.child(self.editor_field(editor, 1, &t, window, cx));
            if !editor.draft.alias {
                fields = fields.child(self.editor_field(editor, 2, &t, window, cx));
                if !editor.details {
                    fields = fields.child(div().flex().gap(px(2.)).children(
                        [(false, "No Auth"), (true, "Identity")].into_iter().map(
                            |(identity, label)| {
                                let selected = editor.draft.use_identity == identity;
                                div()
                                    .id(label)
                                    .flex_1()
                                    .h(px(24.))
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .role(gpui::Role::Tab)
                                    .aria_label(label)
                                    .aria_selected(selected)
                                    .focusable()
                                    .tab_index(0)
                                    .when(selected, |button| {
                                        button.child(shapes::decoration(shapes::Decoration {
                                            radius: Corners::all(px(9999.)),
                                            fill: Some(t.text_alpha_5),
                                            ..Default::default()
                                        }))
                                    })
                                    .child(text(
                                        label,
                                        13.,
                                        18.,
                                        430.,
                                        if selected { t.text } else { t.text_tertiary },
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_identity(identity, cx)
                                    }))
                                    .on_key_down(cx.listener(
                                        move |this, event: &KeyDownEvent, _, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                this.select_identity(identity, cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    ))
                            },
                        ),
                    ));
                }
                if editor.draft.use_identity {
                    fields = fields.child(self.editor_field(editor, 3, &t, window, cx));
                }
            }
        }
        if let Some(version) = &editor.version {
            fields = fields.child(text(format!("Version: {version}"), 13., 20., 430., t.text));
        }
        let mut body = div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text(title, 20., 28., 600., t.text));
        if !editor.errors.is_empty() {
            body =
                body.child(
                    div().pt(px(12.)).child(
                        div()
                            .relative()
                            .p(px(9.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(10.)),
                                corner: shapes::Corner::Squircle,
                                border: Some((px(1.), t.text_error)),
                                ..Default::default()
                            }))
                            .child(
                                div().flex().flex_col().children(editor.errors.iter().map(
                                    |error| text(*error, 13., 18.571_428, 430., t.text_error),
                                )),
                            ),
                    ),
                );
        }
        body = body.child(fields).child(
            div()
                .pt(px(16.))
                .flex()
                .justify_end()
                .gap(px(12.))
                .children(
                    [
                        (false, if editor.details { "Close" } else { "Cancel" }),
                        (true, "Save"),
                    ]
                    .into_iter()
                    .filter(|(save, _)| !*save || !editor.details)
                    .map(|(save, label)| {
                        div()
                            .id(label)
                            .h(px(32.))
                            .px(px(17.))
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .role(gpui::Role::Button)
                            .aria_label(label)
                            .focusable()
                            .tab_index(0)
                            .when(save, |button| {
                                button.child(shapes::decoration(shapes::Decoration {
                                    radius: Corners::all(px(12.5)),
                                    corner: shapes::Corner::Squircle,
                                    fill: Some(t.button_primary_solid),
                                    border: Some((px(1.), t.border)),
                                    ..Default::default()
                                }))
                            })
                            .child(text(
                                label,
                                14.,
                                18.,
                                430.,
                                if save {
                                    primary_text(&t)
                                } else {
                                    t.text_tertiary
                                },
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if save {
                                    this.save_editor(cx);
                                } else {
                                    this.close_ssh(window, cx);
                                }
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        if save {
                                            this.save_editor(cx);
                                        } else {
                                            this.close_ssh(window, cx);
                                        }
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                    }),
                ),
        );
        let height = 372.
            + if editor.draft.use_identity { 72. } else { 0. }
            + if editor.errors.is_empty() {
                0.
            } else {
                30. + 18.571_428 * editor.errors.len() as f32
            };
        let modal = self.ssh_modal(title, height, body, window, cx);
        Some(modal)
    }

    fn color_palette(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        if !editor.color_open {
            return None;
        }
        let t = cx.theme().clone();
        let state = cx.global::<ConnectionsState>();
        let selected = editor.draft.color.as_ref().unwrap_or(&editor.initial_color);
        let default = automatic_color(
            editor.draft.host_id.as_deref().unwrap_or_default(),
            state.connections.as_deref().unwrap_or_default(),
        );
        let colors = PRESETS
            .iter()
            .enumerate()
            .map(|(index, (name, light, dark))| {
                let selected = if index == 0 {
                    selected.is_none()
                } else {
                    selected.as_deref() == Some(*light)
                };
                let color = if index == 0 {
                    default
                } else {
                    parse_hex(if t.appearance == Appearance::Dark {
                        dark
                    } else {
                        light
                    })
                    .expect("preset color")
                };
                div()
                    .id(SharedString::from(format!("ssh-color-{name}")))
                    .size(px(32.))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .aria_label(format!("Use {name}"))
                    .aria_selected(selected)
                    .focusable()
                    .tab_index(0)
                    .when(selected, |button| {
                        button.child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(px(9999.)),
                            fill: Some(t.text_alpha_5),
                            ..Default::default()
                        }))
                    })
                    .child(div().size(px(24.)).rounded_full().bg(gpui::rgb(color)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_color(index, window, cx);
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.select_color(index, window, cx);
                            cx.stop_propagation();
                        }
                    }))
            });
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(44.)))
                    .child(
                        div()
                            .w(px(292.))
                            .p(px(4.))
                            .relative()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .occlude()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(15.)),
                                corner: shapes::Corner::Squircle,
                                fill: Some(t.menu_background),
                                shadows: vec![
                                    crate::composer::shadow(t.border, 0., 0., 0.5, false),
                                    crate::composer::shadow(
                                        gpui::rgba(0x0000001a).into(),
                                        4.,
                                        8.,
                                        -2.,
                                        false,
                                    ),
                                ],
                                ..Default::default()
                            }))
                            .child(div().px(px(8.)).py(px(4.)).child(text(
                                "Connection color",
                                13.,
                                18.571_428,
                                430.,
                                t.text,
                            )))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .id("ssh-color-group")
                                    .role(gpui::Role::Group)
                                    .aria_label("Connection colors")
                                    .children(colors),
                            ),
                    ),
            )
            .with_priority(11)
            .into_any_element(),
        )
    }
}

fn primary_text(t: &Theme) -> Hsla {
    if t.appearance == Appearance::Dark {
        gpui::rgb(0x2d2d2d).into()
    } else {
        t.text_inverse
    }
}

const PRESETS: [(&str, &str, &str); 8] = [
    ("Default", "", ""),
    ("Red", "#fa423e", "#ff6764"),
    ("Orange", "#fb6a22", "#ff8549"),
    ("Yellow", "#ffc300", "#ffd240"),
    ("Green", "#04b84c", "#40c977"),
    ("Blue", "#0285ff", "#339cff"),
    ("Purple", "#924ff7", "#ad7bf9"),
    ("Pink", "#ff66ad", "#ff8cc1"),
];

fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

fn resolved_color(value: Option<&str>, host_id: &str, state: &ConnectionsState, t: &Theme) -> Hsla {
    let preset = value.and_then(|value| PRESETS.iter().find(|(_, light, _)| *light == value));
    let color = preset
        .map(|(_, light, dark)| {
            if t.appearance == Appearance::Dark {
                *dark
            } else {
                *light
            }
        })
        .or(value)
        .and_then(parse_hex)
        .unwrap_or_else(|| {
            automatic_color(host_id, state.connections.as_deref().unwrap_or_default())
        });
    gpui::rgb(color).into()
}

fn automatic_color(host_id: &str, connections: &[Connection]) -> u32 {
    let index = connections
        .iter()
        .filter(|connection| connection.host_id.as_str() < host_id)
        .count();
    let count = connections.len()
        + usize::from(
            !connections
                .iter()
                .any(|connection| connection.host_id == host_id),
        );
    let position = (index as f64 + 0.5) * 215. / count.max(1) as f64;
    let hue = if position <= 50. {
        45. + position
    } else {
        165. + position - 50.
    };
    let chroma = if count <= 1 {
        0.135
    } else {
        0.09 + 0.09 * index as f64 / (count - 1) as f64
    };
    let angle = hue.to_radians();
    let (a, b) = (chroma * angle.cos(), chroma * angle.sin());
    let l = (0.74 + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (0.74 - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (0.74 - 0.0894841775 * a - 1.291485548 * b).powi(3);
    let channel = |value: f64| {
        let encoded = if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1. / 2.4) - 0.055
        };
        (encoded.clamp(0., 1.) * 255.).round() as u32
    };
    (channel(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s) << 16)
        | (channel(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s) << 8)
        | channel(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s)
}

fn trap_focus(root: &FocusHandle, event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    if event.keystroke.key != "tab" {
        return;
    }
    if event.keystroke.modifiers.shift {
        window.focus_prev(cx);
        if !root.contains_focused(window, cx) {
            root.focus(window, cx);
            let mut last = root.clone();
            loop {
                window.focus_next(cx);
                if root.is_focused(window) || !root.contains_focused(window, cx) {
                    break;
                }
                if let Some(focused) = window.focused(cx) {
                    last = focused;
                } else {
                    break;
                }
            }
            last.focus(window, cx);
        }
    } else {
        window.focus_next(cx);
        if !root.contains_focused(window, cx) {
            root.focus(window, cx);
            window.focus_next(cx);
            if !root.contains_focused(window, cx) {
                root.focus(window, cx);
            }
        }
    }
    cx.stop_propagation();
}

#[cfg(test)]
mod tests {
    use super::super::Tab;
    use super::*;
    use std::sync::Arc;

    #[test]
    fn invalid_ssh_form_stays_open_without_saving() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        use gpui::{HeadlessAppContext, size};
        let mut cx = if std::env::var_os("CGX_CONNECTIONS_SHOTS").is_some() {
            HeadlessAppContext::with_platform(
                textsys::text_system(),
                Arc::new(theme::Assets),
                gpui_platform::current_headless_renderer,
            )
        } else {
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets))
        };
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            cx.set_global(ConnectionsState {
                connections: Some(Vec::new()),
                ..Default::default()
            });
        });
        let page = cx.update(|cx| {
            cx.new(|_| ConnectionsPage {
                tab: Tab::Ssh,
                editor: None,
                discovery: None,
                menu: None,
                pairing: None,
                layer: Default::default(),
            })
        });
        let window = cx
            .open_window(size(px(840.), px(650.)), |_, _| page.clone())
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                page.open_editor(None, window, cx);
                let fields = page.editor.as_ref().unwrap().fields.clone();
                for (field, value) in fields.iter().zip(["Build", "example.invalid", "65536", ""]) {
                    field.update(cx, |field, cx| field.set_text(value, cx));
                }
                page.save_editor(cx);
                assert_eq!(
                    page.editor.as_ref().unwrap().errors,
                    ["SSH port must be between 1 and 65535"]
                );
            })
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            cx.update_global::<ConnectionsState, _>(|state, _| {
                state.dispatch = Some(Arc::new(|_, cx| {
                    cx.update_global::<ConnectionsState, _>(|state, _| {
                        state.error = Some("Disk is full".into());
                        state.pending = false;
                    });
                }));
            });
            page.update(cx, |page, cx| {
                let fields = page.editor.as_ref().unwrap().fields.clone();
                fields[2].update(cx, |field, cx| field.set_text("22", cx));
                page.save_editor(cx);
                assert_eq!(
                    page.editor.as_ref().unwrap().draft.hostname,
                    "example.invalid"
                );
                assert_eq!(
                    cx.global::<ConnectionsState>().error.as_deref(),
                    Some("Disk is full")
                );
            });
        });
        cx.run_until_parked();
        cx.update(|cx| assert!(page.read(cx).editor.is_some()));
        if let Some(path) = std::env::var_os("CGX_CONNECTIONS_SHOTS") {
            std::fs::create_dir_all(&path).unwrap();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(std::path::PathBuf::from(path).join("ssh-validation.png"))
                .unwrap();
        }
    }
    #[test]
    fn single_host_color_matches_original() {
        assert_eq!(
            super::automatic_color("remote-ssh-codex-managed:test", &[]),
            0x12bde8
        );
    }
}
