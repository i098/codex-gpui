//! `File > New Project…`: the original's `create-project-dialog` (`projectSetup.createLocalProject`)
//! with a name and marker, ordered source folders whose first entry is primary, and a device
//! picker. The app owns folder picking, creation and the Connections remote flow.

use crate::{
    composer::{Composer, Dismiss, Submit},
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Corners, Div, Entity, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    actions, div, point, prelude::FluentBuilder as _, px, rgb,
};
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Appearance, Theme};

actions!(new_project, [NewProject]);

/// `PROJECT_APPEARANCES` marker icons, in the original's schema order.
pub const MARKERS: [&str; 30] = [
    "folder",
    "currency-dollar",
    "book",
    "graduation-cap",
    "edit",
    "writing",
    "function",
    "terminal",
    "music",
    "popcorn",
    "customize",
    "palette",
    "stethoscope",
    "health",
    "lotus",
    "suitcase",
    "bar-chart",
    "kettlebell",
    "dumbbell",
    "logs",
    "scale",
    "desk-globe",
    "plane",
    "globe",
    "wrench",
    "paw",
    "flask",
    "brain",
    "heart",
    "plant",
];

/// Marker colors with their light and dark accents (`UUr`); `black` follows the icon color.
pub const COLORS: [(&str, u32, u32); 8] = [
    ("black", 0, 0),
    ("blue", 0x0285ff, 0x339cff),
    ("green", 0x04b84c, 0x40c977),
    ("orange", 0xfb6a22, 0xff8549),
    ("pink", 0xff66ad, 0xff8cc1),
    ("purple", 0x924ff7, 0xad7bf9),
    ("red", 0xfa423e, 0xff6764),
    ("yellow", 0xffc300, 0xffd240),
];

pub enum Outcome {
    Dismiss,
    PickFolders,
    AddRemote,
    Create {
        name: String,
        sources: Vec<String>,
        appearance: Option<Value>,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Popover {
    Marker,
    Device,
}

pub struct Dialog {
    name: Entity<Composer>,
    sources: Vec<String>,
    /// `(color, marker)` indices into [`COLORS`] and [`MARKERS`]; `None` keeps the folder default.
    appearance: Option<(usize, usize)>,
    popover: Option<Popover>,
    remotes: Vec<SharedString>,
    saving: bool,
    /// `Edit project` (`projectSetup.editLocalProject`) rather than `Create project`.
    editing: bool,
    focus: FocusHandle,
}

impl EventEmitter<Outcome> for Dialog {}

impl Focusable for Dialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.focus_handle(cx)
    }
}

impl Dialog {
    /// `remotes`: display names of the connected remote hosts the device menu lists.
    pub fn new(remotes: Vec<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::with_name(remotes, "", cx)
    }

    /// `Edit project` over an existing project's name and roots, primary first.
    pub fn edit(name: &str, sources: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self {
            sources,
            editing: true,
            ..Self::with_name(Vec::new(), name, cx)
        }
    }

