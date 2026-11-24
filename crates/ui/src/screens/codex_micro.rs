//! Settings ▸ Codex Micro (`codex-micro-settings-*.js`): the connection state, lighting, the
//! keycap layout with its Edit keycap dialog, and the agent-key options. Host keys, as the
//! original names them: `codex-micro-layout`, `codex-micro-agent-source`,
//! `codex-micro-single-tap-agent-keys`, `codex-micro-lighting-brightness` and
//! `codex-micro-lighting-auto-off`.
//!
//! shortcut: the HID service (`@worklouder/device-kit-oai`, main process) has no Linux port
//! here, so the connection row reads "Not detected" and battery stays hidden. The analog stick,
//! knob and custom agent-key editors, and the microphone-key mode are not built yet.
use super::*;
use theme::Appearance;

const LAYOUT: &str = "host.codex-micro-layout";
const BRIGHTNESS: &str = "host.codex-micro-lighting-brightness";
const AUTO_OFF: &str = "host.codex-micro-lighting-auto-off";
const AGENT_SOURCE: &str = "host.codex-micro-agent-source";
const SINGLE_TAP: &str = "host.codex-micro-single-tap-agent-keys";

/// `w-[4.75rem]`: a keycap tile.
const TILE: f32 = 76.;
/// `w-40` track and `size-5` thumb of the brightness slider.
const TRACK: f32 = 160.;
const THUMB: f32 = 20.;

/// `(id, action label, double)`: the original's keycap catalog (`codex-micro-layout-*.js`
/// and the `keycaps.*` messages).
const KEYCAPS: &[(&str, &str, bool)] = &[
    ("FAST", "Cycle speed", false),
    ("APPR", "Approve", false),
    ("REJ", "Reject", false),
    ("SPLIT", "Fork chat", false),
    ("MIC", "Push to talk", true),
    ("MIC1", "Push to talk", false),
    ("CODEX", "Send message", false),
    ("BUG", "Open feedback", false),
    ("OAI", "Open OpenAI docs", false),
    ("TERM", "Toggle terminal", false),
    ("DWN", "Copy chat as Markdown", false),
    ("DEL", "Archive chat", false),
    ("NEW", "New chat", false),
    ("NAV", "Open browser tab", false),
    ("MAGIC", "Pin or unpin chat", false),
    ("DIFF", "Toggle Changes", false),
    ("PLAY", "Run primary action", false),
    ("GIT", "Commit or push", false),
    ("BRCH", "Create draft PR", false),
    ("BRANCH", "Create branch", false),
    ("MRG", "Merge PR", false),
    ("PR", "Create PR", false),
    ("PAINT", "Add photos", false),
    ("LAB", "Open Settings", false),
    ("PARTY", "Open side chat", false),
    ("TIME", "Open Scheduled", false),
    ("MIND+", "Increase reasoning effort", false),
    ("MIND-", "Decrease reasoning effort", false),
    ("EMPT1", "Assign any shortcut", false),
    ("EMPT2", "Assign any shortcut", false),
    ("EMPT3", "Assign any shortcut", false),
    ("EMPT4", "Assign any shortcut", false),
    ("SETUP", "Open Settings", false),
    ("FOLD", "Open folder", false),
    ("UPL", "Attach files and folders", false),
    ("APPS", "Open plugins", false),
    ("YOLO", "Write :yolo: in the composer", false),
    ("YEET", "Write :yeet: in the composer", false),
    ("EMPT5", "Assign any shortcut", true),
];

/// The factory layout (`M_t`): slot, keycap.
const DEFAULT_SLOTS: &[(&str, &str)] = &[
    ("ACT06", "FAST"),
    ("ACT07", "APPR"),
    ("ACT08", "REJ"),
    ("ACT09", "SPLIT"),
    ("ACT10", "MIC1"),
    ("ACT11", "EMPT1"),
    ("ACT10_ACT11", "MIC"),
    ("ACT12", "CODEX"),
];
/// The slots the grid shows: the wide microphone key replaces `ACT10` and `ACT11` unless the
/// keys are separate.
const GRID: &[&[&str]] = &[
    &["ACT06", "ACT07", "ACT08", "ACT09"],
    &["ACT10_ACT11", "ACT12"],
];

