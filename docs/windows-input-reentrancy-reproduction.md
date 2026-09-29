# Windows input deadlock reproduction (#1265)

On 2026-09-16 the previous `tao 0.35.3` dependency reproduced a native UI
deadlock in a disposable Tauri window. Wisp now uses Tao 0.37.0, which includes
the upstream keyboard/IME reentrancy fix. The example is retained as a native
Windows regression and runs in CI with the locked desktop dependency chain.

Issue: <https://github.com/xuzhougeng/wisp-science/issues/1265>.

## Recorded baseline (before the fix)

The native reproducer ran on the reporting Windows machine using the workspace
at `026857d2` and its unchanged lockfile (`tauri-runtime-wry 2.11.4`,
`tao 0.35.3`). Each trial started a fresh process.

| Scenario | Trials | Result |
| --- | ---: | --- |
| Serial keydown, keyup, char and syschar, each followed by focus loss | 5 processes / 20 message cases | All returned successfully |
| Keydown with a sent focus-loss message pending at the input boundary | 5 | 5 deadlocks / input timeout, exit 1 |
| Keyup with the same pending focus-loss message | 3 | 3 deadlocks / input timeout, exit 1 |
| Plain `WM_CHAR` containing 中 with a pending focus-loss message | 3 | All returned successfully |
| Plain `WM_SYSCHAR` containing 文 with a pending focus-loss message | 3 | All returned successfully |

The deadlock trials reported `queued=true, dispatched=true`. Timeout is five
seconds after sending the input, with roughly three seconds of process startup
on this machine. A timeout by itself would not prove the cause; the captured
debug-build dump resolves the blocked call chain below.

## Mechanism and stack evidence

The `windows_input_reentrancy_smoke` example creates one hidden **native**
window and installs thread-local Win32 message hooks only in its own process.
It does not create a WebView, read the user's database, load a project, contact
services, alter the system input method, or inject input into another app.

A worker sends a key message. Before Tao receives it, `WH_CALLWNDPROC` asks a
second worker to send `WM_KILLFOCUS`. The hook observes `QS_SENDMESSAGE` without
dispatching it, then returns to Tao. Tao's keyboard `PeekMessageW` dispatches
the pending sent message synchronously, reentering the window callback while
the outer callback holds the input-state mutex.

The symbolized stack, shortened from outer caller to current wait, is:

```text
tao::event_loop::public_window_callback_inner
  KEY_EVENT_BUILDERS.lock()          [outer callback owns the lock]
  tao::keyboard::KeyEventBuilder::process_message
    windows::PeekMessageW
      user32!PeekMessageW
        win32u!NtUserPeekMessage
          comctl32!DefSubclassProc
            tao::event_loop::public_window_callback_inner
              Mutex<HashMap<WindowId, KeyEventBuilder>>::lock
                parking_lot::RawMutex::lock_slow
                  parking_lot_core::WaitAddress::park
                    KERNELBASE!WaitOnAddress
```

The source locations in Tao 0.35.3 are
`src/platform_impl/windows/event_loop.rs:975-980` and
`src/platform_impl/windows/keyboard.rs:116-130`. The same thread waits for a
non-reentrant lock held by its outer callback, which cannot return until the
inner callback returns. This explains the low-CPU permanent UI wait.

The original application's stripped dump has the same nested subclass /
keyboard-range `PeekMessageW` / `WaitOnAddress` pattern. It lacks matching PDB
symbols, so the reproducer's function names must not be presented as symbolized
frames from the original EXE. The correspondence strongly supports this cause
for the captured incident; it does not identify every other reported hang.

## Running the reproducer

From the repository root in PowerShell:

```powershell
$env:WISP_CATALOG_OFFLINE = '1'
cargo build --locked -p wisp-tauri --example windows_input_reentrancy_smoke

# Healthy control: serial messages; expected exit 0.
& .\target\debug\examples\windows_input_reentrancy_smoke.exe --control

# Fixed dependency: all four cases must return, expected exit 0.
& .\target\debug\examples\windows_input_reentrancy_smoke.exe

# Select the two formerly failing cases individually; now expected exit 0.
& .\target\debug\examples\windows_input_reentrancy_smoke.exe keydown
& .\target\debug\examples\windows_input_reentrancy_smoke.exe keyup

# Optional dump capture: hold only the disposable process for 15 seconds
# after it reports the verified input timeout.
& .\target\debug\examples\windows_input_reentrancy_smoke.exe keydown --hold
```

Use separate processes for repeated failing trials, because a deadlocked UI
cannot reset itself. Capture a dump **after** the `FAIL` line containing
`queued=true, dispatched=true`; a fixed startup delay can capture asset/context
initialization instead of the hang. The timeout worker exits the disposable
process without requiring the blocked UI event loop to handle an exit request.
There is also a 45-second process deadline.

`char` and `syschar` select the other scenarios. Their successful completion
does not prove that a real Chinese IME session is covered or that the focus
message was dispatched while an input lock was held. Testing actual IME
composition remains part of the manual release checks.

## Fix and evidence boundary

The fresh, WebView-free reproducer shows that accumulated JavaScript errors,
renderer memory pressure and five hours of uptime are **not prerequisites for
this particular deadlock**. It does not explain the original script-error
burst, establish natural user-interaction reproduction frequency, or rule out
other causes of long-running hangs.

Upstream has a targeted fix in
[tao#1215](https://github.com/tauri-apps/tao/pull/1215), commit
`c704261c519c58cfdd0bc2d58ba24e06a0b71c92`. It moves keyboard/IME peeks before
input-state locks. Tao 0.37.0 contains this fix. Published
`tauri-runtime-wry 2.12.0` (Tauri 2.12.0, Wry 0.57.0) requires Tao `^0.37.0`,
so Wisp no longer carries a runtime-wry override. Tao itself is still selected
from `vendor/tao` for the separate macOS redraw fix; see
[`vendor/tao/WISP-PATCH.md`](../vendor/tao/WISP-PATCH.md). Before Tauri 2.12,
a patched copy of `tauri-runtime-wry 2.11.4` raised its Tao requirement from
`^0.35.0` to `0.37.0`.

The `Windows native input reentrancy` CI job builds the full desktop binary
and the regression, then runs five fresh reentrant processes and five serial
controls (40 message cases). Any timeout or incomplete focus dispatch fails
the job. A timeout worker can terminate the disposable process even when its
UI event loop is deadlocked.

Real renderer recovery and child-WebView isolation checks are also available:

```powershell
cargo run --locked -p wisp-tauri --example webview_recovery_smoke
cargo run --locked -p wisp-tauri --example mcp_app_isolation_smoke
```

Manual checks still include Chinese IME composition/candidate confirmation,
English keyboard shortcuts, focus changes between windows, custom titlebar
drag/Snap, tray hide/restore, the transparent always-on-top pet, and extended
interactive/idle use. The native test cannot substitute for these checks or
prove that every long-running freeze is fixed. The macOS sheet/redraw issue
[#1250](https://github.com/xuzhougeng/wisp-science/issues/1250) remains separate;
see the [macOS handoff](macos-modal-redraw-handoff.md).

Local evidence is retained in `test-results/hang-20260916-2033/`: original
`wisp-48648.dmp`, the symbolized reproduction in `repro-keydown-hung.dmp` and
`repro-keydown-hung-stacks.txt`, and per-trial logs/JSON. Raw dumps and application
logs are not published to GitHub.
