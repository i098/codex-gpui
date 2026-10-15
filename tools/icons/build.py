"""Fit the icon images that make GPUI paint Chromium's exact icon pixels.

Chromium blits SVG paths with Skia's integer blitters, and the formula changes from pixel to
pixel and with the raster tile. One alpha mask tinted by GPUI cannot match that. So, for each
(icon, device size, color) in refs/, this script fits a straight-alpha RGBA image. GPUI blends
this image with `rgb * a + dst * (1 - a)`, and the result is the 8-bit pixel of the ref.

1. Read every inline <svg> of refs/*/*.spec.json: glyph, size, color (with the opacity of the
   ancestors), and device position.
2. Rasterize each glyph white on black, plain and mirrored, with chrome-headless-shell on the
   Mac. This gives the coverage, which finds the icon in the ref PNG (+-1 px, plain or
   mirrored) and the background under it.
3. Fit each pixel against the ref crops. The fit takes the alpha nearest Chromium's coverage
   with a rounding margin of 0.2 or more. One image is the default. If the same
   (icon, size, color) renders differently at a different position (tile seams), the script
   writes a position variant `<color>@<x>_<y>.png`.
4. Write assets/icons/px/<name>[-flipx]@<W>x<H>/<rrggbbaa>[@<x>_<y>].png for every asset name
   of the glyph, except the glyphs in RUNTIME, and tools/icons/check.json (the cases of the
   ui::icon parity test).

Run: uv run -p /usr/bin/python3 --with numpy --with pillow tools/icons/build.py
"""

import collections
import importlib.util
import json
import math
import re
import shutil
import subprocess
import time
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "assets/icons/px"
WORK = Path("/tmp/cgx/icons")
MAC = "/tmp/cgx-mac/icons"
CHROME = "/tmp/cgx-mac/chrome-headless-shell-mac-arm64/chrome-headless-shell"
PROFILE = "/tmp/cgx-mac/chs-icons"
SSH = ["-o", "ControlMaster=auto", "-o", f"ControlPath={WORK}/cm-%r@%h", "-o",
       "ControlPersist=600", "-o", "ConnectTimeout=15"]
MARGIN = 0.2  # distance of the fitted value from a rounding boundary, in 8-bit units
# Glyphs (any asset name) that crates/shapes rasterizes at runtime to the ref's exact pixels at
# every ref use, in the screens' own layers (full parity run with no fitted images): no image.
RUNTIME = {"chevron-down-small", "composer-model", "diff-file-python", "diff-jump-to-file",
           "diff-more", "diff-open-in", "diff-show-files", "scheduled-filled",
           "sidebar-cmd-spinner", "thread-chevron-right", "thread-copy", "thread-fork",
           "thread-lang-python", "thread-project", "thread-rate", "thread-read-aloud"}
# Glyphs fitted over a background that varies by 1 level under the box (the send button's
# circle): its uncovered pixels count as background. Elsewhere such a variation is a layer
# effect (a fade mask) that a plain blend does not reproduce, so those uses are skipped.
UNEVEN_BG = {"composer-arrow-up"}

_spec = importlib.util.spec_from_file_location("icons", ROOT / "tools/extract/icons.py")
icons = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(icons)
_spec = importlib.util.spec_from_file_location("inventory", ROOT / "tools/shapes/inventory.py")
inventory = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(inventory)


def run(cmd):
    for _ in range(10):
        if subprocess.run(cmd).returncode == 0:
            return
        time.sleep(3)
    raise RuntimeError(cmd)


def chrome(html, w, h, name):
    """Screenshot of `html` at `w` x `h` CSS px, 2x, by chrome-headless-shell on the Mac."""
    WORK.mkdir(parents=True, exist_ok=True)
    (WORK / f"{name}.html").write_text(html)
    rsync = ["rsync", "-q", "-e", "ssh " + " ".join(SSH)]
    run(["ssh", *SSH, "mac", f"mkdir -p {MAC}"])
    run([*rsync, str(WORK / f"{name}.html"), f"mac:{MAC}/"])
    run(["ssh", *SSH, "mac",
         f"{CHROME} --headless --hide-scrollbars --force-device-scale-factor=2 "
         f"--window-size={w},{h} --user-data-dir={PROFILE} --virtual-time-budget=1000 "
         f"--screenshot={MAC}/{name}.png file://{MAC}/{name}.html 2>/dev/null"])
    run([*rsync, f"mac:{MAC}/{name}.png", str(WORK)])
    run(["ssh", *SSH, "mac", f"rm -rf {PROFILE} {MAC}"])
    return np.array(Image.open(WORK / f"{name}.png").convert("RGB")).astype(np.int64)


def round_half_up(v):
    return math.floor(v + 0.5)

# Icons drawn as `<use href="#symbol">` of the app's sprite have no inline path: map the symbol
# to the assets/icons file that holds its paths (`fill="black"` there means currentColor).
SPRITES = {"file-tree-builtin-python": "diff-file-python"}


