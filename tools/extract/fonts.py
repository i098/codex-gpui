"""Convert the bundled OpenAI Sans / Serif woff2 files to TTF in assets/fonts.

Run: uv run --with fonttools --with brotli python tools/extract/fonts.py
"""

import re
from pathlib import Path

from fontTools.ttLib import TTFont

ASSETS = Path("/tmp/cgx/asar-x/webview/assets")
OUT = Path(__file__).resolve().parents[2] / "assets/fonts"

OUT.mkdir(parents=True, exist_ok=True)
for src in sorted(ASSETS.glob("OpenAI*.woff2")):
    name = re.sub(r"-[0-9a-f]{12}$", "", src.stem)
    font = TTFont(src)
    font.flavor = None
    font.save(OUT / f"{name}.ttf")
    print(name, font["name"].getDebugName(1), font["name"].getDebugName(2))
