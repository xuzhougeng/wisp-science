# Temporary disposed-owner effect override

This directory contains the published `leptos_reactive 0.6.15` crate, under
its MIT license. It is used only by the standalone `ui/` workspace, selected
through `[patch.crates-io]` in `ui/Cargo.toml`. No root-workspace crate
depends on it, so it needs no root `exclude` entry.

## Provenance

- Registry: <https://crates.io/crates/leptos_reactive/0.6.15>
- Published `.crate` SHA-256 (matches `ui/Cargo.lock`):
  `e4161acbf80f59219d8d14182371f57302bc7ff81ee41aba8ba1ff7295727f23`
- `Cargo.toml`, `Cargo.toml.orig`, `Makefile.toml`, `tests/` and `src/` other
  than the local change below are unchanged. Registry markers are omitted.
- The published crate ships no license file. `LICENSE` is the MIT text of the
  upstream repository, <https://github.com/leptos-rs/leptos>.

## Local change

Only `create_effect` in `src/effect.rs` is changed. It queues the effect's
first run in a microtask and called `with_owner(owner.unwrap(), ..)`, which
panics with `OwnerDisposed` when the owner was disposed before the microtask
ran. Keyed rows that are rebuilt inside one tick (streaming turns, artifact
cards) hit this routinely. The patch uses `try_with_owner` and returns early:
a disposed owner also disposed the effect, so there is nothing to run.

Why a panic is not acceptable here: stable `wasm32-unknown-unknown` cannot
unwind (`panic = "unwind"` has no effect and `catch_unwind` cannot catch), so
every panic is a `RuntimeError: unreachable` trap. A trap runs no destructors
(held `RefCell` borrows stay borrowed) and never restores the shadow-stack
pointer, so each one leaks the frames that were live. After enough traps the
1 MiB shadow stack is exhausted: deep calls such as rendering fail with
`memory access out of bounds` first, then every call into the app does — a
dead window while the backend keeps running. The desktop heartbeat counts
these traps as `script_errors`; release logs showed thousands per session.

## Removal

Remove this directory and the `ui/Cargo.toml` patch when the UI moves off
Leptos 0.6, or when a published 0.6 release stops panicking in
`create_effect` for a disposed owner.