def resolve_sprite(svg):
    if m := re.search(r'<use href="#([\w-]+)"', svg):
        if name := SPRITES.get(m.group(1)):
            asset = (ROOT / f"assets/icons/{name}.svg").read_text()
            return asset.replace('fill="black"', 'fill="currentColor"')
    return svg


def read_uses():
    uses = []
    for spec_path in sorted(ROOT.glob("refs/*/*.spec.json")):
        spec = json.loads(spec_path.read_text())
        by_id = {n["id"]: n for n in spec["nodes"]}
        vw, vh = spec["viewport"][0] * 2, spec["viewport"][1] * 2
        for node in spec["nodes"]:
            if not node.get("svg"):
                continue
            svg = resolve_sprite(node["svg"])
            opacity, p, seen = 1.0, node, set()
            while p is not None and p["id"] not in seen:  # some dumps make a node its own parent
                seen.add(p["id"])
                opacity *= float(p["style"].get("opacity", 1))
                p = by_id.get(p["parent"])
            x, y, w, h = node["rect"]
            x0, y0 = round_half_up(x * 2), round_half_up(y * 2)  # Chromium snaps svg to device px
            W, H = math.ceil(w * 2), math.ceil(h * 2)
            if x0 < 2 or y0 < 2 or x0 + W + 2 > vw or y0 + H + 2 > vh:
                continue
            *rgb, own = inventory.rgba8(node["style"]["color"])
            uses.append(dict(
                ref=spec_path.with_suffix("").with_suffix(".png"), glyph=icons.key(svg),
                svg=svg, w=w, h=h, W=W, H=H, x0=x0, y0=y0,
                rgb=tuple(rgb), alpha=own / 255 * opacity, own=own, opacity=opacity))
    return uses


