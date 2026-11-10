# DOCX tracked changes

Wisp can propose text changes in a Word document and save them as native Word
tracked changes. The original file stays intact. Open the new `.docx` in Wisp's
WebView / SwiftUI preview or Word to review the revisions attributed to **Wisp**. Word is not required
on the machine generating the revisions; both tools use local Rust ZIP/XML
processing and are available through the shared CLI/desktop tool registry.

## Workflow

Ask the agent to read the document with `read_docx` and propose changes with
`suggest_docx_edit`. For example:

> Read uploads/manuscript.docx, propose a more cautious wording for the first
> conclusion, and save a Word revision copy as manuscript-review.docx.
> Keep the source intact.

`read_docx` takes `path`, optional zero-based `offset` (default 0), and `limit`
(default 50, maximum 100). Its JSON result includes `source_sha256`, numbered
`paragraph_id` values, paragraph text, editability/reason, and `next_offset`.
Paragraph IDs are valid only for the accompanying source hash. The read view
includes pending insertions and omits deleted text; paragraphs containing
revisions are read-only in this prototype.

Pass the returned hash and ID without inventing or reusing them from a
different version:

```json
{
  "path": "uploads/manuscript.docx",
  "output_path": "manuscript-review.docx",
  "source_sha256": "<hash returned by read_docx>",
  "paragraph_id": "p12",
  "old": "These results prove the hypothesis.",
  "new": "These results support the hypothesis."
}
```

The output must be a new `.docx` inside the project with an existing parent
directory. Neither an existing output nor the source is overwritten. An empty
`new` deletes the selected text. To insert text, replace a unique existing
phrase with that phrase plus the insertion. Common prefixes/suffixes remain
unchanged, so this creates an insertion rather than marking the entire phrase
as rewritten. `old` must match exactly once within the selected paragraph.

The result reports the output path/hash and revision IDs for this proposal.
A rewrite returns a deletion and insertion together. Suggestions are stored
in the DOCX, not in a new Muse file or a suggestions database. The output is
registered through the existing artifact/provenance flow. Binary DOCX files
do not participate in the text-file turn undo mechanism; the unchanged source
is the recovery copy.

To propose an edit in another paragraph, reread the output and write another
new output. A paragraph already containing pending changes must first be
reviewed in Wisp or Word. This version does not implement automatic upload
conversion, a persistent edit-group log, or editing an existing pending suggestion.

The agent is instructed to use these two tools whenever it is asked to reword an
existing `.docx` (polish, rewrite, shorten, translate, delete), instead of
regenerating the file from a script, overwriting it, or answering with
before/after text in chat. When the affected paragraphs are not editable, such
as table cells, it reports that and asks before changing them another way. This
is prompt guidance, not enforcement: a direct overwrite through `shell` or
`python` still bypasses review.

## Review in Wisp

Open a local `.docx` from **Files**, either in the preview or **Open in center**.
Both WebView and SwiftUI previews display native deletion/insertion markup and a review list:

- **Accept** / **Reject** resolves one change, including both halves of a rewrite.
- **Accept all deletions** resolves only standalone deletions, leaving rewrites
  and insertions pending.
- **Accept all rewrites** resolves paired deletions and insertions, leaving
  standalone deletions and insertions pending.
- **Refresh revisions** reloads the document after another editor changes it.

Review controls follow Wisp's UI font size setting.

In the WebView preview, drag the divider between the revision list and the
document (or focus it and press the Up/Down arrow keys) to give either side
more room. A document with no pending revisions shows only a one-line bar with
**Refresh revisions**, so its content is not pushed down.

Each successful choice saves the currently opened local file and refreshes the
list and preview. Before saving, Wisp keeps an exact copy of that version at
`<project>/.wisp/docx-review/<source-sha256>.docx`. A suggestion's original source
is still separate; review the generated copy when you want to preserve it.
These backups can be copied back manually and are not the text-file turn undo.

The list's IDs are bound to its SHA-256 snapshot. Changed files or a conflicting
Wisp editor cause the operation to fail; refresh before continuing. Saves use a
temporary file in the same directory followed by an atomic replacement, with a
second source check before replacement. External programs do not share Wisp's
edit lock, so avoid editing the same document simultaneously in Word.

Adjacent supported deletion/insertion elements in one paragraph, with the same
nonempty author and timestamp, are shown as one rewrite. Word does not store a
universal replacement-group ID: unpaired elements stay separate. Bulk actions
apply to the types shown in the list, across authors, and never include complex
revisions that Wisp cannot safely resolve. Those remain in the file for Word.

Review is limited to flat text revisions in ordinary top-level body paragraphs.
Tables, nested revisions, moves, property/formatting changes and protected spans
are not resolved. Revisions in headers, footers and other package parts are not
listed or modified. Up to 1,000 reviewable changes / 1 MiB of changed text can be
listed. Remote files, artifact references, historical snapshots and read-only
workspaces support preview only; open the current local file in a writable
project to resolve its revisions. In SwiftUI, open the document from the local
**Files** location; artifact previews remain read-only even when their backing
file is local. Native preview bytes are limited to 32 MiB.

SwiftUI routes reads and choices through
`native_conversation_panel_file_docx_review` / `native_conversation_panel_file_docx_resolve`
(or `native_project_file_docx_*` for project-scoped clients). Each request carries
an explicit project, optional session and file path. Responses carry the review
snapshot and render bytes read together, so accepting a change updates both the
list and the mounted WebKit document. Rust and Swift check a shared fixture at
`contracts/native-files/v1/docx-review.json`. A lost save response also requires
refreshing; the client does not retry mutations automatically.

## Example: review three changes

