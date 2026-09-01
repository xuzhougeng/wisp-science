# Project transfer

Wisp supports three deliberately different ways to bring a project onto a device:

- **Open a folder in place** registers an existing local folder as a project. The
  folder remains where it is and Wisp does not copy its files. Use this after
  copying a workspace yourself, checking out a repository, or placing it in a
  cloud-drive folder.
- **Import a ZIP archive** restores a complete Wisp transfer into a new folder.
  The ZIP contains the workspace's regular files plus project-owned conversations,
  artifacts, runs, plans, provenance, and research-graph records.
- **Recover conversations from a workspace** scans `.wisp/history` in an
  orphaned folder whose application database and Wisp project ZIP are no longer
  available. It registers the folder in place and imports the recoverable message
  timelines into a `Recovered` conversation folder.

Opening a folder in place creates a new local Wisp project record. Conversation
history and other records that exist only in another device's Wisp database are
not recovered from a plain folder copy; use ZIP export/import when those records
must move too. Workspace conversation recovery is a best-effort disaster fallback,
not an equivalent replacement for a complete project ZIP.

To move a project from Windows to macOS:

1. Wait for the project's active conversations and jobs to finish.
2. Open the project, then choose **File → Export current project**. The project
   card's export action is also available as a shortcut. Wisp explains that you
   can copy the project folder directly when you only need its files; choose
   **Export ZIP** for the complete portable copy.
3. Copy the ZIP to the Mac and choose **Import project → Import a ZIP archive**.
4. Pick a parent folder. Wisp creates a new folder named after the project; it
   appears on the Projects screen when the import finishes.

During both operations, Wisp shows a non-modal progress card in the lower-right
corner with the current stage, files, bytes, and path being processed. You can
continue using the rest of the app. While an export is active, only its source
project is read-only and cannot be opened in another editable view; unrelated
projects remain available. An export is published as the selected ZIP only
after its completed archive has been checked against its manifest; while it is
running, do not use the temporary or still-empty archive file as a transfer
copy.

For a folder you copied yourself, choose **Import project → Open a folder in
place** instead. Confirm its local name and path; Wisp registers that exact path
without creating a duplicate workspace.

## Recovering conversations from an orphaned workspace

Choose **Import project → Recover conversations from a workspace** only when the
original Wisp application database, project ZIP, and sync revision are unavailable.
After a folder is selected, Wisp scans regular JSON files under `.wisp/history`,
accepts both plain message-array compaction snapshots and structured Exploration
checkpoints, deduplicates exact message arrays by content hash, and keeps the
greatest `message_head` for checkpoints with the same source frame. The preview
shows the recoverable conversation count, message count, valid archive count,
time range, and skipped damaged/duplicate archives before making any database
change.

Confirming the preview registers the existing folder and inserts the project,
`Recovered` folder, conversations, messages, and import provenance in one SQLite
transaction. A failed insert leaves no half-recovered project, and Wisp never
rewrites or deletes the source history archives.

The history files are snapshots rather than a complete portable project database.
Recovery can restore their stored message timelines, including tool calls and
reasoning fields, but cannot reconstruct database-only UI events, reviews,
resource bindings, runs, undo indexes, branch relationships, or other project
records. Plain message arrays do not identify their original frame, so exact
copies can be deduplicated but distinct snapshots from one conversation may be
recovered separately. Ordinary conversations that never produced a compaction or
Exploration checkpoint may not appear in `.wisp/history` at all. Use project ZIP
export/import or manual project sync for complete, planned backup and migration.

## Path rules

The ZIP never treats the source `workspace_dir` as the destination. Workspace
files and local metadata paths are stored with `/`-separated relative paths.
For example, `D:\research\study\figures\plot.png` becomes
`figures/plot.png`; importing under `/Users/me/Research/study` binds it to
`/Users/me/Research/study/figures/plot.png`.

Local absolute paths outside the project root are marked unavailable instead
of being guessed or mapped to another drive. SSH and other remote references
remain references, but the destination computer must configure its own
execution context before using them.

## Deliberately excluded machine-local state

- API keys and other keyring secrets
- global settings and model profiles
- SSH/WSL execution-context configuration
- resumable ACP process/session bindings

Imported jobs that were still recorded as active are marked `lost` and are not
resumed on the destination computer. A project keeps its stable project ID, so
importing the same archive twice on one device is rejected rather than merging
histories. Symbolic links and special filesystem entries are listed in the
archive manifest and are not followed. Export rejects workspaces with more than
100,000 filesystem entries rather than collecting an unbounded archive manifest.

For repeated device switching, use [Manual project sync](project-sync.md). It
uses the same portable project snapshot rules while transferring only changed
workspace files.

## Removing a project

The delete action on a project card offers two distinct choices:

- **Remove from Wisp only** deletes the project's Wisp metadata while keeping
  its project directory and files on disk.
- **Delete project and local data** also permanently deletes the registered
  project directory. This choice opens a second warning that shows the exact
  directory; its final delete button stays disabled for five seconds. The data
  deletion cannot be undone.

Pressing Escape from the permanent-delete warning returns to the first choice
dialog. Pressing Escape again closes project removal without making changes.