def coverage(uses):
    """(glyph, W, H, flip) -> white-on-black Chromium raster, (H + 2) x (W + 2) with a 1 px
    ring; `flip` mirrors the glyph like the original's `-scale-x-100` (Forward arrow)."""
    geo = {}
    for u in uses:
        for flip in (False, True):
            geo.setdefault((u["glyph"], u["W"], u["H"], flip), u)
    cols, cell = 16, 64
    cells = []
    for i, ((*_, flip), u) in enumerate(geo.items()):
        x, y = (i % cols) * cell + 4, (i // cols) * cell + 4
        svg = re.sub(r'fill="#\w{6}"', 'fill="currentColor"', u["svg"])  # two-tone: coverage
        cells.append(f'<div class="c{" f" * flip}" style="left:{x}px;top:{y}px;'
                     f'width:{u["w"]}px;height:{u["h"]}px">{svg}</div>')
    css = ("<style>body{margin:0;background:#000}.c{position:absolute;line-height:0;color:#fff}"
           ".c>svg{display:block;width:100%;height:100%}.f>svg{transform:scaleX(-1)}</style>")
    rows = math.ceil(len(geo) / cols)
    img = chrome(f"<html><head>{css}</head><body>{''.join(cells)}</body></html>",
                 cols * cell, rows * cell, "coverage")
    out = {}
    for i, (k, u) in enumerate(geo.items()):
        x, y = ((i % cols) * cell + 4) * 2, ((i // cols) * cell + 4) * 2
        out[k] = img[y - 1:y + u["H"] + 1, x - 1:x + u["W"] + 1, 0]
    return out


def align(u, cov, refs, tol):
    """Finds the icon in the ref within +-1 px, plain or mirrored: every uncovered pixel of the
    box (and its ring) must show one background color, give or take `tol` levels.
    Returns (x, y, flip, crop with ring, background, per-pixel background) or None; the
    per-pixel background is the crop where uncovered, else the most common background."""
    im = refs[u["ref"]]
    for flip in (False, True):
        c = cov[(u["glyph"], u["W"], u["H"], flip)]
        for dy in (0, -1, 1):
            for dx in (0, -1, 1):
                x, y = u["x0"] + dx, u["y0"] + dy
                crop = im[y - 1:y + u["H"] + 1, x - 1:x + u["W"] + 1]
                if crop.shape[:2] != c.shape:
                    continue
                under = crop[c == 0]
                colors, counts = np.unique(under, axis=0, return_counts=True)
                bg = colors[counts.argmax()]
                if (np.abs(under - bg) <= tol).all():
                    bgpx = np.where((c == 0)[..., None], crop, bg)
                    return x, y, flip, crop, tuple(int(v) for v in bg), bgpx
    return None


def fit(occs, cov, alpha):
    """Per pixel (A, R, margin) such that round(R * A/255 + D * (1 - A/255)) equals every
    occurrence's ref pixel; margin <= 0 marks pixels no image can reproduce."""
    T = np.stack([o["crop"][1:-1, 1:-1] for o in occs]).astype(np.float64)
    D = np.stack([o["bgpx"][1:-1, 1:-1] for o in occs]).astype(np.float64)
    expect = alpha * cov[1:-1, 1:-1]
    shape = T.shape[1:3]
    best = np.full(shape, -1e9)
    out_a, out_r, out_m = np.zeros(shape, int), np.zeros(shape + (3,), int), np.full(shape, -1.0)
    for A in range(256):
        a = A / 255
        r0 = np.zeros(shape + (3,)) if A == 0 else np.round((T[0] - D[0] * (1 - a)) / a)
        for dr in (0,) if A == 0 else (-1, 0, 1):
            R = np.clip(r0 + dr, 0, 255)
            margin = 0.5 - np.abs(R * a + D * (1 - a) - T).max(axis=(0, 3))
            score = np.where(margin >= MARGIN, 1000 - np.abs(A - expect), margin)
            score = np.where(margin > 0, score, -1e9)
            better = score > best
            best = np.where(better, score, best)
            out_a = np.where(better, A, out_a)
            out_m = np.where(better, margin, out_m)
            out_r = np.where(better[..., None], R, out_r)
    return out_a, out_r.astype(int), out_m


def reproduces(f, o):
    A, R, _ = f
    a = (A / 255)[..., None]
    out = np.floor(R * a + o["bgpx"][1:-1, 1:-1] * (1 - a) + 0.5)
    return (out == o["crop"][1:-1, 1:-1]).all()


def greedy(occs, cov, alpha):
    """Largest prefix-greedy subset of `occs` one image reproduces, and that image."""
    f = fit(occs, cov, alpha)
    if (f[2] > 0).all():
        return occs, f
    kept, f = [], None
    for o in occs:
        trial = fit(kept + [o], cov, alpha)
        if (trial[2] > 0).all():
            kept, f = kept + [o], trial
    return kept, f


def aliases():
    names = collections.defaultdict(list)
    for p in sorted((ROOT / "assets/icons").glob("*.svg")):
        names[icons.key(p.read_text())].append(p.stem)
    return names


def save(f, path):
    A, R, _ = f
    rgba = np.dstack([np.where(A[..., None] > 0, R, 0), A]).astype(np.uint8)
    path.parent.mkdir(parents=True, exist_ok=True)
    Image.fromarray(rgba, "RGBA").save(path)


def main():
    uses = read_uses()
    cov = coverage(uses)
    refs = {r: np.array(Image.open(r).convert("RGB")).astype(np.int64)
            for r in {u["ref"] for u in uses}}
    names = aliases()
    shutil.rmtree(OUT, ignore_errors=True)

    groups = collections.defaultdict(list)
    skipped = collections.Counter()
    for u in uses:
        hit = align(u, cov, refs, 1 if UNEVEN_BG & set(names.get(u["glyph"], [])) else 0)
        if hit is None:
            skipped[u["ref"].parent.name] += 1
            continue
        u["x"], u["y"], u["flip"], u["crop"], u["bg"], u["bgpx"] = hit
        u["color"] = "%02x%02x%02x%02x" % (*u["rgb"], round_half_up(u["alpha"] * 255))
        groups[(u["glyph"], u["W"], u["H"], u["flip"], u["color"])].append(u)

    cases, unfit, written = [], 0, 0
    for (glyph, W, H, flip, color), occs in sorted(groups.items()):
        c = cov[(glyph, W, H, flip)]
        alpha = occs[0]["alpha"]
        by_pos = collections.defaultdict(list)
        for o in occs:
            by_pos[(o["x"], o["y"])].append(o)
        ordered = sorted(occs, key=lambda o: (-len(by_pos[(o["x"], o["y"])]), str(o["ref"])))
        kept, default = greedy(ordered, c, alpha)
        images = {"": default}
        for (x, y), at in by_pos.items():
            if all(reproduces(default, o) for o in at):
                continue
            kept_here, variant = greedy(at, c, alpha)
            if variant is not None:
                images[f"@{x}_{y}"] = variant
        for o in occs:
            f = images.get(f"@{o['x']}_{o['y']}", default)
            o["fitted"] = bool(reproduces(f, o))
            unfit += not o["fitted"]
        for name in names.get(glyph, []) if RUNTIME.isdisjoint(names.get(glyph, [])) else []:
            for suffix, f in images.items():
                save(f, OUT / f"{name}{'-flipx' * flip}@{W}x{H}" / f"{color}{suffix}.png")
                written += 1
        alias = (names.get(glyph) or [None])[0]
        cases += [dict(ref=str(o["ref"].relative_to(ROOT)), name=alias, flip=flip, w=o["w"],
                       h=o["h"], x=o["x"], y=o["y"], color=color, bg=list(o["bg"]),
                       css_color="%02x%02x%02x%02x" % (*o["rgb"], o["own"]),
                       opacity=o["opacity"], fitted=o["fitted"])
                  for o in occs]

    (ROOT / "tools/icons/check.json").write_text(json.dumps(cases, indent=0) + "\n")
    no_asset = sum(c["name"] is None for c in cases)
    print(f"{len(uses)} icon uses; {sum(skipped.values())} skipped (background under the icon "
          f"is not one color: {dict(skipped)})")
    print(f"{len(cases)} cases, {len(groups)} (glyph, size, color) groups, {written} images; "
          f"{unfit} cases no image reproduces; {no_asset} cases have no assets/icons svg")


if __name__ == "__main__":
    main()
