"""Build a separate, per-user WinUI preview installer (requires NSIS 3).

Self-contained and framework-dependent variants. The latter installs missing
Microsoft runtimes. No trimming or UPX; symbols/caches are excluded.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET

MIB = 1024 * 1024
MAX_INSTALLER_MIB = 100
MAX_PAYLOAD_MIB = 350
VARIANTS = {'self-contained': (100, 350), 'framework-dependent': (40, 150)}
SCRIPT_ROOT = Path(__file__).resolve().parent
REQUIRED = (
    'Wisp.Science.Preview.exe', 'wisp-service.exe', 'settings-host/wisp-tauri.exe',
    'coreclr.dll', 'Microsoft.UI.Xaml.dll',
    'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll',
    'settings-host/vcruntime140.dll', 'settings-host/vcruntime140_1.dll', 'settings-host/msvcp140.dll',
)
RESOURCES = ('skills', 'python', 'r', 'browser-extension', 'seed')


def nsis_quote(value):
    return str(value).replace('$', '$$').replace('"', '$\\"')


def validate_binary(path):
    with path.open('rb') as source:
        if source.read(2) != b'MZ':
            raise ValueError(f'Not a PE executable: {path}')
        source.seek(0x3c)
        offset = source.read(4)
        if len(offset) != 4:
            raise ValueError(f'Truncated PE header: {path}')
        source.seek(struct.unpack('<I', offset)[0])
        if source.read(6) != b'PE\0\0\x64\x86':
            raise ValueError(f'Expected x64 executable: {path}')


def payload_files(app, variant='self-contained'):
    for relative in REQUIRED:
        if variant == 'framework-dependent' and relative in ('coreclr.dll', 'Microsoft.UI.Xaml.dll'):
            if (app / relative).exists():
                raise ValueError(f'Framework-dependent payload contains bundled runtime: {relative}')
            continue
        validate_binary(app / relative)
    if variant == 'framework-dependent':
        validate_binary(app / 'Microsoft.WindowsAppRuntime.Bootstrap.dll')
        options = json.loads((app / 'Wisp.Science.Preview.runtimeconfig.json').read_text(encoding='utf-8'))['runtimeOptions']
        framework = options.get('framework', {})
        if framework.get('name') != 'Microsoft.NETCore.App' or not framework.get('version', '').startswith('8.0.'):
            raise ValueError('Expected framework-dependent .NET 8 runtime configuration.')
    for resource in RESOURCES:
        folder = app / 'settings-host' / resource
        if not folder.is_dir() or not any(p.is_file() for p in folder.rglob('*')):
            raise ValueError(f'Missing host resource: {resource}')
    files = []
    for path in sorted(app.rglob('*')):
        if path.is_symlink() or (hasattr(path, 'is_junction') and path.is_junction()):
            raise ValueError(f'Links are not allowed in installer payload: {path}')
        relative = path.relative_to(app)
        if any(part.lower() in {'__pycache__', '.git', 'obj', 'node_modules'} for part in relative.parts):
            continue
        if path.is_file() and path.suffix.lower() not in {'.pdb', '.dbg', '.ilk', '.exp', '.lib', '.pyc', '.pyo'}:
            files.append(relative)
    return files


def installer_script(payload, files, installer, version, size, variant='self-contained', runtime_files=None):
    bootstrap = ''
    if variant == 'framework-dependent':
        bootstrap = f'''
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=ensure-runtimes.ps1 "{nsis_quote(runtime_files[0])}"
  File /oname=runtimes.json "{nsis_quote(runtime_files[1])}"
  DetailPrint "Checking .NET 8 and Windows App Runtime (x64)..."
  nsExec::ExecToLog '$\\"$WINDIR\\Sysnative\\WindowsPowerShell\\v1.0\\powershell.exe$\\" -NoProfile -ExecutionPolicy Bypass -File $\\"$PLUGINSDIR\\ensure-runtimes.ps1$\\" -ConfigPath $\\"$PLUGINSDIR\\runtimes.json$\\"'
  Pop $0
  ${{If}} $0 == 3010
    SetRebootFlag true
  ${{ElseIf}} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "Required runtime setup failed. Check the installation log/network, allow the .NET UAC prompt, or use the self-contained installer." /SD IDOK
    SetErrorLevel 1
    Abort
  ${{EndIf}}
'''
    # Explicit delete entries preserve unrelated files and all application data.
    deletes = '\n'.join(f'  Delete "$INSTDIR\\{nsis_quote(str(p).replace(chr(47), chr(92)))}"' for p in files)
    directories = {parent for p in files for parent in p.parents if parent != Path('.')}
    removes = '\n'.join(f'  RMDir "$INSTDIR\\{nsis_quote(str(p).replace(chr(47), chr(92)))}"'
                        for p in sorted(directories, key=lambda p: (-len(p.parts), str(p))))
    return f'''Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!define AtLeastWin1809 'U>= WinVer_BuildNumCheck 17763'
Name "Wisp Science WinUI Preview ({variant})"
OutFile "{nsis_quote(installer)}"
InstallDir "$LOCALAPPDATA\\Programs\\Wisp Science WinUI Preview"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 64
VIProductVersion "{version}.0"
VIAddVersionKey "ProductName" "Wisp Science WinUI Preview"
VIAddVersionKey "FileDescription" "Wisp Science WinUI Preview Setup ({variant})"
VIAddVersionKey "FileVersion" "{version}"
VIAddVersionKey "LegalCopyright" "Wisp Science contributors"
!define MUI_ABORTWARNING
{('!define MUI_WELCOMEPAGE_TEXT "This lightweight installer checks for .NET 8 and Windows App Runtime (x64). Missing runtimes are downloaded from Microsoft. Installing .NET may show a Windows administrator permission prompt. Use the self-contained installer for bundled runtimes."') if variant == 'framework-dependent' else ''}
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${{IfNot}} ${{RunningX64}}
    Abort "This preview requires x64 Windows."
  ${{EndIf}}
  ${{IfNot}} ${{AtLeastWin10}}
    Abort "This preview requires Windows 10 1809 or later."
  ${{EndIf}}
  ${{IfNot}} ${{AtLeastWin1809}}
    Abort "This preview requires Windows 10 1809 or later."
  ${{EndIf}}
FunctionEnd

Section "Install"
  SetShellVarContext current
{bootstrap}
  ; Remove the previous payload with its own file manifest before upgrading,
  ; so retired dependencies cannot survive. User data is outside this tree.
  IfFileExists "$INSTDIR\\Uninstall.exe" 0 fresh_install
  StrCpy $0 1
  ExecWait '$\\"$INSTDIR\\Uninstall.exe$\\" /S _?=$INSTDIR' $0
  ${{If}} $0 != 0
    Abort "Could not remove the previous preview. Close it and retry."
  ${{EndIf}}
fresh_install:
  SetOutPath "$INSTDIR"
  File /r "{nsis_quote(payload)}\\*"
  WriteUninstaller "$INSTDIR\\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\\Wisp Science WinUI Preview.lnk" "$INSTDIR\\Wisp.Science.Preview.exe"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "DisplayName" "Wisp Science WinUI Preview"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "DisplayVersion" "{version}"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "Publisher" "Wisp Science"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "UninstallString" '$\\"$INSTDIR\\Uninstall.exe$\\"'
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "QuietUninstallString" '$\\"$INSTDIR\\Uninstall.exe$\\" /S'
  WriteRegDWORD HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "EstimatedSize" {(size + 1023) // 1024}
  WriteRegDWORD HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "NoModify" 1
  WriteRegDWORD HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
{deletes}
{removes}
  Delete "$INSTDIR\\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\\Wisp Science WinUI Preview.lnk"
  DeleteRegKey HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\WispScienceWinUIPreview"
SectionEnd
'''


def finalize(installer, max_installer_mib=None):
    report_path = installer.with_suffix('.size.json')
    report = json.loads(report_path.read_text(encoding='utf-8')) if report_path.exists() else {}
    if max_installer_mib is None:
        max_installer_mib = VARIANTS[report.get('variant', 'self-contained')][0]
    size = installer.stat().st_size
    if size == 0 or size > max_installer_mib * MIB:
        raise ValueError(f'Installer size {size / MIB:.1f} MiB exceeds budget {max_installer_mib} MiB (or is empty).')
    checksum = hashlib.sha256(installer.read_bytes()).hexdigest()
    installer.with_suffix('.exe.sha256').write_text(f'{checksum}  {installer.name}\n', encoding='utf-8')
    report.update(installer=installer.name, installer_bytes=size, installer_limit_mib=max_installer_mib, sha256=checksum)
    report_path.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


def package(app, tag, output, makensis='makensis', *, version=None,
            max_payload_mib=None, max_installer_mib=None, variant='self-contained'):
    installer_limit, payload_limit = VARIANTS[variant]
    if max_payload_mib is None:
        max_payload_mib = payload_limit
    if max_installer_mib is None:
        max_installer_mib = installer_limit
    if not re.fullmatch(r'v\d+\.\d+\.\d+', tag):
        raise ValueError('Expected a release tag vX.Y.Z.')
    if version is None:
        with (Path(__file__).resolve().parents[1] / 'Cargo.toml').open('rb') as source:
            version = tomllib.load(source)['workspace']['package']['version']
    if tag != f'v{version}':
        raise ValueError(f'Tag {tag} does not match source version {version}.')
    app, output = app.resolve(), output.resolve()
    if app == output or app in output.parents:
        raise ValueError('Installer output must be outside the application directory.')
    files = payload_files(app, variant)
    runtime_files = None
    if variant == 'framework-dependent':
        config = json.loads((SCRIPT_ROOT / 'native_windows_runtimes.json').read_text(encoding='utf-8'))
        project = ET.parse(SCRIPT_ROOT.parent / 'apps/windows/Wisp.Science.Preview/Wisp.Science.Preview.csproj')
        sdk = project.find(".//PackageReference[@Include='Microsoft.WindowsAppSDK']").get('Version')
        if sdk != config['windowsAppSdkVersion']:
            raise ValueError('Update the runtime installer policy for the new Windows App SDK version.')
        options = json.loads((app / 'Wisp.Science.Preview.runtimeconfig.json').read_text(encoding='utf-8'))['runtimeOptions']
        config['dotnetMinimumVersion'] = options['framework']['version']
    sizes = [(str(p), (app / p).stat().st_size) for p in files]
    total = sum(size for _, size in sizes)
    if total > max_payload_mib * MIB:
        raise ValueError(f'Payload {total / MIB:.1f} MiB exceeds budget {max_payload_mib} MiB; largest files: {sorted(sizes, key=lambda p: p[1], reverse=True)[:10]}')
    output.mkdir(parents=True, exist_ok=True)
    name = f'Wisp-Science-WinUI-Preview_{version}_x64-{variant}-setup.exe'
    with tempfile.TemporaryDirectory(prefix='wisp-winui-', dir=output) as temporary:
        staging = Path(temporary)
        payload = staging / 'payload'
        for relative in files:
            destination = payload / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(app / relative, destination)
        candidate = staging / name
        script = staging / 'preview.nsi'
        if variant == 'framework-dependent':
            config_path = staging / 'runtimes.json'
            config_path.write_text(json.dumps(config), encoding='utf-8')
            runtime_files = (SCRIPT_ROOT / 'ensure_native_windows_runtimes.ps1', config_path)
        script.write_text(installer_script(payload, files, candidate, version, total, variant, runtime_files), encoding='utf-8-sig')
        subprocess.run([str(makensis), '/V2', str(script)], check=True)
        finalize(candidate, max_installer_mib)
        installer = output / name
        shutil.move(candidate, installer)
    report = dict(variant=variant, payload_bytes=total, payload_limit_mib=max_payload_mib, file_count=len(files),
                  largest_files=[dict(path=p, bytes=s) for p, s in sorted(sizes, key=lambda p: p[1], reverse=True)[:20]])
    installer.with_suffix('.size.json').write_text(json.dumps(report), encoding='utf-8')
    finalize(installer, max_installer_mib)
    return installer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path, nargs='?')
    parser.add_argument('--tag')
    parser.add_argument('--output', type=Path, default=Path('target/winui-release'))
    parser.add_argument('--makensis', default='makensis')
    parser.add_argument('--variant', choices=VARIANTS, default='self-contained')
    parser.add_argument('--max-payload-mib', type=int)
    parser.add_argument('--max-installer-mib', type=int)
    parser.add_argument('--finalize', type=Path, help='Recheck size and regenerate SHA-256 after signing.')
    args = parser.parse_args()
    if args.finalize:
        print(json.dumps(finalize(args.finalize, args.max_installer_mib), indent=2))
    elif args.app and args.tag:
        print(package(args.app, args.tag, args.output, args.makensis,
                      max_payload_mib=args.max_payload_mib, max_installer_mib=args.max_installer_mib, variant=args.variant))
    else:
        parser.error('Provide APP --tag vX.Y.Z, or --finalize INSTALLER.')


if __name__ == '__main__':
    main()
