//! Advanced Appearance controls from Codex 26.1007.30912.
//!
//! The original `general-settings-65e65871cdcd.js` defines the controls and share format.
//! Theme and preset changes use one host-settings transaction.
//! Font choices use installed CoreText faces; custom family lists keep their fallback order.
//! Run Linux checks with `cargo test -p ui -p app --lib appearance`.
//! Run the Mac interaction check with `cargo test -p app --lib appearance::native_tests`.
//! Set `CGX_APPEARANCE_SHOTS` to retain native captures outside the source tree.
//! Native tests do not replace strict pairs against the original app.

use super::*;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const LIGHT: &str = "host.appearanceLightChromeTheme";
pub const DARK: &str = "host.appearanceDarkChromeTheme";
pub const LIGHT_PRESET: &str = "host.appearanceLightCodeThemeId";
pub const DARK_PRESET: &str = "host.appearanceDarkCodeThemeId";
pub const UI_SIZE: &str = "host.sansFontSize";
pub const CODE_SIZE: &str = "host.codeFontSize";
pub const MARKERS: &str = "host.appearanceDiffMarkerStyle";
pub const MOTION: &str = "host.reduced-motion-preference";
pub const POINTER: &str = "host.usePointerCursors";
pub const SMOOTHING: &str = "host.useFontSmoothing";
pub const DOCK: &str = "host.dock-icon-preference";
const SHARE_PREFIX: &str = "codex-theme-v1:";
pub const PAIR: &str = "appearance.theme-pair";
pub const COPY: &str = "appearance.copy-theme";

