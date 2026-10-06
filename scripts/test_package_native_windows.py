import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package_native_windows as pkg


class WindowsPreviewPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='winui test ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.app = self.root / 'app'
        self.output = self.root / 'release'
        for relative in pkg.REQUIRED:
            path = self.app / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'MZ' + bytes(58) + struct.pack('<I', 64) + b'PE\0\0\x64\x86')
        for name in pkg.RESOURCES:
            folder = self.app / 'settings-host' / name
            folder.mkdir(parents=True, exist_ok=True)
            (folder / 'resource.txt').write_text(name)
        (self.app / 'old.pdb').write_bytes(b'symbols')
        cache = self.app / 'settings-host/python/__pycache__'
        cache.mkdir()
        (cache / 'old.pyc').write_bytes(b'cache')
        self.script = ''

    def fake_compiler(self, args, **kwargs):
        script = Path(args[-1])
        self.script = script.read_text(encoding='utf-8-sig')
        payload = script.parent / 'payload'
        self.assertFalse((payload / 'old.pdb').exists())
        self.assertFalse((payload / 'settings-host/python/__pycache__').exists())
        self.assertTrue((payload / 'settings-host/python/resource.txt').exists())
        name = next(line.split('"')[1] for line in self.script.splitlines() if line.startswith('OutFile '))
        Path(name).write_bytes(b'installer')

    def package(self, **kwargs):
        with patch.object(pkg.subprocess, 'run', self.fake_compiler):
            return pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3', **kwargs)

    def test_package_preserves_dependencies_and_controls_size(self):
        installer = self.package()
        self.assertEqual(installer.name, 'Wisp-Science-WinUI-Preview_1.2.3_x64-self-contained-setup.exe')
        self.assertEqual(installer.with_suffix('.exe.sha256').read_text(),
                         f'{hashlib.sha256(b"installer").hexdigest()}  {installer.name}\n')
        report = json.loads(installer.with_suffix('.size.json').read_text())
        self.assertEqual(report['file_count'], len(pkg.REQUIRED) + len(pkg.RESOURCES))
        self.assertEqual(report['installer_bytes'], len(b'installer'))
        self.assertIn('SetCompressor /SOLID lzma', self.script)
        self.assertIn('RequestExecutionLevel user', self.script)
        self.assertIn('WispScienceWinUIPreview', self.script)
        self.assertIn('ExecWait', self.script, 'Upgrade must remove the previous file manifest.')
        self.assertNotIn('RMDir /r', self.script)
        self.assertNotIn('$APPDATA', self.script)
        self.assertNotIn('latest.json', self.script)
        self.assertFalse(list(self.output.glob('wisp-winui-*')))
        self.assertTrue((self.app / 'old.pdb').exists(), 'Packaging must not edit build output.')

    def test_missing_helper_stops_before_compilation(self):
        (self.app / 'settings-host/wisp-tauri.exe').unlink()
        with self.assertRaises(FileNotFoundError):
            self.package()
        self.assertFalse(self.output.exists())

    def test_missing_runtime_or_resource_is_rejected(self):
        (self.app / 'settings-host/skills/resource.txt').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing host resource'):
            self.package()
        (self.app / 'coreclr.dll').unlink()
        with self.assertRaises(FileNotFoundError):
            self.package()

    def test_wrong_architecture_is_rejected(self):
        path = self.app / 'wisp-service.exe'
        path.write_bytes(path.read_bytes()[:-2] + b'\x4c\x01')
        with self.assertRaisesRegex(ValueError, 'Expected x64'):
            self.package()

    def test_tag_must_match_source_version(self):
        with self.assertRaisesRegex(ValueError, 'does not match source'):
            pkg.package(self.app, 'v1.2.4', self.output, version='1.2.3')
        with self.assertRaisesRegex(ValueError, 'Expected a release tag'):
            pkg.package(self.app, 'v1.2.3"', self.output, version='1.2.3')

    def test_budgets_fail_before_exposing_an_installer(self):
        with self.assertRaisesRegex(ValueError, 'Payload'):
            self.package(max_payload_mib=0)
        self.assertFalse(self.output.exists())
        with self.assertRaisesRegex(ValueError, 'Installer size'):
            self.package(max_installer_mib=0)
        self.assertFalse(list(self.output.glob('*.exe*')))

    def test_compiler_failure_does_not_publish_candidate(self):
        with patch.object(pkg.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['makensis'])):
            with self.assertRaises(subprocess.CalledProcessError):
                pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3')
        self.assertFalse(list(self.output.glob('*.exe*')))

    def test_finalization_hashes_signed_bytes_and_rechecks_budget(self):
        installer = self.package()
        installer.write_bytes(b'signed installer')
        report = pkg.finalize(installer)
        self.assertEqual(report['sha256'], hashlib.sha256(b'signed installer').hexdigest())
        self.assertIn('payload_bytes', report)
        with self.assertRaisesRegex(ValueError, 'Installer size'):
            pkg.finalize(installer, 0)

    def test_output_inside_payload_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'outside the application'):
            pkg.package(self.app, 'v1.2.3', self.app / 'output', version='1.2.3')

    def test_nsis_paths_are_escaped(self):
        self.assertEqual(pkg.nsis_quote('a$b"c'), 'a$$b$\\"c')

    def make_framework_dependent(self):
        (self.app / 'coreclr.dll').unlink()
        (self.app / 'Microsoft.UI.Xaml.dll').unlink()
        (self.app / 'Microsoft.WindowsAppRuntime.Bootstrap.dll').write_bytes((self.app / 'wisp-service.exe').read_bytes())
        (self.app / 'Wisp.Science.Preview.runtimeconfig.json').write_text(json.dumps({
            'runtimeOptions': {'framework': {'name': 'Microsoft.NETCore.App', 'version': '8.0.24'}}}))

    def test_framework_dependent_bootstraps_before_removing_old_app(self):
        self.make_framework_dependent()
        installer = self.package(variant='framework-dependent')
        self.assertIn('_x64-framework-dependent-setup.exe', installer.name)
        self.assertLess(self.script.index('nsExec::ExecToLog'), self.script.index('ExecWait'))
        self.assertIn('Sysnative', self.script)
        self.assertIn('SetErrorLevel 1', self.script)
        self.assertIn('SetRebootFlag true', self.script)
        report = pkg.finalize(installer)
        self.assertEqual(report['installer_limit_mib'], 40)
        self.assertEqual(report['payload_limit_mib'], 150)

    def test_framework_dependent_cannot_accidentally_include_runtimes(self):
        with self.assertRaisesRegex(ValueError, 'contains bundled runtime'):
            self.package(variant='framework-dependent')

    def test_self_contained_has_no_runtime_download_step(self):
        self.package()
        self.assertNotIn('nsExec::ExecToLog', self.script)

    def test_framework_dependent_validates_runtime_config(self):
        self.make_framework_dependent()
        (self.app / 'Wisp.Science.Preview.runtimeconfig.json').write_text('{"runtimeOptions": {}}')
        with self.assertRaisesRegex(ValueError, '.NET 8 runtime configuration'):
            self.package(variant='framework-dependent')

    @unittest.skipUnless(os.environ.get('WISP_TEST_MAKENSIS'), 'Set WISP_TEST_MAKENSIS for real NSIS compilation.')
    def test_real_nsis_compiles_fixture(self):
        installer = pkg.package(self.app, 'v1.2.3', self.output,
                                os.environ['WISP_TEST_MAKENSIS'], version='1.2.3')
        self.assertTrue(installer.read_bytes().startswith(b'MZ'))
        self.make_framework_dependent()
        thin = pkg.package(self.app, 'v1.2.3', self.output,
                           os.environ['WISP_TEST_MAKENSIS'], version='1.2.3', variant='framework-dependent')
        self.assertTrue(thin.read_bytes().startswith(b'MZ'))


if __name__ == '__main__':
    unittest.main()
