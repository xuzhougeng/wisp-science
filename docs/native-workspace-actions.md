# Workspace actions during the GPUI migration

The SwiftUI and WinUI desktop implementations are retired. Their previous
workspace walkthroughs and QA records remain in Git history. Shared Rust
commands, DTOs, persisted data and mobile consumers remain in place.

The [GPUI migration table](gpui-desktop.md) is the current desktop coverage
record. File editing, terminal views, execution contexts, run review, artifact
previews, literature, notebook, and other research surfaces must be migrated
one use case at a time through shared Rust application services. Their presence
in a DTO or in the legacy WebView is not a claim that GPUI implements them.

The existing WebView continues to provide those surfaces during the migration.
