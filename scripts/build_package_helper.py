#!/usr/bin/env python3
"""Rebuild the embedded helper; ordinary Cargo builds do not require an Android SDK."""

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


def version_key(path):
    return tuple(int(part) for part in re.findall(r"\d+", path.name))


def main():
    root = Path(__file__).resolve().parents[1]
    default_sdk = Path.home() / ("Library/Android/sdk" if os.sys.platform == "darwin" else "Android/Sdk")
    sdk = Path(os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME") or default_sdk)
    platforms = list((sdk / "platforms").glob("*/android.jar"))
    tools = list((sdk / "build-tools").glob("*/d8"))
    if not platforms or not tools:
        raise SystemExit("Set ANDROID_SDK_ROOT to an SDK containing an Android platform and build-tools.")
    platform = max(platforms, key=lambda path: version_key(path.parent))
    d8 = max(tools, key=lambda path: version_key(path.parent))
    helper = root / "crates/ikaros-adb/device-helper"
    with tempfile.TemporaryDirectory(prefix="ikaros-package-helper-") as directory:
        build = Path(directory)
        classes = build / "classes"
        dex = build / "dex"
        classes.mkdir()
        dex.mkdir()
        subprocess.run([
            "javac", "--release", "8", "-Xlint:-options", "-classpath", str(platform),
            "-d", str(classes), str(helper / "PackageMetadata.java"),
        ], check=True)
        subprocess.run([
            str(d8), "--min-api", "23", "--lib", str(platform), "--output", str(dex),
            *map(str, classes.rglob("*.class")),
        ], check=True)
        shutil.copyfile(dex / "classes.dex", helper / "package-metadata.dex")
    print(f"Built {helper / 'package-metadata.dex'}")


if __name__ == "__main__":
    main()
