# Session Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add right-click export for the active session as a zip file containing transcript, messages, tool calls, artifacts, and provenance.

**Architecture:** Add one Tauri command for export generation and one frontend context-menu action that calls it with the active session and UI-detected artifact paths. Reuse existing store methods for messages, artifact records, and provenance.

**Tech Stack:** Rust/Tauri, Leptos/WASM, SQLite store, zip crate, Playwright.

---

- [ ] Add a failing Playwright test for the right-click export entry.
- [ ] Add `zip` to `src-tauri/Cargo.toml`.
- [ ] Implement `export_session` in `src-tauri/src/lib.rs`.
- [ ] Register the Tauri command.
- [ ] Add `ctx.export_session` i18n strings and append the action to chat-page context menus.
- [ ] Handle `exportSession` in `ui/src/main.rs` by passing `sessionId` and current file artifact paths.
- [ ] Update the Tauri mock to return a path for `export_session`.
- [ ] Run focused UI and Rust checks.
