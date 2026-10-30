#!/usr/bin/env python3
"""从唯一主图 assets/icon_1024.png 生成 macOS 多分辨率图标包。"""

from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

from PIL import Image


ICON_SIZES = (
    (16, 1), (16, 2),
    (32, 1), (32, 2),
    (128, 1), (128, 2),
    (256, 1), (256, 2),
    (512, 1), (512, 2),
)


def create_iconset(image: Image.Image, directory: Path) -> None:
    directory.mkdir()
    for point_size, scale in ICON_SIZES:
        suffix = "@2x" if scale == 2 else ""
        filename = f"icon_{point_size}x{point_size}{suffix}.png"
        pixels = point_size * scale
        resized = image.resize((pixels, pixels), Image.Resampling.LANCZOS)
        resized.save(directory / filename, "PNG")


def create_mac_icon() -> None:
    assets_dir = Path(__file__).resolve().parent.parent / "assets"
    master_png = assets_dir / "icon_1024.png"
    icns_path = assets_dir / "AppIcon.icns"

    with Image.open(master_png) as source:
        if source.size != (1024, 1024):
            raise ValueError(f"图标主图必须是 1024×1024 像素：{master_png}")
        image = source.convert("RGBA")

    with TemporaryDirectory(prefix=".ikaros-icon-", dir=assets_dir) as temporary:
        temporary_dir = Path(temporary)
        iconset_dir = temporary_dir / "AppIcon.iconset"
        create_iconset(image, iconset_dir)
        generated_icns = temporary_dir / "AppIcon.icns"
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(generated_icns)],
            check=True,
        )
        generated_icns.replace(icns_path)

    print(f"✅ 已从主图生成 macOS 图标包：{icns_path}")


if __name__ == "__main__":
    create_mac_icon()
