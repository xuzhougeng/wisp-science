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

Each conversation runs its own local MCP processes. When a conversation has
not used one for 15 minutes and is not open in any window, not running a turn,
and not showing a live App, Wisp stops that process to free memory; the
conversation's next request starts it again. Servers that keep state in
process memory lose it at that point. HTTP MCP connections are not affected.

## Install and enable

Open **Settings → Plugins**. The install dialog keeps local ZIPs and HTTPS
release assets as separate choices. For remote installs, the release SHA-256 is
required before installation can start. A local ZIP may be installed without
one, but is marked `unverified`. For a local install, choose the ZIP first,
review the selected path and optional checksum, then select **Install plugin**.
Choosing a file does not start installation. The dialog closes after a
successful install and stays open with the entered values when installation
fails. Removing an installed plugin always requires confirmation.

The manifest `id` is the plugin identity. Installing another valid package with
the same ID replaces the existing files and installation record, including
when the version changes. Existing per-project enabled state and permission
grants follow the replacement version. Wisp validates and stages the new
package before moving the previous files aside.

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
turns on the existing chat/workbench split. Later presentations of the same UI
resource (or the same tool name when no resource URI is present) reuse that tab
and replace its contents instead of stacking another window. Switching back to
the conversation parks the live app without reloading it; closing its tab tears
the app down. The latest presented workbench is saved with the conversation and
restored when that conversation is reopened, including after Wisp restarts.

While that live App is still bound to the MCP server that presented it, Wisp
advertises `hostCapabilities.serverTools` and forwards standard `tools/call`
requests to the same server (same-server tools only, including app-only helpers
that never enter the agent catalog). Each App call has a 30s host timeout that
fails only that request; it does not inherit the 120s transport timeout and
does not tear down a stdio MCP process. Restored Apps that no longer have the
original connection do not get that capability and keep using
`ui/update-model-context`.

MCP Apps may publish their current selection or other bounded live state through
the standard `ui/update-model-context` request. Wisp keeps only the latest update
from each open App and adds it to the next model turn in that conversation.
Closing the App clears its state. Wisp currently accepts text content blocks and
structured JSON up to 64 KiB; the App should send a compact selection or summary
rather than its entire workspace.

## Safety boundary

### Tool-result fidelity

Plugin results are not text-only. The native agent receives MCP text and
`structuredContent`, plus all supported model-visible image blocks. Images
supplement text rather than replacing selectors, plan digests, terminal
outcomes, or error flags. App-only `_meta` is not model context. Explicit
content `annotations.audience` that omit `assistant` stay out of the native
agent's projection. An image does not make a failed result successful.

The native image projection accepts base64 PNG/JPEG/GIF/WebP, at most eight
images, 5 MiB of base64-decoded source bytes per image, and 20 MiB per result.
It validates base64, detects the actual image format and decodes image content
with resource limits. New images whose width or height exceeds 2048 pixels
receive a proportional JPEG model-input copy with a resize/detail-loss notice,
even if the compressed source is below 5 MiB. The raw MCP result is unchanged.
Unsupported, malformed or omitted content
produces an explicit notice; it is never silently represented as inspected.
Resource links are reported without automatically fetching them; embedded
resource text is retained and binary blobs are not dumped into model text.
Text in mixed image results uses the same context budget and spill-to-file
path as text-only output. A configured vision model may describe images for a
text-only primary model; those observations supplement the original result
and are identified as model observations, not server facts.

The external-agent MCP bridge preserves the remote CallToolResult envelope
for both direct calls and `wisp_use_tool`, including `isError`,
`structuredContent`, media and `_meta`. The receiving host must apply its
own model-visibility rules. This does not add Apps rendering to a headless
external agent or equate model-visible images with user-visible App previews.

Regression gates: `cargo test -p wisp-mcp -p wisp-core --offline`, the
`custom_bridge_preserves_rich_results_for_direct_and_dispatch_calls` Tauri
test, and the full workspace suite. MCP stdio fixtures use a local test
executable; the bridge test uses an ephemeral loopback HTTP fixture, not a
real plugin, user Library, external API, or credential.

