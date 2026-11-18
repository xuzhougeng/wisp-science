"""Package a built GPUI desktop (`wisp-gpui`) as a preview download.

Two steps, so a platform's signing can happen between them:

    python scripts/package_gpui.py stage   --os macos --arch aarch64 --binary <wisp-gpui> --out dist
    python scripts/package_gpui.py archive --os macos --arch aarch64 --out dist

`stage` lays out what the executable needs beside it; `archive` packs that
into the one file a release offers and prints its path.

    macOS    dist/wisp-science-gpui.app             -> wisp-science-gpui_<version>_<arch>.app.zip
    Windows  dist/wisp-science-gpui/wisp-gpui.exe   -> wisp-science-gpui_<version>_x64.zip
    Linux    dist/wisp-science-gpui/wisp-gpui       -> wisp-science-gpui_<version>_amd64.tar.gz

The executable finds the copied folders through `apps/gpui/src/resources.rs`:
`Contents/Resources` in the macOS bundle, `resources/` beside it elsewhere.

File names follow the WebView installers' `wisp-science_<version>_<arch>.<ext>`
pattern (Tauri's: `x64`/`aarch64` on macOS and Windows, `amd64` on Linux) with
the product name `wisp-science-gpui` in place of `wisp-science`. The download
picker and the updater manifest build exact `wisp-science_<version>_<suffix>`
names, so they cannot pick these up.

Standard library only; runs on the macOS, Windows and Linux runners.
"""

import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import zipfile


# The WebView desktop is `wisp-science` / `science.wisp-science`
# (src-tauri/tauri.conf.json); the GPUI desktop adds `gpui` to both.
PRODUCT = "wisp-science-gpui"
BUNDLE_ID = "science.wisp-science.gpui"
# Oldest macOS the release build targets (MACOSX_DEPLOYMENT_TARGET in
# .github/workflows/release-gpui.yml).
MACOS_MINIMUM = "11.0"
# The folders wisp-paths resolves; the same set the WebView bundle carries
# (`bundle.resources` in src-tauri/tauri.conf.json).
RESOURCES = ("skills", "python", "r", "browser-extension", "seed")
IGNORED = shutil.ignore_patterns("__pycache__", "*.pyc", ".DS_Store", "Thumbs.db")
SYSTEMS = ("macos", "windows", "linux")
ARCHITECTURES = ("aarch64", "x64")
# Linux packages name architectures the Debian way (`wisp-science_<v>_amd64.deb`).
LINUX_ARCH = {"x64": "amd64", "aarch64": "arm64"}

README = """wisp-science-gpui {version} (preview)

This is a preview of Wisp Science's new desktop. It does not yet do everything
the regular Wisp Science desktop does, and it does not update itself: download
a newer preview from the release page. The regular installers are the files
named wisp-science_{version}_* on the same page.

The preview keeps its own data (a folder named wisp-science-gpui) and does not
touch the regular desktop's. "Bring over" on its home page copies projects in.

{launch}

Keep the "resources" folder beside the program: it holds the bundled skills
and the Python and R workers.

Documentation: https://github.com/xuzhougeng/wisp-science/blob/main/docs/gpui-desktop.md
"""
LAUNCH = {
    "windows": "Run wisp-gpui.exe from this folder.",
    "linux": "Run ./wisp-gpui from this folder. It needs a working graphics driver\n"
             "(Vulkan) and a desktop session (X11 or Wayland).",
}


