//! Settings ▸ Mini & Pets (`pets-settings-route-*.js`, the `settings.mini.*` variant): the
//! Show/Hide mini toggle, the pet preview with its Customize (pet size) dialog, and the
//! My pets grid of None, the custom pets and the bundled pets. Host keys, as the original
//! names them: `selected-avatar-id`, `avatar-overlay-pet-visible` and
//! `avatar-overlay-mascot-width-px`. The app loads custom pets and runs Create pet.

use super::*;
use crate::screens::mini::{self, CustomPet, CustomPets, MiniOpen};
use gpui::{Bounds, MouseMoveEvent, Pixels, canvas};
use std::{cell::Cell, rc::Rc};

gpui::actions!(settings_pets, [RefreshPets, CreatePet]);

const SELECTED: &str = "host.selected-avatar-id";
const VISIBLE: &str = "host.avatar-overlay-pet-visible";
const WIDTH: &str = "host.avatar-overlay-mascot-width-px";
/// `tr`: the preview's pet area.
const AREA: f32 = 384.;
/// `w-40`: the pet size slider's track.
const TRACK: f32 = 160.;
const THUMB: f32 = 20.;

#[derive(Default)]
pub(super) struct State {
    customize: bool,
    /// The slider's width while it is dragged; written on release.
    drag: Option<f32>,
    track: Rc<Cell<Bounds<Pixels>>>,
    /// The Customize dialog's focus, so Escape reaches its key handler.
    focus: Option<FocusHandle>,
}

/// A grid entry: id, name and description.
struct Pet {
    id: SharedString,
    name: SharedString,
    description: Option<SharedString>,
}

