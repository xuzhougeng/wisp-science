import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package_native_macos as pkg
from stamp_native_macos import stamp


class PreviewPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.app = self.root / 'Wisp Science Preview.app'
        self.helper = self.app / 'Contents/Helpers/Wisp Desktop Host.app'
        for app, identifier in [(self.app, 'science.wisp-science.native-preview'),
                                (self.helper, 'science.wisp-science')]:
            (app / 'Contents/MacOS').mkdir(parents=True)
            p = app / 'Contents/Info.plist'
            p.write_bytes(plistlib.dumps({'CFBundleIdentifier': identifier}))
            stamp(p, version='1.14.0', revision='a' * 40, dirty=False, configuration='release')
        for p in ['Contents/MacOS/WispSciencePreview', 'Contents/MacOS/wisp-service',
                  'Contents/Helpers/Wisp Desktop Host.app/Contents/MacOS/wisp-tauri']:
            (self.app / p).write_bytes(b'synthetic executable')
        self.calls = []
        self.notary_status = 'Accepted'
        self.dmg_notary_status = 'Accepted'
        self.arch = 'arm64'

    def fake_run(self, args):
        a = [str(x) for x in args]
        self.calls.append(a)
        if a[0] == 'lipo':
            return self.arch
        if a[0] == 'ditto' and '-c' not in a:
            shutil.copytree(a[1], a[2])
        elif a[0] == 'ditto':
            Path(a[-1]).write_bytes(b'zip')
        elif a[0] == 'hdiutil':
            self.assertTrue((Path(a[a.index('-srcfolder') + 1]) / 'Applications').is_symlink())
            Path(a[-1]).write_bytes(b'synthetic notarized installer')
        elif a[:3] == ['xcrun', 'notarytool', 'submit']:
            status = self.dmg_notary_status if a[3].endswith('.dmg') else self.notary_status
            return json.dumps({'id': 'synthetic-ticket', 'status': status})
        return ''

    def edit_metadata(self, **changes):
        path = self.app / 'Contents/Info.plist'
        info = plistlib.loads(path.read_bytes())
        info.update(changes)
        path.write_bytes(plistlib.dumps(info))

    def publish(self):
        with patch.dict(os.environ, {k: 'synthetic' for k in
                        ['APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']}), \
                patch.object(pkg, 'run', self.fake_run):
            return pkg.package(self.app, 'aarch64-apple-darwin', 'v1.14.0', self.root / 'output')

    def test_signs_inside_out_and_staples_before_exposing_installer(self):
        dmg = self.publish()
        self.assertEqual(dmg.name, 'Wisp-Science-SwiftUI-Preview_1.14.0_aarch64.dmg')
        self.assertEqual(dmg.with_suffix('.dmg.sha256').read_text(),
                         f'{hashlib.sha256(dmg.read_bytes()).hexdigest()}  {dmg.name}\n')
        signed = [c[-1] for c in self.calls if c[:2] == ['codesign', '--force']]
        self.assertTrue(signed[0].endswith('/wisp-service'))
        self.assertTrue(signed[1].endswith('/Wisp Desktop Host.app'))
        self.assertTrue(signed[2].endswith('/Wisp Science Preview.app'))
        self.assertTrue(signed[3].endswith('.dmg'))
        self.assertEqual(len([c for c in self.calls if c[:3] == ['xcrun', 'stapler', 'validate']]), 2)
        self.assertEqual(len([c for c in self.calls if c[:3] == ['xcrun', 'notarytool', 'submit']]), 2)
        self.assertEqual((self.app / 'Contents/MacOS/wisp-service').read_bytes(), b'synthetic executable')
        self.assertFalse((self.root / 'output/latest.json').exists())

    def test_rejected_notarization_does_not_create_downloadable_installer(self):
        self.notary_status = 'Invalid'
        with self.assertRaisesRegex(RuntimeError, 'not accepted'):
            self.publish()
        self.assertFalse(list((self.root / 'output').glob('*.dmg')))
        self.assertFalse(any(c[0] == 'hdiutil' for c in self.calls))

    def test_wrong_architecture_is_rejected_before_signing(self):
        self.arch = 'x86_64'
        with self.assertRaisesRegex(ValueError, 'Wrong architecture'):
            self.publish()
        self.assertFalse(any(c[0] == 'codesign' for c in self.calls))

    def test_rejected_dmg_notarization_keeps_candidate_out_of_release_output(self):
        self.dmg_notary_status = 'Invalid'
        with self.assertRaisesRegex(RuntimeError, 'not accepted'):
            self.publish()
        self.assertTrue(any(c[0] == 'hdiutil' for c in self.calls))
        self.assertFalse(list((self.root / 'output').glob('*.dmg*')))
        self.assertEqual(len(list((self.root / 'output').glob('*-notary.json'))), 2)

    def test_command_failure_does_not_log_apple_password(self):
        result = subprocess.CompletedProcess([], 1, '', 'Failed with secret-password')
        with patch.dict(os.environ, {'APPLE_PASSWORD': 'secret-password'}), \
                patch.object(pkg.subprocess, 'run', return_value=result):
            with self.assertRaises(RuntimeError) as raised:
                pkg.run(['xcrun', '--password', 'secret-password'])
        self.assertNotIn('secret-password', str(raised.exception))
        self.assertIn('[redacted]', str(raised.exception))

    def test_intel_bundle_validates_against_intel_target(self):
        self.arch = 'x86_64'
        with patch.object(pkg, 'run', self.fake_run):
            self.assertEqual(pkg.validate(self.app, 'x86_64-apple-darwin', 'v1.14.0')[0], '1.14.0')

    def test_dirty_debug_qa_or_mismatched_version_cannot_publish(self):
        cases = [({'WispSourceDirty': True}, 'clean source'),
                 ({'WispBuildConfiguration': 'debug'}, 'optimized'),
                 ({'CFBundleIdentifier': 'science.wisp-science.native-toolbar-qa.preview'}, 'not QA'),
                 ({'CFBundleShortVersionString': '1.15.0'}, 'tag must match'),
                 ({'WispSourceRevision': 'b' * 40}, 'mismatch')]
        original = (self.app / 'Contents/Info.plist').read_bytes()
        for changes, message in cases:
            with self.subTest(changes=changes):
                (self.app / 'Contents/Info.plist').write_bytes(original)
                self.edit_metadata(**changes)
                with self.assertRaisesRegex(ValueError, message):
                    self.publish()

    def test_adhoc_identity_cannot_be_used_for_release(self):
        with patch.dict(os.environ, {'APPLE_SIGNING_IDENTITY': '-'}), patch.object(pkg, 'run', self.fake_run):
            with self.assertRaisesRegex(ValueError, 'ad-hoc'):
                pkg.package(self.app, 'aarch64-apple-darwin', 'v1.14.0', self.root / 'output')


if __name__ == '__main__':
    unittest.main()
