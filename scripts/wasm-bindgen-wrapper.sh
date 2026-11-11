#!/usr/bin/env bash
# wasm-bindgen wrapper: strips `+reference-types` from the wasm's
# target_features custom section before invoking the real wasm-bindgen-cli.
#
# Why this exists
# ---------------
# webkit2gtk 2.52.3 on aarch64 (Ubuntu 24.04 noble-security) does not expose
# the externref table via `instance.exports` — a real engine bug. Any wasm
# using reference-types therefore renders the webview blank on ARM64.
#
# Trigger chain:
#   1. Rust 1.97's wasm32 target spec enables reference-types by default.
#   2. wasm-bindgen-cli >= 0.2.96 detects `+reference-types` in the wasm's
#      target_features custom section and auto-enables the externref-xform
#      pass, producing an externref table + rewriting the JS glue.
#   3. webkit2gtk ARM64 fails to expose that table → `table.grow(4)` runs on
#      the wrong funcref table → RangeError → UI blank.
#
# This wrapper patches the wasm to remove `+reference-types` *before* the cli
# sees it, causing wasm-bindgen-cli 0.2.96 to skip externref-xform entirely.
#
# Install (one-time, per machine)
# -------------------------------
#   cargo install wasm-bindgen-cli --version 0.2.96 --force
#   mv ~/.cargo/bin/wasm-bindgen ~/.cargo/bin/wasm-bindgen.real
#   ln -sf "$REPO/scripts/wasm-bindgen-wrapper.sh" ~/.cargo/bin/wasm-bindgen
#
# Override paths if needed:
#   WASM_BINDGEN_REAL=/path/to/wasm-bindgen.real
#   WASM_BINDGEN_STRIP=/path/to/strip_reference_types.py
#
# Trunk invokes us as:
#   wasm-bindgen --target web --out-dir <dir> [--no-typescript] <input.wasm>

set -e

# Note: readlink -f resolves the ~/.cargo/bin/wasm-bindgen symlink back to the
# repo, so SCRIPT_DIR is the repo's scripts/ dir. The real cli lives next to
# the symlink in ~/.cargo/bin (per the install steps above), not in SCRIPT_DIR.
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REAL="${WASM_BINDGEN_REAL:-$HOME/.cargo/bin/wasm-bindgen.real}"
STRIP="${WASM_BINDGEN_STRIP:-$SCRIPT_DIR/strip_reference_types.py}"

# Find the positional input wasm argument (last non-flag arg).
INPUT=""
for arg in "$@"; do
    case "$arg" in
        --*) ;;
        -*) ;;
        *) INPUT="$arg" ;;
    esac
done

if [ -n "$INPUT" ] && [ -f "$INPUT" ] && [[ "$INPUT" == *.wasm ]]; then
    TMP="$(mktemp -p "$(dirname "$INPUT")" .wbg-XXXXXX.wasm)"
    if python3 "$STRIP" "$INPUT" > /dev/null 2> /tmp/wbg-wrapper.log; then
        mv "$INPUT.no_reftypes" "$TMP"
        # Replace INPUT in argv with patched tmp file
        new_args=()
        for arg in "$@"; do
            if [ "$arg" = "$INPUT" ]; then
                new_args+=("$TMP")
            else
                new_args+=("$arg")
            fi
        done
        # `set -e` is on: capture the status so the temp file is removed either way.
        rc=0
        "$REAL" "${new_args[@]}" || rc=$?
        rm -f "$TMP"
        exit $rc
    else
        cat /tmp/wbg-wrapper.log >&2
        # Patch failed — fall through to real cli (let it error or run as-is)
    fi
fi

exec "$REAL" "$@"