pub struct PairSaved {
    pub imported: bool,
    pub success: bool,
}
impl EventEmitter<PairSaved> for SettingsPage {}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fonts {
    pub ui: Option<String>,
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_face: Option<FontFace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_face: Option<FontFace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_face: Option<FontFace>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFace {
    pub family: String,
    pub full_name: String,
    pub postscript_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticColors {
    pub diff_added: String,
    pub diff_removed: String,
    pub skill: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeTheme {
    pub accent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_source: Option<String>,
    pub contrast: u8,
    pub fonts: Fonts,
    pub ink: String,
    pub opaque_windows: bool,
    pub semantic_colors: SemanticColors,
    pub surface: String,
}

impl ChromeTheme {
    pub fn default_for(dark: bool) -> Self {
        Self {
            accent: "#339cff".into(),
            accent_source: Some("chatgpt".into()),
            contrast: if dark { 60 } else { 45 },
            fonts: Fonts::default(),
            ink: if dark { "#ffffff" } else { "#1a1c1f" }.into(),
            opaque_windows: false,
            semantic_colors: SemanticColors {
                diff_added: if dark { "#40c977" } else { "#00a240" }.into(),
                diff_removed: if dark { "#fa423e" } else { "#ba2623" }.into(),
                skill: if dark { "#ad7bf9" } else { "#924ff7" }.into(),
            },
            surface: if dark { "#181818" } else { "#ffffff" }.into(),
        }
    }

    pub fn valid(&self) -> bool {
        self.contrast <= 100
            && [
                &self.accent,
                &self.ink,
                &self.surface,
                &self.semantic_colors.diff_added,
                &self.semantic_colors.diff_removed,
                &self.semantic_colors.skill,
            ]
            .iter()
            .all(|s| valid_color(s))
            && self
                .accent_source
                .as_deref()
                .is_none_or(|s| matches!(s, "custom" | "chatgpt"))
    }

    pub fn apply(&self, t: &mut Theme) {
        let dark = t.appearance == theme::Appearance::Dark;
        let baseline = Self::default_for(dark);
        if self.accent == baseline.accent
            && self.ink == baseline.ink
            && self.surface == baseline.surface
            && self.contrast == baseline.contrast
            && self.semantic_colors == baseline.semantic_colors
        {
            if self.opaque_windows {
                t.sidebar_background = over(t.sidebar_background, t.page_surface_layout);
            }
            return;
        }
        let ink = color(&self.ink);
        let surface = color(&self.surface);
        let accent = color(&self.accent);
        let base = if dark { 60. } else { 45. };
        let c = self.contrast as f32 / 100. + (self.contrast as f32 - base) / 60. * 0.7;
        let c = if self.contrast as f32 > base {
            base / 100. + (c - base / 100.) * 2.
        } else {
            c
        };
        let mix = |a, b, p| mix_color(a, b, p);
        let alpha = |mut a: Hsla, p: f32| {
            a.a = (p.clamp(0., 1.) * 1000.).round() / 1000.;
            a
        };
        let black = gpui::rgb(0).into();
        let white = gpui::rgb(0xffffff).into();
        let under = mix(
            surface,
            if dark { black } else { ink },
            if dark {
                0.16 + (self.contrast as f32 - 60.) * 0.0015
            } else {
                0.04 + (self.contrast as f32 - 45.) * 0.0012
            },
        );
        let control = mix(
            surface,
            if dark { ink } else { white },
            if dark {
                0.06 + c * 0.05
            } else {
                0.09 + c * 0.04
            },
        );
        let elevated = mix(surface, if dark { ink } else { white }, 0.08 + c * 0.08);
        let editor = mix(
            surface,
            if dark { ink } else { white },
            if dark { 0.07 } else { 0.12 },
        );
        let secondary = mix(
            surface,
            if dark { ink } else { white },
            if dark {
                0.04 + c * 0.05
            } else {
                0.08 + c * 0.08
            },
        );
        t.surface = surface;
        t.background_surface_under = under;
        t.page_surface_layout = alpha(
            if dark { editor } else { under },
            if dark { 0.8 } else { 0.85 },
        );
        t.sidebar_background = alpha(surface, if self.opaque_windows { 1. } else { 0.65 });
        t.surface_secondary = under;
        t.surface_tertiary = editor;
        t.surface_elevated = alpha(elevated, 0.96);
        t.surface_elevated_secondary = control;
        t.background_elevated_primary_opaque = elevated;
        t.tooltip_background = t.surface_elevated;
        t.background_panel = mix(
            surface,
            if dark { ink } else { white },
            if dark {
                0.03 + c * 0.03
            } else {
                0.18 + c * 0.008
            },
        );
        t.composer_surface = elevated;
        t.composer_action_bar = if dark { secondary } else { under };
        t.menu_background = control;
        t.control_background = control;
        t.button_primary_solid = ink;
        t.button_secondary = alpha(ink, 0.04 + c * 0.02);
        t.button_secondary_hover = alpha(
            ink,
            if dark {
                0.06 + c * 0.03
            } else {
                0.04 + c * 0.03
            },
        );
        t.button_tertiary_hover = alpha(
            ink,
            if dark {
                0.05 + c * 0.03
            } else {
                0.08 + c * 0.04
            },
        );
        t.ghost_hover = t.button_secondary_hover;
        t.menu_item_hover = t.button_secondary_hover;
        t.list_hover = t.button_secondary_hover;
        t.primary_soft_alpha = if dark {
            alpha(ink, 0.02 + c * 0.02)
        } else {
            alpha(secondary, 0.96)
        };
        t.text = ink;
        t.text_default_nav = alpha(ink, 0.85);
        t.text_secondary = alpha(ink, 0.65);
        t.text_description = if dark {
            t.text_secondary
        } else {
            gpui::rgb(0x5d5d5d).into()
        };
        t.text_tertiary = alpha(
            ink,
            if dark {
                0.42 + c * 0.13
            } else {
                0.45 + c * 0.1
            },
        );
        t.text_disabled = t.text_tertiary;
        t.text_inverse = if dark {
            mix(surface, black, 0.38 + c * 0.12)
        } else {
            surface
        };
        t.text_alpha_5 = alpha(ink, 0.05);
        t.text_alpha_10 = alpha(ink, 0.1);
        t.text_alpha_60 = alpha(ink, 0.6);
        t.icon_primary = if dark {
            alpha(ink, 0.82 + c * 0.14)
        } else {
            ink
        };
        t.icon_secondary = alpha(ink, 0.65 + c * 0.1);
        t.icon_tertiary = alpha(ink, 0.45 + c * 0.1);
        t.border = alpha(ink, 0.06 + c * 0.04);
        t.border_subtle = alpha(
            ink,
            if dark {
                0.03 + c * 0.02
            } else {
                0.04 + c * 0.02
            },
        );
        t.border_strong = alpha(
            ink,
            if dark {
                0.12 + c * 0.06
            } else {
                0.09 + c * 0.06
            },
        );
        t.border_composer_rail = alpha(t.border, t.border.a * 0.8);
        t.sidebar_divider = t.border;
        t.page_surface_stroke = alpha(ink, if dark { 0.06 } else { 0.04 });
        t.accent = accent;
        if self.accent_source.as_deref() != Some("chatgpt") {
            t.text_accent = if dark {
                mix(accent, white, 0.3 + c * 0.15)
            } else {
                accent
            };
        }
        t.text_link = accent;
        t.info_solid = accent;
        t.border_focus = if dark {
            alpha(mix(accent, white, 0.3 + c * 0.15), 0.7 + c * 0.1)
        } else {
            accent
        };
        t.focus_ring = t.border_focus;
        t.background_accent = mix(
            if dark { black } else { surface },
            accent,
            if dark {
                0.2 + c * 0.08
            } else {
                0.11 + c * 0.04
            },
        );
        t.switch_track_checked = accent;
        t.switch_track = t.text_alpha_10;
        if self.accent_source.as_deref() != Some("chatgpt") {
            t.composer_primary = ink;
            t.text_composer_primary = control;
            t.user_message_background = alpha(ink, 0.05);
            t.text_selection = alpha(t.text_accent, 0.3);
        }
        t.scrollbar = alpha(ink, 0.3);
        t.scrollbar_hover = alpha(ink, 0.5);
        t.diff_added = alpha(
            color(&self.semantic_colors.diff_added),
            if dark { 0.23 } else { 0.15 },
        );
        t.diff_deleted = alpha(
            color(&self.semantic_colors.diff_removed),
            if dark { 0.23 } else { 0.15 },
        );
        t.git_added = color(&self.semantic_colors.diff_added);
        t.git_deleted = color(&self.semantic_colors.diff_removed);
    }
}

fn valid_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

fn color(s: &str) -> Hsla {
    gpui::rgb(u32::from_str_radix(&s[1..], 16).expect("validated theme color")).into()
}

fn mix_color(a: Hsla, b: Hsla, p: f32) -> Hsla {
    let (a, b): (gpui::Rgba, gpui::Rgba) = (a.into(), b.into());
    let component = |a: f32, b: f32| ((a + (b - a) * p.clamp(0., 1.)) * 255.).round() / 255.;
    gpui::Rgba {
        r: component(a.r, b.r),
        g: component(a.g, b.g),
        b: component(a.b, b.b),
        a: 1.,
    }
    .into()
}

#[derive(Clone, Debug, Deserialize)]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub dark: Option<Value>,
    pub light: Option<Value>,
}

pub static PRESETS: LazyLock<Vec<Preset>> = LazyLock::new(|| {
    let mut presets: Vec<Preset> = serde_json::from_str(include_str!(
        "../../../../../assets/app/appearance-presets.json"
    ))
    .expect("bundled original theme presets");
    presets.sort_by_cached_key(|preset| preset.label.to_lowercase());
    presets
});

impl Preset {
    pub fn theme(&self, dark: bool, current: &ChromeTheme) -> Option<ChromeTheme> {
        let seed = if dark {
            self.dark.as_ref()?
        } else {
            self.light.as_ref()?
        };
        let mut value = serde_json::to_value(current).ok()?;
        for (key, child) in seed.as_object()? {
            if matches!(key.as_str(), "fonts" | "semanticColors") {
                for (field, value_patch) in child.as_object()? {
                    value[key][field] = value_patch.clone();
                }
            } else if key != "syntax" {
                value[key] = child.clone();
            }
        }
        value["accentSource"] = json!("custom");
        serde_json::from_value(value)
            .ok()
            .filter(ChromeTheme::valid)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedTheme {
    code_theme_id: String,
    theme: ChromeTheme,
    variant: String,
}

pub fn share_string(theme: &ChromeTheme, preset: &str, dark: bool) -> String {
    format!(
        "{SHARE_PREFIX}{}",
        serde_json::to_string(&SharedTheme {
            code_theme_id: preset.into(),
            theme: theme.clone(),
            variant: if dark { "dark" } else { "light" }.into(),
        })
        .expect("theme serialization")
    )
}

pub fn import_string(input: &str, dark: bool) -> Result<(ChromeTheme, String), &'static str> {
    let raw = input
        .trim()
        .strip_prefix(SHARE_PREFIX)
        .ok_or("Theme share string mismatch")?;
    let decoded;
    let raw = if raw.starts_with('{') {
        raw
    } else {
        let bytes = raw.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let pair = raw.get(i + 1..i + 3).ok_or("Invalid encoded theme")?;
                out.push(u8::from_str_radix(pair, 16).map_err(|_| "Invalid encoded theme")?);
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        decoded = String::from_utf8(out).map_err(|_| "Invalid encoded theme")?;
        &decoded
    };
    let shared: SharedTheme = serde_json::from_str(raw).map_err(|_| "Invalid theme")?;
    if shared.variant != if dark { "dark" } else { "light" } {
        return Err("Theme variant mismatch");
    }
    if !shared.theme.valid() {
        return Err("Invalid theme colors or contrast");
    }
    if !PRESETS.iter().any(|p| {
        p.id == shared.code_theme_id
            && if dark {
                p.dark.is_some()
            } else {
                p.light.is_some()
            }
    }) {
        return Err("Theme code theme mismatch");
    }
    Ok((shared.theme, shared.code_theme_id))
}

#[derive(Clone)]
pub struct Preferences {
    pub theme: ChromeTheme,
    pub ui_family: SharedString,
    pub code_family: SharedString,
    pub content_family: SharedString,
    pub ui_size: f32,
    pub code_size: f32,
    pub symbols: bool,
    pub pointer: bool,
    pub code_theme: Option<Arc<syntect::highlighting::Theme>>,
    pub preset: String,
    pub smoothing: bool,
    pub motion: String,
    pub dock: String,
}
impl Global for Preferences {}

impl Preferences {
    pub fn read(values: &serde_json::Map<String, Value>, dark: bool) -> Self {
        let key = if dark {
            "appearanceDarkChromeTheme"
        } else {
            "appearanceLightChromeTheme"
        };
        let theme = values
            .get(key)
            .and_then(|v| serde_json::from_value::<ChromeTheme>(v.clone()).ok())
            .filter(ChromeTheme::valid)
            .unwrap_or_else(|| ChromeTheme::default_for(dark));
        let preset = values
            .get(if dark {
                "appearanceDarkCodeThemeId"
            } else {
                "appearanceLightCodeThemeId"
            })
            .and_then(Value::as_str)
            .unwrap_or("codex");
        let code_theme = values
            .contains_key(key)
            .then(|| syntax_theme(preset, dark, &theme))
            .flatten()
            .map(Arc::new);
        Self {
            ui_family: SharedString::from(theme.fonts.ui.clone().unwrap_or_else(|| SANS.into())),
            code_family: SharedString::from(
                theme.fonts.code.clone().unwrap_or_else(|| MONO.into()),
            ),
            content_family: SharedString::from(
                theme
                    .fonts
                    .content
                    .clone()
                    .or_else(|| theme.fonts.ui.clone())
                    .unwrap_or_else(|| SANS.into()),
            ),
            theme,
            code_theme,
            preset: preset.to_owned(),
            ui_size: values
                .get("sansFontSize")
                .and_then(Value::as_f64)
                .filter(|n| (11. ..=16.).contains(n))
                .unwrap_or(14.) as f32,
            code_size: values
                .get("codeFontSize")
                .and_then(Value::as_f64)
                .filter(|n| (8. ..=24.).contains(n))
                .unwrap_or(12.) as f32,
            symbols: values
                .get("appearanceDiffMarkerStyle")
                .and_then(Value::as_str)
                == Some("symbols"),
            pointer: values
                .get("usePointerCursors")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            smoothing: values
                .get("useFontSmoothing")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            motion: values
                .get("reduced-motion-preference")
                .and_then(Value::as_str)
                .unwrap_or("system")
                .into(),
            dock: values
                .get("dock-icon-preference")
                .and_then(Value::as_str)
                .unwrap_or("app-default")
                .into(),
        }
    }
}

fn syntax_theme(id: &str, dark: bool, theme: &ChromeTheme) -> Option<syntect::highlighting::Theme> {
    use syntect::highlighting::{Color, FontStyle, StyleModifier, ThemeItem, ThemeSettings};
    let preset = PRESETS.iter().find(|preset| preset.id == id)?;
    let variant = if dark {
        preset.dark.as_ref()?
    } else {
        preset.light.as_ref()?
    };
    let parse_color = |s: &str| {
        u32::from_str_radix(s.strip_prefix('#')?, 16)
            .ok()
            .filter(|_| s.len() == 7)
            .map(|c| Color {
                r: (c >> 16) as u8,
                g: (c >> 8) as u8,
                b: c as u8,
                a: 255,
            })
    };
    let mut scopes = Vec::new();
    for rule in variant.get("syntax")?.as_array()? {
        let names = match &rule["scope"] {
            Value::String(s) => s.clone(),
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            _ => continue,
        };
        let Ok(scope) = names.parse() else {
            continue;
        };
        let mut style = FontStyle::empty();
        for part in rule["settings"]["fontStyle"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
        {
            match part {
                "italic" => style |= FontStyle::ITALIC,
                "bold" => style |= FontStyle::BOLD,
                "underline" => style |= FontStyle::UNDERLINE,
                _ => {}
            }
        }
        scopes.push(ThemeItem {
            scope,
            style: StyleModifier {
                foreground: rule["settings"]["foreground"]
                    .as_str()
                    .and_then(parse_color),
                font_style: Some(style),
                ..Default::default()
            },
        });
    }
    Some(syntect::highlighting::Theme {
        settings: ThemeSettings {
            foreground: parse_color(&theme.ink),
            ..Default::default()
        },
        scopes,
        ..Default::default()
    })
}

#[derive(Default)]
pub(super) struct State {
    advanced: bool,
    separate: bool,
    submenu: bool,
    fields: Vec<(String, Entity<Composer>)>,
    import: Option<(bool, Entity<Composer>)>,
    color: Option<(bool, &'static str, Entity<Composer>)>,
    picker: Option<(bool, &'static str, f32)>,
    modal_input: Option<FocusHandle>,
    return_focus: Option<FocusHandle>,
    font_styles: std::collections::HashMap<String, Vec<textsys::FontFace>>,
    restore_focus: bool,
}

impl SettingsPane {
    fn chrome_theme(&self, dark: bool, cx: &App) -> ChromeTheme {
        serde_json::from_value(self.page.read(cx).get(theme_key(dark)).clone())
            .ok()
            .filter(ChromeTheme::valid)
            .unwrap_or_else(|| ChromeTheme::default_for(dark))
    }

    fn change_theme(
        &mut self,
        dark: bool,
        change: impl FnOnce(&mut ChromeTheme),
        cx: &mut Context<Self>,
    ) {
        let mut theme = self.chrome_theme(dark, cx);
        change(&mut theme);
        if theme.valid() {
            self.page
                .update(cx, |p, cx| p.set(theme_key(dark), json!(theme), cx));
        }
    }

    pub(super) fn appearance_pair_saved(&mut self, result: &PairSaved, cx: &mut Context<Self>) {
        if result.imported && result.success {
            self.appearance.import = None;
            self.appearance.restore_focus = true;
        }
        if result.success {
            self.appearance
                .fields
                .retain(|(key, _)| !key.starts_with("color-"));
        }
        cx.notify();
    }

    fn save_theme_pair(
        &self,
        dark: bool,
        theme: ChromeTheme,
        preset: String,
        imported: bool,
        cx: &mut Context<Self>,
    ) {
        self.page.update(cx, |_, cx| cx.emit(SettingChanged {
            path: PAIR, value: json!({ "dark": dark, "theme": theme, "preset": preset, "imported": imported }),
        }));
    }

    fn dock_picker(
        &self,
        dark: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.page.read(cx).choice(DOCK, "app-default").to_owned();
        div()
            .flex()
            .gap(px(8.))
            .children(
                [
                    (
                        "app-default",
                        "Use ChatGPT Dock icon",
                        "app/dock-appDefault.png",
                    ),
                    (
                        "codex-system",
                        "Use Codex Dock icon",
                        if dark {
                            "app/dock-codexDark.png"
                        } else {
                            "app/dock-codexLight.png"
                        },
                    ),
                ]
                .into_iter()
                .map(|(value, label, path)| {
                    action(
                        value,
                        img(path).size(px(40.)),
                        window,
                        cx,
                        move |this, _, cx| {
                            this.page
                                .update(cx, |page, cx| page.set(DOCK, json!(value), cx));
                        },
                    )
                    .aria_label(label)
                    .aria_selected(current == value)
                    .size(px(48.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if current == value { t.text } else { t.border })
                    .bg(t.surface)
                }),
            )
            .into_any_element()
    }

    fn mode_control(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let current = self.page.read(cx).choice(THEME, "system").to_owned();
        div()
            .flex()
            .gap(px(16.))
            .w(px(272.))
            .children(
                [("system", "System"), ("light", "Light"), ("dark", "Dark")]
                    .into_iter()
                    .map(|(value, label)| {
                        let art = if value == "system" {
                            div()
                                .flex()
                                .child(img("app/mode-system-light.svg").w(px(40.)).h(px(60.)))
                                .child(img("app/mode-system-dark.svg").w(px(40.)).h(px(60.)))
                                .into_any_element()
                        } else {
                            img(if value == "dark" {
                                "app/mode-dark.svg"
                            } else {
                                "app/mode-light.svg"
                            })
                            .w(px(80.))
                            .h(px(60.))
                            .into_any_element()
                        };
                        action(value, art, window, cx, move |this, window, cx| {
                            this.page
                                .update(cx, |page, cx| page.set(THEME, json!(value), cx));
                            window.dispatch_action(Box::new(SetTheme(value)), cx);
                        })
                        .aria_label(label)
                        .aria_selected(current == value)
                        .w(px(80.))
                        .h(px(60.))
                        .rounded(px(8.))
                        .overflow_hidden()
                        .border_2()
                        .border_color(if current == value {
                            t.focus_ring
                        } else {
                            t.border
                        })
                    }),
            )
            .into_any_element()
    }

    fn numeric_field(
        &mut self,
        key: &'static str,
        default: f64,
        min: f64,
        max: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field =
            if let Some((_, field)) = self.appearance.fields.iter().find(|(id, _)| id == key) {
                field.clone()
            } else {
                let value = self
                    .page
                    .read(cx)
                    .get(key)
                    .as_f64()
                    .unwrap_or(default)
                    .to_string();
                let field = cx.new(|cx| Composer::inline_field("", &value, 13., cx));
                let saved = field.clone();
                cx.on_blur(&field.focus_handle(cx), window, move |this, _, cx| {
                    this.save_number(key, default, min, max, &saved, cx);
                })
                .detach();
                self.appearance.fields.push((key.into(), field.clone()));
                field
            };
        let submit = field.clone();
        div()
            .w(px(64.))
            .h(px(28.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .child(field)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" {
                    this.save_number(key, default, min, max, &submit, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }

    fn save_number(
        &mut self,
        key: &'static str,
        default: f64,
        min: f64,
        max: f64,
        field: &Entity<Composer>,
        cx: &mut Context<Self>,
    ) {
        let saved = self.page.read(cx).get(key).as_f64().unwrap_or(default);
        let value = field
            .read(cx)
            .field_text()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && (min..=max).contains(n));
        let value = value.unwrap_or(saved);
        field.update(cx, |field, cx| field.set_text(&value.to_string(), cx));
        if saved != value {
            self.page.update(cx, |p, cx| p.set(key, json!(value), cx));
        }
    }

    fn preset_picker(
        &self,
        dark: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = preset_key(dark);
        let current = self.page.read(cx).choice(key, "codex");
        let label = PRESETS
            .iter()
            .find(|p| p.id == current)
            .map_or("ChatGPT", |p| p.label.as_str())
            .to_owned();
        let open = self.open_select == Some(key);
        let theme = self.chrome_theme(dark, cx);
        let preview = div()
            .size(px(20.))
            .rounded_full()
            .bg(color(&theme.surface))
            .border_1()
            .border_color(mix_color(color(&theme.surface), color(&theme.ink), 0.16))
            .flex()
            .items_center()
            .justify_center()
            .child(text("Aa", 10., 14., 600., color(&theme.accent)));
        let trigger = action(
            key,
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(preview)
                .child(text(label, 13., 18., 430., t.text))
                .child(icon("chevron-down", 12., t.text_secondary)),
            window,
            cx,
            move |this, _, cx| {
                this.open_select = (!open).then_some(key);
                this.appearance.submenu = false;
                cx.notify();
            },
        )
        .px(px(12.))
        .h(px(30.))
        .w(px(176.))
        .rounded_full()
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .relative()
        .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
            if open && !this.menu_hovered {
                this.open_select = None;
                this.appearance.submenu = false;
                cx.notify();
            }
        }));
        let menu = open.then(|| {
            let mut rows = vec![self.preset_row("codex", dark, t, window, cx)];
            rows.push(
                action(
                    "codex-themes",
                    div()
                        .flex()
                        .justify_between()
                        .child(text("Codex themes", 14., 20., 500., t.text))
                        .child(icon("thread-chevron-right", 16., t.text_secondary)),
                    window,
                    cx,
                    |this, _, cx| {
                        this.appearance.submenu = !this.appearance.submenu;
                        cx.notify();
                    },
                )
                .px(px(8.))
                .py(px(6.))
                .into_any_element(),
            );
            let submenu = self.appearance.submenu.then(|| {
                let options = PRESETS
                    .iter()
                    .filter(|p| {
                        p.id != "codex"
                            && if dark {
                                p.dark.is_some()
                            } else {
                                p.light.is_some()
                            }
                    })
                    .map(|p| self.preset_row(&p.id, dark, t, window, cx))
                    .collect::<Vec<_>>();
                deferred(
                    div().absolute().right(px(288.)).top(px(34.)).child(
                        popover(t, 280., |d| {
                            d.child(
                                div()
                                    .id("preset-options")
                                    .max_h(px(320.))
                                    .overflow_y_scroll()
                                    .children(options),
                            )
                        })
                        .id("appearance-theme-submenu")
                        .on_hover(cx.listener(|this, over: &bool, _, _| this.menu_hovered = *over)),
                    ),
                )
                .into_any_element()
            });
            popup(
                popover(t, 280., |d| d.children(rows).children(submenu))
                    .id("appearance-theme-menu")
                    .on_hover(cx.listener(|this, over: &bool, _, _| this.menu_hovered = *over)),
            )
        });
        trigger.children(menu).into_any_element()
    }

    fn preset_row(
        &self,
        id: &str,
        dark: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let preset = PRESETS
            .iter()
            .find(|p| p.id == id)
            .expect("registered preset");
        let id = preset.id.clone();
        let checked = self.page.read(cx).choice(preset_key(dark), "codex") == id;
        action(
            SharedString::from(format!("preset-{id}")),
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(text(preset.label.clone(), 14., 20., 430., t.text))
                .when(checked, |d| d.child(icon("menu-check", 14., t.text))),
            window,
            cx,
            move |this, _, cx| {
                let preset = PRESETS
                    .iter()
                    .find(|p| p.id == id)
                    .expect("registered preset");
                if let Some(theme) = preset.theme(dark, &this.chrome_theme(dark, cx)) {
                    this.save_theme_pair(dark, theme, id.clone(), false, cx);
                    this.open_select = None;
                    this.appearance.submenu = false;
                    cx.notify();
                }
            },
        )
        .px(px(8.))
        .py(px(6.))
        .rounded(px(8.))
        .hover(|s| s.bg(t.menu_item_hover))
        .into_any_element()
    }

    fn color_control(
        &mut self,
        dark: bool,
        role: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.chrome_theme(dark, cx);
        let value = match role {
            "accent" => theme.accent,
            "surface" => theme.surface,
            _ => theme.ink,
        };
        let key = format!("color-{dark}-{role}");
        let field =
            if let Some((_, field)) = self.appearance.fields.iter().find(|(id, _)| id == &key) {
                field.clone()
            } else {
                let field =
                    cx.new(|cx| Composer::inline_field("#000000", &value.to_uppercase(), 12., cx));
                let mut previous = value.to_uppercase();
                cx.observe(&field, move |this, field, cx| {
                    let input = field.read(cx).field_text().to_owned();
                    if input == previous {
                        return;
                    }
                    previous = input.clone();
                    let filtered: String = input
                        .chars()
                        .filter(char::is_ascii_hexdigit)
                        .take(6)
                        .collect();
                    let normalized = format!("#{filtered}").to_uppercase();
                    if input != normalized {
                        field.update(cx, |field, cx| field.set_text(&normalized, cx));
                    }
                    if valid_color(&normalized) {
                        this.set_color(dark, role, normalized.to_lowercase(), cx);
                    }
                })
                .detach();
                let saved = field.clone();
                cx.on_blur(&field.focus_handle(cx), window, move |this, _, cx| {
                    let theme = this.chrome_theme(dark, cx);
                    let value = match role {
                        "accent" => theme.accent,
                        "surface" => theme.surface,
                        _ => theme.ink,
                    };
                    saved.update(cx, |field, cx| field.set_text(&value.to_uppercase(), cx));
                })
                .detach();
                self.appearance.fields.push((key.clone(), field.clone()));
                field
            };
        let active = self
            .appearance
            .picker
            .is_some_and(|(d, r, _)| d == dark && r == role);
        let swatch = action(
            SharedString::from(key),
            div()
                .size(px(14.))
                .rounded_full()
                .border_1()
                .border_color(t.border)
                .bg(color(&value)),
            window,
            cx,
            move |this, _, cx| {
                this.appearance.picker = (!active).then(|| (dark, role, hsv(&value).0));
                cx.notify();
            },
        );
        let picker = active.then(|| popup(self.color_picker(dark, role, t, window, cx)));
        div()
            .relative()
            .w(px(96.))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_full()
            .border_1()
            .border_color(t.border)
            .child(swatch)
            .child(div().flex_1().min_w_0().child(field))
            .children(picker)
            .into_any_element()
    }

    fn set_color(&mut self, dark: bool, role: &'static str, value: String, cx: &mut Context<Self>) {
        let theme = self.chrome_theme(dark, cx);
        let old = match role {
            "accent" => &theme.accent,
            "surface" => &theme.surface,
            _ => &theme.ink,
        };
        if *old == value {
            return;
        }
        self.change_theme(
            dark,
            |theme| match role {
                "accent" => {
                    theme.accent = value.clone();
                    theme.accent_source = Some("custom".into());
                }
                "surface" => theme.surface = value.clone(),
                _ => theme.ink = value.clone(),
            },
            cx,
        );
        let key = format!("color-{dark}-{role}");
        if let Some((_, field)) = self.appearance.fields.iter().find(|(id, _)| id == &key) {
            field.update(cx, |field, cx| field.set_text(&value.to_uppercase(), cx));
        }
    }

    fn color_picker(
        &self,
        dark: bool,
        role: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.chrome_theme(dark, cx);
        let value = match role {
            "accent" => theme.accent,
            "surface" => theme.surface,
            _ => theme.ink,
        };
        let (_, saturation, brightness) = hsv(&value);
        let hue = self.appearance.picker.map_or(0., |(_, _, hue)| hue);
        let area = Rc::new(Cell::new(Bounds::default()));
        let probe_area = area.clone();
        let area_down = area.clone();
        let area_move = area;
        let square_focus = focus::handle("appearance-saturation", 0, window, cx)
            .read(cx)
            .handle
            .clone();
        let square = div()
            .id("color-saturation")
            .track_focus(&square_focus)
            .w(px(136.))
            .h(px(100.))
            .relative()
            .role(gpui::Role::Slider)
            .focusable()
            .tab_index(0)
            .aria_label("Color")
            .aria_value(format!(
                "Saturation {:.0}%, Brightness {:.0}%",
                saturation * 100.,
                brightness * 100.
            ))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let (s, v) = match event.keystroke.key.as_str() {
                    "left" => (saturation - 0.01, brightness),
                    "right" => (saturation + 0.01, brightness),
                    "down" => (saturation, brightness - 0.01),
                    "up" => (saturation, brightness + 0.01),
                    _ => return,
                };
                this.set_color(dark, role, hsv_hex(hue, s, v), cx);
                cx.stop_propagation();
            }))
            .bg(color(&hsv_hex(hue, 1., 1.)))
            .rounded_t(px(8.))
            .child(div().absolute().size_full().bg(gpui::linear_gradient(
                90.,
                gpui::linear_color_stop(gpui::rgba(0xffffffff), 0.),
                gpui::linear_color_stop(gpui::rgba(0xffffff00), 1.),
            )))
            .child(div().absolute().size_full().bg(gpui::linear_gradient(
                180.,
                gpui::linear_color_stop(gpui::rgba(0x00000000), 0.),
                gpui::linear_color_stop(gpui::rgba(0x000000ff), 1.),
            )))
            .child(
                canvas(move |b, _, _| probe_area.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(saturation * 136. - 8.))
                    .top(px((1. - brightness) * 100. - 8.))
                    .size(px(16.))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::rgb(0xffffff))
                    .bg(color(&value)),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    square_focus.focus(window, cx);
                    let b = area_down.get();
                    let s = (event.position.x - b.origin.x) / b.size.width;
                    let v = 1. - (event.position.y - b.origin.y) / b.size.height;
                    this.set_color(dark, role, hsv_hex(hue, s, v), cx);
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if event.pressed_button != Some(gpui::MouseButton::Left) {
                        return;
                    }
                    let b = area_move.get();
                    let s = (event.position.x - b.origin.x) / b.size.width;
                    let v = 1. - (event.position.y - b.origin.y) / b.size.height;
                    this.set_color(dark, role, hsv_hex(hue, s, v), cx);
                }),
            );
        let hue_bounds = Rc::new(Cell::new(Bounds::default()));
        let hue_probe = hue_bounds.clone();
        let hue_down = hue_bounds.clone();
        let hue_move = hue_bounds;
        let hue_focus = focus::handle("appearance-hue", 0, window, cx)
            .read(cx)
            .handle
            .clone();
        let hues = div()
            .id("color-hue")
            .track_focus(&hue_focus)
            .h(px(24.))
            .w(px(136.))
            .flex()
            .relative()
            .role(gpui::Role::Slider)
            .focusable()
            .tab_index(0)
            .aria_label("Hue")
            .aria_numeric_value(hue as f64)
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(360.)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let value = match event.keystroke.key.as_str() {
                    "left" | "down" => hue - 1.,
                    "right" | "up" => hue + 1.,
                    "home" => 0.,
                    "end" => 360.,
                    _ => return,
                }
                .clamp(0., 360.);
                this.appearance.picker = Some((dark, role, value));
                this.set_color(dark, role, hsv_hex(value, saturation, brightness), cx);
                cx.notify();
                cx.stop_propagation();
            }))
            .children((0..6).map(|i| {
                div().flex_1().h_full().bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(color(&hsv_hex(i as f32 * 60., 1., 1.)), 0.),
                    gpui::linear_color_stop(color(&hsv_hex((i + 1) as f32 * 60., 1., 1.)), 1.),
                ))
            }))
            .child(
                canvas(move |b, _, _| hue_probe.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(hue / 360. * 136. - 8.))
                    .top(px(4.))
                    .size(px(16.))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::rgb(0xffffff))
                    .bg(color(&hsv_hex(hue, 1., 1.))),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    hue_focus.focus(window, cx);
                    let b = hue_down.get();
                    let hue =
                        (((event.position.x - b.origin.x) / b.size.width) * 360.).clamp(0., 360.);
                    this.appearance.picker = Some((dark, role, hue));
                    this.set_color(dark, role, hsv_hex(hue, saturation, brightness), cx);
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if event.pressed_button != Some(gpui::MouseButton::Left) {
                        return;
                    }
                    let b = hue_move.get();
                    let hue =
                        (((event.position.x - b.origin.x) / b.size.width) * 360.).clamp(0., 360.);
                    this.appearance.picker = Some((dark, role, hue));
                    this.set_color(dark, role, hsv_hex(hue, saturation, brightness), cx);
                    cx.notify();
                }),
            );
        div()
            .id("appearance-color-popup")
            .p(px(12.))
            .rounded(px(12.))
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .child(square)
            .child(div().h(px(12.)).bg(gpui::rgb(0)))
            .child(hues)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.appearance.picker = None;
                cx.notify();
            }))
            .into_any_element()
    }

    fn font_style_control(
        &mut self,
        dark: bool,
        role: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.chrome_theme(dark, cx);
        let (family, selected) = match role {
            "ui" => (theme.fonts.ui, theme.fonts.ui_face),
            "content" => (theme.fonts.content, theme.fonts.content_face),
            _ => (theme.fonts.code, theme.fonts.code_face),
        };
        let Some(family) = family else {
            return select("Regular", None, true, t).into_any_element();
        };
        let faces = self
            .appearance
            .font_styles
            .entry(family.clone())
            .or_insert_with(|| {
                family
                    .split(',')
                    .map(|name| textsys::font_faces(name.trim().trim_matches(['\'', '"'])))
                    .find(|faces| !faces.is_empty())
                    .unwrap_or_default()
            });
        if faces.is_empty() {
            return select("Regular", None, true, t).into_any_element();
        }
        let label = selected
            .as_ref()
            .and_then(|selected| {
                faces
                    .iter()
                    .find(|face| face.postscript_name == selected.postscript_name)
            })
            .unwrap_or(&faces[0])
            .style
            .clone();
        let key = match (dark, role) {
            (true, "ui") => "style-dark-ui",
            (false, "ui") => "style-light-ui",
            (true, "content") => "style-dark-content",
            (false, "content") => "style-light-content",
            (true, _) => "style-dark-code",
            (false, _) => "style-light-code",
        };
        let open = self.open_select == Some(key);
        let menu = open.then(|| {
            let rows = faces
                .iter()
                .map(|face| {
                    let choice = FontFace {
                        family: face.family.clone(),
                        full_name: face.full_name.clone(),
                        postscript_name: face.postscript_name.clone(),
                    };
                    action(
                        SharedString::from(format!("style-{}", face.postscript_name)),
                        text(face.style.clone(), 14., 20., 430., t.text),
                        window,
                        cx,
                        move |this, _, cx| {
                            this.change_theme(
                                dark,
                                |theme| set_face(&mut theme.fonts, role, Some(choice.clone())),
                                cx,
                            );
                            this.open_select = None;
                            cx.notify();
                        },
                    )
                    .px(px(8.))
                    .py(px(6.))
                    .into_any_element()
                })
                .collect::<Vec<_>>();
            popup(popover(t, 220., |d| d.children(rows)))
        });
        action(
            key,
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(text(label, 13., 18., 430., t.text))
                .child(icon("chevron-down", 12., t.text_secondary)),
            window,
            cx,
            move |this, _, cx| {
                this.open_select = (!open).then_some(key);
                cx.notify();
            },
        )
        .h(px(30.))
        .px(px(12.))
        .rounded_full()
        .border_1()
        .border_color(t.border)
        .relative()
        .children(menu)
        .into_any_element()
    }

    fn font_control(
        &self,
        dark: bool,
        role: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.chrome_theme(dark, cx);
        let family = match role {
            "ui" => theme.fonts.ui,
            "content" => theme.fonts.content,
            _ => theme.fonts.code,
        };
        let value = family
            .as_deref()
            .unwrap_or(if role == "content" {
                "Same as UI font"
            } else {
                "System"
            })
            .to_owned();
        let key = match (dark, role) {
            (true, "ui") => "font-dark-ui",
            (false, "ui") => "font-light-ui",
            (true, "content") => "font-dark-content",
            (false, "content") => "font-light-content",
            (true, _) => "font-dark-code",
            (false, _) => "font-light-code",
        };
        let open = self.open_select == Some(key);
        let menu =
            open.then(|| {
                let default_label = if role == "content" {
                    "Same as UI font"
                } else {
                    "System"
                };
                let mut names = cx.text_system().all_font_names();
                if role == "code" {
                    names.retain(|name| {
                        let font = gpui::font(SharedString::from(name.clone()));
                        let id = cx.text_system().resolve_font(&font);
                        let Ok(i) = cx.text_system().advance(id, px(14.), 'i') else {
                            return false;
                        };
                        let Ok(w) = cx.text_system().advance(id, px(14.), 'W') else {
                            return false;
                        };
                        (i.width - w.width).abs() < px(0.01)
                    });
                }
                let mut rows =
                    vec![self.font_option(default_label.into(), None, dark, role, t, window, cx)];
                rows.extend(names.into_iter().map(|name| {
                    self.font_option(name.clone(), Some(name), dark, role, t, window, cx)
                }));
                rows.push(
                    action(
                        "custom-font",
                        text("Use custom font value", 14., 20., 430., t.text),
                        window,
                        cx,
                        move |this, window, cx| {
                            let field = cx.new(|cx| {
                                Composer::settings_field(
                                    "Font family",
                                    family.as_deref().unwrap_or(""),
                                    false,
                                    cx,
                                )
                            });
                            this.prepare_modal(&field, window, cx);
                            this.appearance.color = Some((dark, role, field));
                            this.open_select = None;
                            cx.notify();
                        },
                    )
                    .px(px(8.))
                    .py(px(6.))
                    .into_any_element(),
                );
                popup(
                    popover(t, 280., |d| {
                        d.child(
                            div()
                                .id("font-options")
                                .max_h(px(320.))
                                .overflow_y_scroll()
                                .children(rows),
                        )
                    })
                    .id("appearance-font-menu")
                    .on_hover(cx.listener(|this, over: &bool, _, _| this.menu_hovered = *over)),
                )
            });
        action(
            key,
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(text(value, 13., 18., 430., t.text))
                .child(icon("chevron-down", 12., t.text_secondary)),
            window,
            cx,
            move |this, _, cx| {
                this.open_select = (!open).then_some(key);
                cx.notify();
            },
        )
        .px(px(12.))
        .h(px(30.))
        .max_w(px(240.))
        .rounded_full()
        .border_1()
        .border_color(t.border)
        .relative()
        .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
            if open && !this.menu_hovered {
                this.open_select = None;
                cx.notify();
            }
        }))
        .children(menu)
        .into_any_element()
    }

    fn font_option(
        &self,
        label: String,
        family: Option<String>,
        dark: bool,
        role: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        action(
            SharedString::from(format!("font-{label}")),
            text(label, 14., 20., 430., t.text),
            window,
            cx,
            move |this, _, cx| {
                let face = family
                    .as_deref()
                    .and_then(|family| textsys::font_faces(family).into_iter().next())
                    .map(|face| FontFace {
                        family: face.family,
                        full_name: face.full_name,
                        postscript_name: face.postscript_name,
                    });
                this.change_theme(
                    dark,
                    |theme| {
                        set_font(&mut theme.fonts, role, family.clone());
                        set_face(&mut theme.fonts, role, face);
                    },
                    cx,
                );
                this.open_select = None;
                cx.notify();
            },
        )
        .px(px(8.))
        .py(px(6.))
        .rounded(px(8.))
        .hover(|s| s.bg(t.menu_item_hover))
        .into_any_element()
    }

    fn contrast(
        &self,
        dark: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.chrome_theme(dark, cx);
        let value = theme.contrast;
        let id = if dark {
            "contrast-dark"
        } else {
            "contrast-light"
        };
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let paint_bounds = bounds.clone();
        let click_bounds = bounds.clone();
        let drag_bounds = bounds.clone();
        let contrast_focus = focus::handle(id, 0, window, cx).read(cx).handle.clone();
        let slider = div()
            .id(id)
            .track_focus(&contrast_focus)
            .w(px(148.))
            .h(px(36.))
            .relative()
            .focusable()
            .tab_index(0)
            .aria_label("Contrast")
            .child(
                canvas(
                    move |b, _, _| {
                        paint_bounds.set(b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(15.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(color(&theme.accent)),
            )
            .child(
                div()
                    .absolute()
                    .left(px((148. - 14.) * value as f32 / 100.))
                    .top(px(11.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(t.text)
                    .border_1()
                    .border_color(t.border),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    contrast_focus.focus(window, cx);
                    let b = click_bounds.get();
                    let amount: f32 = ((event.position.x - b.origin.x) / b.size.width).into();
                    this.change_theme(
                        dark,
                        |theme| theme.contrast = (amount.clamp(0., 1.) * 100.).round() as u8,
                        cx,
                    );
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if event.pressed_button != Some(gpui::MouseButton::Left) {
                        return;
                    }
                    let b = drag_bounds.get();
                    let amount: f32 = ((event.position.x - b.origin.x) / b.size.width).into();
                    this.change_theme(
                        dark,
                        |theme| theme.contrast = (amount.clamp(0., 1.) * 100.).round() as u8,
                        cx,
                    );
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let next = match event.keystroke.key.as_str() {
                    "left" | "down" => value.saturating_sub(1),
                    "right" | "up" => value.saturating_add(1).min(100),
                    "home" => 0,
                    "end" => 100,
                    _ => return,
                };
                this.change_theme(dark, |theme| theme.contrast = next, cx);
                cx.stop_propagation();
            }));
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(slider)
            .child(
                div()
                    .w(px(36.))
                    .child(text(value.to_string(), 13., 18., 430., t.text)),
            )
            .into_any_element()
    }

    fn variant_card(
        &mut self,
        dark: bool,
        advanced: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let title = if self.appearance.separate {
            if dark { "Dark theme" } else { "Light theme" }
        } else {
            "Theme"
        };
        let mut rows = Vec::new();
        if !advanced {
            let copy = action(
                if dark { "copy-dark" } else { "copy-light" },
                icon("appearance-copy", 16., t.text_secondary),
                window,
                cx,
                move |this, _, cx| {
                    let theme = this.chrome_theme(dark, cx);
                    let preset = this.page.read(cx).choice(preset_key(dark), "codex");
                    let value = share_string(&theme, preset, dark);
                    this.page.update(cx, |_, cx| {
                        cx.emit(SettingChanged {
                            path: COPY,
                            value: json!({ "dark": dark, "value": value }),
                        })
                    });
                },
            );
            let import = action(
                if dark { "import-dark" } else { "import-light" },
                icon("appearance-import", 16., t.text_secondary),
                window,
                cx,
                move |this, window, cx| {
                    let field =
                        cx.new(|cx| Composer::settings_field("codex-theme-v1:", "", false, cx));
                    this.prepare_modal(&field, window, cx);
                    this.appearance.import = Some((dark, field));
                    cx.notify();
                },
            );
            rows.push(settings_row(
                title,
                None,
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(import)
                    .child(copy)
                    .child(self.preset_picker(dark, t, window, cx))
                    .into_any_element(),
                t,
            ));
            for (label, role) in [
                ("Accent", "accent"),
                ("Background", "surface"),
                ("Foreground", "ink"),
            ] {
                let control = self.color_control(dark, role, t, window, cx);
                rows.push(settings_row(label, None, control, t));
            }
            rows.push(settings_row(
                "Font",
                None,
                self.font_control(dark, "ui", t, window, cx),
                t,
            ));
        } else {
            if self.appearance.separate {
                rows.push(settings_row(title, None, div().into_any_element(), t));
            }
            let style = self.font_style_control(dark, "ui", t, window, cx);
            rows.push(settings_row("UI font style", None, style, t));
            for (label, role) in [("Content font", "content"), ("Code font", "code")] {
                let family = self.font_control(dark, role, t, window, cx);
                let style = self.font_style_control(dark, role, t, window, cx);
                rows.push(settings_row(
                    label,
                    None,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(family)
                        .child(style)
                        .into_any_element(),
                    t,
                ));
            }
            let translucent = !self.chrome_theme(dark, cx).opaque_windows;
            rows.push(settings_row(
                "Translucent sidebar",
                None,
                action(
                    if dark {
                        "translucent-dark"
                    } else {
                        "translucent-light"
                    },
                    switch("translucent", translucent, false, t),
                    window,
                    cx,
                    move |this, _, cx| {
                        this.change_theme(dark, |theme| theme.opaque_windows = translucent, cx)
                    },
                )
                .into_any_element(),
                t,
            ));
            rows.push(settings_row(
                "Contrast",
                None,
                self.contrast(dark, t, window, cx),
                t,
            ));
        }
        let _ = window;
        self.appearance_card(rows, t)
    }

    fn appearance_card(&self, rows: Vec<Div>, t: &Theme) -> Div {
        let count = rows.len();
        div()
            .relative()
            .p(px(1.))
            .flex()
            .flex_col()
            .child(deco(
                &self.layer,
                20.,
                Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(rounded_clip(
                px(19.),
                Squircle,
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        row.when(index + 1 < count, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .bottom_0()
                                    .h(px(1.))
                                    .bg(over(t.border, t.background_panel)),
                            )
                        })
                    })),
            ))
    }

    fn reset_appearance(&mut self, cx: &mut Context<Self>) {
        self.appearance.separate = false;
        self.appearance.fields.clear();
        self.page.update(cx, |p, cx| {
            for (key, value) in [
                (UI_SIZE, json!(14)),
                (CODE_SIZE, json!(12)),
                (MOTION, json!("system")),
                (MARKERS, json!("color")),
                (POINTER, json!(false)),
                (SMOOTHING, json!(true)),
                (DOCK, json!("app-default")),
            ] {
                p.set(key, value, cx);
            }
        });
        for dark in [false, true] {
            self.change_theme(
                dark,
                |theme| {
                    theme.contrast = if dark { 60 } else { 45 };
                    theme.opaque_windows = false;
                    theme.fonts.code = None;
                    theme.fonts.content = None;
                    theme.fonts.code_face = None;
                    theme.fonts.content_face = None;
                    theme.fonts.ui_face = None;
                },
                cx,
            );
        }
    }

    pub(super) fn render_appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.appearance.restore_focus {
            self.dismiss_modal(window, cx);
        }
        let t = cx.theme().clone();
        let dark = t.appearance == theme::Appearance::Dark;
        let variants = if self.appearance.separate {
            vec![false, true]
        } else {
            vec![dark]
        };
        let mode = self.mode_control(&t, window, cx);
        let mut visual = vec![self.appearance_card(vec![settings_row("Mode", None, mode, &t)], &t)];
        visual.extend(
            variants
                .iter()
                .map(|&dark| self.variant_card(dark, false, &t, window, cx)),
        );
        let advanced = self.appearance.advanced;
        let mut groups = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(text("Visual style", 14., 21., 500., t.text))
                .children(visual),
        ];
        let reset_visible = {
            let page = self.page.read(cx);
            self.appearance.separate
                || page.get(UI_SIZE).as_f64().unwrap_or(14.) != 14.
                || page.get(CODE_SIZE).as_f64().unwrap_or(12.) != 12.
                || page.choice(MOTION, "system") != "system"
                || page.choice(MARKERS, "color") != "color"
                || page.flag(POINTER, false)
                || !page.flag(SMOOTHING, true)
                || page.choice(DOCK, "app-default") != "app-default"
                || [false, true].into_iter().any(|dark| {
                    let theme = self.chrome_theme(dark, cx);
                    theme.contrast != if dark { 60 } else { 45 }
                        || theme.opaque_windows
                        || theme.fonts.code.is_some()
                        || theme.fonts.content.is_some()
                        || theme.fonts.code_face.is_some()
                        || theme.fonts.content_face.is_some()
                        || theme.fonts.ui_face.is_some()
                })
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(action(
                "advanced-appearance",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(text("Advanced", 14., 21., 500., t.text))
                    .child(icon(
                        if advanced {
                            "appearance-chevron-down"
                        } else {
                            "appearance-chevron-right"
                        },
                        14.,
                        t.text_secondary,
                    )),
                window,
                cx,
                move |this, _, cx| {
                    this.appearance.advanced = !advanced;
                    cx.notify();
                },
            ))
            .children(reset_visible.then(|| {
                action(
                    "reset-appearance",
                    text("Reset to default", 14., 21., 430., t.text_secondary),
                    window,
                    cx,
                    |this, _, cx| this.reset_appearance(cx),
                )
            }));
        let mut extra = div().flex().flex_col().gap(px(16.)).child(header);
        if advanced {
            let ui_size = self.numeric_field(UI_SIZE, 14., 11., 16., window, cx);
            let code_size = self.numeric_field(CODE_SIZE, 12., 8., 24., window, cx);
            extra = extra.child(self.appearance_card(
                vec![
                    settings_row(
                        "UI font size",
                        Some("Adjust the base size used for ChatGPT"),
                        units(ui_size, &t),
                        &t,
                    ),
                    settings_row(
                        "Code font size",
                        Some("Adjust the base size used for code across chats and diffs"),
                        units(code_size, &t),
                        &t,
                    ),
                ],
                &t,
            ));
            let separate = self.appearance.separate;
            let separate_toggle = action(
                "separate-appearance-modes",
                switch("separate-modes", separate, false, &t),
                window,
                cx,
                move |this, _, cx| {
                    this.appearance.separate = !separate;
                    cx.notify();
                },
            )
            .into_any_element();
            extra = extra.child(self.appearance_card(
                vec![
                    settings_row(
                        "Reduce motion",
                        Some("Reduce animations or match your system"),
                        self.segmented(
                            MOTION,
                            "system",
                            &[("system", "System"), ("on", "On"), ("off", "Off")],
                            &t,
                            cx,
                        ),
                        &t,
                    ),
                    settings_row(
                        "Separate light and dark modes",
                        Some("Choose a theme, colors, and font for each"),
                        separate_toggle,
                        &t,
                    ),
                ],
                &t,
            ));
            for variant in &variants {
                extra = extra.child(self.variant_card(*variant, true, &t, window, cx));
            }
            let mut final_rows = vec![
                settings_row(
                    "Diff markers",
                    Some("Show changes using colors or +/− markers"),
                    self.segmented(
                        MARKERS,
                        "color",
                        &[("color", "Color"), ("symbols", "+/−")],
                        &t,
                        cx,
                    ),
                    &t,
                ),
                settings_row(
                    "Use pointer cursors",
                    Some("Change the cursor to a pointer when hovering over interactive elements"),
                    self.toggle(POINTER, false, &t, window, cx),
                    &t,
                ),
            ];
            if cfg!(target_os = "macos") {
                let system_dark = matches!(
                    window.appearance(),
                    gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
                );
                let dock = self.dock_picker(system_dark, &t, window, cx);
                final_rows.push(settings_row(
                    "Dock icon",
                    Some("Choose the icon the app will use in the dock"),
                    dock,
                    &t,
                ));
                final_rows.push(settings_row(
                    "Font smoothing",
                    Some("Use native macOS font anti-aliasing"),
                    self.toggle(SMOOTHING, true, &t, window, cx),
                    &t,
                ));
            }
            extra = extra.child(self.appearance_card(final_rows, &t));
        }
        groups.push(extra);
        let modal = self.appearance_modal(&t, window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("appearance-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .mx_auto()
                            .px(px(20.))
                            .pt(px(28.))
                            .pb(px(20.))
                            .child(text("Appearance", 28., 33.6, 500., t.text)),
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
                            .children(groups),
                    ),
            )
            .children(modal)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.open_select = None;
                    this.appearance.submenu = false;
                    this.appearance.picker = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }

    fn prepare_modal(
        &mut self,
        field: &Entity<Composer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance.return_focus = window.focused(cx);
        let input = field.focus_handle(cx);
        self.appearance.modal_input = Some(input.clone());
        cx.observe(field, |_, _, cx| cx.notify()).detach();
        input.focus(window, cx);
    }

    fn dismiss_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.appearance.import = None;
        self.appearance.color = None;
        self.appearance.modal_input = None;
        self.appearance.restore_focus = false;
        if let Some(focus) = self.appearance.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn submit_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((dark, field)) = self.appearance.import.clone() {
            if let Ok((theme, preset)) = import_string(field.read(cx).field_text(), dark) {
                self.save_theme_pair(dark, theme, preset, true, cx);
            }
        } else if let Some((dark, role, field)) = self.appearance.color.clone() {
            let value = field.read(cx).field_text().trim().to_owned();
            self.change_theme(
                dark,
                |theme| set_font(&mut theme.fonts, role, (!value.is_empty()).then_some(value)),
                cx,
            );
            self.dismiss_modal(window, cx);
        }
    }

    fn appearance_modal(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let controls = [
            self.appearance.modal_input.clone()?,
            focus::handle("cancel-appearance-dialog", 0, window, cx)
                .read(cx)
                .handle
                .clone(),
            focus::handle("submit-appearance-dialog", 0, window, cx)
                .read(cx)
                .handle
                .clone(),
        ];
        let (title, field, enabled) = if let Some((dark, field)) = &self.appearance.import {
            (
                "Import theme",
                field.clone(),
                import_string(field.read(cx).field_text(), *dark).is_ok(),
            )
        } else if let Some((_, _, field)) = &self.appearance.color {
            ("Custom font", field.clone(), true)
        } else {
            return None;
        };
        let cancel = action(
            "cancel-appearance-dialog",
            text("Cancel", 14., 20., 430., t.text),
            window,
            cx,
            |this, window, cx| this.dismiss_modal(window, cx),
        )
        .track_focus(&controls[1])
        .px(px(12.))
        .py(px(8.));
        let save = action(
            "submit-appearance-dialog",
            text(
                if title == "Import theme" {
                    "Import theme"
                } else {
                    "Apply"
                },
                14.,
                20.,
                500.,
                t.text_inverse,
            ),
            window,
            cx,
            move |this, window, cx| {
                if enabled {
                    this.submit_appearance(window, cx);
                }
            },
        )
        .track_focus(&controls[2])
        .aria_disabled(!enabled)
        .opacity(if enabled { 1. } else { 0.5 })
        .px(px(12.))
        .py(px(8.))
        .rounded_full()
        .bg(t.button_primary_solid);
        let tab_controls = controls.clone();
        Some(
            div()
                .id("appearance-dialog-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.dialog_backdrop)
                .capture_action(
                    cx.listener(|this, _: &crate::composer::Submit, window, cx| {
                        this.submit_appearance(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .capture_action(
                    cx.listener(|this, _: &crate::composer::Dismiss, window, cx| {
                        this.dismiss_modal(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .capture_action(cx.listener(
                    move |_, _: &crate::composer::CompleteCommand, window, cx| {
                        let current = tab_controls
                            .iter()
                            .position(|f| f.is_focused(window))
                            .unwrap_or(0);
                        tab_controls[(current + 1) % 3].focus(window, cx);
                        cx.stop_propagation();
                    },
                ))
                .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => this.dismiss_modal(window, cx),
                        "tab" => {
                            let current = controls
                                .iter()
                                .position(|f| f.is_focused(window))
                                .unwrap_or(0);
                            let next = if event.keystroke.modifiers.shift {
                                (current + 2) % 3
                            } else {
                                (current + 1) % 3
                            };
                            controls[next].focus(window, cx);
                        }
                        "enter" if controls[1].is_focused(window) => this.dismiss_modal(window, cx),
                        "enter" => this.submit_appearance(window, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                }))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.dismiss_modal(window, cx)),
                )
                .child(
                    div()
                        .id("appearance-dialog")
                        .role(gpui::Role::Dialog)
                        .aria_label(title)
                        .w(px(480.))
                        .p(px(24.))
                        .rounded(px(24.))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .child(text(title, 20., 26., 600., t.text))
                        .child(field)
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(8.))
                                .child(cancel)
                                .child(save),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn theme_key(dark: bool) -> &'static str {
    if dark { DARK } else { LIGHT }
}
fn preset_key(dark: bool) -> &'static str {
    if dark { DARK_PRESET } else { LIGHT_PRESET }
}

fn set_font(fonts: &mut Fonts, role: &str, value: Option<String>) {
    match role {
        "ui" => {
            fonts.ui = value;
            fonts.ui_face = None;
        }
        "content" => {
            fonts.content = value;
            fonts.content_face = None;
        }
        _ => {
            fonts.code = value;
            fonts.code_face = None;
        }
    }
}

fn set_face(fonts: &mut Fonts, role: &str, face: Option<FontFace>) {
    match role {
        "ui" => fonts.ui_face = face,
        "content" => fonts.content_face = face,
        _ => fonts.code_face = face,
    }
}

fn units(field: AnyElement, t: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(field)
        .child(text("px", 13., 18., 430., t.text_secondary))
        .into_any_element()
}

fn popup(content: impl IntoElement) -> AnyElement {
    deferred(
        div().absolute().top_0().right_0().child(
            anchored()
                .anchor(Anchor::TopRight)
                .offset(point(px(0.), px(34.)))
                .child(content),
        ),
    )
    .into_any_element()
}

fn action(
    id: impl Into<gpui::ElementId>,
    content: impl IntoElement,
    window: &mut Window,
    cx: &mut Context<SettingsPane>,
    callback: impl Fn(&mut SettingsPane, &mut Window, &mut Context<SettingsPane>) + 'static,
) -> Stateful<Div> {
    let id = id.into();
    let control_focus = focus::handle(id.clone(), 0, window, cx);
    let handle = control_focus.read(cx).handle.clone();
    let callback = Rc::new(callback);
    let click = callback.clone();
    focus::ring(
        div()
            .id(id)
            .role(gpui::Role::Button)
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .child(content),
        &control_focus,
        px(8.),
        px(2.),
        cx,
    )
    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
        handle.focus(window, cx)
    })
    .on_click(cx.listener(move |this, _, window, cx| click(this, window, cx)))
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            callback(this, window, cx);
            cx.stop_propagation();
        }
    }))
}

