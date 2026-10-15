"""Resolve Codex theme tokens to exact 8-bit RGBA with headless Chromium.

Loads the original renderer CSS, applies the inline :root vars the app's own JS computes
(theme_vars.mjs), then evaluates each token expression in light and dark. RGB bytes come from
a canvas fill (Chromium's own oklab/color-mix -> sRGB conversion and rounding); alpha is
round(a * 255) of the computed value. Writes crates/theme/src/tokens.rs.

Run: uv run --with playwright python tools/extract/theme.py [--check]
Needs node and a Playwright chromium_headless_shell in ~/.cache/ms-playwright.
--check compares each refs/*/<variant>@*.spec.json against the resolved :root vars and maps
every color in its node styles to a token ("-- no token" marks a missing one).
"""

import glob
import json
import re
import subprocess
import tempfile
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
ASSETS = Path("/tmp/cgx/asar-x/webview/assets")
CSS = ["app-shared-*.css", "app-initial-*.css", "app-primary-*.css"]

# (field, css expression, note). The expression is what the original applies; doc comments
# name it so screen code can map spec.json styles to fields.
COLORS = [
    # Surfaces
    ("background_surface_under", "var(--app-color-background-surface-under)", "Window background (`body` in opaque windows)."),
    ("surface", "var(--color-surface)", "Main panel (`_MainContentSurface`)."),
    ("page_surface_layout", "var(--lightningcss-light,color-mix(in srgb, var(--color-surface-secondary) 85%, transparent))var(--lightningcss-dark,color-mix(in srgb, var(--color-surface-tertiary) 80%, transparent))", "`[data-app-shell-page-surface=true]` (`_PageSurfaceLayout`): window behind rail and sidebar."),
    ("sidebar_background", "color-mix(in oklab, var(--color-surface) 65%, transparent)", "Sidebar (`.sidebar-navigation` inside the page surface layout)."),
    ("surface_secondary", "var(--color-surface-secondary)", ""),
    ("surface_tertiary", "var(--color-surface-tertiary)", ""),
    ("surface_elevated", "var(--color-surface-elevated)", "Cards, suggestion surface (`_surface_or5n7`)."),
    ("surface_elevated_secondary", "var(--color-surface-elevated-secondary)", ""),
    ("background_elevated_primary_opaque", "var(--app-color-background-elevated-primary-opaque)", ""),
    ("composer_surface", "var(--color-background-composer-surface)", "Composer body (`_ComposerLayoutBody`)."),
    ("composer_action_bar", "var(--color-background-composer-action-bar)", "Composer action rail item (`bg-background-composer-action-bar`)."),
    ("menu_background", "var(--menu-background-color)", "Dropdown menus and popovers."),
    ("menu_item_hover", "var(--menu-item-background-color)", ""),
    ("tooltip_background", "var(--tooltip-background-color)", ""),
    ("control_background", "var(--color-background-control-opaque)", ""),
    ("button_primary_solid", "var(--color-background-primary-solid)", "Primary solid button (e.g. Start Voice)."),
    ("button_primary_solid_hover", "var(--color-background-primary-solid-hover)", ""),
    ("button_secondary", "var(--app-color-background-button-secondary)", ""),
    ("button_secondary_hover", "var(--app-color-background-button-secondary-hover)", ""),
    ("button_tertiary_hover", "var(--app-color-background-button-tertiary-hover)", ""),
    ("ghost_hover", "var(--color-background-primary-ghost-hover)", "Row/ghost button hover."),
    ("list_hover", "var(--color-token-list-hover-background)", ""),
    ("primary_soft_alpha", "var(--color-background-primary-soft-alpha)", "Toolbar button (`bg-primary-soft-alpha`)."),
    ("text_alpha_5", "color-mix(in oklab, var(--color-text) 5%, transparent)", "`bg-text/5` toolbar button."),
    ("text_alpha_10", "color-mix(in oklab, var(--color-text) 10%, transparent)", "`hover:bg-text/10`."),
    ("background_panel", "var(--color-background-panel)", "Settings group card."),
    ("composer_primary", "var(--color-background-composer-primary)", "Composer send / voice button."),
    ("user_message_background", "var(--color-background-user-message)", ""),
    ("text_composer_primary", "var(--color-text-composer-primary)", "Icon on the composer primary button."),
    ("code_block_background", "var(--codeblock-background-color)", ""),
    ("text_selection", "var(--color-background-text-selection)", ""),
    ("dialog_backdrop", "var(--modal-backdrop-background)", ""),
    # Text
    ("text", "var(--color-text)", "Primary text."),
    ("text_default_nav", "color-mix(in oklab, var(--color-text) 85%, transparent)", "Sidebar nav rows and header title."),
    ("text_secondary", "var(--color-token-text-secondary)", ""),
    ("text_description", "var(--lightningcss-light,#5d5d5d)var(--lightningcss-dark,var(--color-text-secondary))", "Suggestion card description (`_description_or5n7`)."),
    ("text_tertiary", "var(--color-text-tertiary)", "Muted labels, placeholders."),
    ("text_disabled", "var(--color-text-disabled)", ""),
    ("text_inverse", "var(--color-text-inverse)", "Text on primary solid buttons."),
    ("text_accent", "var(--app-color-text-accent)", ""),
    ("text_link", "var(--link-primary-text-color)", ""),
    ("text_error", "var(--app-color-text-error)", ""),
    ("text_success", "var(--app-color-text-success)", ""),
    ("text_warning", "var(--app-color-text-warning)", ""),
    ("icon_primary", "var(--app-color-icon-primary)", ""),
    ("icon_secondary", "var(--app-color-icon-secondary)", ""),
    ("icon_tertiary", "var(--app-color-icon-tertiary)", ""),
    # Borders
    ("border", "var(--color-border)", ""),
    ("border_subtle", "var(--color-border-subtle)", ""),
    ("border_strong", "var(--color-border-strong)", ""),
    ("border_composer_rail", "color-mix(in oklab, var(--color-border) 80%, transparent)", "Composer rail item border (`_item_1hrlq`)."),
    ("sidebar_divider", "var(--app-shell-sidebar-divider)", "Main panel left hairline."),
    ("composer_inset_highlight", "#fff3", "Composer inset shadow (`--elevation-composer-dark`, dark only)."),
    ("page_surface_stroke", "var(--lightningcss-light,rgb(from var(--color-border-strong) r g b / .04))var(--lightningcss-dark,rgb(from var(--color-border-strong) r g b / .06))", "`_PageSurface` 0.5px stroke (`--elevation-stroke-subtle`)."),
    ("shadow_card", "#0000000d", "`--shadow-card` color (0 4px 16px)."),
    ("shadow_sm", "#00000014", "`--shadow-sm` color (0 1px 2px -1px), switch thumb."),
    # Accent and states
    ("control_thumb", "var(--color-control-thumb-on-accent)", "Switch thumb."),
    ("accent", "var(--codex-base-accent)", ""),
    ("info_solid", "var(--color-background-info-solid)", "Unread dot."),
    ("focus_ring", "var(--color-ring)", ""),
    ("border_focus", "var(--app-color-border-focus)", ""),
    ("background_accent", "var(--app-color-background-accent)", ""),
    ("diff_added", "var(--color-codex-diff-added)", ""),
    ("diff_deleted", "var(--color-codex-diff-deleted)", ""),
    ("git_added", "var(--color-codex-git-added)", ""),
    ("git_deleted", "var(--color-codex-git-deleted)", ""),
    ("git_modified", "var(--color-codex-git-modified)", ""),
    ("scrollbar", "var(--scrollbar-color)", ""),
    ("scrollbar_hover", "var(--color-token-scrollbar-slider-hover-background)", ""),
    ("switch_track", "var(--switch-track-color)", ""),
    ("switch_track_checked", "var(--switch-track-color-checked)", ""),
]

