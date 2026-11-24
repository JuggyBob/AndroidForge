"""Render the AndroidForge worker-robot icon from icon.svg.

The SVG is the source of truth; Edge/Chrome renders it at 512px, then Pillow
writes the PNG and multi-size Windows ICO.

Run from the repository root:
    python desktop/assets/generate_icon.py
"""

import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent
SVG = ROOT / "icon.svg"
PNG = ROOT / "icon.png"
ICO = ROOT / "icon.ico"

BROWSER_CANDIDATES = [
    Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
    Path(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
    Path(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
    Path(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
]


def find_browser() -> Path:
    for candidate in BROWSER_CANDIDATES:
        if candidate.is_file():
            return candidate
    for name in ("msedge", "chrome"):
        found = shutil.which(name)
        if found:
            return Path(found)
    raise SystemExit("Edge or Chrome is required to render desktop/assets/icon.svg.")


def render_svg(browser: Path) -> Image.Image:
    with tempfile.TemporaryDirectory(prefix="androidforge-icon-") as directory:
        directory = Path(directory)
        screenshot = directory / "icon.png"
        profile = directory / "profile"
        subprocess.run(
            [
                str(browser),
                "--headless=new",
                "--disable-gpu",
                "--disable-extensions",
                "--disable-sync",
                "--hide-scrollbars",
                "--no-first-run",
                "--force-device-scale-factor=1",
                f"--user-data-dir={profile}",
                "--window-size=512,512",
                f"--screenshot={screenshot}",
                SVG.as_uri(),
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=120,
        )
        image = Image.open(screenshot)
        with image:
            image = image.convert("RGBA")
        if image.size != (512, 512):
            image = image.resize((512, 512), Image.Resampling.LANCZOS)
        return image


def main() -> None:
    image = render_svg(find_browser())
    image.save(PNG)
    image.resize((256, 256), Image.Resampling.LANCZOS).save(
        ICO,
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )


if __name__ == "__main__":
    main()