#[cfg(test)]
mod appearance_tests {
    use super::*;

    #[test]
    fn custom_palette_matches_original_surface_derivation() {
        let mut custom = ChromeTheme::default_for(true);
        custom.surface = "#203040".into();
        custom.accent = "#e21d44".into();
        custom.accent_source = Some("custom".into());
        let mut palette = Theme::dark();
        custom.apply(&mut palette);
        assert_eq!(
            palette.surface_secondary,
            gpui::Hsla::from(gpui::rgb(0x1b2836))
        );
        assert_eq!(
            palette.surface_tertiary,
            gpui::Hsla::from(gpui::rgb(0x303e4d))
        );
        assert_eq!(
            palette.control_background,
            gpui::Hsla::from(gpui::rgb(0x344351))
        );
        assert_eq!(
            palette.background_panel,
            gpui::Hsla::from(gpui::rgb(0x2b3a49))
        );
        assert_eq!(palette.menu_background, palette.control_background);
    }

    #[test]
    fn shared_theme_preserves_values_and_rejects_wrong_variant() {
        let mut theme = ChromeTheme::default_for(true);
        theme.accent = "#e21d44".into();
        theme.contrast = 98;
        theme.fonts.ui = Some("Inter".into());
        let shared = share_string(&theme, "dracula", true);
        assert_eq!(import_string(&shared, true), Ok((theme, "dracula".into())));
        assert_eq!(import_string(&shared, false), Err("Theme variant mismatch"));
        assert!(import_string(&shared.replace("#e21d44", "red"), true).is_err());
        assert!(
            import_string(&shared.replace("\"contrast\":98", "\"contrast\":101"), true).is_err()
        );
        assert!(import_string("codex-theme-v1:%GG", true).is_err());
    }