impl SettingsPane {
    pub(super) fn render_pets(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = self.page.read(cx);
        let visible = page.flag(VISIBLE, true);
        let stored = mini::width(page.get(WIDTH).as_f64());
        let chosen = page.get(SELECTED).as_str().unwrap_or("codex").to_owned();
        let empty = CustomPets::default();
        let custom = cx.try_global::<CustomPets>().unwrap_or(&empty);
        let pets: Vec<Pet> = custom
            .pets
            .iter()
            .map(|pet: &CustomPet| Pet {
                id: pet.id.clone().into(),
                name: pet.name.clone().into(),
                description: pet.description.clone().map(Into::into),
            })
            .chain(mini::BUNDLED.iter().map(|(id, name, description)| Pet {
                id: (*id).into(),
                name: (*name).into(),
                description: Some((*description).into()),
            }))
            .collect();
        // `uWs`: the stored id when it is listed, else Codex.
        let selected: SharedString = if pets.iter().any(|pet| pet.id == chosen.as_str()) {
            chosen.into()
        } else {
            "codex".into()
        };
        let (loading, failed, creating) = (custom.loading, custom.failed, custom.creating);
        let custom_pets = custom.pets.clone();
        let open = cx.try_global::<MiniOpen>().is_some_and(|open| open.0);
        let width = self.pets.drag.unwrap_or(stored);

        let header = div()
            .min_h(px(60.))
            .py(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text("Mini & Pets", 28., 33.6, 500., t.text))
                    .child(subtitle(t, cx)),
            )
            .child(
                button(
                    if open { "Hide mini" } else { "Show mini" },
                    t.button_secondary,
                    None,
                    t,
                )
                .id("pets-toggle-mini")
                .role(gpui::Role::Button)
                .flex_none()
                .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::OpenMini), cx)),
            );

        let mut content = Vec::new();
        if visible {
            content.push(
                self.preview(&selected, &custom_pets, stored, width, t, window, cx)
                    .into_any_element(),
            );
        }
        content.push(
            self.my_pets(
                &pets,
                &selected,
                visible,
                &custom_pets,
                loading,
                failed,
                creating,
                t,
                window,
                cx,
            )
            .into_any_element(),
        );

        let dialog = self.pets.customize.then(|| {
            self.customize_dialog(stored, width, t, window, cx)
                .into_any_element()
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("settings-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div().pt(px(16.)).pb(px(1.)).child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .mx_auto()
                                .px(px(20.))
                                .child(header),
                        ),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(40.))
                            .children(content),
                    ),
            )
            .children(dialog)
            .into_any_element()
    }

    /// `Qn`: the chosen pet over the Mini bar, and `Customize`. The area grows to the
    /// largest pet while the dialog is open, so resizing does not move the card.
    /// `stored` sizes the sprite decode, `width` its box: a drag scales the stored frames.
    #[allow(clippy::too_many_arguments)]
    fn preview(
        &self,
        selected: &str,
        custom: &[CustomPet],
        stored: f32,
        width: f32,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let figure = mini::pet_size(width);
        let tallest = if self.pets.customize {
            mini::pet_size(mini::MAX_WIDTH).height
        } else {
            figure.height
        };
        let area = tallest + mini::BAR_SIZE.height;
        let customize = div()
            .id("pets-customize")
            .debug_selector(|| "pets-customize".into())
            .role(gpui::Role::Button)
            .absolute()
            .right(px(16.))
            .bottom(px(16.))
            .h(px(32.))
            .px(px(16.))
            .flex()
            .items_center()
            .child(on_card(
                12.5,
                Squircle,
                t.text.opacity(0.03),
                Some(t.border),
            ))
            .child(text("Customize", 14., 18., 430., t.text))
            .on_click(cx.listener(|this, _, window, cx| {
                let focus = this
                    .pets
                    .focus
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                window.focus(&focus, cx);
                this.pets.customize = true;
                cx.notify();
            }));
        div()
            .id("pets-preview")
            .relative()
            .w_full()
            .p(px(24.))
            .h(area + px(48.))
            .child(deco(
                &self.layer,
                20.,
                Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(
                div()
                    .relative()
                    .mx_auto()
                    .w(px(AREA))
                    .h(area)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px((AREA - width) / 2.))
                            .w(figure.width)
                            .h(figure.height)
                            .child(sprite(selected, custom, stored, window, cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left((px(AREA) - mini::BAR_SIZE.width) / 2.)
                            .child(mini::bar(t)),
                    ),
            )
            .child(customize)
    }

    #[allow(clippy::too_many_arguments)]
    fn my_pets(
        &self,
        pets: &[Pet],
        selected: &SharedString,
        visible: bool,
        custom: &[CustomPet],
        loading: bool,
        failed: bool,
        creating: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let refresh = div()
            .id("pets-refresh")
            .role(gpui::Role::Button)
            .aria_label("Refresh")
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .hover(|d| d.bg(t.ghost_hover))
            .child(icon("settings-hooks-reload", 16., t.text_tertiary))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(RefreshPets), cx));
        let refresh = crate::tooltip::tooltip(refresh, "Refresh", None, crate::tooltip::Side::Top);
        let create = div()
            .id("pets-create")
            .role(gpui::Role::Button)
            .aria_label("Create pet")
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .relative()
            .gap(px(4.))
            .child(on_card(12.5, Squircle, t.button_secondary, None))
            .when(creating, |d| {
                d.child(icon("sidebar-cmd-spinner", 14., t.text_secondary))
            })
            .child(text("Create pet", 13., 18., 430., t.text))
            .on_click(move |_, window, cx| {
                if !creating {
                    window.dispatch_action(Box::new(CreatePet), cx)
                }
            });
        let mut tiles = vec![self.tile(
            "none",
            mini::bar(t).into_any_element(),
            px(120.),
            "None",
            Some("The lightweight Codex companion".into()),
            !visible,
            t,
            cx.listener(|this, _, _, cx| this.choose_none(cx)),
        )];
        if loading {
            tiles.push(
                div().col_span_full().child(
                    div()
                        .id("pets-loading")
                        .role(gpui::Role::Status)
                        .px(px(16.))
                        .py(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("sidebar-cmd-spinner", 12., t.text_secondary))
                        .child(text(
                            "Loading custom pets…",
                            13.,
                            18.57,
                            430.,
                            t.text_secondary,
                        )),
                ),
            );
        }
        if failed {
            tiles.push(div().col_span_full().child(settings_row(
                "Unable to load custom pets",
                None,
                div().into_any_element(),
                t,
            )));
        }
        for pet in pets {
            let id = pet.id.clone();
            let size = mini::pet_size(72.);
            let artwork = div()
                .w(size.width)
                .h(size.height)
                .child(sprite(&pet.id, custom, 72., window, cx))
                .into_any_element();
            tiles.push(self.tile(
                pet.id.clone(),
                artwork,
                px(96.),
                pet.name.clone(),
                pet.description.clone(),
                visible && pet.id == *selected,
                t,
                cx.listener(move |this, _, _, cx| this.choose(id.clone(), cx)),
            ));
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(46.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("My pets", 14., 21., 500., t.text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(refresh)
                            .child(create),
                    ),
            )
            .child(div().grid().grid_cols(3).gap(px(16.)).children(tiles))
    }

    #[allow(clippy::too_many_arguments)]
    fn tile(
        &self,
        id: impl Into<SharedString>,
        artwork: AnyElement,
        artwork_width: Pixels,
        label: impl Into<SharedString>,
        description: Option<SharedString>,
        selected: bool,
        t: &Theme,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        let id: SharedString = id.into();
        let label: SharedString = label.into();
        div().child(
            div()
                .id(gpui::ElementId::Name(format!("pets-tile-{id}").into()))
                .debug_selector({
                    let id = id.clone();
                    move || format!("pets-tile-{id}")
                })
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .aria_selected(selected)
                .min_h(px(224.))
                .h_full()
                .p(px(16.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .rounded(px(16.))
                .bg(t.surface)
                .when(selected, |d| d.border_2().border_color(t.border_strong))
                .when(!selected, |d| d.border_1().border_color(t.border))
                .child(
                    div()
                        .w(artwork_width)
                        .h(px(96.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(artwork),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.))
                        .child(text(label, 13., 18.57, 500., t.text))
                        .children(description.map(|d| {
                            text(d, 13., 18.57, 430., t.text_secondary)
                                .whitespace_normal()
                                .text_center()
                        })),
                )
                .on_click(on_click),
        )
    }

    /// `F`: choosing a pet also turns the pet on.
    fn choose(&mut self, id: SharedString, cx: &mut Context<Self>) {
        let page = self.page.read(cx);
        let visible = page.flag(VISIBLE, true);
        if visible && page.get(SELECTED).as_str() == Some(id.as_ref()) {
            return;
        }
        let mut pairs = vec![(SELECTED, Value::from(id.to_string()))];
        if !visible {
            pairs.push((VISIBLE, true.into()));
        }
        self.write_pets(pairs, cx);
    }

    /// `L`: None hides the pet and keeps the chosen one.
    fn choose_none(&mut self, cx: &mut Context<Self>) {
        if self.page.read(cx).flag(VISIBLE, true) {
            self.write_pets(vec![(VISIBLE, false.into())], cx);
        }
    }

    fn write_pets(&mut self, pairs: Vec<(&'static str, Value)>, cx: &mut Context<Self>) {
        self.page
            .update(cx, |page, cx| page.set_host_batch(pairs, cx));
        // Mini draws from the same keys.
        cx.refresh_windows();
    }

    fn set_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.round().clamp(mini::MIN_WIDTH, mini::MAX_WIDTH);
        self.pets.drag = None;
        if mini::width(self.page.read(cx).get(WIDTH).as_f64()) != width {
            self.write_pets(vec![(WIDTH, Value::from(width as i64))], cx);
        }
        cx.notify();
    }

    /// The `Customize pet` dialog: the `Pet size` slider (80–224) and `Reset` (112).
    fn customize_dialog(
        &mut self,
        stored: f32,
        width: f32,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let track = self.pets.track.clone();
        let at = move |track: &Rc<Cell<Bounds<Pixels>>>, x: Pixels| {
            let bounds = track.get();
            let usable = (bounds.size.width - px(THUMB)).max(px(1.));
            let fraction = ((x - bounds.left() - px(THUMB / 2.)) / usable).clamp(0., 1.);
            mini::MIN_WIDTH + fraction * (mini::MAX_WIDTH - mini::MIN_WIDTH)
        };
        let fraction = (width - mini::MIN_WIDTH) / (mini::MAX_WIDTH - mini::MIN_WIDTH);
        let focus = focus::handle("pets-size", 0, window, cx);
        let slider = div()
            .id("pets-size")
            .role(gpui::Role::Slider)
            .aria_label("Pet size")
            .w(px(TRACK))
            .h(px(THUMB))
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h(px(2.))
                    .relative()
                    .bg(t.border_subtle)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(TRACK * fraction))
                            .bg(t.text_accent),
                    )
                    .child({
                        let track = track.clone();
                        canvas(move |bounds, _, _| track.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .size_full()
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px((TRACK - THUMB) * fraction))
                    .size(px(THUMB))
                    .rounded_full()
                    .bg(gpui::white())
                    .border_1()
                    .border_color(t.border_strong),
            )
            .on_mouse_down(gpui::MouseButton::Left, {
                let track = track.clone();
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    let handle = focus.read(cx).handle.clone();
                    handle.focus(window, cx);
                    this.pets.drag = Some(at(&track, event.position.x).round());
                    cx.notify();
                })
            })
            .on_mouse_move({
                let track = track.clone();
                cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if this.pets.drag.is_some() && event.dragging() {
                        this.pets.drag = Some(at(&track, event.position.x).round());
                        cx.notify();
                    }
                })
            })
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::end_size_drag))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::end_size_drag))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let step = match event.keystroke.key.as_str() {
                    "left" | "down" => -1.,
                    "right" | "up" => 1.,
                    _ => return,
                };
                this.set_width(stored + step, cx);
                cx.stop_propagation();
            }));
        let focus = focus::handle("pets-size", 0, window, cx);
        let slider = focus::ring(slider, &focus, Theme::RADIUS_FULL, px(2.), cx);
        let reset_disabled = width == mini::DEFAULT_WIDTH;
        let reset = div()
            .id("pets-size-reset")
            .role(gpui::Role::Button)
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(10.))
            .when(!reset_disabled, |d| d.hover(|d| d.bg(t.ghost_hover)))
            .when(reset_disabled, |d| d.opacity(0.5))
            .child(text("Reset", 13., 18., 430., t.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !reset_disabled {
                    this.set_width(mini::DEFAULT_WIDTH, cx);
                }
            }));
        let close = div()
            .id("pets-customize-close")
            .debug_selector(|| "pets-customize-close".into())
            .role(gpui::Role::Button)
            .aria_label("Close")
            .absolute()
            .top(px(16.))
            .right(px(16.))
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .hover(|d| d.bg(t.ghost_hover))
            .child(icon("close", 16., t.text_secondary))
            .on_click(cx.listener(|this, _, _, cx| this.close_customize(cx)));
        let dialog = div()
            .id("pets-customize-dialog")
            .debug_selector(|| "pets-customize-dialog".into())
            .role(gpui::Role::Dialog)
            .aria_label("Customize pet")
            .when_some(self.pets.focus.as_ref(), |d, focus| d.track_focus(focus))
            .relative()
            .w(px(440.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(on_card(20., Squircle, t.menu_background, Some(t.border)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .pr(px(32.))
                    .child(text("Customize pet", 16., 24., 500., t.text)),
            )
            .child(close)
            .child(settings_row(
                "Pet size",
                Some("Adjust the size of your pet"),
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(8.))
                    .child(slider)
                    .child(reset)
                    .into_any_element(),
                t,
            ));
        let viewport = window.viewport_size();
        gpui::deferred(
            gpui::anchored()
                .position(gpui::point(px(0.), px(0.)))
                .child(
                    div()
                        .id("pets-customize-scrim")
                        .w(viewport.width)
                        .h(viewport.height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::rgba(0x00000022))
                        .occlude()
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.close_customize(cx)),
                        )
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            if event.keystroke.key == "escape" {
                                this.close_customize(cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(dialog),
                ),
        )
        .with_priority(1)
    }

    fn end_size_drag(&mut self, _: &gpui::MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(width) = self.pets.drag {
            self.set_width(width, cx);
        }
    }

    fn close_customize(&mut self, cx: &mut Context<Self>) {
        self.pets.customize = false;
        self.pets.drag = None;
        cx.notify();
    }
}

