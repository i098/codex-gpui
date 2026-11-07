#!/usr/bin/env python3
"""Summarise raw runs: python3 analyze.py raw  -> table of median (min-max) per series/app/kind."""
import json, glob, os, re, statistics as st, sys
root = sys.argv[1] if len(sys.argv) > 1 else "raw"
groups = {}
for p in sorted(glob.glob(f"{root}/*/result.json")):
    name = os.path.basename(os.path.dirname(p))
    m = re.match(r"([ABCD])-(orig|ours)-(cold|warm|occ|plain|thread|scroll)-?(\d*)", name)
    if not m: continue
    groups.setdefault(m.groups()[:3], []).append(json.load(open(p)))
keys = ["first_frame", "shell_dom", "shell_frame", "sidebar_dom", "sidebar_frame", "cdp_target", "mock_first_ms", "mock_thread_list_ms",
        "rss_mb_30s", "rss_mb_90s", "footprint_mb_90s", "cpu_pct_30_90", "nproc"]
def f(v):
    v = [x for x in v if isinstance(x, (int, float))]
    if not v: return "-"
    return f"{st.median(v):.1f} ({min(v):.1f}-{max(v):.1f}) n={len(v)}"
for g, runs in groups.items():
    print("==", "/".join(g))
    for k in keys:
        print(f"  {k:22s} {f([r.get(k) for r in runs])}")
