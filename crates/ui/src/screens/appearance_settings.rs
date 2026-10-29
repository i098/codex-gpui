//! Settings ▸ Appearance (`general-settings` `qs`): Mode, the per-mode chrome theme (preset,
//! accent, background, foreground, font, share string) and the Advanced disclosure.
//!
//! Host keys, as the original names them: `appearance{Light,Dark}ChromeTheme` (JSON theme),
//! `appearance{Light,Dark}CodeThemeId`, `sansFontSize`, `codeFontSize`,
//! `reduced-motion-preference`, `appearanceDiffMarkerStyle`, `usePointerCursors` and
//! `useFontSmoothing`. A preset or an imported theme writes its theme and code theme id as one
//! batch, so a failed save keeps both previous values.

use super::*;
use crate::{
    composer::SettingsFieldSubmit,
    settings_kit::{Choice, Select, Selected},
    toast::Toast,
};
use gpui::{Bounds, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, canvas};

const SHARE_PREFIX: &str = "codex-theme-v1:";
const CODEX: &str = "codex";
const SANS_SIZE: (&str, f64, f64, f64) = ("host.sansFontSize", 14., 11., 16.);
const CODE_SIZE: (&str, f64, f64, f64) = ("host.codeFontSize", 12., 8., 24.);
const MOTION: &str = "host.reduced-motion-preference";
const DIFF_MARKERS: &str = "host.appearanceDiffMarkerStyle";
const POINTER: &str = "host.usePointerCursors";
const SMOOTHING: &str = "host.useFontSmoothing";
const DOCK_ICON: &str = "host.dock-icon-preference";
/// `isSystemBackdropSupported`: the translucent sidebar needs the macOS window backdrop.
const BACKDROP: bool = cfg!(target_os = "macos");

/// One `codeThemes` preset with the seed `mba` derives from its theme file for `variant`.
struct Preset {
    id: &'static str,
    label: &'static str,
    variant: &'static str,
    accent: &'static str,
    ink: &'static str,
    surface: &'static str,
    seed: &'static str,
}

