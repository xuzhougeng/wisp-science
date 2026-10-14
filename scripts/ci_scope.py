"""Select CI jobs from the complete PR or push diff, without an API file limit."""

import json
import os
from pathlib import Path
import re
import subprocess


SCOPES = (
    "rust", "headless", "desktop", "ui", "macos", "windows", "ios", "release_notes",
    "shared_assets", "frontend_assets",
)
ALL = frozenset(SCOPES)
APPLICATIONS = ALL - {"release_notes", "frontend_assets"}
BACKEND = frozenset({"rust", "headless", "desktop", "macos", "windows"})
NATIVE = frozenset({"macos", "windows", "ios", "shared_assets"})
TEST = ALL - NATIVE
DESIGN_INPUTS = frozenset({
    "ui/src/app_support/messages.rs", "ui/src/i18n.rs", "ui/src/app_support/settings.rs",
    "ui/src/automation.rs", "ui/src/settings_view.rs", "ui/src/styles/base.css",
    "ui/src/styles/chat.css", "docs/assets/wordmark-light.svg", "docs/assets/wordmark-dark.svg",
    "apps/macos/native-settings-english.json",
})


def scopes_for_path(path):
    if path in {"scripts/ci_scope.py", "scripts/test_ci_scope.py",
                ".github/workflows/ci-changes.yml"}:
        return ALL
    if path == ".github/workflows/test.yml":
        return TEST
    if path == ".github/workflows/native-preview.yml":
        return NATIVE
    if path.startswith("apps/ios/"):
        return {"ios"}
    # WinUI links this particular resource directory from the macOS tree.
    # Ordinary Swift/C# edits do not require the other platform's build.
    if path.startswith("apps/macos/Sources/WispProjectBrowserUI/Resources/"):
        return {"macos", "windows", "shared_assets"}
    if path.startswith("apps/macos/"):
        return {"macos", "shared_assets"} if path in DESIGN_INPUTS else {"macos"}
    if path.startswith("apps/windows/"):
        return {"windows", "shared_assets"}
    if path.startswith(("crates/wisp-dto/", "crates/wisp-sync/", "src-tauri/", "contracts/", ".cargo/")):
        return APPLICATIONS
    if path.startswith("crates/"):
        return BACKEND
    if path.startswith("vendor/leptos_reactive/"):
        return {"ui"}
    if path.startswith("vendor/tao/"):
        return {"rust", "desktop", "macos", "windows"}
    if path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}:
        return APPLICATIONS
    if path.startswith(("python/", "skills/", "seed/", "r/", "browser-extension/")):
        return BACKEND
    if path.startswith("ui/"):
        scopes = {"ui"}
        if path in {"ui/build-fast.mjs", "ui/build-fast.test.mjs"}:
            scopes.add("frontend_assets")
        if path in DESIGN_INPUTS:
            scopes.add("shared_assets")
        # These are actually bundled by native clients, unlike general Leptos
        # source which is validated by the independent wasm/browser jobs.
        if path.startswith("ui/vendor-src/") or path in {
            "ui/native-host.html", "ui/sync-vendor.mjs", "ui/package.json", "ui/package-lock.json",
        }:
            scopes.update({"macos", "windows", "shared_assets"})
        return scopes
    if path.startswith("ui-tests/website-tests/") or path == "ui-tests/playwright.website.config.ts":
        return set()  # website.yml has its own path filters.
    if path.startswith("ui-tests/"):
        return {"ui"}
    if path.startswith("scripts/"):
        name = Path(path).name
        if name == "github_release_notes.sh":
            return {"release_notes"}
        if name in {"sync_native_design.py", "sync_native_settings_contract.py",
                    "test_sync_native_settings_contract.py"}:
            return {"shared_assets"}
        if "native_macos" in name:
            return {"macos"}
        if "native_windows" in name:
            return {"windows"}
        if name in {"wasm-bindgen-wrapper.sh", "strip_reference_types.py"}:
            return {"ui"}
    if path in DESIGN_INPUTS:
        return {"shared_assets"}
    if path.startswith(".github/release-notes/"):
        return {"release_notes"}
    if path.startswith(("docs/", ".github/ISSUE_TEMPLATE/")) or path in {
        "README.md", "README_zh.md", "AGENTS.md", "LICENSE", "LICENSE.md", ".github/CI.md",
        ".github/PULL_REQUEST_TEMPLATE.md", ".github/workflows/website.yml",
        ".github/workflows/pages.yml", ".github/workflows/docs-cloudflare-deploy.yml",
        ".github/workflows/docs-downloads-r2-sync.yml",
    }:
        return set()
    # New build inputs and unknown directories must not silently lose coverage.
    return ALL


def select_scopes(paths):
    selected = set()
    for path in paths:
        selected.update(scopes_for_path(path))
    return selected


def diff_range(event_name, event):
    if event_name == "pull_request":
        pr = event["pull_request"]
        base, head, separator = pr["base"]["sha"], pr["head"]["sha"], "..."
    elif event_name == "push":
        base, head, separator = event["before"], event["after"], ".."
    else:
        raise ValueError("No bounded diff for this event")
    for sha in (base, head):
        if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", sha) or sha == "0" * 40:
            raise ValueError("Missing or invalid commit in event")
    return f"{base}{separator}{head}"


def changed_paths(event_name, event, cwd=None):
    revision_range = diff_range(event_name, event)
    result = subprocess.run(
        ["git", "diff", "--no-ext-diff", "--no-renames", "--name-only", "-z", revision_range, "--"],
        cwd=cwd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    # Disabling rename detection includes both old and new paths. NUL delimiters
    # preserve filenames with spaces/newlines; no filename becomes shell code.
    return [os.fsdecode(path) for path in result.stdout.split(b"\0") if path]


def detect(event_name, event, cwd=None):
    try:
        paths = changed_paths(event_name, event, cwd)
    except (KeyError, TypeError, ValueError, OSError, subprocess.CalledProcessError):
        return ALL, "Diff unavailable; conservatively selected every job."
    return select_scopes(paths), f"Classified {len(paths)} changed paths from the complete {event_name} diff."


def main():
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        selected, reason = detect(os.environ["GITHUB_EVENT_NAME"], event)
    except (KeyError, ValueError, OSError):
        selected, reason = ALL, "Event unavailable; conservatively selected every job."
    outputs = "".join(f"{scope}={'true' if scope in selected else 'false'}\n" for scope in SCOPES)
    print(reason)
    print(outputs, end="")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(outputs)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write("## CI scope\n\n" + reason + "\n\n| Job group | Run |\n| --- | --- |\n")
            for scope in SCOPES:
                summary.write(f"| {scope} | {'yes' if scope in selected else 'skip'} |\n")


if __name__ == "__main__":
    main()