The checked-in [DOCX review sample](../ui-tests/fixtures/docx-review.docx) contains
three changes attributed to **Wisp**. The same file is used by the browser,
Rust contract and SwiftUI tests. Its
[generator](../ui-tests/fixtures/generate_docx_review.py) also maintains the
shared native response fixture.

| Type shown in Wisp | Original text | Proposed text |
| --- | --- | --- |
| Rewrite | `Old conclusion` | `Revised conclusion` |
| Deletion | `Remove this sentence` | Removed |
| Insertion | No text | `New supporting sentence` |

Copy the sample into a writable Wisp project's local files before trying it;
review actions save the opened file. In either WebView or SwiftUI:

1. Open the copy from **Files**. The preview shows the tracked text and three
   review entries; the rewrite's deletion and insertion share one entry.
2. Choose **Accept all deletions**. `Remove this sentence` disappears, and the
   rewrite and insertion remain pending (two entries).
3. Choose **Accept all rewrites**. `Revised conclusion` becomes ordinary text,
   and only the insertion remains pending (one entry).
4. **Reject** the insertion. `New supporting sentence` disappears, with no
   pending entries. Reopen the file to confirm the saved result.
5. Start with a fresh copy to try individual **Accept** / **Reject** on each
   type. Rejecting the rewrite restores `Old conclusion` in one action.

Each save preserves its pre-review version in the project's `.wisp/docx-review/`
directory. The checked-in sample stays reusable when you work on a copy.

## Supported scope and preservation

- Transitional `.docx` packages with the standard `word/document.xml` main
  part; legacy `.doc`, Strict OOXML, ODT and RTF are outside this prototype.
- Ordinary top-level body paragraphs containing plain text runs, including
  text split across multiple runs and mixed character formatting.
- Deleted fragments retain their original run properties. Inserted text
  inherits the first affected run's properties. This is a defined insertion
  policy, not an attempt to infer formatting for newly written words.
- Text outside the affected runs and all other ZIP entries are preserved.
  Images, styles, headers and footers are not rebuilt. New text may reflow
  pages when Word lays out the document.
- Paragraphs containing links, existing revisions, formulas, fields, drawings,
  footnote references, section breaks or other complex content are read-only.
  Table/text-box paragraphs are read-only. Cross-paragraph bookmarks, fields,
  comment ranges and protected ranges also block edits within their span.
- The source hash is checked before editing; source bytes are checked again
  before saving. A conflict requires a fresh read. Existing unrelated Word
  revisions are preserved; new main-document annotation IDs avoid collisions.
- Sources/outputs are capped at 50 MiB, expanded packages at 128 MiB, individual
  parsed XML parts at 8 MiB, and each old/new string at 64 KiB. No external
  relationships are fetched. DTDs, unsafe archive paths, macro-bearing packages
  and digitally signed packages are rejected.

## Validation

Offline test code covers original/final text projections, Unicode, edits across
formatted runs, untouched package parts, insertion/deletion, stale versions,
ambiguous matches, protected content, existing revisions, pagination, source
preservation, no-overwrite outputs, path restrictions, plan mode and cancellation.
Tests create their own small packages and do not require Word or a network.
Review coverage includes whole-rewrite acceptance/rejection, distinct deletion
and rewrite batches, namespace/format preservation, unsupported changes, stale
snapshots, atomic saving with backups, and read-only controls. Browser tests use
a mocked Tauri bridge plus a real DOCX fixture rendered by the bundled preview.
SwiftUI tests cover owner validation, single and batch actions, read-only files,
lost replies, closed previews, and actual WebKit rendering / native buttons.

After explicit permission, the focused command is:

```bash
cargo test -p wisp-tools docx::tests
WISP_CATALOG_OFFLINE=1 cargo test -p wisp-tauri docx_review::tests
cd ui-tests && npx playwright test tests/docx-review.spec.ts
```

For the native DOCX tests on macOS (after permission), run from the repository root:

```bash
WISP_NATIVE_DOCUMENT_SMOKE=1 swift test --package-path apps/macos \
  --scratch-path target/native-macos/swift --disable-sandbox \
  --filter NativeDocxReviewTests
```

The opt-in smoke test opens a disposable native window and serves only local
renderer assets on loopback. It needs no Word, model API key or external host.

An offline driver exercises the registered tools without an API key. After
permission, use a fresh output path (it will not overwrite an existing file):

```bash
cargo run -p wisp-tools --example docx_revision -- \
  ui-tests/fixtures/office-preview.docx /tmp/wisp-review.docx \
  p2 'Word local fixture' 'Word reviewed fixture'
```

On Windows, use an output such as `C:\Temp\wisp-review.docx` in an existing
directory. The source is read-only; the driver's output directory is its write
scope. The result prints the new file's hash and native revision IDs.

Manual compatibility check after permission: use a disposable DOCX containing
two formatted body paragraphs and an image. Propose a replacement spanning two
runs, open the output in Word, and verify the author and insertion/deletion
markers. Reject all Wisp revisions and check the original wording; reopen the
revision copy and accept them to check the proposed wording. Confirm the other
paragraph, image and character styles remain intact, and that the original
file's checksum has not changed. Real Word rendering/round-trip compatibility
is a separate check from XML test coverage.

For a WebView smoke check, open the generated review copy from Files. Reject a
rewrite, then verify the old wording and the unchanged source. Reopen a fresh
copy with a deletion, rewrite and insertion; accept all deletions, confirm the
rewrite remains, then accept all rewrites and resolve the insertion individually.
Verify a backup exists and reopen the saved DOCX. Change a previewed file from
another editor and verify that accepting its old snapshot fails until refreshed.