const AUTO_OFFS: &[(&str, &str)] = &[
    ("off", "Off"),
    ("30-seconds", "30 seconds"),
    ("1-minute", "1 minute"),
    ("3-minutes", "3 minutes"),
    ("10-minutes", "10 minutes"),
    ("30-minutes", "30 minutes"),
    ("1-hour", "1 hour"),
];
const SOURCES: &[(&str, &str)] = &[
    ("pinned", "Pinned chats"),
    ("recent", "Most recent chats"),
    ("priority", "Priority chats"),
];
const SOURCE_DETAILS: &[(&str, &str)] = &[
    ("pinned", "First six chats in Pinned"),
    ("recent", "First six recently updated chats, pinned or unpinned"),
    ("priority", "Waiting, unread, and active chats first"),
];

#[derive(Default)]
pub(super) struct State {
    /// The slot whose Edit keycap dialog is open, and the keycap picked in it.
    edit: Option<(&'static str, Option<&'static str>)>,
    query: String,
    /// The dialog's focus, so Escape and typing reach its key handler.
    focus: Option<FocusHandle>,
    /// The brightness while its slider is dragged; written on release.
    drag: Option<f32>,
    track: Rc<Cell<Bounds<Pixels>>>,
}

fn keycap(id: &str) -> Option<&'static (&'static str, &'static str, bool)> {
    KEYCAPS.iter().find(|k| k.0 == id)
}

/// The stored layout, or the factory layout (`E_t`/`D_t` are the analog and knob defaults the
/// original's schema fills in).
fn default_layout() -> Value {
    let slots: serde_json::Map<String, Value> = DEFAULT_SLOTS
        .iter()
        .map(|&(slot, id)| (slot.into(), json!({ "keycapId": id })))
        .collect();
    json!({
        "version": 1,
        "slots": slots,
        "analogStick": {
            "up": {"type": "command", "commandId": "composer.togglePlanMode"},
            "right": {"type": "command", "commandId": "navigateForward"},
            "down": {"type": "command", "commandId": "toggleSidebar"},
            "left": {"type": "command", "commandId": "navigateBack"},
        },
        "encoder": {"left": null, "right": null, "click": null, "longPress": null},
        "encoderMode": "composer-navigation",
        "voiceButtonMode": "push-to-talk",
        "separateMicrophoneKeys": false,
    })
}

fn slot_keycap<'a>(layout: &'a Value, slot: &str) -> &'a str {
    layout["slots"][slot]["keycapId"]
        .as_str()
        .unwrap_or_else(|| DEFAULT_SLOTS.iter().find(|s| s.0 == slot).map_or("EMPT1", |s| s.1))
}

/// Puts `id` on `slot`; a keycap already on another slot of the same size swaps places, as the
/// original's editor does.
fn assign(layout: &mut Value, slot: &str, id: &str) {
    let double = slot == "ACT10_ACT11";
    let previous = slot_keycap(layout, slot).to_owned();
    let other = DEFAULT_SLOTS
        .iter()
        .map(|s| s.0)
        .find(|&s| s != slot && (s == "ACT10_ACT11") == double && slot_keycap(layout, s) == id);
    if let Some(other) = other {
        layout["slots"][other] = json!({ "keycapId": previous });
    }
    layout["slots"][slot] = json!({ "keycapId": id });
}

impl SettingsPane {
    fn layout(&self, cx: &App) -> Value {
        let page = self.page.read(cx);
        let mut layout = default_layout();
        if let Some(stored) = page.get(LAYOUT).as_object() {
            for (key, value) in stored {
                if key == "slots" {
                    for (slot, keycap) in value.as_object().into_iter().flatten() {
                        layout["slots"][slot] = keycap.clone();
                    }
                } else {
                    layout[key] = value.clone();
                }
            }
        }
        layout
    }