PAGE_JS = """
async ([variant, vars, colors]) => {
  const root = document.documentElement;
  root.dataset.theme = variant;
  root.removeAttribute('style');
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, v);
  const probe = document.getElementById('probe');
  const ctx = document.createElement('canvas').getContext('2d', {willReadFrequently: true});
  const out = {};
  for (const [name, expr] of colors) {
    probe.style.color = '';
    probe.style.color = expr;
    const cs = getComputedStyle(probe).color;
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = `rgb(from ${cs} r g b / 1)`;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    out[name] = [r, g, b, cs];
  }
  const cssVars = {};
  const st = getComputedStyle(root);
  for (let i = 0; i < st.length; i++) if (st[i].startsWith('--')) cssVars[st[i]] = st.getPropertyValue(st[i]).trim();
  return [out, cssVars];
}
"""


def alpha(cs: str) -> int:
    m = re.search(r"/\s*([\d.e-]+)\)$", cs) or re.match(r"rgba\([^,]+,[^,]+,[^,]+,\s*([\d.e-]+)\)", cs)
    return round(float(m.group(1)) * 255) if m else 255


# Inline vars the app sets from user settings (defaults as captured in the refs).
SETTINGS = {
    "--font-ui-size": "14px",
    "--font-code-size": "12px",
    "--codex-sidebar-preferred-width": "340px",
    "--codex-chat-font-size": "14px",
}


