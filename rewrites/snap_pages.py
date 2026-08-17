#!/usr/bin/env python3
"""Full-page PNG screenshots of build/typ-pages/<id>.html via headless
Chrome, trimmed of trailing background and split into readable chunks.

Usage: snap_pages.py OUTDIR ID [ID ...]
Writes OUTDIR/<id>-c0.png, <id>-c1.png, ... (1280 wide, <=2400 tall).
"""

import subprocess
import sys
from pathlib import Path

from PIL import Image

CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
ROOT = Path(__file__).resolve().parent.parent
PAGES = ROOT / "build/typ-pages"
SHOT_H = 16384
CHUNK_H = 2400
OVERLAP = 60
MAX_CHUNKS = 6


def trim(im: Image.Image) -> Image.Image:
    """Crop trailing rows that equal the page background color."""
    rgb = im.convert("RGB")
    w, h = rgb.size
    r, g, b = rgb.getpixel((5, h - 5))  # type: ignore[misc]
    bg_row = bytes((r, g, b)) * w
    y = h - 1
    step = 256
    while y > 0:
        if rgb.crop((0, y, w, y + 1)).tobytes() != bg_row:
            break
        y -= step if y - step > 0 else 1
    y = min(h - 1, y + step)
    while y > 0 and rgb.crop((0, y, w, y + 1)).tobytes() == bg_row:
        y -= 1
    return rgb.crop((0, 0, w, min(h, y + 40)))


def snap(page_id: str, outdir: Path) -> list[Path]:
    html = PAGES / f"{page_id}.html"
    if not html.exists():
        print(f"{page_id}: no html", file=sys.stderr)
        return []
    raw = outdir / f"{page_id}-raw.png"
    subprocess.run(
        [
            CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
            f"--screenshot={raw}", f"--window-size=1280,{SHOT_H}",
            "--virtual-time-budget=8000",
            f"file://{html}",
        ],
        capture_output=True,
        timeout=60,
    )
    if not raw.exists():
        print(f"{page_id}: screenshot failed", file=sys.stderr)
        return []
    im = trim(Image.open(raw))
    raw.unlink()
    outs = []
    y = 0
    for i in range(MAX_CHUNKS):
        if y >= im.height:
            break
        chunk = im.crop((0, y, im.width, min(im.height, y + CHUNK_H)))
        p = outdir / f"{page_id}-c{i}.png"
        chunk.save(p)
        outs.append(p)
        y += CHUNK_H - OVERLAP
    return outs


def main():
    outdir = Path(sys.argv[1])
    outdir.mkdir(parents=True, exist_ok=True)
    for pid in sys.argv[2:]:
        outs = snap(pid, outdir)
        print(f"{pid}: {len(outs)} chunks", flush=True)


if __name__ == "__main__":
    main()
