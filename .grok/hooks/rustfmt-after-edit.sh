#!/usr/bin/env bash
# Format a Rust file after Grok Build writes it. Fail-open: never block the tool.
set -u

payload=$(cat)
path=$(printf '%s' "$payload" | jq -r '.toolInput.file_path // .toolInput.filePath // .toolInput.path // empty' 2>/dev/null || true)
case "$path" in
  *.rs) ;;
  *) exit 0 ;;
esac

root=$(printf '%s' "$payload" | jq -r '.workspaceRoot // .cwd // empty' 2>/dev/null || true)
if [ -n "$root" ] && [ "${path#/}" = "$path" ]; then
  path="${root%/}/$path"
fi
[ -f "$path" ] || exit 0

dir=$(dirname -- "$path")
cargo_root=""
while [ "$dir" != "/" ]; do
  if [ -f "$dir/Cargo.toml" ]; then
    cargo_root=$dir
    break
  fi
  dir=$(dirname -- "$dir")
done

if [ -n "$cargo_root" ] && command -v cargo >/dev/null 2>&1; then
  cargo fmt --manifest-path "$cargo_root/Cargo.toml" -- "$path" >/dev/null 2>&1 || true
elif command -v rustfmt >/dev/null 2>&1; then
  rustfmt --edition 2021 "$path" >/dev/null 2>&1 || true
fi
exit 0
