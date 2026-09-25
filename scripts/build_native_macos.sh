#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo 'The SwiftUI preview must be built on macOS.' >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
QA_MODE=0
case "${1:-}" in
  '') ;;
  --qa) QA_MODE=1 ;;
  *) echo 'Usage: build_native_macos.sh [--qa]' >&2; exit 2 ;;
esac
if [[ "$#" -gt 1 ]]; then
  echo 'Usage: build_native_macos.sh [--qa]' >&2
  exit 2
fi
BUILD="$ROOT/target/native-macos"
APP_NAME='Wisp Science Preview'
if [[ "$QA_MODE" == 1 ]]; then
  BUILD="$ROOT/target/native-macos-qa"
  APP_NAME='Wisp Science QA'
fi
APP="$BUILD/$APP_NAME.app"
HOST_IDENTIFIER="$(python3 -c 'import json,sys; print("science.wisp-science.native-toolbar-qa" if sys.argv[2] == "1" else json.load(open(sys.argv[1]))["identifier"])' "$ROOT/src-tauri/tauri.conf.json" "$QA_MODE")"
export CLANG_MODULE_CACHE_PATH="$BUILD/clang-module-cache"

python3 "$ROOT/scripts/sync_native_design.py" --check
cargo build --manifest-path "$ROOT/Cargo.toml" --target-dir "$ROOT/target" --locked -p wisp-service
# A small inert document hosts legacy command extractors. No WebView settings
# UI is bundled in this helper; all visible settings controls are native.
HOST_ASSETS="$BUILD/host-assets"
mkdir -p "$HOST_ASSETS"
cp "$ROOT/ui/native-host.html" "$HOST_ASSETS/native-host.html"
cp "$ROOT/ui/native-host.html" "$HOST_ASSETS/index.html"
TAURI_CONFIG="$(python3 -c 'import json,sys; print(json.dumps({"identifier":sys.argv[2],"build":{"frontendDist":sys.argv[1]}}))' "$HOST_ASSETS" "$HOST_IDENTIFIER")" \
  cargo build --manifest-path "$ROOT/Cargo.toml" --target-dir "$ROOT/target" --locked -p wisp-tauri --features custom-protocol
swift build --package-path "$ROOT/apps/macos" --scratch-path "$BUILD/swift" --disable-sandbox
SWIFT_BIN="$(swift build --package-path "$ROOT/apps/macos" --scratch-path "$BUILD/swift" --show-bin-path)"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$SWIFT_BIN/WispSciencePreview" "$APP/Contents/MacOS/WispSciencePreview"
install -m 755 "$ROOT/target/debug/wisp-service" "$APP/Contents/MacOS/wisp-service"
cp "$ROOT/src-tauri/icons/icon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp -R "$SWIFT_BIN/WispSciencePreview_WispProjectBrowserUI.bundle" "$APP/Contents/Resources/"
cp -R "$SWIFT_BIN/SwiftTerm_SwiftTerm.bundle" "$APP/Contents/Resources/"
# SwiftTerm 1.19 probes Contents/Resources itself. A resource symlink at the
# .app root makes codesign reject the bundle as unsealed; remove the legacy
# link left by earlier native preview builds.
if [[ -L "$APP/SwiftTerm_SwiftTerm.bundle" ]]; then
  rm "$APP/SwiftTerm_SwiftTerm.bundle"
fi
HOST_APP="$APP/Contents/Helpers/Wisp Desktop Host.app"
mkdir -p "$HOST_APP/Contents/MacOS" "$HOST_APP/Contents/Resources"
install -m 755 "$ROOT/target/debug/wisp-tauri" "$HOST_APP/Contents/MacOS/wisp-tauri"
for resource in skills python r browser-extension seed; do
  rm -rf "$HOST_APP/Contents/Resources/$resource"
  cp -R "$ROOT/$resource" "$HOST_APP/Contents/Resources/$resource"
done
cp "$ROOT/apps/macos/HostInfo.plist" "$HOST_APP/Contents/Info.plist"
cp "$ROOT/apps/macos/Info.plist" "$APP/Contents/Info.plist"
if [[ "$QA_MODE" == 1 ]]; then
  python3 - "$APP/Contents/Info.plist" "$HOST_IDENTIFIER" <<'PY_QA'
import plistlib, sys
with open(sys.argv[1], "rb") as source:
    info = plistlib.load(source)
info["CFBundleIdentifier"] = sys.argv[2] + ".preview"
info["CFBundleName"] = info["CFBundleDisplayName"] = "Wisp Science QA"
with open(sys.argv[1], "wb") as target:
    plistlib.dump(info, target)
PY_QA
fi
python3 "$ROOT/scripts/stamp_native_macos.py" "$APP" "$HOST_IDENTIFIER"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
printf 'Built: %s\n' "$APP"
if [[ "$QA_MODE" == 1 ]]; then
  printf 'QA host identifier: %s\nLaunch the executable with WISP_BROWSER_DATABASE pointing to the isolated host database; do not use open without that configuration.\n' "$HOST_IDENTIFIER"
else
  printf 'Open with: open "%s"\n' "$APP"
fi
