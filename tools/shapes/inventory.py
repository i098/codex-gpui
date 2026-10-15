"""Inventory of the box decorations in refs/*/*.spec.json that crates/shapes reproduces.

Every painted rounded background, rounded border and box-shadow layer becomes a layer key in
device px (2x): kind, corner shape, Blink-constrained radii [tl, tr, br, bl], border width or
shadow (x, y, blur, spread, inset). Writes tools/shapes/cases.json for the lab
(crates/shapes/examples/lab.rs): the keys with their ref counts and one verification sample
per key (shadow samples get a backdrop the shadow color shows on). Prints the count table.

`corner-shape` is not in the spec dump, so every rounded key gets a round and a
superellipse(1.5) variant; the ref count belongs to the pair.

Run: python3 tools/shapes/inventory.py
"""

import glob
import json
import math
import re
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORNERS = ["borderTopLeftRadius", "borderTopRightRadius", "borderBottomRightRadius", "borderBottomLeftRadius"]
SIDES = ["Top", "Right", "Bottom", "Left"]


def split_top(s):
    return [p.strip() for p in re.split(r",(?![^(]*\))", s) if p.strip()]


def oklab_to_srgb(L, a, b):
    l_, m_, s_ = L + 0.3963377774 * a + 0.2158037573 * b, L - 0.1055613458 * a - 0.0638541728 * b, L - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_**3, m_**3, s_**3
    lin = (4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
           -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
           -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s)
    enc = lambda c: 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055
    return [enc(min(max(c, 0), 1)) for c in lin]


def rgba8(css):
    """CSS color -> [r, g, b, a] bytes (Skia SkColor: round(x * 255))."""
    nums = [float(x) for x in re.findall(r"-?[\d.]+(?:e-?\d+)?", css)]
    if css.startswith("rgb"):
        r, g, b, a = (nums + [1])[:4]
        return [round(r), round(g), round(b), round(a * 255)]
    if css.startswith("color(srgb"):
        r, g, b, a = (nums + [1])[:4]
    elif css.startswith("oklab"):
        r, g, b = oklab_to_srgb(*nums[:3])
        a = (nums + [1])[3]
    else:
        raise ValueError(css)
    return [round(r * 255), round(g * 255), round(b * 255), round(a * 255)]


def px(v):
    return float(v[:-2]) * 2


def shadows(s):
    """Visible box-shadow layers in paint order (CSS lists the top layer first)."""
    out = []
    for layer in split_top(s.get("boxShadow", "")):
        m = re.match(r"((?:\w+\([^)]*\))|\S+)\s+(.*)", layer)
        color, rest = m.group(1), m.group(2).split()
        inset = "inset" in rest
        x, y, blur, spread = [px(v) for v in rest if v != "inset"]
        c = rgba8(color)
        if c[3]:
            out.append(dict(color=c, x=x, y=y, blur=blur, spread=spread, inset=inset))
    return out[::-1]


def snap(rect):
    x, y, w, h = rect
    l, t = round(x * 2), round(y * 2)
    return l, t, round((x + w) * 2) - l, round((y + h) * 2) - t


def eff_radii(s, w, h):
    """Blink `ConstrainRadii` on the pixel-snapped border box, device px."""
    r = []
    for k in CORNERS:
        v = s.get(k, "0px")
        if v.endswith("%"):
            assert w == h, (v, w, h)
            r.append(w * float(v[:-1]) / 100)
        else:
            r.append(px(v))
    tl, tr, br, bl = r
    f = min([side / sum_ for side, sum_ in ((w, tl + tr), (w, bl + br), (h, tl + bl), (h, tr + br)) if sum_] + [1])
    return [v * f for v in r]


def backdrop(nodes, n):
    """Nearest opaque ancestor background (spec ids are list indexes; a hidden parent's id can
    point at a later node, so only walk to lower ids)."""
    while 0 <= n["parent"] < n["id"]:
        n = nodes[n["parent"]]
        c = n["style"].get("backgroundColor")
        if c and rgba8(c)[3] == 255:
            return rgba8(c)
    return [0, 0, 0, 255]


def node_layers(s, w, h):
    radii = eff_radii(s, w, h)
    rounded = any(radii)
    layers = [dict(kind="shadow", **sh) for sh in shadows(s) if not sh["inset"]]
    bg = s.get("backgroundColor")
    if rounded and bg and rgba8(bg)[3]:
        layers.append(dict(kind="fill", color=rgba8(bg)))
    layers += [dict(kind="shadow", **sh) for sh in shadows(s) if sh["inset"]]
    widths = {s.get(f"border{d}Width", "0px") for d in SIDES}
    colors = {s.get(f"border{d}Color", s["color"]) for d in SIDES}
    if rounded and widths != {"0px"}:
        assert len(widths) == 1 and len(colors) == 1, (widths, colors)
        c = rgba8(colors.pop())
        if c[3]:
            layers.append(dict(kind="border", width=px(widths.pop()), color=c))
    return radii, layers


def key_of(layer, radii):
    k = dict(kind=layer["kind"], radii=radii)
    k.update({f: layer[f] for f in ("width", "x", "y", "blur", "spread", "inset") if f in layer})
    return json.dumps(k, sort_keys=True)


def reach(k):
    """Device px a layer's coverage extends beyond the border box (outer shadows)."""
    if k["kind"] != "shadow" or k["inset"]:
        return 0
    return math.ceil(max(abs(k["x"]), abs(k["y"])) + max(k["spread"], 0) + 2 * k["blur"]) + 2


def invisible(color, backdrop):
    """A shadow color that leaves its backdrop unchanged (black on black) tests nothing."""
    return max(abs(c - b) for c, b in zip(color[:3], backdrop[:3])) * color[3] < 255 * 16


def main():
    keys = defaultdict(lambda: dict(count=0, sample=None))
    for path in sorted(glob.glob(str(ROOT / "refs/*/*.spec.json"))):
        nodes = json.load(open(path))["nodes"]
        for n in nodes:
            s = n["style"]
            _, _, w, h = snap(n["rect"])
            if not w or not h:
                continue
            radii, layers = node_layers(s, w, h)
            radii = [round(r * 64) / 64 for r in radii]
            for layer in layers:
                e = keys[key_of(layer, radii)]
                e["count"] += 1
                sample = dict(w=w, h=h, color=layer["color"], backdrop=backdrop(nodes, n))
                if e["sample"] is None or invisible(e["sample"]["color"], e["sample"]["backdrop"]):
                    e["sample"] = sample
    cases = []
    for key, e in sorted(keys.items(), key=lambda t: -t[1]["count"]):
        k = json.loads(key)
        shapes = ["round", "squircle"] if any(k["radii"]) else ["round"]
        sample = e["sample"]
        if invisible(sample["color"], sample["backdrop"]):
            sample["backdrop"] = [255, 255, 255, 255] if sum(sample["color"][:3]) < 384 else [0, 0, 0, 255]
        for corner in shapes:
            cases.append(dict(k, corner=corner, count=e["count"], reach=reach(k), sample=sample))
    (ROOT / "tools/shapes/cases.json").write_text(json.dumps(cases, indent=1) + "\n")
    print("| kind | params (device px) | corner | ref nodes |")
    print("|---|---|---|---|")
    for c in cases:
        p = {f: c[f] for f in ("radii", "width", "x", "y", "blur", "spread", "inset") if f in c}
        print(f"| {c['kind']} | {p} | {c['corner']} | {c['count']} |")


if __name__ == "__main__":
    main()