    pub(super) fn codex_micro_page(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<(&'static str, Vec<Div>)> {
        let lighting = vec![
            settings_row_content(
                "Connection",
                None,
                text("Not detected", 13., 18., 430., t.text_secondary).into_any_element(),
                t,
            ),
            settings_row_content(
                "Brightness",
                Some(
                    text(
                        "Adjusts the brightness of all Codex Micro lighting",
                        13.,
                        18.,
                        430.,
                        t.text_secondary,
                    )
                    .into_any_element(),
                ),
                self.brightness(t, window, cx),
                t,
            ),
            settings_row(
                "Auto-dim",
                Some("Turns lighting off after inactivity and back on when you use Codex Micro or an agent key changes color or state"),
                self.select_menu_or(AUTO_OFF, AUTO_OFFS, "3-minutes", None, |s| s, cx),
                t,
            ),
        ];
        let layout = vec![self.layout_row(t, window, cx)];
        let options = vec![
            settings_row(
                "Agent keys",
                Some("Choose what the six agent keys follow or trigger"),
                self.select_menu_or(
                    AGENT_SOURCE,
                    SOURCES,
                    "recent",
                    None,
                    |s| s.with_details(SOURCE_DETAILS),
                    cx,
                ),
                t,
            ),
            settings_row(
                "Focus Codex with a single tap",
                Some("Open the assigned task and focus the Codex window with a single tap instead of a double tap"),
                self.toggle(SINGLE_TAP, false, t, window, cx),
                t,
            ),
        ];
        vec![("", lighting), ("Layout", layout), ("Options", options)]
    }

    /// `fi`: the brightness slider with its percentage.
    fn brightness(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let stored = self.page.read(cx).get(BRIGHTNESS).as_f64().unwrap_or(100.) as f32;
        let value = self.codex_micro.drag.unwrap_or(stored).clamp(0., 100.);
        let track = self.codex_micro.track.clone();
        let at = |track: &Rc<Cell<Bounds<Pixels>>>, x: Pixels| {
            let bounds = track.get();
            let usable = (bounds.size.width - px(THUMB)).max(px(1.));
            (((x - bounds.left() - px(THUMB / 2.)) / usable).clamp(0., 1.) * 100.).round()
        };
        let fraction = value / 100.;
        let focus = focus::handle("codex-micro-brightness", 0, window, cx);
        let slider = div()
            .id("codex-micro-brightness")
            .role(gpui::Role::Slider)
            .aria_label("Brightness")
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
                    .rounded_full()
                    .bg(t.border)
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
                let handle = focus.read(cx).handle.clone();
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    handle.focus(window, cx);
                    this.codex_micro.drag = Some(at(&track, event.position.x));
                    cx.notify();
                })
            })
            .on_mouse_move({
                let track = track.clone();
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.codex_micro.drag.is_some() && event.dragging() {
                        this.codex_micro.drag = Some(at(&track, event.position.x));
                        cx.notify();
                    }
                })
            })
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::end_brightness_drag))
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::end_brightness_drag))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let step = match event.keystroke.key.as_str() {
                    "left" | "down" => -1.,
                    "right" | "up" => 1.,
                    _ => return,
                };
                this.set_brightness((stored + step).clamp(0., 100.), cx);
                cx.stop_propagation();
            }));
        let slider = focus::ring(slider, &focus, Theme::RADIUS_FULL, px(2.), cx);
        div()
            .h(px(36.))
            .min_w(px(192.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(slider)
            .child(
                div()
                    .w(px(48.))
                    .flex()
                    .justify_end()
                    .child(text(format!("{value:.0}%"), 13., 18., 430., t.text_secondary)),
            )
            .into_any_element()
    }

    fn end_brightness_drag(
        &mut self,
        _: &gpui::MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(value) = self.codex_micro.drag.take() {
            self.set_brightness(value, cx);
        }
    }

    fn set_brightness(&mut self, value: f32, cx: &mut Context<Self>) {
        self.page
            .update(cx, |page, cx| page.set(BRIGHTNESS, json!(value as u32), cx));
        cx.notify();
    }

    /// The keycap grid with `Reset layout`.
    fn layout_row(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let layout = self.layout(cx);
        let rows = GRID.iter().map(|row| {
            let tiles: Vec<_> = row.iter().map(|&slot| {
                let id = slot_keycap(&layout, slot);
                let double = slot == "ACT10_ACT11";
                let label = keycap(id).map_or("", |k| k.1);
                let focus = focus::handle((gpui::ElementId::from("codex-micro-slot"), slot), 0, window, cx);
                let tile = tile(id, label, double, false, t)
                    .id(slot)
                    .role(gpui::Role::Button)
                    .aria_label(format!("Edit {slot}: {label}"));
                focus::ring(tile, &focus, px(13.), px(2.), cx)
                    .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_keycap_editor(slot, window, cx)
                    }))
            }).collect();
            div().flex().gap(px(8.)).children(tiles)
        }).collect::<Vec<_>>();
        let reset = button("Reset layout", t.text.opacity(0.05), None, t)
            .id("codex-micro-reset")
            .role(gpui::Role::Button)
            .on_click(cx.listener(|this, _, _, cx| {
                this.page
                    .update(cx, |page, cx| page.set(LAYOUT, default_layout(), cx));
            }));
        div()
            .relative()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(rows)
            .child(div().absolute().top(px(16.)).right(px(16.)).child(reset))
    }

    fn open_keycap_editor(&mut self, slot: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .codex_micro
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus, cx);
        self.codex_micro.edit = Some((slot, None));
        self.codex_micro.query.clear();
        cx.notify();
    }

    fn close_keycap_editor(&mut self, cx: &mut Context<Self>) {
        self.codex_micro.edit = None;
        cx.notify();
    }

    fn save_keycap(&mut self, cx: &mut Context<Self>) {
        if let Some((slot, Some(id))) = self.codex_micro.edit {
            let mut layout = self.layout(cx);
            assign(&mut layout, slot, id);
            self.page.update(cx, |page, cx| page.set(LAYOUT, layout, cx));
        }
        self.close_keycap_editor(cx);
    }

    /// `yi`: the Edit keycap dialog.
    pub(super) fn keycap_dialog(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (slot, picked) = self.codex_micro.edit?;
        let layout = self.layout(cx);
        let current = picked.unwrap_or_else(|| slot_keycap(&layout, slot));
        let double = slot == "ACT10_ACT11";
        let needle = self.codex_micro.query.to_lowercase();
        let keycaps: Vec<_> = KEYCAPS
            .iter()
            .filter(|k| k.2 == double)
            .filter(|k| needle.is_empty() || k.0.to_lowercase().contains(&needle))
            .collect();
        let tiles = keycaps.iter().map(|&&(id, label, _)| {
            tile(id, label, false, id == current, t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(format!("{id}: {label}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.codex_micro.edit = Some((slot, Some(id)));
                    cx.notify();
                }))
        });
        let query = self.codex_micro.query.clone();
        let search = div()
            .h(px(40.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .relative()
            .child(on_card(8., Squircle, t.text.opacity(0.03), Some(t.border)))
            .child(icon("search", 16., t.text_tertiary))
            .child(if query.is_empty() {
                text("Search keycaps", 14., 20., 430., t.text_tertiary)
            } else {
                text(query, 14., 20., 430., t.text)
            });
        let body = if keycaps.is_empty() {
            div()
                .id("codex-micro-no-keycaps")
                .min_h(px(112.))
                .flex()
                .items_center()
                .justify_center()
                .child(text("No matching keycaps", 13., 18., 430., t.text_secondary))
        } else {
            div()
                .id("codex-micro-keycaps")
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .p(px(4.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .children(tiles)
        };
        let footer = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(
                button("Cancel", gpui::transparent_black(), None, t)
                    .id("codex-micro-cancel")
                    .role(gpui::Role::Button)
                    .on_click(cx.listener(|this, _, _, cx| this.close_keycap_editor(cx))),
            )
            .child(
                div()
                    .h(px(28.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .relative()
                    .child(on_card(12.5, Squircle, t.text, None))
                    .child(text("Save", 13., 18., 430., t.background_panel))
                    .id("codex-micro-save")
                    .role(gpui::Role::Button)
                    .on_click(cx.listener(|this, _, _, cx| this.save_keycap(cx))),
            );
        let dialog = div()
            .id("codex-micro-dialog")
            .role(gpui::Role::Dialog)
            .aria_label("Edit keycap")
            .when_some(self.codex_micro.focus.as_ref(), |d, f| d.track_focus(f))
            .relative()
            .w(px(480.))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(on_card(20., Squircle, t.menu_background, Some(t.border)))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(text("Edit keycap", 16., 24., 500., t.text))
                    .child(text(
                        format!("Choose what appears on {slot}"),
                        13.,
                        18.,
                        430.,
                        t.text_secondary,
                    )),
            )
            .child(search)
            .child(body)
            .child(footer);
        let viewport = window.viewport_size();
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(gpui::point(px(0.), px(0.)))
                    .child(
                        div()
                            .id("codex-micro-scrim")
                            .w(viewport.width)
                            .h(viewport.height)
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(t.dialog_overlay)
                            .occlude()
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(|this, _, _, cx| this.close_keycap_editor(cx)),
                            )
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                let k = &event.keystroke;
                                match (k.key.as_str(), &k.key_char) {
                                    ("escape", _) => this.close_keycap_editor(cx),
                                    ("backspace", _) => {
                                        this.codex_micro.query.pop();
                                    }
                                    (_, Some(c))
                                        if !k.modifiers.platform
                                            && !k.modifiers.control
                                            && !c.chars().any(char::is_control) =>
                                    {
                                        this.codex_micro.query.push_str(c)
                                    }
                                    _ => return,
                                }
                                cx.notify();
                                cx.stop_propagation();
                            }))
                            .child(dialog),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// A keycap: `rounded-[0.8rem]` tile, light `#f7f5f1` or dark `#202127`, the id in mono 11 px.
fn tile(id: &str, _label: &str, double: bool, selected: bool, t: &Theme) -> Div {
    let dark = t.appearance == Appearance::Dark;
    let (fill, ink) = if dark {
        (gpui::rgb(0x202127), t.text)
    } else {
        (gpui::rgb(0xf7f5f1), gpui::rgb(0x171717).into())
    };
    let edge = if selected {
        t.text_accent
    } else if dark {
        gpui::white().opacity(0.14)
    } else {
        gpui::white().opacity(0.86)
    };
    div()
        .relative()
        .w(px(if double { TILE * 2. + 8. } else { TILE }))
        .h(px(TILE))
        .flex()
        .items_center()
        .justify_center()
        .child(on_card(12.8, Squircle, fill.into(), Some(edge)))
        .child(
            text(id.to_owned(), 11., 16., 400., ink).font_family(MONO),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigning_a_used_keycap_swaps_slots() {
        let mut layout = default_layout();
        assign(&mut layout, "ACT06", "APPR");
        assert_eq!(slot_keycap(&layout, "ACT06"), "APPR");
        assert_eq!(slot_keycap(&layout, "ACT07"), "FAST");
    }

    #[test]
    fn double_and_single_keycaps_do_not_swap() {
        let mut layout = default_layout();
        assign(&mut layout, "ACT10_ACT11", "EMPT5");
        assert_eq!(slot_keycap(&layout, "ACT10_ACT11"), "EMPT5");
        assert_eq!(slot_keycap(&layout, "ACT10"), "MIC1");
    }
}
