#!/usr/bin/env python3
"""Export the native settings allowlist and original Rust argument signatures.

Shared by SwiftUI and WinUI 3; CI --check detects contract/registration drift.
Payload objects retain the existing command DTOs; this is not a second schema.
"""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / 'contracts/native-settings/v1/commands.json'
COMMAND_ALIASES = {'copy_demo_to_project': 'copy_demo_to_project_cmd'}
ADAPTERS = {
    'native_workflow_conversion_progress': ([{'name': 'conversionId', 'rust_type': 'String', 'optional': False}], 'Option<WorkflowConversionProgress>'),
    'native_settings_capabilities': ([], 'NativeSettingsCapabilities'),
    'native_download_update': ([], '()'),
    'native_terminal_snapshot': ([{'name': 'sessionId', 'rust_type': 'String', 'optional': False}], 'TerminalSnapshot'),
}

def export():
    source = (ROOT / 'crates/wisp-dto/src/native_settings.rs').read_text(encoding="utf-8")
    block = source.split('pub const COMMANDS:')[1].split('];')[0]
    commands = re.findall(r'"([a-z_]+)"', block)
    if len(commands) != len(set(commands)):
        raise ValueError('Duplicate native settings command')
    handlers = (ROOT / 'src-tauri/src/lib.rs').read_text(encoding="utf-8").split('.invoke_handler(')[-1]
    sources = [(p, p.read_text(encoding="utf-8")) for p in (ROOT / 'src-tauri/src').rglob('*.rs')]
    exported = []
    for name in sorted(commands + ['native_settings_capabilities']):
        if name in ADAPTERS:
            args, result = ADAPTERS[name]
            location = 'src-tauri/src/native_settings.rs'
        else:
            handler = COMMAND_ALIASES.get(name, name)
            if not re.search(r'\b' + handler + r'\s*,', handlers):
                raise ValueError(f'{name} is not registered in Tauri')
            matches = [(p, re.search(r'#\[tauri::command(?:\([^\]]*\))?\]\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+' + handler + r'\s*\((.*?)\)\s*(?:->\s*(.*?))?\s*\{', text, re.S)) for p, text in sources]
            matches = [(p, m) for p, m in matches if m]
            if len(matches) != 1: raise ValueError(f"Expected one Tauri command definition for {name}, got {len(matches)}")
            path, match = matches[0]
            if handler != name and f'rename = "{name}"' not in match[0]:
                raise ValueError(f'{handler} does not declare the {name} command alias')
            args = []
            for line in match[1].splitlines():
                m = re.match(r'\s*(?:mut\s+)?(\w+)\s*:\s*(.*?)\s*,?\s*$', line)
                if not m: continue
                key, typ = m.groups(); typ = typ.rstrip(',')
                if re.search(r'(?:State<|AppHandle|WorkspaceSurface|WebviewWindow)', typ): continue
                camel = re.sub(r'_([a-z])', lambda m: m[1].upper(), key)
                args.append({'name': camel, 'rust_type': typ, 'optional': typ.startswith('Option<')})
            result = match[2]
            location = path.relative_to(ROOT).as_posix()
        exported.append({'command': name, 'arguments': args, 'result': result, 'source': location})
    return (json.dumps({'schema': 'wisp.native-settings.v1', 'commands': exported}, ensure_ascii=False, indent=2) + '\n').encode()

def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument('--check', action='store_true'); args = parser.parse_args()
    expected = export()
    if args.check:
        if not TARGET.exists() or TARGET.read_bytes().replace(b"\r\n", b"\n") != expected: parser.exit(1, 'Native settings contract drift. Run scripts/sync_native_settings_contract.py\n')
    else:
        TARGET.parent.mkdir(parents=True, exist_ok=True); TARGET.write_bytes(expected)

if __name__ == '__main__': main()
