"""Exercise the real build shell with fake compilers; no SDK or network required."""
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parent.parent
FAKE_TOOL = '''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
name=Path(sys.argv[0]).name;args=sys.argv[1:]
root=Path(os.environ['FAKE_ROOT'])
with (root/'calls.jsonl').open('a') as f:f.write(json.dumps({'tool':name,'args':args,'config':os.environ.get('TAURI_CONFIG')})+'\\n')
def value(flag):return args[args.index(flag)+1]
if name=='uname':print('Darwin')
elif name=='git':
 if args==['rev-parse','HEAD']:print('a'*40)
elif name=='cargo':
 profile='release' if '--release' in args else 'debug'
 path=Path(value('--target-dir'))
 if '--target' in args:path=path/value('--target')
 path=path/profile;path.mkdir(parents=True,exist_ok=True)
 (path/value('-p')).write_text('binary for '+(' '.join(args)))
elif name=='swift':
 path=Path(value('--scratch-path'))/value('--configuration')/'bin';path.mkdir(parents=True,exist_ok=True)
 (path/'WispSciencePreview').write_text('swift binary')
 for n in ['WispSciencePreview_WispProjectBrowserUI.bundle','SwiftTerm_SwiftTerm.bundle','SwiftMath_SwiftMath.bundle']:
  (path/n).mkdir(exist_ok=True)
  (path/n/'resource.txt').write_text(n)
 if '--show-bin-path' in args:print(path)
'''


class NativeBuildRoutingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wisp native routing ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for folder in ['scripts', 'apps/macos', 'src-tauri/icons', 'ui/vendor-src/office-chunks', 'skills', 'python', 'r', 'browser-extension', 'seed', 'bin']:
            (self.root / folder).mkdir(parents=True, exist_ok=True)
        for path in ['scripts/build_native_macos.sh', 'scripts/stamp_native_macos.py',
                     'apps/macos/Info.plist', 'apps/macos/HostInfo.plist']:
            shutil.copyfile(SOURCE / path, self.root / path)
        (self.root / 'scripts/sync_native_design.py').write_text('')
        (self.root / 'src-tauri/tauri.conf.json').write_text(json.dumps({'identifier':'science.wisp-science','version':'1.14.0'}))
        (self.root / 'src-tauri/icons/icon.icns').write_bytes(b'icon')
        (self.root / 'ui/native-host.html').write_text('<html></html>')
        (self.root / 'ui/vendor-src/rdkit-worker.mjs').write_text('offline worker')
        (self.root / 'ui/vendor-src/office-chunks/renderer.mjs').write_text('offline Office')
        for tool in ['uname','cargo','swift','git','codesign']:
            path=self.root/'bin'/tool;path.write_text(FAKE_TOOL);path.chmod(0o755)

    def build(self, *args):
        env=dict(os.environ,FAKE_ROOT=str(self.root),PATH=str(self.root/'bin')+os.pathsep+os.environ['PATH'])
        result=subprocess.run(['bash',str(self.root/'scripts/build_native_macos.sh'),*args],env=env,capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        return [json.loads(x) for x in (self.root/'calls.jsonl').read_text().splitlines()]

    def assert_swift_resources(self, app):
        for bundle in ['WispSciencePreview_WispProjectBrowserUI.bundle',
                       'SwiftTerm_SwiftTerm.bundle', 'SwiftMath_SwiftMath.bundle']:
            with self.subTest(bundle=bundle):
                resource = app / 'Contents/Resources' / bundle / 'resource.txt'
                self.assertTrue(resource.is_file(), f'Missing packaged resource: {resource}')
                self.assertEqual(resource.read_text(), bundle)
        vendor = app / 'Contents/Resources/WispSciencePreview_WispProjectBrowserUI.bundle/vendor-runtime'
        self.assertEqual((vendor / 'rdkit-worker.mjs').read_text(), 'offline worker')
        self.assertEqual((vendor / 'office-chunks/renderer.mjs').read_text(), 'offline Office')
        self.assertFalse((vendor / 'vendor-src').exists())

    def test_rebuild_replaces_offline_assets_without_nesting_old_vendor_files(self):
        self.build('--qa')
        app = self.root / 'target/native-macos-qa/Wisp Science QA.app'
        vendor = app / 'Contents/Resources/WispSciencePreview_WispProjectBrowserUI.bundle/vendor-runtime'
        (vendor / 'obsolete.mjs').write_text('old resource')
        self.build('--qa')
        self.assert_swift_resources(app)
        self.assertFalse((vendor / 'obsolete.mjs').exists())

    def test_default_debug_preview_packages_all_swift_resources(self):
        self.build()
        self.assert_swift_resources(self.root/'target/native-macos/Wisp Science Preview.app')

    def test_both_release_architectures_route_all_three_binaries(self):
        for target, triple in [('aarch64-apple-darwin','arm64-apple-macosx13.0'),('x86_64-apple-darwin','x86_64-apple-macosx13.0')]:
            with self.subTest(target=target):
                (self.root/'calls.jsonl').write_text('')
                calls=self.build('--release','--target',target)
                cargo=[x for x in calls if x['tool']=='cargo'];swift=[x for x in calls if x['tool']=='swift']
                self.assertEqual(len(cargo),2)
                for call in cargo:
                    self.assertIn('--release',call['args']);self.assertIn(target,call['args'])
                for call in swift:
                    self.assertIn('release',call['args']);self.assertIn(triple,call['args'])
                app=self.root/'target/native-macos-release'/target/'Wisp Science Preview.app'
                self.assert_swift_resources(app)
                info=plistlib.loads((app/'Contents/Info.plist').read_bytes())
                self.assertEqual(info['WispBuildConfiguration'],'release')
                self.assertIn(target,(app/'Contents/MacOS/wisp-service').read_text())
                self.assertIn(target,(app/'Contents/Helpers/Wisp Desktop Host.app/Contents/MacOS/wisp-tauri').read_text())
                self.assertFalse((self.root/'target/native-macos/Wisp Science Preview.app').exists())

    def test_existing_qa_debug_mode_keeps_its_isolated_identifier_and_path(self):
        calls=self.build('--qa')
        app=self.root/'target/native-macos-qa/Wisp Science QA.app'
        self.assert_swift_resources(app)
        info=plistlib.loads((app/'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleIdentifier'],'science.wisp-science.native-toolbar-qa.preview')
        self.assertEqual(info['WispBuildConfiguration'],'debug')
        host=next(x for x in calls if x['tool']=='cargo' and 'wisp-tauri' in x['args'])
        self.assertEqual(json.loads(host['config'])['identifier'],'science.wisp-science.native-toolbar-qa')
        self.assertNotIn('--release',host['args'])

    def test_native_release_does_not_overwrite_the_running_debug_preview(self):
        self.build('--release')
        self.assert_swift_resources(self.root/'target/native-macos-release/Wisp Science Preview.app')
        self.assertTrue((self.root/'target/native-macos-release/Wisp Science Preview.app').exists())
        self.assertFalse((self.root/'target/native-macos/Wisp Science Preview.app').exists())


if __name__ == '__main__':
    unittest.main()