def workspace_version(repo):
    """The version every Wisp package shares: `[workspace.package]` in Cargo.toml."""
    text = (Path(repo) / "Cargo.toml").read_text(encoding="utf-8")
    section = re.search(r"^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    version = section and re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), re.M)
    if not version:
        raise SystemExit("Cargo.toml has no [workspace.package] version")
    return version.group(1)


def archive_name(version, system, arch):
    """`wisp-science-gpui_<version>_<suffix>`, the suffix spelled as Tauri spells
    the WebView installers' on the same system."""
    if system == "macos":
        suffix = f"{arch}.app.zip"
    elif system == "windows":
        suffix = f"{arch}.zip"
    else:
        suffix = f"{LINUX_ARCH.get(arch, arch)}.tar.gz"
    return f"{PRODUCT}_{version}_{suffix}"


def staged_path(out, system):
    return Path(out) / (f"{PRODUCT}.app" if system == "macos" else PRODUCT)


def executable_name(system):
    return "wisp-gpui.exe" if system == "windows" else "wisp-gpui"


def resource_root(staged, system):
    return staged / "Contents" / "Resources" if system == "macos" else staged / "resources"


def executable_path(staged, system):
    folder = staged / "Contents" / "MacOS" if system == "macos" else staged
    return folder / executable_name(system)


def info_plist(version):
    return {
        "CFBundleName": PRODUCT,
        "CFBundleDisplayName": PRODUCT,
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleExecutable": executable_name("macos"),
        "CFBundleIconFile": "icon.icns",
        "CFBundlePackageType": "APPL",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
        "LSMinimumSystemVersion": MACOS_MINIMUM,
        "NSHighResolutionCapable": True,
        "NSSupportsAutomaticGraphicsSwitching": True,
    }


def stage(repo, binary, system, out):
    """Lay out the executable and the folders it reads. Returns the staged path."""
    repo, binary = Path(repo), Path(binary)
    if not binary.is_file():
        raise SystemExit(f"No executable at {binary}")
    version = workspace_version(repo)
    staged = staged_path(out, system)
    if staged.exists():
        shutil.rmtree(staged)
    target = executable_path(staged, system)
    target.parent.mkdir(parents=True)
    shutil.copy2(binary, target)
    if system != "windows":
        target.chmod(target.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    resources = resource_root(staged, system)
    for name in RESOURCES:
        source = repo / name
        if not source.is_dir():
            raise SystemExit(f"Missing bundled folder: {source}")
        shutil.copytree(source, resources / name, symlinks=True, ignore=IGNORED)

    if system == "macos":
        shutil.copy2(repo / "src-tauri" / "icons" / "icon.icns", resources / "icon.icns")
        with open(staged / "Contents" / "Info.plist", "wb") as file:
            plistlib.dump(info_plist(version), file)
    else:
        shutil.copy2(repo / "LICENSE", staged / "LICENSE")
        with open(staged / "README.txt", "w", encoding="utf-8",
                  newline="\r\n" if system == "windows" else "\n") as file:
            file.write(README.format(version=version, launch=LAUNCH[system]))
    check(staged, system)
    return staged


def check(staged, system):
    """Refuse to pack a layout the executable could not run from."""
    wanted = [executable_path(staged, system)]
    wanted += [resource_root(staged, system) / name for name in RESOURCES]
    if system == "macos":
        wanted += [staged / "Contents" / "Info.plist", resource_root(staged, system) / "icon.icns"]
    missing = [str(path) for path in wanted if not path.exists()]
    if not any(resource_root(staged, system).glob("skills/*/SKILL.md")):
        missing.append(str(resource_root(staged, system) / "skills/*/SKILL.md"))
    if missing:
        raise SystemExit("Incomplete package, missing: " + ", ".join(missing))


def members(staged):
    """Every folder and file under the staged tree, in a stable order."""
    for folder, folders, files in os.walk(staged):
        folders.sort()
        yield Path(folder)
        for name in sorted(files):
            yield Path(folder) / name


def archive(repo, system, arch, out):
    """Pack the staged tree into the release file. Returns the archive path."""
    staged = staged_path(out, system)
    check(staged, system)
    path = Path(out) / archive_name(workspace_version(repo), system, arch)
    if path.exists():
        path.unlink()
    if system == "macos":
        # ditto keeps the bundle's symlinks, modes and signature intact, and
        # is the archive form notarytool accepts.
        subprocess.run(["ditto", "-c", "-k", "--keepParent", str(staged), str(path)], check=True)
    elif system == "windows":
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
            for member in members(staged):
                if member.is_file():
                    bundle.write(member, member.relative_to(staged.parent).as_posix())
    else:
        # tar keeps the executable bit, which a zip made here would lose.
        with tarfile.open(path, "w:gz") as bundle:
            bundle.add(staged, arcname=staged.name)
    return path


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("step", choices=("stage", "archive"))
    parser.add_argument("--os", dest="system", required=True, choices=SYSTEMS)
    parser.add_argument("--arch", required=True, choices=ARCHITECTURES)
    parser.add_argument("--out", required=True, help="folder the package is built in")
    parser.add_argument("--repo", default=Path(__file__).resolve().parent.parent,
                        help="checkout the version and bundled folders come from")
    parser.add_argument("--binary", help="the built wisp-gpui executable (stage)")
    args = parser.parse_args(argv)
    if args.step == "stage":
        if not args.binary:
            parser.error("stage needs --binary")
        result = stage(args.repo, args.binary, args.system, args.out)
    else:
        result = archive(args.repo, args.system, args.arch, args.out)
    # Forward slashes, so the path reads the same in bash on every runner.
    print(Path(result).as_posix())


if __name__ == "__main__":
    sys.exit(main())
