"""Renders icons.html with headless Microsoft Edge and cuts out the icons
as the game's TGA files, into the TPF3-MP mod.

    python tools/art/icons/render.py

Needs Pillow and Edge (Windows). The TGAs are committed; run this again
only after changing icons.html.
"""

import pathlib
import subprocess
import tempfile

from PIL import Image

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / "mod" / "tpf3mp_1" / "content" / "gui" / "tpf3mp" / "icons"
EDGE = pathlib.Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")

CELL = 256
COLUMNS = 12
BUTTONS = ["multiplayer", "minimap", "chat", "invite"]
STATES = ["normal", "selected", "unavailable"]
MARKERS = ["passenger", "cargo", "bus", "tram", "truck", "train", "ship", "aircraft", "depot"]
# The size the game gets: bottom-bar discs, and map markers.
BUTTON_SIZE = 128
MARKER_SIZE = 96


def screenshot(path):
    cells = len(BUTTONS) * len(STATES) + len(MARKERS)
    rows = -(-cells // COLUMNS)
    with tempfile.TemporaryDirectory() as profile:
        subprocess.run(
            [
                str(EDGE),
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                "--default-background-color=00000000",
                f"--window-size={CELL * COLUMNS},{CELL * rows}",
                "--virtual-time-budget=3000",
                f"--user-data-dir={profile}",
                f"--screenshot={path}",
                (HERE / "icons.html").as_uri(),
            ],
            check=True,
            capture_output=True,
        )


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as work:
        sheet_path = pathlib.Path(work) / "sheet.png"
        screenshot(sheet_path)
        sheet = Image.open(sheet_path).convert("RGBA")
    cells = [(f"button_{b}" + ("" if s == "normal" else f"_{s}"), True) for b in BUTTONS for s in STATES]
    cells += [(f"marker_{m}", False) for m in MARKERS]
    preview = Image.new("RGBA", (CELL * COLUMNS // 2, CELL * 2 // 2 * 1), (62, 70, 73, 255))
    for index, (name, button) in enumerate(cells):
        x, y = (index % COLUMNS) * CELL, (index // COLUMNS) * CELL
        icon = sheet.crop((x, y, x + CELL, y + CELL))
        if button:
            size = BUTTON_SIZE
        else:
            # White glyph on black: brightness becomes the mask.
            alpha = icon.convert("L")
            icon = Image.new("RGBA", icon.size, (255, 255, 255, 0))
            icon.putalpha(alpha)
            size = MARKER_SIZE
        out = icon.resize((size, size), Image.LANCZOS)
        out.save(OUT / f"{name}.tga")
        small = icon.resize((CELL // 2, CELL // 2), Image.LANCZOS)
        preview.alpha_composite(small, ((index % COLUMNS) * CELL // 2, (index // COLUMNS) * CELL // 2))
    preview.save(HERE / "preview.png")
    print(f"{len(cells)} icons in {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
