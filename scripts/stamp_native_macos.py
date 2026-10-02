#!/usr/bin/env python3
"""Stamp the native shell and embedded host from the same product/source build."""

import argparse
import json
import plistlib
import subprocess
from pathlib import Path


def stamp(path, *, version, revision, dirty, identifier=None, configuration="debug"):
    with path.open("rb") as source:
        info = plistlib.load(source)
    info.update(
        CFBundleShortVersionString=version,
        CFBundleVersion=version,
        WispSourceRevision=revision,
        WispSourceDirty=dirty,
        WispBuildConfiguration=configuration,
    )
    if identifier is not None:
        info["CFBundleIdentifier"] = identifier
    with path.open("wb") as target:
        plistlib.dump(info, target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    parser.add_argument("host_identifier")
    parser.add_argument("--configuration", choices=("debug", "release"), default="debug")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    version = json.loads((root / "src-tauri/tauri.conf.json").read_text())["version"]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    # Include untracked sources: a local preview must not claim a clean release.
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True).strip())
    metadata = dict(version=version, revision=revision, dirty=dirty, configuration=args.configuration)
    stamp(args.app / "Contents/Info.plist", **metadata)
    stamp(args.app / "Contents/Helpers/Wisp Desktop Host.app/Contents/Info.plist",
          identifier=args.host_identifier, **metadata)


if __name__ == "__main__":
    main()
