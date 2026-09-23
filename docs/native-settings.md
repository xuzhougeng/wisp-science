# Native settings implementation

The accepted scope is the complete settings surface rendered in SwiftUI, with
existing desktop command behavior, and an equivalent transport seam for WinUI3.
The existing WebView client remains supported. Settings include all 19 top-level
sections from `SETTINGS_NAV_GROUPS`, their editors, and their real actions.

## Architecture

The project-browser sidecar retains its small query/project-star scope. Settings
connect to the existing desktop runtime through a separately enabled native
settings broker. This reuses model validation, the existing secret backend,
plugin ownership, execution-context probing, and all other desktop services.
The native UI does not edit settings directly in SQLite.

The broker is enabled only with `--native-settings-host`, binds an ephemeral IPv4
loopback port, and requires a random per-process bearer token. Its descriptor is
stored next to the desktop database, mode 0600 on Unix. Requests with browser
Origins are rejected. The client checks schema, loopback URL, request identity,
and database identity. Only enumerated settings commands are permitted.

Each project gets an isolated, invisible Tauri command context. Its inert local
document runs no settings frontend; the controls and editors are SwiftUI. The
context exists because legacy command extractors require a desktop surface.
Native settings never switch the WebView's active project or active session.

No mutation is automatically retried following a timeout. The user can refresh
to discover whether the previous operation completed.

## Settings coverage

- General: locale, workspace, notifications, resume behavior, local paths, network, update preferences.
- Session: iteration limits, compaction, continuation, follow-ups, review.
- Appearance: theme, palettes, fonts, selection popup, send shortcut, WebView stylesheet preferences.
- Pet: enablement, directory, runtime status.
- Models/API: model profiles, keys, catalog limits, capabilities, roles, active model, ACP agents.
- Quick actions: bindings, scripts/prompts, workflow references.
- Workflows: templates, nodes/dependencies, copy/save/delete, conversion and source provenance.
- Specialists: prompts, models, tools/skills, reviewer configuration.
- Memory: enablement, project/global memory, edits, clearing, automatic failure analysis.
- Skills: search, tags, enablement, files, install/remove, skill store.
- Plugins: install, grants, enablement, removal.
- Browser: connection status, extension update, tab lifecycle, URL filters.
- Connections: built-in tools, MCP/HTTP, authentication, testing, approval settings.
- Channels/sync: Feishu, Weixin, device bridge, project synchronization.
- Credentials: built-in and custom credentials through the existing secret backend.
- Permissions: grants, revocation, approval scope.
- Environments: SSH/WSL where supported, defaults, probes, interpreters, trust edges.
- Storage: usage, project retention, context preferences and disposal.
- Usage: projects, days, models, tools, session drill-down.

The tests cover broker authentication, protocol fixtures, client error handling,
project-aware draft preservation, navigation search and generated design assets.
The manual smoke steps below cover native window/menu layering and real host reads.

## Presentation and WinUI 3 handoff

The SwiftUI shell follows the WebView settings information architecture:
188–240 pt sidebar, four navigation groups, a centered content column capped at
1040 pt (920 pt for the model list), 16 pt rounded cards and shared color tokens.
Model management separates API models from ACP agents; editors replace the
content pane with breadcrumb navigation. Advanced generation and request-header
fields start collapsed. Appearance has theme tiles, palette/font controls and a
live draft preview. Memory, browser rules and remote access use two columns when
space permits and stack in narrower windows. Escape dismisses the topmost native
menu, editor or settings page, in that order.

`python3 scripts/sync_native_design.py` exports palettes, shared SVG icons,
translations, navigation labels/search aliases and API presets from the existing WebView sources. `--check` detects
drift in CI. WinUI can consume the same JSON/SVG resources; it does not need to
reconstruct the command protocol or depend on SwiftUI.