    fn with_name(remotes: Vec<SharedString>, name: &str, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| Composer::dialog_field("Name", "Project name", name, cx));
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        Self {
            name,
            sources: Vec::new(),
            appearance: None,
            popover: None,
            remotes,
            saving: false,
            editing: false,
            focus: cx.focus_handle(),
        }
    }

    /// Appends picked folders after the existing ones, skipping duplicates (`jZr`).
    pub fn add_sources(&mut self, paths: impl IntoIterator<Item = String>, cx: &mut Context<Self>) {
        for path in paths {
            if !self.sources.contains(&path) {
                self.sources.push(path);
            }
        }
        cx.notify();
    }

    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    pub fn make_primary(&mut self, index: usize, cx: &mut Context<Self>) {
        let source = self.sources.remove(index);
        self.sources.insert(0, source);
        cx.notify();
    }

    pub fn remove_source(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sources.remove(index);
        cx.notify();
    }

    /// Creation failed or was declined: the form stays open with its input.
    pub fn finish(&mut self, cx: &mut Context<Self>) {
        self.saving = false;
        cx.notify();
    }

    pub fn appearance(&self) -> Option<Value> {
        self.appearance.map(|(color, marker)| {
            json!({"color": COLORS[color].0, "marker": {"kind": "icon", "icon": MARKERS[marker]}})
        })
    }

    fn title(&self) -> &'static str {
        if self.editing {
            "Edit project"
        } else {
            "Create project"
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.popover = None;
        cx.emit(Outcome::Create {
            name: self.name.read(cx).field_text().trim().to_owned(),
            sources: self.sources.clone(),
            appearance: self.appearance(),
        });
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.saving {
            cx.emit(Outcome::Dismiss);
        }
    }

    fn toggle(&mut self, popover: Popover, cx: &mut Context<Self>) {
        self.popover = (self.popover != Some(popover)).then_some(popover);
        cx.notify();
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        self.popover = None;
        cx.emit(Outcome::PickFolders);
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" if self.popover.is_some() => {
                self.popover = None;
                cx.notify();
            }
            "escape" => self.dismiss(cx),
            _ => return crate::focus::tab(event, window, cx),
        }
        cx.stop_propagation();
    }

    fn color(&self, t: &Theme) -> Hsla {
        match self.appearance {
            None => t.text_description,
            Some((0, _)) => t.text_secondary,
            Some((color, _)) => {
                let (_, light, dark) = COLORS[color];
                rgb(if t.appearance == Appearance::Dark {
                    dark
                } else {
                    light
                })
                .into()
            }
        }
    }

    fn name_input(&self, window: &Window, t: &Theme, cx: &mut Context<Self>) -> Div {
        let focused = self.name.focus_handle(cx).is_focused(window);
        let marker = self.appearance.map_or("folder".to_owned(), |(_, marker)| {
            format!("project-marker-{}", MARKERS[marker])
        });
        div()
            .relative()
            .h(px(40.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pr(px(12.))
            .child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.)),
                fill: Some(t.primary_soft_alpha),
                border: Some((px(1.), if focused { t.focus_ring } else { t.border })),
                ..Default::default()
            }))
            .child(
                div()
                    .id("project-marker")
                    .relative()
                    .w(px(40.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .aria_label("Change marker for project")
                    .focusable()
                    .tab_index(0)
                    .hover(|s| s.bg(t.ghost_hover))
                    .when(self.popover == Some(Popover::Marker), |d| {
                        d.bg(t.ghost_hover)
                    })
                    .child(div().absolute().right_0().w(px(1.)).h_full().bg(t.border))
                    .child(icon(&marker, 16., self.color(t)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .children(self.marker_popover(t, cx)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(Popover::Marker, cx)))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.toggle(Popover::Marker, cx);
                            cx.stop_propagation();
                        }
                    })),
            )
            .child(div().flex_1().min_w_0().child(self.name.clone()))
    }

    // shortcut: the original's emoji/icon SymbolBrowser chunk is not in the capture; this grid
    // offers its 30 schema icons and 8 colors. Upgrade when the browser chunk is recorded.
    fn marker_popover(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.popover != Some(Popover::Marker) {
            return None;
        }
        let (selected_color, selected_marker) = self.appearance.unwrap_or((0, 0));
        let color = self.color(t);
        let icons = MARKERS.iter().enumerate().map(|(index, name)| {
            let selected = self.appearance.is_some() && selected_marker == index;
            div()
                .id(("project-marker-icon", index))
                .size(px(32.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .justify_center()
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(*name))
                .aria_selected(selected)
                .hover(|s| s.bg(t.ghost_hover))
                .when(selected, |d| d.bg(t.text_alpha_10))
                .child(icon(&format!("project-marker-{name}"), 16., color))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.appearance = Some((selected_color, index));
                    this.popover = None;
                    cx.notify();
                }))
        });
        let swatches = COLORS
            .iter()
            .enumerate()
            .map(|(index, (name, light, dark))| {
                let fill: Hsla = if index == 0 {
                    t.text_secondary
                } else {
                    rgb(if t.appearance == Appearance::Dark {
                        *dark
                    } else {
                        *light
                    })
                    .into()
                };
                div()
                    .id(("project-marker-color", index))
                    .size(px(24.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(*name))
                    .aria_selected(self.appearance.is_some() && selected_color == index)
                    .when(self.appearance.is_some() && selected_color == index, |d| {
                        d.border_2().border_color(t.focus_ring)
                    })
                    .child(div().size(px(16.)).rounded_full().bg(fill))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.appearance = Some((index, selected_marker));
                        cx.notify();
                    }))
            });
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(44.)))
                    .child(
                        div()
                            .id("project-marker-popover")
                            .w(px(336.))
                            .p(px(8.))
                            .relative()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(popover_surface(t))
                            .child(div().flex().flex_wrap().children(icons))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .children(swatches)
                                    .child(div().flex_1())
                                    .when(self.appearance.is_some(), |row| {
                                        row.child(
                                            div()
                                                .id("project-marker-clear")
                                                .px(px(8.))
                                                .rounded(px(6.))
                                                .role(gpui::Role::Button)
                                                .hover(|s| s.bg(t.ghost_hover))
                                                .child(text("Clear", 13., 20., 430., t.text))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.appearance = None;
                                                    this.popover = None;
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                    ),
            )
            .with_priority(11)
            .into_any_element(),
        )
    }

    /// `Add a folder on this computer ⌄`: the `ProjectDevicePicker` trigger and its menu.
    fn device_picker(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let menu = (self.popover == Some(Popover::Device)).then(|| {
            let item = |id: &'static str, glyph: &'static str, label: SharedString, fg: Hsla| {
                div()
                    .id(id)
                    .h(px(32.))
                    .px(px(8.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(glyph, 16., fg))
                    .child(text(label, 13., 20., 430., fg).flex_1())
            };
            let mut list = div()
                .id("project-device-menu")
                .w(px(240.))
                .p(px(4.))
                .relative()
                .flex()
                .flex_col()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(popover_surface(t))
                .child(
                    item(
                        "project-device-local",
                        "laptop",
                        "This computer".into(),
                        t.text,
                    )
                    .role(gpui::Role::MenuItem)
                    .hover(|s| s.bg(t.menu_item_hover))
                    .child(icon("menu-check", 16., t.text))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.popover = None;
                        cx.notify();
                    })),
                )
                .child(div().px(px(8.)).pt(px(8.)).pb(px(4.)).child(text(
                    "Remote devices",
                    12.,
                    16.,
                    430.,
                    t.text_tertiary,
                )));
            if self.remotes.is_empty() {
                list = list.child(item(
                    "project-device-none",
                    "settings-connections",
                    "No connected remotes".into(),
                    t.text_disabled,
                ));
            }
            // shortcut: remote projects have no native store yet, so connected remotes are listed
            // but not selectable. Upgrade when the Connections owner adds remote projects.
            for (index, remote) in self.remotes.iter().enumerate() {
                list = list.child(
                    item(
                        "project-device-remote",
                        "settings-connections",
                        remote.clone(),
                        t.text_disabled,
                    )
                    .id(("project-device-remote", index))
                    .aria_disabled(true),
                );
            }
            list = list.child(
                item(
                    "project-device-add-remote",
                    "plus",
                    "Add remote".into(),
                    t.text,
                )
                .role(gpui::Role::MenuItem)
                .hover(|s| s.bg(t.menu_item_hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.popover = None;
                    cx.emit(Outcome::AddRemote);
                })),
            );
            gpui::deferred(
                gpui::anchored()
                    .position_mode(gpui::AnchoredPositionMode::Local)
                    .position(point(px(0.), px(24.)))
                    .child(list),
            )
            .with_priority(11)
        });
        div().relative().child(
            div()
                .id("project-device")
                .flex()
                .items_center()
                .gap(px(4.))
                .role(gpui::Role::Button)
                .aria_label("Project device")
                .focusable()
                .tab_index(0)
                .child(text("Add a folder on", 13., 20., 430., t.text_secondary))
                .child(text("this computer", 13., 20., 430., t.text))
                .child(icon("chevron-down-small", 16., t.text))
                .on_click(cx.listener(|this, _, _, cx| this.toggle(Popover::Device, cx)))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.toggle(Popover::Device, cx);
                        cx.stop_propagation();
                    }
                }))
                .children(menu),
        )
    }

    fn sources_box(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let frame =
            div()
                .relative()
                .flex()
                .flex_col()
                .child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(8.)),
                    fill: Some(t.primary_soft_alpha),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }));
        if self.sources.is_empty() {
            return frame.child(
                div()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(self.device_picker(t, cx))
                    .child(
                        div()
                            .id("project-add-source")
                            .h(px(24.))
                            .px(px(8.))
                            .rounded_full()
                            .bg(t.text_alpha_5)
                            .hover(|s| s.bg(t.text_alpha_10))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .role(gpui::Role::Button)
                            .aria_label("Add")
                            .focusable()
                            .tab_index(0)
                            .child(icon("folder-add", 16., t.text))
                            .child(text("Add", 13., 18., 430., t.text))
                            .on_click(cx.listener(|this, _, _, cx| this.pick(cx)))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.pick(cx);
                                    cx.stop_propagation();
                                }
                            })),
                    ),
            );
        }
        let several = self.sources.len() > 1;
        let rows = self.sources.iter().enumerate().map(|(index, source)| {
            let name = std::path::Path::new(source)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(source)
                .to_owned();
            div()
                .id(("project-source", index))
                .group("project-source")
                .h(px(48.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .when(index > 0, |row| row.border_t_1().border_color(t.border))
                .child(icon("folder", 16., t.text_description))
                .child(
                    text(name, 13., 20., 430., t.text)
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis(),
                )
                .when(index == 0 && several, |row| {
                    row.child(
                        div()
                            .h(px(28.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.border)
                            .flex()
                            .items_center()
                            .child(text("Primary", 13., 18., 500., t.text_secondary)),
                    )
                })
                .when(index > 0, |row| {
                    row.child(
                        div()
                            .id(("project-source-primary", index))
                            .invisible()
                            .group_hover("project-source", |s| s.visible())
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(8.))
                            .bg(t.text_alpha_5)
                            .flex()
                            .items_center()
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(format!("Make {source} primary")))
                            .child(text("Make primary", 13., 18., 430., t.text))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.make_primary(index, cx)),
                            ),
                    )
                })
                .child(
                    div()
                        .id(("project-source-remove", index))
                        .size(px(24.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(t.ghost_hover))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(format!("Remove {source}")))
                        .child(icon("close", 16., t.text_tertiary))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_source(index, cx))),
                )
        });
        frame.children(rows).child(
            div()
                .id("project-add-folder")
                .h(px(48.))
                .px(px(12.))
                .border_t_1()
                .border_color(t.border)
                .flex()
                .items_center()
                .gap(px(8.))
                .hover(|s| s.bg(t.ghost_hover))
                .role(gpui::Role::Button)
                .aria_label("Add folder")
                .child(icon("folder-add", 16., t.text_description))
                .child(text("Add folder", 13., 20., 430., t.text))
                .on_click(cx.listener(|this, _, _, cx| this.pick(cx))),
        )
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        primary: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .h(px(32.))
            .px(px(17.))
            .relative()
            .flex()
            .items_center()
            .role(gpui::Role::Button)
            .aria_label(label)
            .aria_disabled(self.saving)
            .focusable()
            .tab_index(0)
            .when(primary, |button| {
                button.child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.button_primary_solid),
                    border: Some((px(1.), t.border)),
                    ..Default::default()
                }))
            })
            .when(!primary, |button| {
                button.rounded(px(12.5)).hover(|s| s.bg(t.ghost_hover))
            })
            .when(self.saving, |button| button.opacity(0.4))
            .child(text(
                label,
                14.,
                18.,
                430.,
                if primary {
                    t.text_inverse
                } else {
                    t.text_tertiary
                },
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.press(primary, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.press(primary, cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn press(&mut self, primary: bool, cx: &mut Context<Self>) {
        if primary {
            self.submit(cx)
        } else {
            self.dismiss(cx)
        }
    }
}

fn popover_surface(t: &Theme) -> impl IntoElement {
    shapes::decoration(shapes::Decoration {
        radius: Corners::all(px(15.)),
        corner: shapes::Corner::Squircle,
        fill: Some(t.menu_background),
        shadows: vec![
            crate::composer::shadow(t.border, 0., 0., 0.5, false),
            crate::composer::shadow(gpui::rgba(0x0000001a).into(), 4., 8., -2., false),
        ],
        ..Default::default()
    })
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let viewport = window.viewport_size();
        let label_row = div()
            .flex()
            .items_center()
            .justify_between()
            .child(text("Source folders", 13., 20., 500., t.text))
            .when(!self.sources.is_empty(), |row| {
                row.child(self.device_picker(&t, cx))
            });
        let body = div()
            .p(px(20.))
            .flex()
            .flex_col()
            .child(text(self.title(), 20., 28., 600., t.text))
            .child(div().pt(px(12.)).child(self.name_input(window, &t, cx)))
            .child(
                div()
                    .pt(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label_row)
                    .child(self.sources_box(&t, cx)),
            )
            .child(
                div()
                    .pt(px(32.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(self.button("new-project-cancel", "Cancel", false, &t, cx))
                    .child(self.button(
                        "new-project-create",
                        if self.editing {
                            "Save"
                        } else {
                            "Create project"
                        },
                        true,
                        &t,
                        cx,
                    )),
            );
        let frame = div()
            .id("new-project-dialog")
            .relative()
            .w(px(520.))
            .max_w(viewport.width * 0.92)
            .role(gpui::Role::Dialog)
            .aria_label(self.title())
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.popover.take().is_some() {
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(Self::key))
            // The name field's Enter binding runs before key listeners, so submit from its action.
            .capture_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.name.focus_handle(cx).contains_focused(window, cx) {
                    this.submit(cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.popover.take().is_some() {
                    cx.notify();
                } else {
                    this.dismiss(cx);
                }
            }))
            .child(shapes::backdrop_blur(
                px(24.),
                shapes::Fill::Color,
                false,
                shapes::Decoration {
                    radius: Corners::all(px(24.)),
                    fill: Some(Hsla {
                        a: 0.9,
                        ..t.surface_elevated_secondary
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
                    .id("new-project-close")
                    .absolute()
                    .top(px(16.))
                    .right(px(16.))
                    .size(px(24.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(t.ghost_hover))
                    .role(gpui::Role::Button)
                    .aria_label("Close dialog")
                    .child(icon("close", 16., Hsla { a: 0.8, ..t.text }))
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(gpui::rgba(0x00000022))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.dismiss(cx)),
            )
            .child(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn sources_keep_order_and_primary_moves_first(cx: &mut gpui::TestAppContext) {
        let view = cx.new(|cx| Dialog::new(Vec::new(), cx));
        view.update(cx, |view, cx| {
            view.add_sources(["/a".into(), "/b".into(), "/a".into(), "/c".into()], cx);
            assert_eq!(view.sources(), ["/a", "/b", "/c"]);
            view.make_primary(2, cx);
            assert_eq!(view.sources(), ["/c", "/a", "/b"]);
            view.remove_source(1, cx);
            assert_eq!(view.sources(), ["/c", "/b"]);
            assert_eq!(view.appearance(), None);
            view.appearance = Some((1, 27));
            assert_eq!(
                view.appearance(),
                Some(json!({"color": "blue", "marker": {"kind": "icon", "icon": "brain"}}))
            );
        });
    }
}
