# Feature plugins

Wisp feature plugins package reusable Skills, local stdio MCP servers, and MCP
Apps as one installable unit. A plugin is installed globally and enabled per
project. Installation never starts package code; enabling the plugin makes its
Skills available and starts its MCP servers when a new agent session is built.

## Supported packages

The native format uses `.wisp-plugin/plugin.json` with schema
`wisp.plugin.v1`. Wisp also accepts the Claude plugin layout used by Motif:

```text
.claude-plugin/plugin.json
.mcp.json
skills/*/SKILL.md
server/...
```

Claude packages are normalized into the native manifest at install time.
`${CLAUDE_PLUGIN_ROOT}` and `${WISP_PLUGIN_ROOT}` are both resolved to the
immutable installed package directory. MCP processes are launched directly,
without a command shell.

## Install and enable

Open **Settings → Plugins**. The install dialog keeps local ZIPs and HTTPS
release assets as separate choices. For remote installs, the release SHA-256 is
required before installation can start. A local ZIP may be installed without
one, but is marked `unverified`. The dialog closes after a successful install
and stays open with the entered values when installation fails. Removing an
installed plugin always requires confirmation.

Review the displayed MCP command and runtime status, then enable the plugin for
the current project. **Enable & use** both enables a disabled plugin and starts
the required fresh session with a guided request; **Use in new session** does
the same for an already enabled plugin. Enabled third-party tools still require
confirmation before each call. Idle agent sessions are invalidated
automatically when plugin state changes.

Plugin-provided Skills appear in **Settings → Skills** with a “Managed by …”
badge. Their files, enabled state, and removal are owned by the parent plugin,
so they do not expose duplicate Skill controls.

When a tool presents an MCP App such as Motif, Wisp opens it as a center tab and
turns on the existing chat/workbench split. Switching back to the conversation
parks the live app without reloading it; closing its tab tears the app down.

## Safety boundary

- ZIP extraction rejects traversal, symbolic links, duplicate paths, oversized
  files, excessive file counts, and expansion beyond the configured limit.
- Install does not run `npm install`, `postinstall`, shell scripts, or any other
  package code.
- Remote downloads require HTTPS and a matching SHA-256; HTTPS redirects may
  not downgrade to HTTP.
- MCP commands are either a PATH-resolved executable or a file inside the
  installed plugin. Arguments are passed as an argv array, never through a
  shell. Child processes are terminated when their owning agent session is
  released.
- Third-party MCP tool names may not replace an existing Wisp tool.
- MCP Apps receive structured tool input/results in a script-only, opaque-origin
  iframe. Network origins are restricted to the resource's declared CSP. Wisp
  does not currently grant app-initiated tool calls, external links, downloads,
  forms, camera, microphone, or geolocation.
- Embedded `text/html` MCP resources are materialized under
  `.wisp/plugin-artifacts/` and opened through Wisp's sandboxed HTML preview.

This is a process and browser isolation boundary, not a complete operating
system sandbox: an enabled local MCP process runs with the current user's file
permissions. Only enable packages whose source and checksum you trust.

## Motif acceptance test

Build the released plugin from a pinned Motif checkout:

```bash
git clone https://github.com/jvogan/motif.git
cd motif
npm ci
npm run build:motif
```

Use the SHA-256 from
`dist-motif/motif-for-claude-science.checksums.json` when installing
`dist-motif/motif-for-claude-science.zip`. Enable it for a test project and use
**Use in new session**. That action attaches the plugin-managed Skill to the
first turn so the agent follows the plugin's startup instructions instead of
guessing MCP tools from its display name. The acceptance checks are:

1. The `motif-for-claude-science` Skill appears as plugin-managed.
2. The MCP server exposes `motif_open_workbench` and
   `motif_create_workbench_artifact`.
3. Calling `motif_open_workbench` opens `ui://motif/workbench.html` and loads
   the structured demo payload in the isolated MCP App.
4. Calling `motif_create_workbench_artifact` creates a self-contained HTML file
   under `.wisp/plugin-artifacts/`, and that file opens in Wisp's artifact
   preview.

Run this acceptance test natively on Windows as well. Wisp keeps canonical
containment checks but passes ordinary drive-letter paths to Node MCP entrypoints;
Windows verbatim (`\\?\`) paths are not valid Node entry-script arguments.
