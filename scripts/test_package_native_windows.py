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


class WindowsPackagingTests(unittest.TestCase):
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
        for relative in pkg.APP_FILES:
            path = self.app / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('{}')
        (self.app / 'old.pdb').write_bytes(b'symbols')
        cache = self.app / 'settings-host/python/__pycache__'
        cache.mkdir()
        (cache / 'old.pyc').write_bytes(b'cache')
        self.bootstrapper = self.root / 'MicrosoftEdgeWebview2Setup.exe'
        self.bootstrapper.write_bytes(b'MZ bootstrapper fixture')
        self.validated_bootstrapper = None
        self.script = ''

    def fake_compiler(self, args, **kwargs):
        if args[0] == 'powershell':
            self.assertIn('-ValidateOnly', args)
            self.assertIn(str(pkg.SCRIPT_ROOT / 'prepare_native_windows_webview2.ps1'), args)
            self.validated_bootstrapper = Path(args[-1])
            self.assertEqual(self.validated_bootstrapper.read_bytes(), self.bootstrapper.read_bytes())
            self.assertTrue(kwargs['check'])
            return
        script = Path(args[-1])
        self.script = script.read_text(encoding='utf-8-sig')
        self.assertEqual(self.validated_bootstrapper.parent, script.parent)
        self.assertIn(f'File /oname=MicrosoftEdgeWebview2Setup.exe "{pkg.nsis_quote(self.validated_bootstrapper)}"', self.script)
        payload = script.parent / 'payload'
        self.assertFalse((payload / 'old.pdb').exists())
        self.assertFalse((payload / 'settings-host/python/__pycache__').exists())
        self.assertTrue((payload / 'settings-host/python/resource.txt').exists())
        name = next(line.split('"')[1] for line in self.script.splitlines() if line.startswith('OutFile '))
        Path(name).write_bytes(b'installer')

    def package(self, **kwargs):
        with patch.object(pkg.subprocess, 'run', self.fake_compiler):
            return pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3',
                               webview2_bootstrapper=self.bootstrapper, **kwargs)

    def test_package_preserves_dependencies_and_controls_size(self):
        installer = self.package()
        self.assertEqual(installer.name, 'Wisp-Science-WinUI_1.2.3_x64-setup.exe')
        self.assertEqual(installer.with_suffix('.exe.sha256').read_text(),
                         f'{hashlib.sha256(b"installer").hexdigest()}  {installer.name}\n')
        report = json.loads(installer.with_suffix('.size.json').read_text())
        self.assertEqual(report['file_count'], len(pkg.REQUIRED) + len(pkg.RESOURCES) + len(pkg.APP_FILES))
        self.assertEqual(report['installer_bytes'], len(b'installer'))
        self.assertIn('SetCompressor /SOLID lzma', self.script)
        self.assertIn('RequestExecutionLevel user', self.script)
        self.assertIn('WispScienceWinUIPreview', self.script)
        self.assertIn('Name "Wisp Science WinUI"', self.script)
        self.assertIn('InstallDir "$LOCALAPPDATA\\Programs\\Wisp Science WinUI"', self.script)
        self.assertIn('CreateShortcut "$SMPROGRAMS\\Wisp Science WinUI.lnk"', self.script)
        self.assertNotIn('"DisplayName" "Wisp Science WinUI Preview"', self.script)
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

    def test_standalone_package_cannot_borrow_missing_files_from_another_install(self):
        for relative in pkg.REQUIRED + pkg.APP_FILES:
            path = self.app / relative
            original = path.read_bytes()
            with self.subTest(relative=relative):
                path.unlink()
                with self.assertRaises((FileNotFoundError, ValueError)):
                    self.package()
                self.assertFalse(self.output.exists())
                path.write_bytes(original)

    def test_preview_upgrade_keeps_its_existing_install_directory(self):
        self.package()
        init = self.script.split('Function .onInit', 1)[1].split('FunctionEnd', 1)[0]
        self.assertIn('IfFileExists "$INSTDIR\\Uninstall.exe" winui_dir_ready 0', init)
        self.assertIn('IfFileExists "$LOCALAPPDATA\\Programs\\Wisp Science WinUI Preview\\Uninstall.exe" 0 winui_dir_ready', init)
        self.assertIn('StrCpy $INSTDIR "$LOCALAPPDATA\\Programs\\Wisp Science WinUI Preview"', init)
        self.assertNotIn('wisp-science', init, 'Upgrade discovery must not borrow the WebView installation.')

    def test_current_packager_uses_the_requested_tag_source_version(self):
        source = self.root / 'tagged-source'
        source.mkdir()
        (source / 'Cargo.toml').write_text('[workspace.package]\nversion = "1.24.0"\n')
        with patch.object(pkg.subprocess, 'run', self.fake_compiler):
            installer = pkg.package(self.app, 'v1.24.0', self.output, source_root=source,
                                    webview2_bootstrapper=self.bootstrapper)
            self.assertEqual(installer.name, 'Wisp-Science-WinUI_1.24.0_x64-setup.exe')
            with self.assertRaisesRegex(ValueError, 'does not match source'):
                pkg.package(self.app, 'v1.25.0', self.output, source_root=source)

    def test_ci_only_publishes_the_standalone_variant(self):
        workflows = Path(__file__).resolve().parents[1] / '.github/workflows'
        for name in ('native-preview.yml', 'release-winui.yml'):
            with self.subTest(workflow=name):
                workflow = (workflows / name).read_text(encoding='utf-8')
                self.assertIn('-RuntimeMode SelfContained', workflow)
                self.assertNotIn('-RuntimeMode Both', workflow)
                self.assertNotIn('framework-dependent', workflow)
                self.assertNotIn('Wisp-Science-WinUI-Preview', workflow)
        release = (workflows / 'release-winui.yml').read_text(encoding='utf-8')
        self.assertIn('name: Windows WinUI Release', release)
        self.assertIn('_release_ci/scripts/package_native_windows.py target/native-windows-release --source-root .', release)
        self.assertIn('$installers.Count -ne 1', release)
        self.assertIn('$files.Count -ne 2', release)
        self.assertIn("'wisp-service.exe'", release)
        self.assertIn("(Join-Path 'settings-host' 'wisp-tauri.exe')", release)
        self.assertFalse((workflows / 'release-winui-preview.yml').exists())
        sync = (workflows / 'docs-downloads-r2-sync.yml').read_text(encoding='utf-8')
        self.assertIn('Windows WinUI Release]', sync)
        for name, prefix in (('release-winui.yml', '_release_ci/'), ('native-preview.yml', '')):
            workflow = (workflows / name).read_text(encoding='utf-8')
            self.assertIn(prefix + 'scripts/test_prepare_native_windows_webview2.ps1', workflow)
            self.assertIn(prefix + 'scripts/prepare_native_windows_webview2.ps1 -Path', workflow)
            self.assertIn('--webview2-bootstrapper "$env:RUNNER_TEMP/MicrosoftEdgeWebview2Setup.exe"', workflow)
        native = (workflows / 'native-preview.yml').read_text(encoding='utf-8')
        prepare = native.index('name: Prepare NSIS and WebView2 bootstrapper')
        validate = native.index('name: Validate installer before compiling WinUI')
        build = native.index('pwsh -File scripts/build_native_windows.ps1')
        self.assertLess(prepare, validate)
        self.assertLess(validate, build)
        self.assertIn('WISP_TEST_MAKENSIS', native[validate:build])
        self.assertIn('WISP_TEST_WEBVIEW2_BOOTSTRAPPER', native[validate:build])

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
        def fail_compiler(args, **kwargs):
            if args[0] == 'powershell':
                return self.fake_compiler(args, **kwargs)
            raise subprocess.CalledProcessError(1, args)
        with patch.object(pkg.subprocess, 'run', side_effect=fail_compiler):
            with self.assertRaises(subprocess.CalledProcessError):
                pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3',
                            webview2_bootstrapper=self.bootstrapper)
        self.assertFalse(list(self.output.glob('*.exe*')))

    def test_missing_or_untrusted_bootstrapper_cannot_be_packaged(self):
        with patch.object(pkg.subprocess, 'run') as run:
            for path in (None, self.root / 'absent.exe'):
                with self.assertRaisesRegex(ValueError, '--webview2-bootstrapper'):
                    pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3',
                                webview2_bootstrapper=path)
            run.assert_not_called()
        self.assertFalse(self.output.exists())
        with patch.object(pkg.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['powershell'])) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3',
                            webview2_bootstrapper=self.bootstrapper)
            self.assertEqual(run.call_count, 1, 'Signature failure must prevent NSIS compilation.')
            self.assertIn('-ValidateOnly', run.call_args.args[0])
        self.assertFalse(list(self.output.iterdir()), 'Rejected payload must not leave installer candidates.')

    def test_signature_check_does_not_inherit_powershell_7_modules(self):
        for key in ('PSModulePath', 'PSMODULEPATH', 'psmodulepath'):
            with self.subTest(key=key), patch.dict(os.environ, {
                key: 'C:/Program Files/PowerShell/7/Modules',
                'WISP_PACKAGING_ENV_TEST': 'preserved',
            }):
                with patch.object(pkg.subprocess, 'run', side_effect=self.fake_compiler) as run:
                    pkg.package(self.app, 'v1.2.3', self.output, version='1.2.3',
                                webview2_bootstrapper=self.bootstrapper)
                signature, compiler = run.call_args_list
                child_env = signature.kwargs['env']
                self.assertFalse(any(name.upper() == 'PSMODULEPATH' for name in child_env))
                self.assertEqual(child_env['WISP_PACKAGING_ENV_TEST'], 'preserved')
                self.assertEqual(os.environ[key], 'C:/Program Files/PowerShell/7/Modules',
                                 'Only the child environment may change.')
                self.assertNotIn('env', compiler.kwargs, 'NSIS keeps its normal environment.')

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
        self.assertLess(self.script.index('nsExec::ExecToLog'), self.script.index('ExecWait \'$\\"$INSTDIR'))
        self.assertIn('Sysnative', self.script)
        self.assertIn('SetErrorLevel 1', self.script)
        self.assertIn('SetRebootFlag true', self.script)
        report = pkg.finalize(installer)
        self.assertEqual(report['installer_limit_mib'], 40)
        self.assertEqual(report['payload_limit_mib'], 150)

    def test_framework_dependent_cannot_accidentally_include_runtimes(self):
        with self.assertRaisesRegex(ValueError, 'contains bundled runtime'):
            self.package(variant='framework-dependent')

    def test_self_contained_only_bootstraps_webview2_before_touching_the_existing_app(self):
        self.package()
        detection = self.script.split('Function CheckWebView2', 1)[1].split('FunctionEnd', 1)[0]
        key = 'SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
        self.assertIn(f'ReadRegStr $1 HKLM32 "{key}" "pv"', detection)
        self.assertIn(f'ReadRegStr $1 HKCU64 "{key}" "pv"', detection)
        self.assertEqual(detection.count('${VersionCompare} "$1" "0.0.0.0" $0'), 2)
        self.assertNotIn('SetRegView', detection, 'Detection must preserve the uninstall registration view.')
        self.assertNotIn('/oname=ensure-runtimes.ps1', self.script, '.NET and Windows App SDK are already bundled.')
        self.assertNotIn('powershell.exe', self.script, 'Use the verified Microsoft bootstrapper directly at install time.')
        gate = self.script.split('Section "Install"', 1)[1].split('; Remove the previous payload', 1)[0]
        self.assertIn('Call CheckWebView2\n  ${If} $0 != 1\n    InitPluginsDir', gate,
                      'Installed runtime must skip extraction and execution entirely.')
        self.assertIn('ExecWait \'$\\"$PLUGINSDIR\\MicrosoftEdgeWebview2Setup.exe$\\" /silent /install\' $0', gate)
        self.assertEqual(gate.count('Call CheckWebView2'), 2, 'Installation must recheck runtime availability.')
        self.assertLess(gate.index('ExecWait'), gate.rindex('Call CheckWebView2'))
        for instruction in ('IfErrors webview2_failed', '${If} $0 == 3010', '${ElseIf} $0 != 0',
                            'SetRebootFlag true', 'SetErrorLevel 1', 'Abort', '/SD IDOK'):
            self.assertIn(instruction, gate)
        self.assertIn('automatically downloads and installs it if missing', self.script)
        self.assertNotIn('Webview2', self.script.split('Section "Uninstall"', 1)[1],
                         'Uninstall must leave the shared Microsoft runtime installed.')

    def test_framework_dependent_validates_runtime_config(self):
        self.make_framework_dependent()
        (self.app / 'Wisp.Science.Preview.runtimeconfig.json').write_text('{"runtimeOptions": {}}')
        with self.assertRaisesRegex(ValueError, '.NET 8 runtime configuration'):
            self.package(variant='framework-dependent')

    @unittest.skipUnless(os.environ.get('WISP_TEST_MAKENSIS') and os.environ.get('WISP_TEST_WEBVIEW2_BOOTSTRAPPER'),
                         'Set WISP_TEST_MAKENSIS and WISP_TEST_WEBVIEW2_BOOTSTRAPPER for real NSIS compilation.')
    def test_real_nsis_compiles_fixture(self):
        installer = pkg.package(self.app, 'v1.2.3', self.output,
                                os.environ['WISP_TEST_MAKENSIS'], version='1.2.3',
                                webview2_bootstrapper=Path(os.environ['WISP_TEST_WEBVIEW2_BOOTSTRAPPER']))
        self.assertTrue(installer.read_bytes().startswith(b'MZ'))
        self.make_framework_dependent()
        thin = pkg.package(self.app, 'v1.2.3', self.output,
                           os.environ['WISP_TEST_MAKENSIS'], version='1.2.3', variant='framework-dependent',
                           webview2_bootstrapper=Path(os.environ['WISP_TEST_WEBVIEW2_BOOTSTRAPPER']))
        self.assertTrue(thin.read_bytes().startswith(b'MZ'))


if __name__ == '__main__':
    unittest.main()
