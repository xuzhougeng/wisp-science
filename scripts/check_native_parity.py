"""Require a native parity ledger entry when a PR changes the WebView UI only.

`sync_native_design.py` re-exports strings into the Swift resource bundle, so a
WebView feature can look native-aware in `git log` while the SwiftUI client never
gets it (see docs/superpowers/plans/2026-10-19-macos-swiftui-ui-analysis.md).
A PR that touches `ui/src/` must therefore either change the SwiftUI sources or
tests, or say in docs/native-parity-ledger.md why the native client follows later
or not at all.
"""

import json
import os
from pathlib import Path
import sys

from ci_scope import changed_paths

LEDGER = "docs/native-parity-ledger.md"
NATIVE_RESOURCES = "apps/macos/Sources/WispProjectBrowserUI/Resources/"


def is_webview_ui(path):
    return path.startswith("ui/src/")


def is_native_ui(path):
    # Re-exported resources alone do not change what the SwiftUI client does.
    return path.startswith(("apps/macos/Sources/", "apps/macos/Tests/")) and not path.startswith(NATIVE_RESOURCES)


def missing_ledger(paths):
    """WebView UI paths that still need a ledger entry; empty when the PR is covered."""
    paths = list(paths)
    if LEDGER in paths or any(is_native_ui(path) for path in paths):
        return []
    return [path for path in paths if is_webview_ui(path)]


def main():
    event_name = os.environ.get("GITHUB_EVENT_NAME")
    if event_name != "pull_request":
        print("Not a pull request; the native parity ledger check does not apply.")
        return 0
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    missing = missing_ledger(changed_paths(event_name, event))
    if not missing:
        print("Native parity ledger: covered (SwiftUI changed, ledger updated, or no WebView UI change).")
        return 0
    print(f"::error file={LEDGER}::This PR changes the WebView UI without a SwiftUI source change. "
          f"Add a row to {LEDGER} saying whether the SwiftUI client needs the same change.")
    for path in missing:
        print(f"  {path}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
