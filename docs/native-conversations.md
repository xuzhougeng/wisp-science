# Shared conversation protocol

SwiftUI/WinUI desktop views have been retired; [GPUI](gpui-desktop.md) is the
replacement desktop implementation. Previous platform walkthroughs remain in
Git history. This does not retire the shared Rust conversation boundary.

`crates/wisp-dto/src/native_conversations.rs` defines
`wisp.native-conversations.v1`. The server, mobile clients and Tauri host use
its snapshots, send requests, approval requests, and transcript items.
Snapshots replace the current view; sequence orders replies within an epoch.
Clients must also verify project/session identity and reject replies from an
obsolete selection. Pagination uses `before_seq` / `next_before_seq`.

A send uses a unique request ID and clears the draft only after durable
acceptance. Completion arrives through snapshots. Stop requests cancel a turn;
approvals apply only to the exact pending ID and supported scope. The embedded
GPUI host supports one-time approval and denial, with cancellation and runtime
cleanup on exit. Host-specific capabilities must be checked before presenting
additional actions; the server does not implement every Tauri command.

Fixtures in `contracts/native-conversations/` are shared protocol coverage,
not evidence of GPUI feature parity. See the migration coverage table for
implemented and pending UI surfaces.
