"""Offline coverage for the native parity ledger rule."""

import io
import os
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import check_native_parity
from check_native_parity import LEDGER, missing_ledger


class LedgerRuleTests(unittest.TestCase):
    def test_webview_only_change_needs_an_entry(self):
        self.assertEqual(missing_ledger(["ui/src/settings_view.rs", "ui/src/i18n.rs", "Cargo.lock"]),
                         ["ui/src/settings_view.rs", "ui/src/i18n.rs"])

    def test_resync_alone_is_not_native_work(self):
        self.assertEqual(missing_ledger([
            "ui/src/i18n.rs",
            "apps/macos/Sources/WispProjectBrowserUI/Resources/native-english.json",
        ]), ["ui/src/i18n.rs"])

    def test_swift_sources_tests_or_the_ledger_cover_the_change(self):
        for covering in ["apps/macos/Sources/WispProjectBrowserUI/NativePanelView.swift",
                         "apps/macos/Tests/WispProjectBrowserUITests/NativePanelTests.swift",
                         LEDGER]:
            with self.subTest(covering=covering):
                self.assertEqual(missing_ledger(["ui/src/main.rs", covering]), [])

    def test_changes_outside_the_webview_ui_need_nothing(self):
        for path in ["src-tauri/src/lib.rs", "ui-tests/tests/chat.spec.ts", "ui/Cargo.toml",
                     "apps/windows/WispScience/App.xaml.cs", "docs/native-settings.md"]:
            with self.subTest(path=path):
                self.assertEqual(missing_ledger([path]), [])

    def test_main_reports_paths_and_skips_pushes(self):
        with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push"}), redirect_stdout(io.StringIO()):
            self.assertEqual(check_native_parity.main(), 0)
        event = {"GITHUB_EVENT_NAME": "pull_request", "GITHUB_EVENT_PATH": __file__}
        with patch.dict(os.environ, event), patch.object(check_native_parity.json, "loads", return_value={}), \
                patch.object(check_native_parity, "changed_paths", return_value=["ui/src/main.rs"]):
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(check_native_parity.main(), 1)
            self.assertIn("ui/src/main.rs", out.getvalue())
            self.assertIn(LEDGER, out.getvalue())


if __name__ == "__main__":
    unittest.main()