### Isolation and execution policy

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
- Plugin MCP processes start from a cleared environment. Wisp copies only a
  small non-secret allowlist: `PATH`, temp/locale/runtime plumbing, Windows
  user-profile directories (`USERPROFILE`, `APPDATA`, `LOCALAPPDATA`,
  `HOMEDRIVE`, `HOMEPATH`), and XDG directories when the host already has
  them. Token variables such as `GH_TOKEN` and `GITHUB_TOKEN` are not copied.
- Third-party MCP tool names may not replace an existing Wisp tool.
- MCP Apps receive structured tool input/results in a script-only, opaque-origin
  iframe. Network origins are restricted to the resource's declared CSP. A live
  App may call tools on the same MCP server through the host (`tools/call`);
  those calls use the existing approval policy, are keyed by connector + tool
  (bundled `dev-mcp` and `mcp_bio` use those stable ids; an empty id is not
  collapsed to `_`), time out after 30s without tearing down the MCP process,
  and cannot reach another server. Arguments are checked against a JSON Schema
  subset (`type`, `required`, `properties`, `additionalProperties: false`,
  `items`, `enum`, numeric bounds, `pattern`). Combinators such as `oneOf` /
  `anyOf` / `allOf` are not evaluated. Apps may also update the next model
  turn's bounded text/JSON context. Wisp does not grant external links,
  downloads, forms, camera, microphone, or geolocation.
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

When the live Motif workbench is open, its host toolbar also provides **Load
DNA file**. The browser picker can select a SnapGene `.dna`, FASTA, GenBank,
raw-sequence, or Motif JSON file from anywhere the user can access; the file does not need
to be copied into the Wisp project first. Wisp reads only the explicitly
selected file and sends its bounded text content through the existing
`motif_open_workbench` MCP connection. SnapGene packets are parsed locally;
the DNA sequence, name, topology, and modern SnapGene feature annotations
(names, types, ranges, direction, colors, segments, and qualifiers) are sent to
Motif. Features therefore remain visible on the sequence and plasmid map instead
of being reduced to an unannotated sequence. Malformed or other
unknown binary files fail instead of being interpreted as protein. Binary AB1/ABI traces continue to use
Motif's own **Add Entry -> Choose files** importer.

Supported sequence files in the project Files pane also expose **Add to
Motif**. With a live Motif workbench in the current conversation, Wisp parses
the file through `motif_open_workbench` and appends the returned records via
Motif's workspace API, preserving the existing inventory. Without a live
workbench, Wisp attaches the file and prepares an instruction to open Motif and
add it, rather than silently dropping the action.

The Motif host toolbar provides **Add selection to chat**. Wisp asks the
sandboxed workbench for the browser's highlighted sequence text, verifies that
it is an exact substring of Motif's active record, and calculates one-based
coordinates. The composer receives both a visible reference card and a
structured text block containing record identity, coordinates, strand,
molecule type, and exact sequence. Highlighted UI text that does not match the
active record fails closed; Wisp never guesses sequence coordinates.

Clicking an annotated feature on Motif's plasmid map also scrolls the sequence
pane to that feature. **Add selection to chat** resolves the selected feature
by its annotation ID before considering browser text selection, and includes
the feature name, full coordinates, strand, and exact genomic sequence. This
prevents a stale two-base browser selection from replacing a map feature.

The Motif selection bar shows the selected sequence length in base pairs as
soon as a range or annotated feature is selected. The same deterministic
length is included in the composer reference card and structured chat context;
it is calculated locally from the selected sequence and does not require a
model call.

Run this acceptance test natively on Windows as well. Wisp keeps canonical
containment checks but passes ordinary drive-letter paths to Node MCP entrypoints;
Windows verbatim (`\\?\`) paths are not valid Node entry-script arguments.
