import plistlib
import tempfile
import unittest
from pathlib import Path

from stamp_native_macos import stamp


class NativeMetadataTests(unittest.TestCase):
    def test_shell_and_helper_share_version_without_losing_bundle_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            shell, helper = (Path(directory) / name for name in ("shell.plist", "helper.plist"))
            for path, identifier in ((shell, "test.preview"), (helper, "test.host")):
                path.write_bytes(plistlib.dumps({"CFBundleIdentifier": identifier,
                                                "CFBundleVersion": "1", "LSUIElement": True}))
            metadata = dict(version="1.14.0", revision="a" * 40, dirty=True)
            stamp(shell, **metadata)
            stamp(helper, identifier="isolated.host", **metadata)
            first = plistlib.loads(shell.read_bytes())
            second = plistlib.loads(helper.read_bytes())
            self.assertEqual(first["CFBundleIdentifier"], "test.preview")
            self.assertEqual(second["CFBundleIdentifier"], "isolated.host")
            for info in (first, second):
                self.assertEqual(info["CFBundleShortVersionString"], "1.14.0")
                self.assertEqual(info["CFBundleVersion"], "1.14.0")
                self.assertEqual(info["WispSourceRevision"], "a" * 40)
                self.assertTrue(info["WispSourceDirty"])
                self.assertEqual(info["WispBuildConfiguration"], "debug")
                self.assertTrue(info["LSUIElement"])
            stamp(shell, **dict(metadata, version="1.15.0", dirty=False, configuration="release"))
            info = plistlib.loads(shell.read_bytes())
            self.assertFalse(info["WispSourceDirty"])
            self.assertEqual(info["WispBuildConfiguration"], "release")


if __name__ == "__main__":
    unittest.main()