def spec_colors(spec):
    """Every distinct color string in a spec's node styles and text runs."""
    found = set()
    for n in spec["nodes"]:
        for k, v in n["style"].items():
            if "olor" in k or k == "boxShadow":
                found.update(re.findall(r"(?:rgba?|oklab|oklch|color)\([^)]*\)", v))
    found.update(t["color"] for t in spec["texts"])
    return sorted(found)


def resolve():
    js = json.loads(subprocess.check_output(["node", str(ROOT / "tools/extract/theme_vars.mjs"), str(ASSETS)]))
    specs = {p: json.loads(p.read_text()) for p in sorted(ROOT.glob("refs/*/*@*.spec.json"))}
    links = "".join(f'<link rel="stylesheet" href="file://{sorted(ASSETS.glob(p))[0]}">' for p in CSS)
    html = Path(tempfile.mkdtemp()) / "theme-probe.html"
    html.write_text(
        f'<!doctype html><html class="electron-opaque" data-codex-window-type="electron"><head>{links}</head>'
        '<body><div id="probe">x</div></body></html>'
    )
    exe = sorted(glob.glob(str(Path.home() / ".cache/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell")))[-1]
    result = {}
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=exe, args=["--allow-file-access-from-files"])
        page = browser.new_page(device_scale_factor=2)
        page.goto(f"file://{html}")
        page.wait_for_load_state("load")
        for variant in ("dark", "light"):
            probes = [(n, e) for n, e, _ in COLORS]
            probes += [(c, c) for p, s in specs.items() if p.name.startswith(variant) for c in spec_colors(s)]
            result[variant] = page.evaluate(PAGE_JS, [variant, {**js[variant], **SETTINGS}, probes])
        browser.close()
    return result, specs


def rgba8(probe):
    r, g, b, cs = probe
    return f"{r:02x}{g:02x}{b:02x}{alpha(cs):02x}"


def check(result, specs):
    for path, spec in specs.items():
        variant = path.name.split("@")[0]
        colors, got = result[variant]
        ref = spec["rootVars"]
        bad = [(k, v, got.get(k)) for k, v in ref.items() if got.get(k, "").replace(" ", "") != v.replace(" ", "")]
        print(f"{path.relative_to(ROOT)}: {len(ref) - len(bad)}/{len(ref)} root vars match")
        for b in bad:
            print("  ", *(str(x)[:90] for x in b))
        tokens = {}
        for name, _, _ in COLORS:
            tokens.setdefault(rgba8(colors[name]), []).append(name)
        for c in spec_colors(spec):
            hex8 = rgba8(colors[c])
            print(f"   {hex8} {c:58} {' '.join(tokens.get(hex8, ['-- no token']))}")


def rust(result):
    out = [
        "// Generated by tools/extract/theme.py. Do not edit.",
        "",
        "use gpui::{Hsla, rgba};",
        "",
        "/// Colors resolved from the original renderer CSS. Each doc comment is the CSS value the",
        "/// original applies; values are the 8-bit sRGB+alpha bytes Chromium paints.",
        "#[derive(Clone, Debug, PartialEq)]",
        "pub struct Theme {",
        "    pub appearance: Appearance,",
    ]
    for name, expr, note in COLORS:
        out.append(f"    /// `{expr}`{' ' + note if note else ''}")
        out.append(f"    pub {name}: Hsla,")
    out += ["}", "", "#[derive(Clone, Copy, Debug, PartialEq, Eq)]", "pub enum Appearance {", "    Dark,", "    Light,", "}", ""]
    out += ["impl Theme {"]
    for variant in ("dark", "light"):
        out += [f"    pub fn {variant}() -> Self {{", "        Self {", f"            appearance: Appearance::{variant.title()},"]
        for name, _, _ in COLORS:
            out.append(f"            {name}: rgba(0x{rgba8(result[variant][0][name])}).into(),")
        out += ["        }", "    }", ""]
    out[-1] = "}"
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    res, specs = resolve()
    if "--check" in sys.argv:
        check(res, specs)
    target = ROOT / "crates/theme/src/tokens.rs"
    target.write_text(rust(res))
    subprocess.run(["rustfmt", "--edition", "2024", str(target)], check=True)
    for name, expr, _ in COLORS:
        print(f"{name} | {expr} | {rgba8(res['dark'][0][name])} | {rgba8(res['light'][0][name])}")
