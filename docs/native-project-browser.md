# Project-browser service

The macOS SwiftUI and Windows WinUI desktops are retired. Their implementation
walkthroughs remain in Git history. New desktop work belongs in
[the GPUI frontend](gpui-desktop.md); iOS and Android remain supported.

`crates/wisp-app/src/projects.rs` owns shared project queries. `wisp-service`
exposes the `wisp.project-browser.v1` JSONL protocol over stdin/stdout, with
DTOs in `crates/wisp-dto/src/project_browser.rs` and fixtures under `contracts/`.

Start the service with `wisp-service --database <existing-wisp.sqlite>`.
It opens an existing database read-only and reports persisted activity only.
`--allow-project-writes` explicitly enables project starring; normal reads do
not migrate or create a database. Commands include listing projects/sessions,
paginated transcripts, project chat search, match location, and capabilities.

GPUI's live conversations use the embedded `wisp-server` host instead of this
read-only service. Shared DTOs and fixtures remain because server, mobile and
WebView clients still consume them.
