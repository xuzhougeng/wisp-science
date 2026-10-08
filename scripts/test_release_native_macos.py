"""Exercise release workflow routing for current and historical SwiftUI tags."""
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import textwrap
import unittest

SOURCE = Path(__file__).resolve().parent.parent


def packaging_step():
    workflow = (SOURCE / '.github/workflows/release-macos.yml').read_text()
    step = workflow.split('      - name: Sign and notarize SwiftUI\n', 1)[1]
    body = step.split('        run: |\n', 1)[1].split('\n      - ', 1)[0]
    return textwrap.dedent(body)


class SwiftUIReleaseRoutingTests(unittest.TestCase):
    def test_packaging_uses_tagged_app_name_and_exports_actual_asset_prefix(self):
        for app_name, prefix in [
            ('Wisp Science SwiftUI', 'Wisp-Science-SwiftUI'),
            ('Wisp Science Preview', 'Wisp-Science-SwiftUI-Preview'),
        ]:
            for arch in ['aarch64', 'x86_64']:
                with self.subTest(app=app_name, arch=arch), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    (root / 'apps/macos').mkdir(parents=True)
                    (root / 'scripts').mkdir()
                    (root / 'apps/macos/Info.plist').write_bytes(
                        plistlib.dumps({'CFBundleName': app_name}))
                    (root / 'scripts/package_native_macos.py').write_text(textwrap.dedent('''\
                        import json, os, sys
                        from pathlib import Path
                        Path('package-args.json').write_text(json.dumps(sys.argv[1:]))
                        print(Path(sys.argv[-1]).resolve() / os.environ['TEST_ASSET'])
                    '''))
                    target = arch + '-apple-darwin'
                    output = root / 'github-output'
                    env = dict(os.environ, RELEASE_TAG='v1.18.0', GITHUB_OUTPUT=str(output),
                               TEST_ASSET=f'{prefix}_1.18.0_{arch}.dmg')
                    script = packaging_step().replace('${{ matrix.target }}', target)
                    result = subprocess.run(['bash', '-e', '-c', script], cwd=root, env=env,
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(json.loads((root / 'package-args.json').read_text()), [
                        f'target/native-macos-release/{target}/{app_name}.app',
                        '--target', target, '--tag', 'v1.18.0',
                        '--output', f'target/swiftui-release/{target}',
                    ])
                    self.assertEqual(output.read_text(), f'asset-prefix={prefix}\n')


if __name__ == '__main__':
    unittest.main()
