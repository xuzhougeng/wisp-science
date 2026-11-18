"""Packaging layout checks for scripts/package_gpui.py.

Everything runs in a temporary folder on a made-up checkout: no build, no
signing, no network. The macOS archive step (ditto) is only exercised there.
"""

from pathlib import Path
import plistlib
import sys
import tarfile
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent))

import package_gpui  # noqa: E402
from package_gpui import RESOURCES, archive, archive_name, stage, workspace_version  # noqa: E402


def checkout(root, version="9.8.7"):
    """A checkout with only what packaging reads."""
    root = Path(root)
    root.mkdir(parents=True)
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = []\n\n[workspace.dependencies]\nversion = "0.0.1"\n\n'
        f'[workspace.package]\nversion = "{version}"\nedition = "2021"\n', encoding="utf-8")
    for name in RESOURCES:
        (root / name).mkdir(parents=True)
        (root / name / "kept.txt").write_text(name, encoding="utf-8")
    (root / "skills" / "example").mkdir()
    (root / "skills" / "example" / "SKILL.md").write_text("# Example\n", encoding="utf-8")
    (root / "python" / "__pycache__").mkdir()
    (root / "python" / "__pycache__" / "worker.pyc").write_bytes(b"\0")
    (root / "src-tauri" / "icons").mkdir(parents=True)
    (root / "src-tauri" / "icons" / "icon.icns").write_bytes(b"icns")
    (root / "LICENSE").write_text("license\n", encoding="utf-8")
    binary = root / "built" / "wisp-gpui"
    binary.parent.mkdir()
    binary.write_bytes(b"binary")
    return root, binary


class PackageGpuiTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.repo, self.binary = checkout(Path(self.scratch.name) / "repo")
        self.out = Path(self.scratch.name) / "dist"

    def test_version_comes_from_the_workspace_package_table(self):
        self.assertEqual(workspace_version(self.repo), "9.8.7")
        (self.repo / "Cargo.toml").write_text('[package]\nversion = "1.0.0"\n', encoding="utf-8")
        with self.assertRaises(SystemExit):
            workspace_version(self.repo)

    def test_names_cannot_be_taken_for_webview_installers(self):
        names = [archive_name("1.26.0", system, arch) for system, arch in
                 [("macos", "aarch64"), ("macos", "x64"), ("windows", "x64"), ("linux", "x64")]]
        # The WebView installers' pattern with `gpui` added to the product:
        # wisp-science_1.26.0_aarch64.dmg, _x64-setup.exe, _amd64.deb.
        self.assertEqual(names, [
            "wisp-science-gpui_1.26.0_aarch64.app.zip",
            "wisp-science-gpui_1.26.0_x64.app.zip",
            "wisp-science-gpui_1.26.0_x64.zip",
            "wisp-science-gpui_1.26.0_amd64.tar.gz",
        ])
        self.assertEqual(len(set(names)), len(names))
        # The download picker and the updater select `wisp-science_<version>_*`
        # and the updater's own `latest.json`, `.app.tar.gz` and `.sig` files.
        for name in names:
            self.assertFalse(name.startswith("wisp-science_"))
            self.assertFalse(name.endswith((".app.tar.gz", ".sig", ".json", ".dmg", ".msi",
                                            ".exe", ".deb", ".AppImage", ".apk")))

    def test_windows_and_linux_keep_resources_beside_the_executable(self):
        for system, executable in [("windows", "wisp-gpui.exe"), ("linux", "wisp-gpui")]:
            with self.subTest(system=system):
                staged = stage(self.repo, self.binary, system, self.out)
                self.assertEqual(staged, self.out / "wisp-science-gpui")
                self.assertEqual((staged / executable).read_bytes(), b"binary")
                for name in RESOURCES:
                    self.assertTrue((staged / "resources" / name / "kept.txt").is_file())
                self.assertFalse((staged / "resources" / "python" / "__pycache__").exists())
                self.assertIn("9.8.7", (staged / "README.txt").read_text(encoding="utf-8"))
                self.assertTrue((staged / "LICENSE").is_file())

    def test_macos_bundle_layout_and_identity(self):
        staged = stage(self.repo, self.binary, "macos", self.out)
        self.assertEqual(staged.name, "wisp-science-gpui.app")
        contents = staged / "Contents"
        self.assertTrue((contents / "MacOS" / "wisp-gpui").is_file())
        self.assertTrue((contents / "Resources" / "skills" / "example" / "SKILL.md").is_file())
        self.assertTrue((contents / "Resources" / "icon.icns").is_file())
        with open(contents / "Info.plist", "rb") as file:
            plist = plistlib.load(file)
        self.assertEqual(plist["CFBundleExecutable"], "wisp-gpui")
        self.assertEqual(plist["CFBundleName"], "wisp-science-gpui")
        self.assertEqual(plist["CFBundleShortVersionString"], "9.8.7")
        # Not the WebView desktop's `science.wisp-science`: both can be installed.
        self.assertEqual(plist["CFBundleIdentifier"], "science.wisp-science.gpui")

    def test_staging_again_replaces_the_previous_tree(self):
        staged = stage(self.repo, self.binary, "linux", self.out)
        (staged / "stale.txt").write_text("old", encoding="utf-8")
        stage(self.repo, self.binary, "linux", self.out)
        self.assertFalse((staged / "stale.txt").exists())

    def test_missing_inputs_stop_before_anything_is_packed(self):
        with self.assertRaises(SystemExit):
            stage(self.repo, self.repo / "built" / "absent", "linux", self.out)
        (self.repo / "seed" / "kept.txt").unlink()
        (self.repo / "seed").rmdir()
        with self.assertRaises(SystemExit):
            stage(self.repo, self.binary, "linux", self.out)
        with self.assertRaises(SystemExit):
            archive(self.repo, "linux", "x64", self.out)

    def test_windows_zip_and_linux_tarball_unpack_to_one_folder(self):
        stage(self.repo, self.binary, "windows", self.out)
        path = archive(self.repo, "windows", "x64", self.out)
        self.assertEqual(path.name, "wisp-science-gpui_9.8.7_x64.zip")
        with zipfile.ZipFile(path) as bundle:
            names = bundle.namelist()
        self.assertIn("wisp-science-gpui/wisp-gpui.exe", names)
        self.assertIn("wisp-science-gpui/resources/skills/example/SKILL.md", names)
        self.assertTrue(all(name.startswith("wisp-science-gpui/") for name in names))

        stage(self.repo, self.binary, "linux", self.out)
        path = archive(self.repo, "linux", "x64", self.out)
        self.assertEqual(path.name, "wisp-science-gpui_9.8.7_amd64.tar.gz")
        with tarfile.open(path) as bundle:
            names = bundle.getnames()
            binary = bundle.getmember("wisp-science-gpui/wisp-gpui")
        self.assertIn("wisp-science-gpui/resources/r/kept.txt", names)
        if sys.platform != "win32":  # Windows has no executable bit to keep.
            self.assertTrue(binary.mode & 0o111)

    def test_command_line_prints_the_path_it_made(self):
        import contextlib
        import io
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            package_gpui.main(["stage", "--os", "linux", "--arch", "x64", "--repo", str(self.repo),
                               "--binary", str(self.binary), "--out", str(self.out)])
            package_gpui.main(["archive", "--os", "linux", "--arch", "x64", "--repo", str(self.repo),
                               "--out", str(self.out)])
        staged, packed = printed.getvalue().split("\n")[:2]
        self.assertTrue(staged.endswith("dist/wisp-science-gpui"))
        self.assertTrue(packed.endswith("dist/wisp-science-gpui_9.8.7_amd64.tar.gz"))
        self.assertNotIn("\\", packed)


if __name__ == "__main__":
    unittest.main()