Use `INativeSettingsClient` in
`apps/windows/Wisp.ProjectBrowser.Contracts/INativeSettingsClient.cs` as the
WinUI view-model dependency. `NativeSettingsClient.ConnectAsync(databasePath,
hostExecutable)` discovers or starts the host. `contracts/native-settings/v1`
contains successful, void and failed response fixtures, plus the command catalog
with argument names and Rust result types. Generate/check the catalog with
`scripts/sync_native_settings_contract.py`. The shared Rust DTOs are authoritative;
never infer camelCase command parameters from snake_case stored DTO fields.

```csharp
using var client = await NativeSettingsClient.ConnectAsync(databasePath, hostExe);
var prefs = await client.InvokeAsync("get_appearance_prefs", new JsonObject());
// Preserve fields the current UI does not edit, including future additions.
prefs!["theme"] = "dark";
await client.InvokeAsync("set_appearance_prefs",
    new JsonObject { ["prefs"] = prefs.DeepClone() });
// Project-scoped reads/writes always pass the selected project's stable ID.
var memory = await client.InvokeAsync("get_memory_view", new JsonObject(), projectId);
```

Windows must package the full desktop host and runtime resources with WebView2.
The invisible document is an adapter for existing Tauri command extractors, not
an embedded settings UI. The WinUI preview now hosts categorized settings in the
main window, with all 19 categories connected to native editors. The C# transport and fixture tests run
without WinUI or a real backend process.

## Manual smoke procedure

1. Build with `bash scripts/build_native_macos.sh`; open the resulting preview.
2. Open Settings / Cmd+, and visit all 19 sections. Check backend errors, project
   selection and consistency with the existing WebView values.
3. In Models, open Add API access, press Escape immediately, and confirm only
   the editor closes. Open an editor's protocol menu; Escape closes the menu,
   then a second Escape closes the editor, leaving Settings open.
4. Check Appearance and Remote access at wide and narrow widths. Preview a
   theme/font draft, cancel it, and verify persisted values are unchanged.
5. With disposable settings/test credentials, save and reload a preference,
   model and project-scoped record. Verify an intentional backend validation
   error keeps the draft. Confirm another WebView project's active context does
   not change.
6. On Windows, build with `scripts/build_native_windows.ps1` and run the C#
   contract harness. Repeat launch, read/save, WSL listing and selected-project
   isolation with dedicated test services.
7. In WinUI Settings, open Workflows, then click the blank center/right area of
   the Appearance navigation button, away from its text. Verify Appearance
   opens with one click. Repeat Appearance → Workflows → General → Appearance,
   including a narrow window. Each whole row must remain clickable after it
   becomes unselected. Press Escape immediately after opening Settings to
   verify the parent window returns. This regression requires the actual
   WinUI control template; transport-only contract tests do not cover hit testing.

## Remaining differences and limits

- Appearance previews draft changes immediately but requires Save to persist;
  API access is added one model at a time. WebView's batch-add and auto-save
  interactions are not reproduced yet.
- Native settings configure the existing runtime's pet/browser integrations.
  WinUI conversations connect to the desktop host for sending and queueing;
  see [native-conversations.md](native-conversations.md).
- Update actions update the desktop host. Distribution/updating of the SwiftUI
  preview itself remains a separate packaging task.
- Long operations show busy/result states. Update/download and conversion events
  are not streamed into detailed native progress bars yet.
- Feishu/Weixin binding uses native QR rendering with an explicit status check.
  Live external API/OAuth/SSH/WSL flows require manual platform checks with the
  user's configured services; automated tests never require credentials.
- A host supports up to 32 project settings contexts during its lifetime; closing
  a settings page does not terminate the shared runtime or active operations.

## Windows incremental implementation

The WinUI preview connects all 19 settings categories through the shared
transport and packages the full settings host. It retains unknown preference
fields, preserves dirty drafts on refresh and errors, and never retries writes
automatically. Saved themes, palettes and independent UI/code typography apply
to native controls, including controls inserted after initial rendering.
Font-family and CSS editors are available; custom CSS applies only to WebView.
See [native-windows-parity.md](native-windows-parity.md) for current verification
and remaining platform/external-service acceptance, and
[native-project-browser.md](native-project-browser.md#windows-alignment-after-1281)
for build prerequisites, host/database boundaries and manual smoke steps.