/// Every preset seed, sorted by label as `Cya` sorts the menu. Extracted by running the original
/// bundle's `Dya(id, variant, "electron")` over its theme files.
#[rustfmt::skip]
const PRESETS: &[Preset] = &[
    Preset { id: "absolutely", label: "Absolutely", variant: "light", accent: "#cc7d5e", ink: "#2d2d2b", surface: "#f9f9f7", seed: r##"{"accent":"#cc7d5e","ink":"#2d2d2b","semanticColors":{"diffAdded":"#00c853","diffRemoved":"#ff5f38","skill":"#cc7d5e"},"surface":"#f9f9f7","accentSource":"custom"}"## },
    Preset { id: "absolutely", label: "Absolutely", variant: "dark", accent: "#cc7d5e", ink: "#f9f9f7", surface: "#2d2d2b", seed: r##"{"accent":"#cc7d5e","ink":"#f9f9f7","semanticColors":{"diffAdded":"#00c853","diffRemoved":"#ff5f38","skill":"#cc7d5e"},"surface":"#2d2d2b","accentSource":"custom"}"## },
Preset { id: "ayu", label: "Ayu", variant: "dark", accent: "#e6b450", ink: "#bfbdb6", surface: "#10141c", seed: r##"{"accent":"#e6b450","ink":"#bfbdb6","semanticColors":{"diffAdded":"#70bf56","diffRemoved":"#f26d78","skill":"#d0a1ff"},"surface":"#10141c","accentSource":"custom"}"## },
    Preset { id: "catppuccin", label: "Catppuccin", variant: "light", accent: "#8839ef", ink: "#4c4f69", surface: "#eff1f5", seed: r##"{"accent":"#8839ef","ink":"#4c4f69","semanticColors":{"diffAdded":"#40a02b","diffRemoved":"#d20f39","skill":"#8839ef"},"surface":"#eff1f5","accentSource":"custom"}"## },
    Preset { id: "catppuccin", label: "Catppuccin", variant: "dark", accent: "#cba6f7", ink: "#cdd6f4", surface: "#1e1e2e", seed: r##"{"accent":"#cba6f7","ink":"#cdd6f4","semanticColors":{"diffAdded":"#a6e3a1","diffRemoved":"#f38ba8","skill":"#cba6f7"},"surface":"#1e1e2e","accentSource":"custom"}"## },
    Preset { id: "codex", label: "Codex", variant: "light", accent: "#0169cc", ink: "#0d0d0d", surface: "#ffffff", seed: r##"{"accent":"#0169cc","ink":"#0d0d0d","semanticColors":{"diffAdded":"#00a240","diffRemoved":"#e02e2a","skill":"#751ed9"},"surface":"#ffffff","accentSource":"chatgpt"}"## },
    Preset { id: "codex", label: "Codex", variant: "dark", accent: "#0169cc", ink: "#fcfcfc", surface: "#111111", seed: r##"{"accent":"#0169cc","ink":"#fcfcfc","semanticColors":{"diffAdded":"#00a240","diffRemoved":"#e02e2a","skill":"#b06dff"},"surface":"#111111","accentSource":"chatgpt"}"## },
    Preset { id: "dracula", label: "Dracula", variant: "dark", accent: "#ff79c6", ink: "#f8f8f2", surface: "#282a36", seed: r##"{"accent":"#ff79c6","ink":"#f8f8f2","semanticColors":{"diffAdded":"#50fa7b","diffRemoved":"#ff5555","skill":"#ff79c6"},"surface":"#282a36","accentSource":"custom"}"## },
    Preset { id: "everforest", label: "Everforest", variant: "light", accent: "#93b259", ink: "#5c6a72", surface: "#fdf6e3", seed: r##"{"accent":"#93b259","ink":"#5c6a72","semanticColors":{"diffAdded":"#8da101","diffRemoved":"#f85552","skill":"#df69ba"},"surface":"#fdf6e3","accentSource":"custom"}"## },
    Preset { id: "everforest", label: "Everforest", variant: "dark", accent: "#a7c080", ink: "#d3c6aa", surface: "#2d353b", seed: r##"{"accent":"#a7c080","ink":"#d3c6aa","semanticColors":{"diffAdded":"#a7c080","diffRemoved":"#e67e80","skill":"#d699b6"},"surface":"#2d353b","accentSource":"custom"}"## },
    Preset { id: "github", label: "GitHub", variant: "light", accent: "#0969da", ink: "#1f2328", surface: "#ffffff", seed: r##"{"accent":"#0969da","ink":"#1f2328","semanticColors":{"diffAdded":"#1a7f37","diffRemoved":"#cf222e","skill":"#8250df"},"surface":"#ffffff","accentSource":"custom"}"## },
    Preset { id: "github", label: "GitHub", variant: "dark", accent: "#1f6feb", ink: "#e6edf3", surface: "#0d1117", seed: r##"{"accent":"#1f6feb","ink":"#e6edf3","semanticColors":{"diffAdded":"#3fb950","diffRemoved":"#f85149","skill":"#bc8cff"},"surface":"#0d1117","accentSource":"custom"}"## },
    Preset { id: "gruvbox", label: "Gruvbox", variant: "light", accent: "#458588", ink: "#3c3836", surface: "#fbf1c7", seed: r##"{"accent":"#458588","ink":"#3c3836","semanticColors":{"diffAdded":"#3c3836","diffRemoved":"#cc241d","skill":"#b16286"},"surface":"#fbf1c7","accentSource":"custom"}"## },
    Preset { id: "gruvbox", label: "Gruvbox", variant: "dark", accent: "#458588", ink: "#ebdbb2", surface: "#282828", seed: r##"{"accent":"#458588","ink":"#ebdbb2","semanticColors":{"diffAdded":"#ebdbb2","diffRemoved":"#cc241d","skill":"#b16286"},"surface":"#282828","accentSource":"custom"}"## },
    Preset { id: "linear", label: "Linear", variant: "light", accent: "#5e6ad2", ink: "#1b1b1b", surface: "#fcfcfd", seed: r##"{"accent":"#5e6ad2","ink":"#1b1b1b","semanticColors":{"diffAdded":"#52a450","diffRemoved":"#c94446","skill":"#8160d8"},"surface":"#fcfcfd","fonts":{"ui":"Inter"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "linear", label: "Linear", variant: "dark", accent: "#606acc", ink: "#e3e4e6", surface: "#0f0f11", seed: r##"{"accent":"#606acc","ink":"#e3e4e6","semanticColors":{"diffAdded":"#69c967","diffRemoved":"#ff7e78","skill":"#c2a1ff"},"surface":"#0f0f11","fonts":{"ui":"Inter"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "lobster", label: "Lobster", variant: "dark", accent: "#ff5c5c", ink: "#e4e4e7", surface: "#111827", seed: r##"{"accent":"#ff5c5c","ink":"#e4e4e7","semanticColors":{"diffAdded":"#22c55e","diffRemoved":"#ff5c5c","skill":"#3b82f6"},"surface":"#111827","fonts":{"ui":"Satoshi"},"accentSource":"custom"}"## },
    Preset { id: "material", label: "Material", variant: "dark", accent: "#80cbc4", ink: "#eeffff", surface: "#212121", seed: r##"{"accent":"#80cbc4","ink":"#eeffff","semanticColors":{"diffAdded":"#c3e88d","diffRemoved":"#f07178","skill":"#c792ea"},"surface":"#212121","accentSource":"custom"}"## },
    Preset { id: "matrix", label: "Matrix", variant: "dark", accent: "#1eff5a", ink: "#b8ffca", surface: "#040805", seed: r##"{"accent":"#1eff5a","ink":"#b8ffca","semanticColors":{"diffAdded":"#1eff5a","diffRemoved":"#fa423e","skill":"#1eff5a"},"surface":"#040805","fonts":{"code":null,"ui":"ui-monospace, \"SFMono-Regular\", \"SF Mono\", Menlo, Consolas, \"Liberation Mono\", monospace"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "monokai", label: "Monokai", variant: "dark", accent: "#99947c", ink: "#f8f8f2", surface: "#272822", seed: r##"{"accent":"#99947c","ink":"#f8f8f2","semanticColors":{"diffAdded":"#86b42b","diffRemoved":"#c4265e","skill":"#8c6bc8"},"surface":"#272822","accentSource":"custom"}"## },
    Preset { id: "night-owl", label: "Night Owl", variant: "dark", accent: "#44596b", ink: "#d6deeb", surface: "#011627", seed: r##"{"accent":"#44596b","ink":"#d6deeb","semanticColors":{"diffAdded":"#c5e478","diffRemoved":"#ef5350","skill":"#c792ea"},"surface":"#011627","accentSource":"custom"}"## },
    Preset { id: "nord", label: "Nord", variant: "dark", accent: "#88c0d0", ink: "#d8dee9", surface: "#2e3440", seed: r##"{"accent":"#88c0d0","ink":"#d8dee9","semanticColors":{"diffAdded":"#a3be8c","diffRemoved":"#bf616a","skill":"#b48ead"},"surface":"#2e3440","accentSource":"custom"}"## },
    Preset { id: "notion", label: "Notion", variant: "light", accent: "#3183d8", ink: "#37352f", surface: "#ffffff", seed: r##"{"accent":"#3183d8","ink":"#37352f","semanticColors":{"diffAdded":"#008000","diffRemoved":"#a31515","skill":"#0000ff"},"surface":"#ffffff","fonts":{"code":null,"ui":null},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "notion", label: "Notion", variant: "dark", accent: "#3183d8", ink: "#d9d9d8", surface: "#191919", seed: r##"{"accent":"#3183d8","ink":"#d9d9d8","semanticColors":{"diffAdded":"#4ec9b0","diffRemoved":"#fa423e","skill":"#3183d8"},"surface":"#191919","fonts":{"code":null,"ui":null},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "og", label: "OG", variant: "dark", accent: "#10a37f", ink: "#d1d5db", surface: "#343541", seed: r##"{"accent":"#10a37f","ink":"#d1d5db","semanticColors":{"diffAdded":"#65c99a","diffRemoved":"#f28b82","skill":"#b4a1e5"},"surface":"#343541","fonts":{"code":null,"content":"Arial, sans-serif","ui":"Arial, sans-serif"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "one", label: "One", variant: "light", accent: "#526fff", ink: "#383a42", surface: "#fafafa", seed: r##"{"accent":"#526fff","ink":"#383a42","semanticColors":{"diffAdded":"#3bba54","diffRemoved":"#e45649","skill":"#526fff"},"surface":"#fafafa","accentSource":"custom"}"## },
    Preset { id: "one", label: "One", variant: "dark", accent: "#4d78cc", ink: "#abb2bf", surface: "#282c34", seed: r##"{"accent":"#4d78cc","ink":"#abb2bf","semanticColors":{"diffAdded":"#8cc265","diffRemoved":"#e05561","skill":"#c162de"},"surface":"#282c34","accentSource":"custom"}"## },
    Preset { id: "oscurange", label: "Oscurange", variant: "dark", accent: "#f9b98c", ink: "#e6e6e6", surface: "#0b0b0f", seed: r##"{"accent":"#f9b98c","ink":"#e6e6e6","semanticColors":{"diffAdded":"#40c977","diffRemoved":"#fa423e","skill":"#479ffa"},"surface":"#0b0b0f","accentSource":"custom"}"## },
    Preset { id: "proof", label: "Proof", variant: "light", accent: "#3d755d", ink: "#2f312d", surface: "#f5f3ed", seed: r##"{"accent":"#3d755d","ink":"#2f312d","semanticColors":{"diffAdded":"#3d755d","diffRemoved":"#ba2623","skill":"#5f6ac2"},"surface":"#f5f3ed","fonts":{"code":null,"ui":null},"opaqueWindows":false,"accentSource":"custom"}"## },
    Preset { id: "raycast", label: "Raycast", variant: "light", accent: "#ff6363", ink: "#030303", surface: "#ffffff", seed: r##"{"accent":"#ff6363","ink":"#030303","semanticColors":{"diffAdded":"#006b4f","diffRemoved":"#b12424","skill":"#9a1b6e"},"surface":"#ffffff","fonts":{"code":"\"Jetbrains Mono\"","ui":"Inter"},"opaqueWindows":false,"accentSource":"custom"}"## },
    Preset { id: "raycast", label: "Raycast", variant: "dark", accent: "#ff6363", ink: "#fefefe", surface: "#101010", seed: r##"{"accent":"#ff6363","ink":"#fefefe","semanticColors":{"diffAdded":"#59d499","diffRemoved":"#ff6363","skill":"#cf2f98"},"surface":"#101010","fonts":{"code":"\"Jetbrains Mono\"","ui":"Inter"},"opaqueWindows":false,"accentSource":"custom"}"## },
    Preset { id: "rose-pine", label: "Rose Pine", variant: "light", accent: "#d7827e", ink: "#575279", surface: "#faf4ed", seed: r##"{"accent":"#d7827e","ink":"#575279","semanticColors":{"diffAdded":"#56949f","diffRemoved":"#797593","skill":"#907aa9"},"surface":"#faf4ed","accentSource":"custom"}"## },
    Preset { id: "rose-pine", label: "Rose Pine", variant: "dark", accent: "#ea9a97", ink: "#e0def4", surface: "#232136", seed: r##"{"accent":"#ea9a97","ink":"#e0def4","semanticColors":{"diffAdded":"#9ccfd8","diffRemoved":"#908caa","skill":"#c4a7e7"},"surface":"#232136","accentSource":"custom"}"## },
    Preset { id: "sentry", label: "Sentry", variant: "dark", accent: "#7055f6", ink: "#e6dff9", surface: "#2d2935", seed: r##"{"accent":"#7055f6","ink":"#e6dff9","semanticColors":{"diffAdded":"#8ee6d7","diffRemoved":"#fa423e","skill":"#7055f6"},"surface":"#2d2935","fonts":{"code":null,"ui":null},"accentSource":"custom"}"## },
    Preset { id: "solarized", label: "Solarized", variant: "light", accent: "#b58900", ink: "#657b83", surface: "#fdf6e3", seed: r##"{"accent":"#b58900","ink":"#657b83","semanticColors":{"diffAdded":"#859900","diffRemoved":"#dc322f","skill":"#d33682"},"surface":"#fdf6e3","accentSource":"custom"}"## },
    Preset { id: "solarized", label: "Solarized", variant: "dark", accent: "#d30102", ink: "#839496", surface: "#002b36", seed: r##"{"accent":"#d30102","ink":"#839496","semanticColors":{"diffAdded":"#859900","diffRemoved":"#dc322f","skill":"#d33682"},"surface":"#002b36","accentSource":"custom"}"## },
    Preset { id: "temple", label: "Temple", variant: "dark", accent: "#e4f222", ink: "#c7e6da", surface: "#02120c", seed: r##"{"accent":"#e4f222","ink":"#c7e6da","semanticColors":{"diffAdded":"#40c977","diffRemoved":"#fa423e","skill":"#e4f222"},"surface":"#02120c","accentSource":"custom"}"## },
    Preset { id: "tokyo-night", label: "Tokyo Night", variant: "dark", accent: "#3d59a1", ink: "#a9b1d6", surface: "#1a1b26", seed: r##"{"accent":"#3d59a1","ink":"#a9b1d6","semanticColors":{"diffAdded":"#449dab","diffRemoved":"#914c54","skill":"#9d7cd8"},"surface":"#1a1b26","accentSource":"custom"}"## },
    Preset { id: "vercel", label: "Vercel", variant: "light", accent: "#006aff", ink: "#171717", surface: "#ffffff", seed: r##"{"accent":"#006aff","ink":"#171717","semanticColors":{"diffAdded":"#28A948","diffRemoved":"#EB001D","skill":"#A100F8"},"surface":"#ffffff","contrast":40,"fonts":{"code":"\"Geist Mono\", ui-monospace, \"SFMono-Regular\"","ui":"Geist, Inter"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "vercel", label: "Vercel", variant: "dark", accent: "#006efe", ink: "#ededed", surface: "#000000", seed: r##"{"accent":"#006efe","ink":"#ededed","semanticColors":{"diffAdded":"#00AD3A","diffRemoved":"#F13342","skill":"#9540D5"},"surface":"#000000","contrast":50,"fonts":{"code":"\"Geist Mono\", ui-monospace, \"SFMono-Regular\"","ui":"Geist, Inter"},"opaqueWindows":true,"accentSource":"custom"}"## },
    Preset { id: "vscode-plus", label: "VS Code Plus", variant: "light", accent: "#007acc", ink: "#000000", surface: "#ffffff", seed: r##"{"accent":"#007acc","ink":"#000000","semanticColors":{"diffAdded":"#008000","diffRemoved":"#ee0000","skill":"#0000ff"},"surface":"#ffffff","accentSource":"custom"}"## },
    Preset { id: "vscode-plus", label: "VS Code Plus", variant: "dark", accent: "#007acc", ink: "#d4d4d4", surface: "#1e1e1e", seed: r##"{"accent":"#007acc","ink":"#d4d4d4","semanticColors":{"diffAdded":"#369432","diffRemoved":"#f44747","skill":"#000080"},"surface":"#1e1e1e","accentSource":"custom"}"## },
    Preset { id: "xcode", label: "Xcode", variant: "light", accent: "#0e0eff", ink: "#000000", surface: "#ffffff", seed: r##"{"accent":"#0e0eff","ink":"#000000","semanticColors":{"diffAdded":"#00a240","diffRemoved":"#c41a16","skill":"#0e0eff"},"surface":"#ffffff","fonts":{"code":"\"SFMono-Regular\""},"accentSource":"custom"}"## },
    Preset { id: "xcode", label: "Xcode", variant: "dark", accent: "#5482ff", ink: "#ffffff", surface: "#1f1f24", seed: r##"{"accent":"#5482ff","ink":"#ffffff","semanticColors":{"diffAdded":"#67b7a4","diffRemoved":"#fc6a5d","skill":"#5482ff"},"surface":"#1f1f24","fonts":{"code":"\"SFMono-Medium\""},"accentSource":"custom"}"## },
];

/// The ChatGPT accent menu (`settings.appearance.chatGptAccent.*`), `custom` last.
const ACCENTS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("blue", "Blue"),
    ("green", "Green"),
    ("yellow", "Yellow"),
    ("pink", "Pink"),
    ("orange", "Orange"),
    ("purple", "Purple"),
    ("black", "Black"),
    ("custom", "Custom"),
];

/// `gba(name, variant).accent`: the ChatGPT accent a chrome theme takes.
fn chatgpt_accent(name: &str) -> &'static str {
    match name {
        "green" => "#53b559",
        "yellow" => "#f6c543",
        "pink" => "#f077af",
        "orange" => "#ee7c37",
        "purple" => "#a67df2",
        _ => "#3a83f7",
    }
}

/// The accent menu's swatch (`GUr`; `black` is the icon color).
fn accent_swatch(name: &str, dark: bool) -> &'static str {
    let (light, dark_hex) = match name {
        "green" => ("#04b84c", "#40c977"),
        "yellow" => ("#ffc300", "#ffd240"),
        "pink" => ("#ff66ad", "#ff8cc1"),
        "orange" => ("#fb6a22", "#ff8549"),
        "purple" => ("#924ff7", "#ad7bf9"),
        "black" => ("#0d0d0d", "#ffffff"),
        "blue" => ("#0285ff", "#339cff"),
        _ => ("#339cff", "#339cff"),
    };
    if dark { dark_hex } else { light }
}

fn keys(variant: &str) -> (&'static str, &'static str) {
    if variant == "light" {
        (
            "host.appearanceLightChromeTheme",
            "host.appearanceLightCodeThemeId",
        )
    } else {
        (
            "host.appearanceDarkChromeTheme",
            "host.appearanceDarkCodeThemeId",
        )
    }
}

fn variant_label(variant: &str) -> &'static str {
    if variant == "light" { "Light" } else { "Dark" }
}

/// `VJ`: the chrome theme of a fresh install.
fn defaults(variant: &str) -> Value {
    if variant == "light" {
        json!({"accent": "#339cff", "contrast": 45, "fonts": {"code": null, "ui": null},
            "ink": "#1a1c1f", "opaqueWindows": false, "surface": "#ffffff",
            "semanticColors": {"diffAdded": "#00a240", "diffRemoved": "#ba2623", "skill": "#924ff7"}})
    } else {
        json!({"accent": "#339cff", "contrast": 60, "fonts": {"code": null, "ui": null},
            "ink": "#ffffff", "opaqueWindows": false, "surface": "#181818",
            "semanticColors": {"diffAdded": "#40c977", "diffRemoved": "#fa423e", "skill": "#ad7bf9"}})
    }
}

/// `bya`: a `#rrggbb` color, lowercased.
fn hex(value: &Value) -> Option<String> {
    let s = value.as_str()?.trim();
    (s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| s.to_ascii_lowercase())
}

/// `hya`: the stored theme with missing or invalid fields taken from the variant's defaults.
pub(crate) fn chrome_theme(stored: &Value, variant: &str) -> Value {
    let base = defaults(variant);
    let color = |key: &str| hex(&stored[key]).map_or_else(|| base[key].clone(), Value::from);
    let fresh = stored.is_null()
        || ["contrast", "ink", "surface", "accent"]
            .iter()
            .all(|key| stored[*key] == base[*key]);
    let font = |key: &str| {
        let name = stored["fonts"][key].as_str().map(str::trim).unwrap_or("");
        (!name.is_empty()).then(|| Value::from(name))
    };
    let mut fonts = json!({"code": font("code"), "ui": font("ui")});
    for (key, face) in [
        ("code", "codeFace"),
        ("ui", "uiFace"),
        ("content", "contentFace"),
    ] {
        if key == "content" && !stored["fonts"]["content"].is_null() {
            fonts["content"] = font("content").into();
        }
        if !fonts[key].is_null() && !stored["fonts"][face].is_null() {
            fonts[face] = stored["fonts"][face].clone();
        }
    }
    let semantic = &base["semanticColors"];
    let mut theme = json!({
        "accent": color("accent"),
        "contrast": stored["contrast"].as_f64().filter(|c| !c.is_nan())
            .map_or_else(|| base["contrast"].clone(), |c| c.round().clamp(0., 100.).into()),
        "fonts": fonts,
        "ink": color("ink"),
        "opaqueWindows": stored["opaqueWindows"].as_bool().unwrap_or(false),
        "semanticColors": {
            "diffAdded": hex(&stored["semanticColors"]["diffAdded"]).map_or_else(|| semantic["diffAdded"].clone(), Value::from),
            "diffRemoved": hex(&stored["semanticColors"]["diffRemoved"]).map_or_else(|| semantic["diffRemoved"].clone(), Value::from),
            "skill": hex(&stored["semanticColors"]["skill"]).map_or_else(|| semantic["skill"].clone(), Value::from),
        },
        "surface": color("surface"),
    });
    match stored["accentSource"].as_str() {
        Some(source) => theme["accentSource"] = source.into(),
        None if fresh => theme["accentSource"] = "chatgpt".into(),
        None => {}
    }
    theme
}

/// `n0s`: `theme` with `patch` applied; `fonts` and `semanticColors` merge key by key, a new
/// accent makes the source `custom`, and a new family drops its stale face.
fn patched(theme: &Value, patch: Value) -> Value {
    let mut out = theme.clone();
    let custom = patch.get("accent").is_some() && patch.get("accentSource").is_none();
    for (key, value) in patch.as_object().into_iter().flatten() {
        match key.as_str() {
            "fonts" | "semanticColors" => {
                for (name, inner) in value.as_object().into_iter().flatten() {
                    out[key][name] = inner.clone();
                }
                if key == "fonts" {
                    for (family, face) in [
                        ("code", "codeFace"),
                        ("ui", "uiFace"),
                        ("content", "contentFace"),
                    ] {
                        if value.get(family).is_some() && value.get(face).is_none() {
                            out["fonts"].as_object_mut().map(|fonts| fonts.remove(face));
                        }
                    }
                }
            }
            _ => out[key] = value.clone(),
        }
    }
    if custom {
        out["accentSource"] = "custom".into();
    }
    out
}

/// `hasAdvancedThemeChanges`.
fn advanced_changed(theme: &Value, variant: &str) -> bool {
    let base = defaults(variant);
    let fonts = &theme["fonts"];
    theme["contrast"] != base["contrast"]
        || (BACKDROP && theme["opaqueWindows"] != base["opaqueWindows"])
        || ["code", "content", "codeFace", "contentFace", "uiFace"]
            .iter()
            .any(|key| !fonts[*key].is_null())
}

/// `resetAdvancedThemeSettings`.
fn advanced_reset(theme: &Value, variant: &str) -> Value {
    let base = defaults(variant);
    let mut out = theme.clone();
    out["contrast"] = base["contrast"].clone();
    if BACKDROP {
        out["opaqueWindows"] = base["opaqueWindows"].clone();
    }
    out["fonts"]["code"] = Value::Null;
    out["fonts"]["content"] = Value::Null;
    if let Some(fonts) = out["fonts"].as_object_mut() {
        for face in ["codeFace", "contentFace", "uiFace"] {
            fonts.remove(face);
        }
    }
    out
}

/// `r0s`: the share string `Copy theme` puts on the clipboard.
pub(crate) fn share_string(code_theme: &str, theme: &Value, variant: &str) -> String {
    let body = json!({"codeThemeId": code_theme, "theme": theme, "variant": variant});
    format!("{SHARE_PREFIX}{body}")
}

/// `i0s` then `e0s`: the code theme id and normalized theme a share string holds, if it is for
/// `variant` and names one of the variant's presets.
pub(crate) fn parse_share(s: &str, variant: &str) -> Option<(&'static str, Value)> {
    let body = s.trim().strip_prefix(SHARE_PREFIX)?;
    let body = if body.starts_with('{') {
        body.to_owned()
    } else {
        percent_decode(body)?
    };
    let value: Value = serde_json::from_str(&body).ok()?;
    let theme = &value["theme"];
    let color = |v: &Value| hex(v).is_some();
    let optional_text = |v: &Value| v.is_null() || v.is_string();
    let valid = value["variant"] == variant
        && color(&theme["accent"])
        && color(&theme["ink"])
        && color(&theme["surface"])
        && matches!(
            theme["accentSource"].as_str(),
            None | Some("chatgpt" | "custom")
        )
        && (theme.get("accentSource").is_none() || theme["accentSource"].is_string())
        && theme["contrast"].as_u64().is_some_and(|c| c <= 100)
        && theme["opaqueWindows"].is_boolean()
        && ["diffAdded", "diffRemoved", "skill"]
            .iter()
            .all(|key| color(&theme["semanticColors"][*key]))
        && theme["fonts"]
            .as_object()
            .is_some_and(|fonts| fonts.values().all(optional_text));
    if !valid {
        return None;
    }
    let id = value["codeThemeId"].as_str()?;
    let preset = PRESETS
        .iter()
        .find(|preset| preset.id == id && preset.variant == variant)?;
    Some((preset.id, chrome_theme(theme, variant)))
}

/// `decodeURIComponent`.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn color_of(hex: &str) -> Hsla {
    u32::from_str_radix(hex.trim_start_matches('#'), 16)
        .map(|rgb| gpui::rgb(rgb).into())
        .unwrap_or_default()
}

/// `color-mix(in srgb, a p%, b)`.
fn mix(a: Hsla, p: f32, b: Hsla) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    Rgba {
        r: a.r * p + b.r * (1. - p),
        g: a.g * p + b.g * (1. - p),
        b: a.b * p + b.b * (1. - p),
        a: 1.,
    }
    .into()
}

/// `EE`: the round `Aa` preview of a theme's surface, ink and accent.
fn preview(accent: &str, ink: &str, surface: &str, size: f32) -> AnyElement {
    let surface = color_of(surface);
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(mix(color_of(ink), 0.16, surface))
        .bg(surface)
        .child(text("Aa", 12., 12., 600., color_of(accent)))
        .into_any_element()
}

/// The preset menu values are `<variant>:<id>` so the icon callback knows the variant.
fn preset_icon(value: &str, _: &Theme) -> Option<AnyElement> {
    let (variant, id) = value.split_once(':')?;
    let preset = PRESETS
        .iter()
        .find(|preset| preset.id == id && preset.variant == variant)?;
    Some(preview(preset.accent, preset.ink, preset.surface, 20.))
}

fn accent_icon(value: &str, _: &Theme) -> Option<AnyElement> {
    let (variant, name) = value.split_once(':')?;
    (name != "custom").then(|| {
        div()
            .size(px(12.))
            .flex_none()
            .rounded_full()
            .border_1()
            .border_color(gpui::black().opacity(0.1))
            .bg(color_of(accent_swatch(name, variant == "dark")))
            .into_any_element()
    })
}

#[derive(Default)]
pub(super) struct State {
    advanced: bool,
    separate: bool,
    selects: std::collections::HashMap<(&'static str, &'static str), Entity<Select>>,
    /// Each one-line field and the saved value it last showed.
    fields: std::collections::HashMap<(&'static str, &'static str), (Entity<Composer>, String)>,
    fonts: Option<std::rc::Rc<Vec<String>>>,
    /// The open import dialog's variant and field, and the focus to restore when it closes.
    import: Option<(&'static str, Entity<Composer>, Option<gpui::FocusHandle>)>,
    /// The contrast thumb being dragged: variant and value, written on release.
    drag: Option<(&'static str, f64)>,
    tracks: std::collections::HashMap<&'static str, std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>>,
}

impl SettingsPane {
    fn theme_of(&self, variant: &str, cx: &App) -> Value {
        chrome_theme(self.page.read(cx).get(keys(variant).0), variant)
    }

    fn code_theme_of(&self, variant: &str, cx: &App) -> String {
        self.page.read(cx).choice(keys(variant).1, CODEX).to_owned()
    }

    fn set_theme(&mut self, variant: &'static str, patch: Value, cx: &mut Context<Self>) {
        let theme = patched(&self.theme_of(variant, cx), patch);
        self.page
            .update(cx, |page, cx| page.set(keys(variant).0, theme, cx));
    }

    fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(toaster) = self.page.read(cx).toaster.clone() {
            toaster.update(cx, |toaster, cx| toaster.push(toast, cx));
        }
    }

    /// The variants the page shows: both with separate modes, else the window's.
    fn variants(&self, t: &Theme) -> Vec<&'static str> {
        if self.appearance.separate {
            vec!["light", "dark"]
        } else if t.appearance == theme::Appearance::Dark {
            vec!["dark"]
        } else {
            vec!["light"]
        }
    }

    fn preset_select(&mut self, variant: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let current = format!("{variant}:{}", self.code_theme_of(variant, cx));
        let select = self
            .appearance
            .selects
            .entry((variant, "preset"))
            .or_insert_with(|| {
                // `to`: ChatGPT (the Codex preset) first, the rest under `Codex themes`.
                let mut presets: Vec<_> = PRESETS.iter().filter(|p| p.variant == variant).collect();
                presets.sort_by_key(|preset| preset.id != CODEX);
                let choices = presets
                    .iter()
                    .map(|preset| Choice {
                        value: format!("{variant}:{}", preset.id).into(),
                        label: if preset.id == CODEX {
                            "ChatGPT"
                        } else {
                            preset.label
                        }
                        .into(),
                        checked: false,
                        disabled: false,
                    })
                    .collect();
                let select = cx.new(|cx| {
                    Select::new(current.clone(), choices, None, cx)
                        .with_icons(preset_icon)
                        .with_width(240.)
                        .keep_default_visible()
                        .with_heading("Codex themes")
                });
                cx.subscribe(&select, move |this, _, selected: &Selected, cx| {
                    let id = selected.0.split_once(':').map_or("", |(_, id)| id);
                    this.select_preset(variant, id, cx);
                })
                .detach();
                select
            });
        select.update(cx, |select, cx| select.set_value(current, cx));
        div().w(px(176.)).child(select.clone()).into_any_element()
    }

    /// `setCodeThemeId`: the preset's seed over the current theme, saved with its id.
    fn select_preset(&mut self, variant: &'static str, id: &str, cx: &mut Context<Self>) {
        let Some(preset) = PRESETS
            .iter()
            .find(|preset| preset.id == id && preset.variant == variant)
        else {
            return;
        };
        let mut seed: Value = serde_json::from_str(preset.seed).unwrap_or_default();
        seed["accentSource"] = "custom".into();
        let theme = patched(&self.theme_of(variant, cx), seed);
        let (theme_key, code_key) = keys(variant);
        self.page.update(cx, |page, cx| {
            page.set_host_batch(vec![(theme_key, theme), (code_key, json!(preset.id))], cx)
        });
    }

    fn accent_select(&mut self, variant: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme_of(variant, cx);
        let name = if theme["accentSource"] == "custom" {
            "custom"
        } else {
            self.page.read(cx).choice("host.chatgpt-accent", "default")
        };
        let current = format!("{variant}:{name}");
        let select = self
            .appearance
            .selects
            .entry((variant, "accent"))
            .or_insert_with(|| {
                let choices = ACCENTS
                    .iter()
                    .map(|&(name, label)| Choice {
                        value: format!("{variant}:{name}").into(),
                        label: if name == "black" && variant == "dark" {
                            "White"
                        } else {
                            label
                        }
                        .into(),
                        checked: false,
                        disabled: false,
                    })
                    .collect();
                let select = cx.new(|cx| {
                    Select::new(current.clone(), choices, None, cx)
                        .with_icons(accent_icon)
                        .with_width(180.)
                });
                cx.subscribe(&select, move |this, _, selected: &Selected, cx| {
                    let name = selected.0.split_once(':').map_or("", |(_, name)| name);
                    if name == "custom" {
                        let accent = this.theme_of(variant, cx)["accent"].clone();
                        this.set_theme(
                            variant,
                            json!({"accent": accent, "accentSource": "custom"}),
                            cx,
                        );
                    } else {
                        this.page.update(cx, |page, cx| {
                            page.set("host.chatgpt-accent", json!(name), cx)
                        });
                        let accent = chatgpt_accent(name);
                        this.set_theme(
                            variant,
                            json!({"accent": accent, "accentSource": "chatgpt"}),
                            cx,
                        );
                    }
                })
                .detach();
                select
            });
        select.update(cx, |select, cx| select.set_value(current, cx));
        // `max-w-36`: the pill fits its accent name.
        div()
            .max_w(px(144.))
            .child(select.clone())
            .into_any_element()
    }

    /// `Ta`: a swatch and the hex field; Enter or blur saves a valid color, else restores it.
    fn color_field(
        &mut self,
        variant: &'static str,
        role: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = self.theme_of(variant, cx)[role]
            .as_str()
            .unwrap_or("")
            .to_uppercase();
        let label = match role {
            "accent" => "Accent color",
            "surface" => "Background color",
            _ => "Foreground color",
        };
        let field = self.text_field(
            variant,
            role,
            label,
            &value,
            window,
            cx,
            move |this, text, cx| {
                let text = text.trim();
                let text = if text.starts_with('#') {
                    text.to_owned()
                } else {
                    format!("#{text}")
                };
                match hex(&json!(text)) {
                    Some(color) if this.theme_of(variant, cx)[role] != color.as_str() => {
                        this.set_theme(variant, json!({role: color}), cx);
                        true
                    }
                    _ => false,
                }
            },
        );
        // `Ta`: `h-7 w-24 rounded-full border px-2 gap-2`, a 14 px swatch, uppercase hex.
        div()
            .h(px(28.))
            .w(px(96.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_full()
            .border_1()
            .border_color(self.theme_border(cx))
            .child(
                div()
                    .size(px(14.))
                    .flex_none()
                    .rounded_full()
                    .border_1()
                    .border_color(self.theme_border(cx))
                    .bg(color_of(&value)),
            )
            .child(div().flex_1().min_w_0().child(field))
            .into_any_element()
    }

    fn theme_border(&self, cx: &App) -> Hsla {
        cx.theme().border
    }

    /// A one-line field for `(variant, name)` showing `value` while unfocused; `commit` runs on
    /// Enter and blur with the text and returns whether it saved, else the text is restored.
    #[allow(clippy::too_many_arguments)]
    fn text_field(
        &mut self,
        variant: &'static str,
        name: &'static str,
        label: &'static str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut Self, &str, &mut Context<Self>) -> bool + Copy + 'static,
    ) -> Entity<Composer> {
        let (field, shown) = self
            .appearance
            .fields
            .entry((variant, name))
            .or_insert_with(|| {
                let field = cx.new(|cx| Composer::dialog_field(label, "", value, cx));
                // A rejected edit shows the saved value again.
                let done =
                    move |this: &mut Self, field: &Entity<Composer>, cx: &mut Context<Self>| {
                        let text = field.read(cx).field_text().to_owned();
                        if !commit(this, &text, cx)
                            && let Some((_, shown)) = this.appearance.fields.get(&(variant, name))
                        {
                            let shown = shown.clone();
                            field.update(cx, |field, cx| field.set_text(&shown, cx));
                        }
                    };
                cx.subscribe(&field, move |this, field, _: &SettingsFieldSubmit, cx| {
                    done(this, &field, cx)
                })
                .detach();
                let handle = field.focus_handle(cx);
                let weak = field.downgrade();
                cx.on_blur(&handle, window, move |this, _, cx| {
                    if let Some(field) = weak.upgrade() {
                        done(this, &field, cx);
                    }
                })
                .detach();
                (field, value.to_owned())
            });
        // Blur listeners run after the frame that drops focus, so render must not overwrite
        // the typed text; it follows only a change of the saved value.
        if *shown != value && !field.focus_handle(cx).is_focused(window) {
            *shown = value.to_owned();
            field.update(cx, |field, cx| field.set_text(value, cx));
        }
        field.clone()
    }

    /// `Wa` (family control): the system's fonts, filtered by typing.
    fn font_select(
        &mut self,
        variant: &'static str,
        role: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.theme_of(variant, cx)["fonts"][role]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let fonts = self
            .appearance
            .fonts
            .get_or_insert_with(|| {
                let mut names = window.text_system().all_font_names();
                names.retain(|name| !name.starts_with('.'));
                names.sort_by_key(|name| name.to_lowercase());
                names.dedup();
                std::rc::Rc::new(names)
            })
            .clone();
        let select = self
            .appearance
            .selects
            .entry((variant, role))
            .or_insert_with(|| {
                let default = if role == "content" {
                    "Same as UI font"
                } else {
                    "System"
                };
                let choices = std::iter::once(("", default))
                    .chain(fonts.iter().map(|name| (name.as_str(), name.as_str())))
                    .map(|(value, label)| Choice {
                        value: value.to_owned().into(),
                        label: label.to_owned().into(),
                        checked: false,
                        disabled: false,
                    })
                    .collect();
                let select = cx.new(|cx| {
                    Select::new(current.clone(), choices, Some("Search fonts"), cx)
                        .with_width(240.)
                        .keep_default_visible()
                });
                cx.subscribe(&select, move |this, _, selected: &Selected, cx| {
                    let family = (!selected.0.is_empty()).then(|| selected.0.to_string());
                    this.set_theme(variant, json!({"fonts": {role: family}}), cx);
                })
                .detach();
                select
            });
        select.update(cx, |select, cx| select.set_value(current, cx));
        // `max-w-36`: the pill fits its font name.
        div()
            .max_w(px(144.))
            .child(select.clone())
            .into_any_element()
    }

    /// `Cc` / `wc`: a px number field; out-of-range input restores the saved size.
    fn size_field(
        &mut self,
        (key, default, min, max): (&'static str, f64, f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = self.page.read(cx).get(key).as_f64().unwrap_or(default);
        let label = if key == SANS_SIZE.0 {
            "UI font size"
        } else {
            "Code font size"
        };
        let field = self.text_field(
            "",
            key,
            label,
            &value.to_string(),
            window,
            cx,
            move |this, text, cx| {
                let saved = this.page.read(cx).get(key).as_f64().unwrap_or(default);
                match text.trim().parse::<f64>() {
                    Ok(size) if (min..=max).contains(&size) && size != saved => {
                        let size =
                            serde_json::Number::from_f64(size).map_or(Value::Null, Value::Number);
                        let size = size
                            .as_f64()
                            .filter(|s| s.fract() == 0.)
                            .map_or(size, |s| json!(s as i64));
                        this.page.update(cx, |page, cx| page.set(key, size, cx));
                        true
                    }
                    _ => false,
                }
            },
        );
        let t = cx.theme().clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .w(px(80.))
                    .h(px(32.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(t.border)
                    .child(field),
            )
            .child(text("px", 13., 18., 430., t.text_secondary))
            .into_any_element()
    }

    /// `oo`: the 0–100 contrast slider over the accent gradient, and its value.
    fn contrast_slider(
        &mut self,
        variant: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme_of(variant, cx);
        let saved = theme["contrast"].as_f64().unwrap_or(0.);
        let value = match self.appearance.drag {
            Some((dragged, value)) if dragged == variant => value,
            _ => saved,
        };
        let accent = color_of(theme["accent"].as_str().unwrap_or(""));
        let start = mix(
            accent,
            0.35,
            color_of(theme["surface"].as_str().unwrap_or("")),
        );
        let track = self.appearance.tracks.entry(variant).or_default().clone();
        let at = |track: &std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>, x: Pixels| {
            let bounds = track.get();
            (((x - bounds.left()) / bounds.size.width.max(px(1.))).clamp(0., 1.) * 100.).round()
                as f64
        };
        let focus = focus::handle(
            (gpui::ElementId::from("appearance-contrast"), variant),
            0,
            window,
            cx,
        );
        let thumb = px(16.);
        let control = div()
            .id((gpui::ElementId::from("appearance-contrast-slider"), variant))
            .role(gpui::Role::Slider)
            .aria_label(format!("{} contrast", variant_label(variant)))
            .w(px(192.))
            .h(px(20.))
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded_full()
                    .bg(gpui::linear_gradient(
                        90.,
                        gpui::linear_color_stop(start, 0.),
                        gpui::linear_color_stop(accent, 0.32),
                    ))
                    .child({
                        let track = track.clone();
                        canvas(move |bounds, _, _| track.set(bounds), |_, _, _, _| {}).size_full()
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left((px(192.) - thumb) * (value as f32 / 100.))
                    .size(thumb)
                    .rounded_full()
                    .bg(gpui::white())
                    .border_1()
                    .border_color(t.border)
                    .shadow_sm(),
            )
            .on_mouse_down(gpui::MouseButton::Left, {
                let track = track.clone();
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    let handle = focus.read(cx).handle.clone();
                    handle.focus(window, cx);
                    this.appearance.drag = Some((variant, at(&track, event.position.x)));
                    cx.notify();
                })
            })
            .on_mouse_move({
                let track = track.clone();
                cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                    if let Some((dragged, value)) = &mut this.appearance.drag
                        && *dragged == variant
                        && event.dragging()
                    {
                        *value = at(&track, event.position.x);
                        cx.notify();
                    }
                })
            })
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::end_drag))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::end_drag))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let step = match event.keystroke.key.as_str() {
                    "left" | "down" => -1.,
                    "right" | "up" => 1.,
                    _ => return,
                };
                let contrast = (saved + step).clamp(0., 100.);
                this.set_theme(variant, json!({"contrast": contrast as i64}), cx);
                cx.stop_propagation();
            }));
        let focus = focus::handle(
            (gpui::ElementId::from("appearance-contrast"), variant),
            0,
            window,
            cx,
        );
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(focus::ring(control, &focus, Theme::RADIUS_FULL, px(2.), cx))
            .child(div().w(px(36.)).flex().justify_end().child(text(
                format!("{value}"),
                13.,
                18.,
                430.,
                t.text,
            )))
            .into_any_element()
    }

    fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((variant, value)) = self.appearance.drag.take() {
            if self.theme_of(variant, cx)["contrast"].as_f64() != Some(value) {
                self.set_theme(variant, json!({"contrast": value as i64}), cx);
            }
            cx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn icon_button(
        &self,
        id: (gpui::ElementId, &'static str),
        name: &'static str,
        label: String,
        t: &Theme,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focus = focus::handle(id.clone(), 0, window, cx);
        let button = div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .hover(|d| d.bg(t.ghost_hover))
            .child(icon(name, 20., t.text_secondary));
        let button = focus::ring(button, &focus, Theme::RADIUS_FULL, px(2.), cx)
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)));
        crate::tooltip::tooltip(button, label, None, crate::tooltip::Side::Top).into_any_element()
    }

    /// `Qa` for one variant: the header row (title, import, copy, preset) and its rows.
    fn visual_card(
        &mut self,
        variant: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let label = variant_label(variant);
        let title = if self.appearance.separate {
            format!("{label} theme")
        } else {
            "Theme".into()
        };
        let import = self.icon_button(
            (gpui::ElementId::from("appearance-import"), variant),
            "appearance-import",
            format!("Import {label} theme"),
            t,
            move |this, window, cx| this.open_import(variant, window, cx),
            window,
            cx,
        );
        let copy = self.icon_button(
            (gpui::ElementId::from("appearance-copy"), variant),
            "appearance-copy",
            format!("Copy {label} theme"),
            t,
            move |this, _, cx| {
                let share = share_string(
                    &this.code_theme_of(variant, cx),
                    &this.theme_of(variant, cx),
                    variant,
                );
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(share));
                this.toast(Toast::success(format!("{label} theme copied")), cx);
            },
            window,
            cx,
        );
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(import)
            .child(copy)
            .child(self.preset_select(variant, cx))
            .into_any_element();
        let theme = self.theme_of(variant, cx);
        let accent = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.accent_select(variant, cx))
            .when(theme["accentSource"] == "custom", |row| {
                row.child(self.color_field(variant, "accent", window, cx))
            })
            .into_any_element();
        vec![
            settings_row_content(title, None, header, t),
            settings_row("Accent", None, accent, t),
            settings_row(
                "Background",
                None,
                self.color_field(variant, "surface", window, cx),
                t,
            ),
            settings_row(
                "Foreground",
                None,
                self.color_field(variant, "ink", window, cx),
                t,
            ),
            settings_row("Font", None, self.font_select(variant, "ui", window, cx), t),
        ]
    }

    fn advanced_card(
        &mut self,
        variant: &'static str,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut rows = Vec::new();
        if self.appearance.separate {
            rows.push(settings_row(
                if variant == "light" {
                    "Light theme"
                } else {
                    "Dark theme"
                },
                None,
                div().into_any_element(),
                t,
            ));
        }
        rows.push(settings_row(
            "Content font",
            None,
            self.font_select(variant, "content", window, cx),
            t,
        ));
        rows.push(settings_row(
            "Code font",
            None,
            self.font_select(variant, "code", window, cx),
            t,
        ));
        if BACKDROP {
            let opaque = self.theme_of(variant, cx)["opaqueWindows"] == true;
            let focus = focus::handle(
                (gpui::ElementId::from("appearance-translucent"), variant),
                0,
                window,
                cx,
            );
            let toggle = focus::ring(
                switch(
                    (gpui::ElementId::from("appearance-translucent"), variant),
                    !opaque,
                    false,
                    t,
                )
                .relative(),
                &focus,
                Theme::RADIUS_FULL,
                px(2.),
                cx,
            )
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_theme(variant, json!({"opaqueWindows": !opaque}), cx)
            }))
            .into_any_element();
            rows.push(settings_row("Translucent sidebar", None, toggle, t));
        }
        rows.push(settings_row(
            "Contrast",
            None,
            self.contrast_slider(variant, t, window, cx),
            t,
        ));
        rows
    }

    /// `Jo`: shown while separate modes are on or anything Advanced differs from its default.
    fn reset_visible(&self, cx: &App) -> bool {
        let page = self.page.read(cx);
        self.appearance.separate
            || self
                .advanced_defaults()
                .iter()
                .any(|(key, default)| !page.get(key).is_null() && page.get(key) != default)
            || ["light", "dark"]
                .iter()
                .any(|variant| advanced_changed(&self.theme_of(variant, cx), variant))
    }

    /// `$o`: the Advanced keys and their defaults.
    fn advanced_defaults(&self) -> Vec<(&'static str, Value)> {
        let mut keys = vec![
            (CODE_SIZE.0, json!(12)),
            (SANS_SIZE.0, json!(14)),
            (MOTION, json!("system")),
            (DIFF_MARKERS, json!("color")),
            (POINTER, json!(false)),
        ];
        if cfg!(target_os = "macos") {
            keys.push((SMOOTHING, json!(true)));
            keys.push((DOCK_ICON, json!("app-default")));
        }
        keys
    }

    /// `pc`: the macOS Dock icon radio group; the system choices preview the current appearance.
    fn dock_icon_picker(
        &self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self
            .page
            .read(cx)
            .choice(DOCK_ICON, "app-default")
            .to_owned();
        let dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let options = [
            (
                "app-default",
                "Use ChatGPT Dock icon",
                "dock/app-default.png",
            ),
            (
                "codex-system",
                "Use Codex Dock icon",
                if dark {
                    "dock/codex-dark.png"
                } else {
                    "dock/codex-light.png"
                },
            ),
            (
                "space-system",
                "Use Space Dock icon",
                if dark {
                    "dock/space-dark.png"
                } else {
                    "dock/space-light.png"
                },
            ),
        ];
        let tiles = options
            .into_iter()
            .enumerate()
            .map(|(index, (value, label, preview))| {
                let checked = current == value;
                let focus = focus::handle("appearance-dock-icon", index as isize, window, cx);
                let tile = div()
                    .id(("appearance-dock-icon", index))
                    .role(gpui::Role::RadioButton)
                    .aria_label(label)
                    .aria_toggled(if checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(12.))
                    .border_1()
                    .when(checked, |d| d.border_color(t.text).bg(t.ghost_hover))
                    .when(!checked, |d| {
                        d.border_color(t.border)
                            .bg(t.surface)
                            .hover(|d| d.border_color(t.border_strong))
                    })
                    .child(gpui::img(preview).size(px(40.)));
                focus::ring(tile, &focus, px(12.), px(2.), cx)
                    .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page
                            .update(cx, |page, cx| page.set(DOCK_ICON, json!(value), cx));
                    }))
            });
        div()
            .id("appearance-dock-icons")
            .role(gpui::Role::RadioGroup)
            .aria_label("Dock icon")
            .flex()
            .gap(px(8.))
            .children(tiles)
            .into_any_element()
    }

    fn reset_advanced(&mut self, cx: &mut Context<Self>) {
        self.appearance.separate = false;
        let mut pairs = self.advanced_defaults();
        for variant in ["light", "dark"] {
            let theme = self.theme_of(variant, cx);
            if advanced_changed(&theme, variant) {
                pairs.push((keys(variant).0, advanced_reset(&theme, variant)));
            }
        }
        self.page
            .update(cx, |page, cx| page.set_host_batch(pairs, cx));
    }

    fn open_import(&mut self, variant: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let example = share_string(
            &self.code_theme_of(variant, cx),
            &self.theme_of(variant, cx),
            variant,
        );
        let field = cx.new(|cx| {
            let label = if variant == "light" {
                "Light theme share string"
            } else {
                "Dark theme share string"
            };
            let mut field = Composer::settings_form_field(label, "", "", false, cx);
            field.set_placeholder(example, cx);
            field
        });
        cx.subscribe_in(
            &field,
            window,
            move |this, _, _: &SettingsFieldSubmit, window, cx| this.submit_import(window, cx),
        )
        .detach();
        cx.observe(&field, |_, _, cx| cx.notify()).detach();
        let trigger = window.focused(cx);
        field.focus_handle(cx).focus(window, cx);
        self.appearance.import = Some((variant, field, trigger));
        cx.notify();
    }

    fn close_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, _, Some(trigger))) = self.appearance.import.take() {
            trigger.focus(window, cx);
        }
        cx.notify();
    }

    fn submit_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((variant, field, _)) = self.appearance.import.clone() else {
            return;
        };
        let label = variant_label(variant);
        let Some((id, theme)) = parse_share(field.read(cx).field_text(), variant) else {
            self.toast(Toast::danger(format!("Couldn’t import {label} theme")), cx);
            return;
        };
        let (theme_key, code_key) = keys(variant);
        self.page.update(cx, |page, cx| {
            page.set_host_batch(vec![(theme_key, theme), (code_key, json!(id))], cx)
        });
        self.close_import(window, cx);
        self.toast(Toast::success(format!("{label} theme imported")), cx);
    }

    /// `$a`: the import dialog over the page.
    fn import_dialog(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (variant, field, _) = self.appearance.import.clone()?;
        let valid = parse_share(field.read(cx).field_text(), variant).is_some();
        let close = cx.listener(|this, _, window, cx| this.close_import(window, cx));
        Some(
            gpui::deferred(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(t.dialog_backdrop)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(gpui::MouseButton::Left, close)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.close_import(window, cx);
                        }
                    }))
                    .child(
                        div()
                            .id("appearance-import-dialog")
                            .role(gpui::Role::Dialog)
                            .aria_label("Import theme")
                            .w(px(448.))
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .relative()
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(on_card(24., Squircle, t.background_panel, Some(t.border)))
                            .child(text("Import theme", 16., 24., 500., t.text))
                            .child(field)
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(8.))
                                    .child(
                                        button("Cancel", gpui::transparent_black(), None, t)
                                            .id("appearance-import-cancel")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.close_import(window, cx)
                                            })),
                                    )
                                    .child(
                                        button("Import theme", t.button_primary_solid, None, t)
                                            .id("appearance-import-submit")
                                            .text_color(t.text_inverse)
                                            .when(!valid, |d| d.opacity(0.5))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if valid {
                                                    this.submit_import(window, cx)
                                                }
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element(),
        )
    }

    fn group_title(title: &'static str, t: &Theme) -> Div {
        div()
            .h(px(46.))
            .pb(px(6.))
            .flex()
            .items_center()
            .child(text(title, 14., 21., 500., t.text))
    }

    pub(super) fn render_appearance(
        &mut self,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card = |rows: Vec<Div>, layer: &Layer| super::worktrees::card(rows, Some(layer), t);
        let mode = self.segmented(
            THEME,
            "system",
            &[("light", "Light"), ("dark", "Dark"), ("system", "System")],
            t,
            window,
            cx,
        );
        let mut visual = vec![card(vec![settings_row("Mode", None, mode, t)], &self.layer)];
        for variant in self.variants(t) {
            let rows = self.visual_card(variant, t, window, cx);
            visual.push(card(rows, &self.layer));
        }

        let open = self.appearance.advanced;
        let focus = focus::handle("appearance-advanced", 0, window, cx);
        let disclosure = div()
            .id("appearance-advanced")
            .role(gpui::Role::Button)
            .aria_expanded(open)
            .aria_label("Advanced")
            .flex()
            .items_center()
            .gap(px(4.))
            .child(text("Advanced", 14., 21., 500., t.text_tertiary))
            .child(icon(
                // `s ? Mn : de`: down while open, right while closed.
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                16.,
                t.text_tertiary,
            ));
        let disclosure = focus::ring(disclosure, &focus, Theme::RADIUS_LG, px(2.), cx)
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
            .on_click(cx.listener(|this, _, _, cx| {
                this.appearance.advanced = !this.appearance.advanced;
                cx.notify();
            }));
        let reset = self.reset_visible(cx).then(|| {
            let focus = focus::handle("appearance-reset", 0, window, cx);
            let reset = div()
                .id("appearance-reset")
                .role(gpui::Role::Button)
                .child(text("Reset to default", 14., 21., 430., t.text_tertiary));
            focus::ring(reset, &focus, Theme::RADIUS_LG, px(2.), cx)
                .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
                .on_click(cx.listener(|this, _, _, cx| this.reset_advanced(cx)))
        });
        let mut advanced = Vec::new();
        if open {
            let sans = self.size_field(SANS_SIZE, window, cx);
            let code = self.size_field(CODE_SIZE, window, cx);
            advanced.push(card(
                vec![
                    settings_row(
                        "UI font size",
                        Some("Adjust the base size used for ChatGPT"),
                        sans,
                        t,
                    ),
                    settings_row(
                        "Code font size",
                        Some("Adjust the base size used for code across chats and diffs"),
                        code,
                        t,
                    ),
                ],
                &self.layer,
            ));
            let motion = self.segmented(
                MOTION,
                "system",
                &[("system", "System"), ("on", "On"), ("off", "Off")],
                t,
                window,
                cx,
            );
            let focus = focus::handle("appearance-separate", 0, window, cx);
            let separate = self.appearance.separate;
            let separate = focus::ring(
                switch("appearance-separate", separate, false, t).relative(),
                &focus,
                Theme::RADIUS_FULL,
                px(2.),
                cx,
            )
            .on_mouse_down(gpui::MouseButton::Left, focus_on_pointer(focus))
            .on_click(cx.listener(|this, _, _, cx| {
                this.appearance.separate = !this.appearance.separate;
                cx.notify();
            }))
            .into_any_element();
            advanced.push(card(
                vec![
                    settings_row(
                        "Reduce motion",
                        Some("Reduce animations or match your system"),
                        motion,
                        t,
                    ),
                    settings_row(
                        "Separate light and dark modes",
                        Some("Choose a theme, colors, and font for each"),
                        separate,
                        t,
                    ),
                ],
                &self.layer,
            ));
            for variant in self.variants(t) {
                let rows = self.advanced_card(variant, t, window, cx);
                advanced.push(card(rows, &self.layer));
            }
            let markers = self.segmented(
                DIFF_MARKERS,
                "color",
                &[("color", "Color"), ("symbols", "+/-")],
                t,
                window,
                cx,
            );
            let mut rows = vec![
                settings_row(
                    "Diff markers",
                    Some("Show changes using colors or +/- markers"),
                    markers,
                    t,
                ),
                settings_row(
                    "Use pointer cursors",
                    Some("Change the cursor to a pointer when hovering over interactive elements"),
                    self.toggle(POINTER, false, t, window, cx),
                    t,
                ),
            ];
            if cfg!(target_os = "macos") {
                rows.push(settings_row(
                    "Dock icon",
                    Some("Choose the icon the app will use in the dock"),
                    self.dock_icon_picker(t, window, cx),
                    t,
                ));
                rows.push(settings_row(
                    "Font smoothing",
                    Some("Use native macOS font anti-aliasing"),
                    self.toggle(SMOOTHING, true, t, window, cx),
                    t,
                ));
            }
            advanced.push(card(rows, &self.layer));
        }

        let content = [
            div()
                .flex()
                .flex_col()
                .child(Self::group_title("Visual style", t))
                .child(div().flex().flex_col().gap(px(16.)).children(visual)),
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
                        .child(disclosure)
                        .children(reset),
                )
                .child(div().flex().flex_col().gap(px(16.)).children(advanced)),
        ];
        let dialog = self.import_dialog(t, cx);
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
                            div().w_full().max_w(px(768.)).mx_auto().px(px(20.)).child(
                                div()
                                    .min_h(px(60.))
                                    .py(px(12.))
                                    .flex()
                                    .flex_col()
                                    .justify_center()
                                    .child(text("Appearance", 28., 33.6, 500., t.text)),
                            ),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_string_round_trips_and_checks_variant_and_preset() {
        let theme = patched(
            &chrome_theme(&Value::Null, "dark"),
            json!({"accent": "#ABCDEF", "contrast": 70}),
        );
        let share = share_string("dracula", &theme, "dark");
        let (id, parsed) = parse_share(&format!("  {share} "), "dark").unwrap();
        assert_eq!(id, "dracula");
        assert_eq!(parsed["contrast"], 70);
        assert_eq!(parsed["accentSource"], "custom");
        // The original also accepts the URI-encoded form.
        let encoded = share
            .replace('{', "%7B")
            .replace('}', "%7D")
            .replace('"', "%22");
        assert!(parse_share(&encoded, "dark").is_some());
        assert!(parse_share(&share, "light").is_none(), "variant mismatch");
        assert!(parse_share(&share.replace("dracula", "nope"), "dark").is_none());
        assert!(parse_share(&share.replace("#abcdef", "red"), "dark").is_none());
        assert!(parse_share("codex-theme-v2:{}", "dark").is_none());
    }

    #[test]
    fn stored_theme_falls_back_per_field() {
        let theme = chrome_theme(
            &json!({"accent": "nope", "contrast": 140.4, "ink": "#ABCDEF", "fonts": {"ui": "  "}}),
            "light",
        );
        assert_eq!(theme["accent"], "#339cff");
        assert_eq!(theme["contrast"], 100.);
        assert_eq!(theme["ink"], "#abcdef");
        assert!(theme["fonts"]["ui"].is_null());
        assert!(theme.get("accentSource").is_none());
        assert_eq!(
            chrome_theme(&Value::Null, "dark")["accentSource"],
            "chatgpt"
        );
    }

    #[test]
    fn a_new_family_drops_its_face_and_reset_clears_advanced_fonts() {
        let theme = json!({"contrast": 10, "opaqueWindows": false,
            "fonts": {"code": "Menlo", "codeFace": "Menlo-Bold", "ui": "Inter", "uiFace": "Inter-Medium"}});
        let theme = patched(&theme, json!({"fonts": {"code": "Monaco"}}));
        assert!(theme["fonts"].get("codeFace").is_none());
        assert_eq!(theme["fonts"]["uiFace"], "Inter-Medium");
        assert!(advanced_changed(&theme, "dark"));
        let reset = advanced_reset(&theme, "dark");
        assert!(!advanced_changed(&reset, "dark"));
        assert_eq!(reset["fonts"]["ui"], "Inter", "the visual font stays");
    }

    #[test]
    fn every_preset_seed_parses() {
        for preset in PRESETS {
            let seed: Value = serde_json::from_str(preset.seed).unwrap();
            assert_eq!(seed["accent"], preset.accent, "{}", preset.id);
            assert!(
                PRESETS
                    .iter()
                    .any(|p| p.id == CODEX && p.variant == preset.variant)
            );
        }
    }
}
