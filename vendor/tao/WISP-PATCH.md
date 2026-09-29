# Temporary macOS redraw-reentry override

This directory contains the published `tao 0.37.0` crate, under its original
Apache-2.0 license. It is excluded from Wisp's workspace members and selected
through the root `[patch.crates-io]` section.

## Provenance

- Registry: <https://crates.io/crates/tao/0.37.0>
- Published `.crate` SHA-256:
  `5d12c8607b01516c5d4a37e471a9cfb40b4a25cebce8fecbc6a559e125e51b6b`
- Upstream tag: `tao-v0.37.0`
- `src/` other than the local change below, `examples/`, `README.md`,
  `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock`, and both license files are
  unchanged from the published crate. Registry cache markers are omitted.

## Local change

Only `src/platform_impl/macos/app_state.rs` `AppState::handle_redraw` is
changed. `wakeup` / `cleared` already skip work when `in_callback` is set
(winit#1779); `handle_redraw` did not. Nested AppKit run loops from
`NSSavePanel` sheets or `-[NSView display]` can therefore call `drawRect:`
while `handle_user_events` still holds the non-recursive `callback` mutex.

The patched function keeps the outer `in_callback` guard, enqueues a
deduplicated redraw via the existing `queue_redraw` path, and lets `cleared`
dispatch `RedrawRequested` after the outer callback returns. It does not
switch the callback mutex to a recursive lock and does not drop the frame.

This fixes the reproduced macOS deadlock tracked in
[Wisp #1250](https://github.com/xuzhougeng/wisp-science/issues/1250).
It is independent of the Windows input-reentrancy fix in Tao 0.37.0
([Wisp #1265](https://github.com/xuzhougeng/wisp-science/issues/1265)).

## Removal

When a published `tao` includes this `handle_redraw` guard, remove this
directory, the root patch, and the workspace exclusion. Keep the native
redraw regression after removing the override. As of 2026-09-29, `tao 0.37.1`
still has the unguarded `handle_redraw`; published `tauri-runtime-wry 2.12.0`
requires Tao `^0.37.0`, so this 0.37.0 override applies without a runtime patch.