    #[test]
    fn presets_replace_colors_without_erasing_custom_fonts() {
        let mut original = ChromeTheme::default_for(false);
        original.fonts.code = Some("Menlo".into());
        let linear = PRESETS.iter().find(|p| p.id == "linear").unwrap();
        let selected = linear.theme(false, &original).unwrap();
        assert_eq!(selected.fonts.ui.as_deref(), Some("Inter"));
        assert_eq!(selected.fonts.code, original.fonts.code);
        assert!(selected.valid());
        let dracula = PRESETS.iter().find(|p| p.id == "dracula").unwrap();
        assert!(dracula.theme(false, &original).is_none());
    }

    #[test]
    fn color_picker_preserves_greys_and_hue_endpoints() {
        for value in ["#000000", "#ffffff", "#808080", "#ff0000", "#0000ff"] {
            let (hue, saturation, brightness) = hsv(value);
            assert_eq!(hsv_hex(hue, saturation, brightness), value);
        }
        assert_eq!(hsv_hex(360., 1., 1.), "#ff0000");
        assert_eq!(hsv_hex(120., 2., -1.), "#000000");
    }
}

fn hsv(value: &str) -> (f32, f32, f32) {
    let color = color(value);
    let rgb: gpui::Rgba = color.into();
    let maximum = rgb.r.max(rgb.g).max(rgb.b);
    let minimum = rgb.r.min(rgb.g).min(rgb.b);
    (
        color.h * 360.,
        if maximum == 0. {
            0.
        } else {
            (maximum - minimum) / maximum
        },
        maximum,
    )
}

fn hsv_hex(hue: f32, saturation: f32, value: f32) -> String {
    let value = value.clamp(0., 1.);
    let chroma = value * saturation.clamp(0., 1.);
    let h = hue.rem_euclid(360.) / 60.;
    let x = chroma * (1. - (h % 2. - 1.).abs());
    let (r, g, b) = match h as u8 {
        0 => (chroma, x, 0.),
        1 => (x, chroma, 0.),
        2 => (0., chroma, x),
        3 => (0., x, chroma),
        4 => (x, 0., chroma),
        _ => (chroma, 0., x),
    };
    let channel = |c: f32| ((c + value - chroma) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b))
}
