"""Extract every distinct inline <svg> from refs/*/*.spec.json into assets/icons/<name>.svg.

Icons are keyed by a hash of viewBox + path data, so the same glyph found on several screens
is written once. GPUI renders an svg as an alpha mask tinted with the text color, so
`currentColor` becomes black and presentation-only attributes are dropped. New glyphs print as
`unnamed-<key>`: add them to NAMES and re-run.

Run: python3 tools/extract/icons.py
"""

import hashlib
import json
import re
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets/icons"

NAMES = {
    # Navigation rail
    "eb02f05388": "home-filled",
    "8649a4e09d": "home",
    "a0d5b02bf9": "space",
    "ee30e318e9": "scheduled",
    "44f0e1591f": "plugins",
    "42e7c49335": "explore",
    "da39a3ee5e": "code-review",
    "2c370ecbbd": "sites",
    # Sidebar header and window toolbar
    "44fbbf3301": "codex-wordmark",
    "e06d40d7cb": "chevron-down-medium",
    "8c95f6018f": "bell",
    "1e0845ea5b": "search",
    "ff6e9568f5": "bell-unread",
    "ff58624875": "new-chat",
    "fb6a94331e": "arrow-left",  # Back; Forward is the same glyph mirrored (-scale-x-100)
    "d3e3e867e7": "sidebar",
    "915d9f4750": "new-tab",
    # Home
    "6e15153969": "codex-logo",
    "9191379992": "close",
    "7fc26855c6": "folder",
    "3fe223bc62": "laptop",
    "42aceca2b3": "plus",
    "06481900bd": "permissions",
    "a3d37e2c0a": "chevron-down-small",
    "8bebc0cef1": "mic",
    "3dfa550c30": "voice",
    # Settings
    "e9b5b5cfbd": "settings-general",
    "059db4e82f": "settings-import",
    "8329e503b1": "settings-profile",
    "cc0f03532e": "arrow-up-right",
    "81e64a6d8c": "settings-appearance",
    "d8decddf8b": "settings-archived-chats",
    "afd69a119c": "settings-voice",
    "26c8f9f033": "settings-storage",
    "78193de9fb": "settings-configuration",
    "14114513f3": "settings-personalization",
    "81eff7fdf4": "settings-mini-pets",
    "159bf060d9": "settings-keyboard-shortcuts",
    "7c1c603063": "settings-usage",
    "1fabbd13a5": "settings-data-controls",
    "d4744ee918": "settings-plugins",
    "e0e3ccd75e": "settings-passwords",
    "105abcfbd1": "settings-computer-use",
    "880e0b0466": "settings-computer-history",
    "428176a014": "settings-appshots",
    "4ea75c08cb": "settings-browser",
    "705aa53ec7": "settings-cloud-computer",
    "a66f45b097": "settings-hooks",
    "98bb43b755": "settings-connections",
    "bc0456503e": "settings-codex-cloud",
    "af670f486f": "settings-legacy-codex-cloud",
    "c5875f3474": "settings-code-review",
    "8a5c66e76a": "settings-git",
    "c9e426407c": "settings-worktrees",
    "76d8980617": "chevron-down",
    "cc1b800935": "pencil",
}

DROP = re.compile(r'\s(?:class|role|focusable|aria-[\w-]+|data-[\w-]+)="[^"]*"')


def key(svg: str) -> str:
    view_box = re.search(r'viewBox="([^"]*)"', svg)
    data = (view_box.group(1) if view_box else "") + "".join(re.findall(r' d="([^"]*)"', svg))
    return hashlib.sha1(data.encode()).hexdigest()[:10]


def clean(svg: str) -> str:
    # Masked <image> icons (e.g. Code Review) paint currentColor through an embedded svg: use it.
    inner = re.search(r'href="data:image/svg\+xml,([^"]*)"', svg)
    if inner:
        svg = urllib.parse.unquote(inner.group(1)).replace('fill="white"', 'fill="black"')
    svg = re.sub(r"currentcolor", "black", DROP.sub("", svg), flags=re.I)
    return svg.strip() + "\n"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    written = {}
    for spec in sorted(ROOT.glob("refs/*/*.spec.json")):
        for node in json.loads(spec.read_text())["nodes"]:
            svg = node.get("svg")
            if not svg:
                continue
            k = key(svg)
            name = NAMES.get(k, f"unnamed-{k}")
            if name not in written:
                (OUT / f"{name}.svg").write_text(clean(svg))
                written[name] = spec.parent.name
    for name, screen in sorted(written.items()):
        print(f"{name}.svg  ({screen})")


if __name__ == "__main__":
    main()
