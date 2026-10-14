# CI routing

`Test` and `Native Apps` run a small `changes / detect` job first, then select
builds and tests using `scripts/ci_scope.py`. Its job summary lists which groups
run or skip. Existing job names are retained; unrelated jobs are skipped before
allocating a platform runner. New commits cancel older runs of the same workflow
and PR or branch.

| Changed inputs | Selected checks |
| --- | --- |
| Only `apps/ios/**` | iOS protocol package tests, simulator app/test compilation and app unit tests |
| Ordinary `apps/windows/**` source | WinUI only, plus a lightweight shared asset check |
| Ordinary `apps/macos/**` source | macOS only |
| Shared DTO/sync crates, `src-tauri/**`, `contracts/**`, root Cargo/toolchain/config | Rust/MSRV, headless, desktop regressions, web UI, all native apps and contract checks |
| Other `crates/**`, runtime scripts/resources | Rust backend/headless, desktop regressions and native desktop builds; no web/iOS build |
| Ordinary `ui/**` or `vendor/leptos_reactive/**` source | Independent wasm/MSRV and browser checks; no native/backend build |
| `ui-tests/**` | Web UI, except website-specific tests/config handled by Website |
| `ui/build-fast*` | Web checks plus lightweight Node tests on three OSes; no Rust headless eval |
| Shared design source, settings export scripts or brand wordmarks | Lightweight `shared-assets` check, in addition to the source module's own checks |
| Shared native resources under macOS, `ui/vendor-src/**` or native host assets | The native clients that actually bundle those resources, plus shared asset checks |
| `vendor/tao/**` | Rust and native desktop builds/regressions; no web/iOS build |
| Known native build/package scripts | Their corresponding native platform |
| Website/docs inputs | Existing `Website` path filters; no app builds |
| Release notes/helper | Release-notes helper |
| CI selector changes or unknown inputs | All groups, conservatively |

Workflow changes select the jobs of that workflow. This CI routing PR therefore
runs broad checks itself; subsequent module-only PRs take the narrow path. A mobile
feature that also changes Rust storage, remote commands or shared contracts still
needs backend and integration coverage.

Web wasm compilation has its own `ui-check` matrix instead of sharing the
`cargo test --workspace` job. Shared design/settings checks run in a small Ubuntu
job, so C# or web icon changes do not start a macOS build just to check generated
JSON/SVGs. WinUI directly links `apps/macos/Sources/WispProjectBrowserUI/Resources/*`,
and both native clients bundle web vendor modules: those specific shared inputs
still select their real consumers. The macOS packaging script builds the web UI
when macOS is selected; an ordinary web-only change is validated by web CI without
repeating the native package build.

The selector uses the entire PR merge-base-to-head diff, or the push's
before-to-after diff, rather than just the last commit. It reads NUL-delimited
Git paths with rename detection disabled, so deletions and both sides of a rename
affect routing and there is no changed-files API pagination limit. A missing
commit/event/diff selects every group. A failed selector regression prevents the
dependent jobs from running and leaves a failed check visible.

The reusable workflow requires only `contents: read`, including fork PRs. It uses
the caller's event, as described in the [GitHub reusable workflow reference](https://docs.github.com/en/actions/reference/workflows-and-actions/reusing-workflow-configurations#github-context).
Routing is at job level so the main app workflows still report a result when a
component is irrelevant. Wasm checks now report `ui-check` instead of being hidden
inside `rust`; update required checks accordingly if enabling branch protection.
If configuring required checks, include each workflow's
`changes / detect` check so a broken selector cannot be accepted through skipped
dependent jobs. Release/tag/manual deployment workflows keep their existing triggers.

The `ios` job runs `swift test` for the Foundation remote package, then
`xcodebuild build-for-testing` for the shared WispScience scheme and
`test-without-building -only-testing:WispScienceTests` on an installed iPhone
simulator. UI test sources are compiled, but the UI interaction suite, physical
camera/OCR behavior, device signing and distribution are not exercised by this
job. Xcode test results are uploaded when available. The existing `swiftui` job
is the macOS app, not the iOS app.

When changing routing, update `scripts/test_ci_scope.py`. It covers iOS-only and
mixed changes, shared native assets, unknown paths, missing commits, multiple
commits, divergent PR bases, deletions, cross-component renames, unusual filenames
and large diffs. Subject to `AGENTS.md`'s local test permission rule:

```sh
python3 -m unittest discover -s scripts -p test_ci_scope.py
```

For manual CI review, inspect `changes / detect` and verify that an iOS-only PR
selects just `ios`, a WinUI-only PR selects `windows` and the small shared asset
check, a normal web UI change selects only `ui`, a backend/iOS PR selects both,
and an unknown build input
selects all groups. A docs-only PR should retain the Website checks without
building the apps.
