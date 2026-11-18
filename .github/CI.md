# CI routing

`ci-changes.yml` calls `scripts/ci_scope.py` for the complete PR merge-base diff
or push before/after diff. NUL-delimited paths with rename detection disabled
include deletions and both sides of renames. Unknown build inputs and missing
commits/events/diffs conservatively select all groups.

| Change | Selected jobs |
| --- | --- |
| `apps/gpui/**`, `gpui.yml` | GPUI checks and offline state tests on macOS/Windows/Linux |
| `release-gpui.yml`, `scripts/package_gpui.py`, GPUI manifest/build script/resource lookup, `crates/wisp-paths/**` | GPUI Release's own filters: unsigned release build and package on macOS (both architectures), Windows and Linux, kept as workflow artifacts |
| `apps/ios/**`, `ios.yml` | iOS protocol tests, app compilation and unit tests |
| `apps/android/**` | Independent Android workflow filters |
| Shared DTO/sync, Tauri, contracts, root Cargo/config | Rust, headless, desktop, WebView, GPUI, iOS |
| Other Rust crates, skills, runtime assets | Backend/headless, desktop regressions, GPUI |
| `ui/**`, `ui-tests/**`, Leptos vendor | WebView; fast asset inputs also select frontend-assets |
| Tao vendor | Rust and legacy desktop regressions |
| Relay JavaScript | Relay worker checks |
| Website/docs | Existing Website filters; no application builds |
| Release notes/helper | Release-notes helper |

SwiftUI/WinUI desktop jobs, generated design checks and the parity-ledger gate
have been removed. `ios.yml` retains protocol tests, unsigned simulator builds,
app unit tests and result artifacts. Android remains independent.

GPUI has its own Cargo workspace/lockfile and stays outside the root
`cargo test --workspace` dependency graph. Its matrix compiles the frontend and
runs state tests without opening a window or requiring model credentials.
Release workflows still publish WebView installers during migration; GPUI
Release attaches the GPUI preview archives beside them.

When changing routing, update `scripts/test_ci_scope.py`. Local execution needs
permission under `AGENTS.md`. Cover iOS-only, GPUI-only, mixed backend,
renamed/deleted, large and unknown-path diffs. Require `changes / detect` so a
failed selector cannot pass through skipped dependent jobs.
