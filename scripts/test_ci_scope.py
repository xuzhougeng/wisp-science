"""Offline regression coverage for CI routing and Git event ranges."""

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from ci_scope import ALL, APPLICATIONS, BACKEND, changed_paths, detect, diff_range, select_scopes


class ScopeTests(unittest.TestCase):
    def test_ios_only_skips_every_desktop_and_backend_job(self):
        self.assertEqual(select_scopes([
            "apps/ios/WispScience/ChecklistModel.swift",
            "apps/ios/WispScienceTests/OfflineResearchTests.swift",
            "apps/ios/WispScience.xcodeproj/project.pbxproj",
            "apps/ios/Package.swift",
        ]), {"ios"})

    def test_ios_with_shared_backend_keeps_integration_coverage(self):
        for shared in ["crates/wisp-dto/src/checklist.rs",
                       "src-tauri/src/channels/remote.rs", "contracts/research-checklist/v1/sync.json",
                       "Cargo.lock", ".cargo/config.toml"]:
            with self.subTest(shared=shared):
                self.assertEqual(select_scopes(["apps/ios/Package.swift", shared]), APPLICATIONS)
        self.assertEqual(select_scopes(["apps/ios/Package.swift", "crates/wisp-store/src/lib.rs"]),
                         BACKEND | {"ios"})

    def test_website_only_does_not_build_apps(self):
        self.assertEqual(select_scopes([
            "docs/index.html", "README.md", "ui-tests/website-tests/downloads.spec.ts",
            "ui-tests/playwright.website.config.ts", ".github/workflows/website.yml",
        ]), set())
        self.assertEqual(select_scopes(["ui-tests/package-lock.json"]), {"ui"})

    def test_native_platform_source_is_independent(self):
        self.assertEqual(select_scopes(["apps/windows/Wisp.Science.Preview/Theme.cs"]),
                         {"windows", "shared_assets"})
        self.assertEqual(select_scopes(["apps/macos/Sources/WispScience/App.swift"]), {"macos"})
        self.assertEqual(select_scopes(["apps/macos/native-settings-english.json"]),
                         {"macos", "shared_assets"})
        self.assertEqual(select_scopes(["scripts/build_native_macos.sh"]), {"macos"})
        self.assertEqual(select_scopes(["scripts/test_build_native_windows.ps1"]), {"windows"})

    def test_shared_resources_only_select_their_consumers(self):
        self.assertEqual(select_scopes(["apps/macos/Sources/WispProjectBrowserUI/Resources/palette.json"]),
                         {"macos", "windows", "shared_assets"})
        for path in ["scripts/sync_native_design.py", "scripts/sync_native_settings_contract.py",
                     "docs/assets/wordmark-light.svg"]:
            with self.subTest(path=path):
                self.assertEqual(select_scopes([path]), {"shared_assets"})
        self.assertEqual(select_scopes(["ui/vendor-src/scientific-preview.mjs"]),
                         {"ui", "macos", "windows", "shared_assets"})

    def test_web_ui_dependencies_and_shared_icons(self):
        for path in ["ui/src/app.rs", "ui/Cargo.lock", "vendor/leptos_reactive/src/lib.rs"]:
            with self.subTest(path=path):
                self.assertEqual(select_scopes([path]), {"ui"})
        self.assertEqual(select_scopes(["ui/build-fast.mjs"]), {"ui", "frontend_assets"})
        self.assertEqual(select_scopes(["ui/src/app_support/messages.rs"]), {"ui", "shared_assets"})
        self.assertEqual(select_scopes(["vendor/tao/src/lib.rs"]), {"rust", "desktop", "macos", "windows"})

    def test_runtime_skills_are_not_documentation(self):
        self.assertIn("headless", select_scopes(["skills/example/SKILL.md"]))
        self.assertIn("rust", select_scopes(["python/test_worker.py"]))

    def test_unknown_inputs_and_routing_changes_run_everything(self):
        for path in ["new-component/build.toml", "scripts/new_build.py", "scripts/ci_scope.py",
                     "scripts/test_ci_scope.py", ".github/workflows/ci-changes.yml"]:
            with self.subTest(path=path):
                self.assertEqual(select_scopes([path]), ALL)
        self.assertEqual(select_scopes([".github/workflows/native-preview.yml"]),
                         {"macos", "windows", "ios", "shared_assets"})
        self.assertEqual(select_scopes(["scripts/github_release_notes.sh"]), {"release_notes"})

    def test_no_changes_skip_jobs_but_invalid_or_missing_diffs_run_all(self):
        self.assertEqual(select_scopes([]), set())
        sha = "a" * 40
        for name, event in [("workflow_dispatch", {}), ("pull_request", {}),
                            ("push", {"before": "0" * 40, "after": sha}),
                            ("push", {"before": "--option", "after": sha})]:
            with self.subTest(name=name, event=event):
                self.assertEqual(detect(name, event)[0], ALL)
        with patch("ci_scope.subprocess.run", side_effect=subprocess.CalledProcessError(128, "git")):
            self.assertEqual(detect("push", {"before": sha, "after": "b" * 40})[0], ALL)


class GitDiffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "--initial-branch=main")
        self.base = self.commit({"README.md": "initial"})

    def git(self, *args):
        return subprocess.run(
            ["git", "-c", "user.name=CI scope test", "-c", "user.email=ci@example.test",
             "-c", "commit.gpgsign=false", *args],
            cwd=self.root, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        ).stdout.decode().strip()

    def commit(self, files):
        for name, contents in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents)
        self.git("add", "--all")
        self.git("commit", "-m", "fixture change")
        return self.git("rev-parse", "HEAD")

    def test_pr_uses_all_branch_commits_and_excludes_unmerged_base_changes(self):
        main = self.commit({"crates/core/src/lib.rs": "main only"})
        self.git("switch", "-c", "feature", self.base)
        self.commit({"ui/src/app.rs": "first PR commit"})
        head = self.commit({"apps/ios/App.swift": "second PR commit"})
        event = {"pull_request": {"base": {"sha": main}, "head": {"sha": head}}}
        self.assertEqual(diff_range("pull_request", event), f"{main}...{head}")
        self.assertEqual(set(changed_paths("pull_request", event, self.root)),
                         {"ui/src/app.rs", "apps/ios/App.swift"})

    def test_push_includes_every_commit_and_deleted_paths(self):
        self.commit({"apps/ios/App.swift": "first commit"})
        (self.root / "README.md").unlink()
        head = self.commit({"src-tauri/src/lib.rs": "second commit"})
        event = {"before": self.base, "after": head}
        self.assertEqual(diff_range("push", event), f"{self.base}..{head}")
        self.assertEqual(set(changed_paths("push", event, self.root)),
                         {"README.md", "apps/ios/App.swift", "src-tauri/src/lib.rs"})

    def test_rename_keeps_original_component_and_unusual_filenames(self):
        before = self.commit({"src-tauri/old file": "same content"})
        new_name = "apps/ios/file with\na newline.swift"
        destination = self.root / new_name
        destination.parent.mkdir(parents=True)
        (self.root / "src-tauri/old file").rename(destination)
        head = self.commit({})
        event = {"before": before, "after": head}
        self.assertEqual(set(changed_paths("push", event, self.root)), {"src-tauri/old file", new_name})
        self.assertEqual(detect("push", event, self.root)[0], APPLICATIONS)

    def test_large_diff_does_not_truncate_late_backend_changes(self):
        files = {f"apps/ios/fixture-{i:04}.swift": "fixture" for i in range(3001)}
        files["src-tauri/src/lib.rs"] = "must not be missed"
        head = self.commit(files)
        event = {"before": self.base, "after": head}
        self.assertEqual(len(changed_paths("push", event, self.root)), 3002)
        self.assertEqual(detect("push", event, self.root)[0], APPLICATIONS)


if __name__ == "__main__":
    unittest.main()
