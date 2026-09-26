#!/usr/bin/env python3
"""Sign, notarize and package an optimized SwiftUI preview for GitHub Releases.

Uses the macOS release job's temporary Developer ID keychain and Apple secrets.
Never changes release notes or the stable Tauri updater manifest.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile

ARCHES = {"aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64"}


def run(args):
    result = subprocess.run([str(x) for x in args], capture_output=True, text=True)
    if result.returncode:
        detail = (result.stdout + result.stderr).strip()
        password = os.environ.get("APPLE_PASSWORD")
        if password:
            detail = detail.replace(password, "[redacted]")
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): {detail}")
    return result.stdout.strip()


def validate(app, target, tag):
    helper = app / "Contents/Helpers/Wisp Desktop Host.app"
    infos = [plistlib.loads((p / "Contents/Info.plist").read_bytes()) for p in (app, helper)]
    shell, host = infos
    if shell.get("CFBundleIdentifier") != "science.wisp-science.native-preview":
        raise ValueError("Only the distributable preview bundle may be published (not QA).")
    if host.get("CFBundleIdentifier") != "science.wisp-science":
        raise ValueError("Unexpected embedded host identifier.")
    version = shell.get("CFBundleShortVersionString", "")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", version) or tag != "v" + version:
        raise ValueError("Release tag must match the bundle version.")
    for key in ("CFBundleShortVersionString", "CFBundleVersion", "WispSourceRevision"):
        if not shell.get(key) or shell[key] != host.get(key):
            raise ValueError(f"Shell/helper {key} mismatch.")
    if not re.fullmatch(r"[0-9a-f]{40}", shell["WispSourceRevision"]):
        raise ValueError("Missing full source revision.")
    for info in infos:
        if info.get("WispSourceDirty") is not False or info.get("WispBuildConfiguration") != "release":
            raise ValueError("Publishing requires clean source and an optimized release build.")
    binaries = [app / "Contents/MacOS/WispSciencePreview", app / "Contents/MacOS/wisp-service",
                helper / "Contents/MacOS/wisp-tauri"]
    for binary in binaries:
        if run(["lipo", "-archs", binary]).split() != [ARCHES[target]]:
            raise ValueError(f"Wrong architecture: {binary.name}")
    return version, helper


def notarize(path, log):
    result = json.loads(run(["xcrun", "notarytool", "submit", path,
                             "--apple-id", os.environ["APPLE_ID"],
                             "--password", os.environ["APPLE_PASSWORD"],
                             "--team-id", os.environ["APPLE_TEAM_ID"],
                             "--wait", "--timeout", "30m", "--output-format", "json"]))
    log.write_text(json.dumps(result, indent=2) + "\n")
    if result.get("status") != "Accepted":
        raise RuntimeError(f"Notarization was not accepted; inspect {log.name}.")


def package(app, target, tag, output):
    version, _ = validate(app, target, tag)
    for key in ("APPLE_SIGNING_IDENTITY", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"):
        if not os.environ.get(key) or os.environ[key] == "-":
            raise ValueError(f"{key} is required; release packaging cannot use ad-hoc signing.")
    output.mkdir(parents=True, exist_ok=True)
    stem = f"Wisp-Science-SwiftUI-Preview_{version}_{target.removesuffix('-apple-darwin')}"
    dmg = output / (stem + ".dmg")
    identity = os.environ["APPLE_SIGNING_IDENTITY"]
    # Work on a copy: never invalidate the CI/debug bundle if notarization fails.
    with tempfile.TemporaryDirectory(prefix="wisp-swiftui-package-") as temp:
        work = Path(temp)
        payload = work / "payload"
        payload.mkdir()
        staged = payload / app.name
        run(["ditto", app, staged])
        helper = staged / "Contents/Helpers/Wisp Desktop Host.app"
        for code in (staged / "Contents/MacOS/wisp-service", helper, staged):
            run(["codesign", "--force", "--options", "runtime", "--timestamp", "--sign", identity, code])
        run(["codesign", "--verify", "--deep", "--strict", staged])
        archive = work / "notarize.zip"
        run(["ditto", "-c", "-k", "--keepParent", staged, archive])
        notarize(archive, output / (stem + "-app-notary.json"))
        run(["xcrun", "stapler", "staple", staged])
        run(["xcrun", "stapler", "validate", staged])
        run(["spctl", "--assess", "--type", "execute", staged])
        (payload / "Applications").symlink_to("/Applications")
        candidate = work / (stem + ".dmg")
        run(["hdiutil", "create", "-volname", "Wisp Science SwiftUI Preview", "-srcfolder", payload,
             "-format", "UDZO", candidate])
        run(["codesign", "--force", "--timestamp", "--sign", identity, candidate])
        notarize(candidate, output / (stem + "-dmg-notary.json"))
        run(["xcrun", "stapler", "staple", candidate])
        run(["xcrun", "stapler", "validate", candidate])
        shutil.copy2(candidate, dmg)
    digest = hashlib.sha256(dmg.read_bytes()).hexdigest()
    (output / (stem + ".dmg.sha256")).write_text(f"{digest}  {dmg.name}\n")
    return dmg


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    parser.add_argument("--target", choices=ARCHES, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(package(args.app.resolve(), args.target, args.tag, args.output.resolve()))
    except (ValueError, RuntimeError, OSError, KeyError) as error:
        parser.exit(1, f"SwiftUI preview packaging failed: {error}\n")


if __name__ == "__main__":
    main()
