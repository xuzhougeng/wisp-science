# Session Export Design

**Date:** 2026-07-07
**Status:** Approved
**Scope:** Add a right-click export action for the active chat session. The export is a `.zip` containing the readable transcript, raw messages with tool calls, extracted tool-call records, artifact files including images, and provenance JSON when available.

## Goal

Users can right-click the current chat page and choose export. The app opens a native save dialog and writes a zip for the active session.

## Architecture

The frontend only owns the menu entry and the current artifact path list, because produced files are detected in the UI transcript. The backend owns zip generation, because it can read persisted messages, stored artifacts, provenance, and workspace files safely.

Transcript artifact detection normalizes common assistant shorthand such as `figure.png/.pdf` to the previewable image path (`figure.png`) before display or export path collection.

## Zip Contents

- `manifest.json`: export metadata, included files, missing artifacts.
- `transcript.md`: readable user/assistant/reasoning/tool transcript.
- `messages.json`: raw persisted messages, preserving `tool_calls`.
- `tool-calls.json`: normalized tool calls with matched tool results.
- `artifacts/`: copied artifact files, including images.
- `provenance/`: provenance JSON for artifact paths with recorded lineage.

## Error Handling

If no active session exists, the frontend does not call export. If the user cancels the save dialog, the command returns `None`. Missing artifact files do not fail the export; they are listed in `manifest.json`.

## Tests

Add a Playwright test that enters the chat, opens the custom context menu, clicks the export action, and verifies that `export_session` is invoked with the active session id.