/// The subtitle: the `Show Mini` shortcut as a chip, an unassigned prompt, or the plain line
/// before the keymap loads.
fn subtitle(t: &Theme, cx: &App) -> Div {
    let line = |s: &'static str| text(s, 14., 24., 430., t.text_secondary);
    let state = cx
        .try_global::<keyboard_shortcuts::KeyboardState>()
        .filter(|state| state.loaded);
    let command = keyboard_shortcuts::commands()
        .iter()
        .find(|command| command.id == "openAvatarOverlay");
    let (Some(state), Some(command)) = (state, command) else {
        return line("Pets give you quick access to ChatGPT");
    };
    let chip = |label: String| {
        div()
            .px(px(8.))
            .py(px(2.))
            .rounded_full()
            .bg(t.text.opacity(0.05))
            .child(text(label, 13., 18., 430., t.text))
    };
    let row = div().flex().items_center().gap(px(6.));
    match state.preferences.keys(command).next() {
        Some(key) => row
            .child(line("Show and focus your pet by pressing"))
            .child(chip(keyboard_shortcuts::key_label(key))),
        None => row
            .child(line("Set a shortcut to show and focus your pet:"))
            .child(chip("Off".into())),
    }
}

/// Pet `id` at `width`, or its load error.
fn sprite(id: &str, custom: &[CustomPet], width: f32, window: &Window, cx: &App) -> AnyElement {
    let t = cx.theme();
    match mini::pet(
        id,
        custom,
        width,
        window.scale_factor(),
        crate::motion::reduced(cx),
    ) {
        Ok(image) => gpui::img(image).size_full().into_any_element(),
        Err(error) => text(error.to_string(), 12., 16., 400., t.text_error)
            .whitespace_normal()
            .into_any_element(),
    }
}
