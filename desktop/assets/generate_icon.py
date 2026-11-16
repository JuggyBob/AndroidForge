"""Generate AndroidForge desktop icon PNG/ICO from the same geometry as icon.svg.

Run from the repository root:
    python desktop/assets/generate_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent
SCALE = 1024
BG = "#0B0E13"
BORDER = "#252C39"
ACCENT = "#F5A524"
BLUE = "#58A6FF"
GREEN = "#3FB950"


def scaled(value: float) -> int:
    return round(value * SCALE / 512)


def main() -> None:
    image = Image.new("RGBA", (SCALE, SCALE), BG)
    draw = ImageDraw.Draw(image)

    draw.rounded_rectangle(
        [0, 0, SCALE - 1, SCALE - 1],
        radius=scaled(112),
        fill=BG,
    )
    draw.rounded_rectangle(
        [scaled(20), scaled(20), SCALE - scaled(20), SCALE - scaled(20)],
        radius=scaled(96),
        outline=BORDER,
        width=scaled(8),
    )

    hexagon = [
        (scaled(256), scaled(82)),
        (scaled(401), scaled(165)),
        (scaled(401), scaled(347)),
        (scaled(256), scaled(430)),
        (scaled(111), scaled(347)),
        (scaled(111), scaled(165)),
        (scaled(256), scaled(82)),
    ]
    draw.line(hexagon, fill=ACCENT, width=scaled(24), joint="curve")
    draw.polygon(
        [(scaled(232), scaled(205)), (scaled(318), scaled(256)), (scaled(232), scaled(307))],
        fill=ACCENT,
    )

    radius = scaled(18)
    cx, cy = scaled(401), scaled(347)
    draw.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=BLUE)

    radius = scaled(12)
    cx, cy = scaled(111), scaled(165)
    draw.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=GREEN)

    image.resize((512, 512), Image.Resampling.LANCZOS).save(ROOT / "icon.png")
    image.resize((256, 256), Image.Resampling.LANCZOS).save(
        ROOT / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )


if __name__ == "__main__":
    main()
