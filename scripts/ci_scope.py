"""Select CI jobs from the complete PR or push diff, without an API file limit."""

import json
import os
from pathlib import Path
import re
import subprocess


SCOPES = (
    "rust", "headless", "desktop", "ui", "gpui", "ios", "release_notes",
    "frontend_assets", "relay_worker",
)
ALL = frozenset(SCOPES)
APPLICATIONS = ALL - {"release_notes", "frontend_assets", "relay_worker"}
BACKEND = frozenset({"rust", "headless", "desktop", "gpui"})
TEST = ALL - {"gpui", "ios"}


def scopes_for_path(path):
    if path in {"scripts/ci_scope.py", "scripts/test_ci_scope.py",
                ".github/workflows/ci-changes.yml"}:
        return ALL
    if path == ".github/workflows/test.yml":
        return TEST
    if path == ".github/workflows/ios.yml" or path.startswith("apps/ios/"):
        return {"ios"}
    if path == ".github/workflows/gpui.yml" or path.startswith("apps/gpui/"):
        return {"gpui"}
    if path.startswith("apps/android/") or path == ".github/workflows/android.yml":
        return set()  # android.yml has its own path filters.
    if path.startswith("crates/wisp-sync/cloudflare/"):
        return {"relay_worker"}
    if path.startswith(("crates/wisp-dto/", "crates/wisp-sync/", "src-tauri/", "contracts/", ".cargo/")):
        return APPLICATIONS
    if path.startswith("crates/"):
        return BACKEND
    if path.startswith("vendor/leptos_reactive/"):
        return {"ui"}
    if path.startswith("vendor/tao/"):
        return {"rust", "desktop"}
    if path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}:
        return APPLICATIONS
    if path.startswith(("python/", "skills/", "seed/", "r/", "browser-extension/")):
        return BACKEND
    if path.startswith("ui/"):
        return {"ui", "frontend_assets"} if path in {"ui/build-fast.mjs", "ui/build-fast.test.mjs"} else {"ui"}
    if path.startswith("ui-tests/website-tests/") or path == "ui-tests/playwright.website.config.ts":
        return set()
    if path.startswith("ui-tests/"):
        return {"ui"}
    if path.startswith("scripts/"):
        name = Path(path).name
        if name == "github_release_notes.sh":
            return {"release_notes"}
        if name in {"wasm-bindgen-wrapper.sh", "strip_reference_types.py"}:
            return {"ui"}
    if path.startswith(".github/release-notes/"):
        return {"release_notes"}
    if path.startswith(("docs/", ".github/ISSUE_TEMPLATE/")) or path in {
        "README.md", "README_zh.md", "AGENTS.md", "LICENSE", "LICENSE.md", ".github/CI.md",
        ".github/PULL_REQUEST_TEMPLATE.md", ".github/workflows/website.yml",
        ".github/workflows/pages.yml", ".github/workflows/docs-cloudflare-deploy.yml",
        ".github/workflows/docs-downloads-r2-sync.yml",
    }:
        return set()
    return ALL  # New build inputs must not silently lose coverage.


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
