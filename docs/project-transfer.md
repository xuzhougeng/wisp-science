# Project transfer

Wisp exports a complete project in two formats:

- **Export directory** creates an uncompressed project package in a selected
  parent directory. Existing directories are not overwritten; a numeric suffix
  is added when the project name is already taken. Choose a location outside the
  source workspace.
- **Export ZIP** saves the same project files and database snapshot in a compressed
  archive. ZIP import extracts the workspace into a new directory.

Both contain regular workspace files, conversations, artifacts, runs, plans,
provenance and research-graph records. Their shared layout is:

```text
project-package/
  manifest.json
  metadata/project.sqlite
  workspace/
```

The metadata database contains only the exported project's records. It is a
snapshot taken at export time. The live project database is now stored in the
workspace at `.wisp/project.sqlite`, with identity metadata in `.wisp/project.json`.
New projects and successfully migrated existing projects own their conversations,
artifacts, runs and research records there. The application database retains
device configuration and project locations. Removing a project from the list
preserves its directory and database.

**Import project folder** accepts either an export package or a live Wisp project
directory with both metadata files. It validates their identity before registering
the project. A fresh application profile can recover the project without the
original application database. Stop work and close Wisp before copying a live
directory; copy its entire `.wisp` directory too. Use export for a consistent
snapshot while Wisp is running. Ordinary source folders without project metadata
still belong under **New project**.

Old projects migrate on application startup using a verified, filtered database
copy. Their local runtime state is preserved during this same-device migration.
If a legacy workspace is unavailable or unwritable, its original records remain
in the application database and migration is retried on the next startup. A
missing database for an already migrated project is an error; Wisp never creates
a blank replacement or uses stale exported metadata as a fallback.

To move a project from Windows to macOS:

1. Wait for active conversations and jobs to finish.
2. Choose **File → Export current project**, or the project card's export action.
3. Choose **Export directory** or **Export ZIP**, select a destination and wait for
   the completion notice before moving the result.
4. On the destination device, choose **Import project → Import project folder**
   and select the package root containing `manifest.json`. Wisp validates the
   manifest and metadata, restores project records, and opens `workspace/` in
   place with its Files panel visible. No second workspace copy is made.
   For ZIP, choose **Import a ZIP archive**, then select a parent directory for
   extraction and open the imported project from the project list.

Folder import rejects ordinary folders, missing or corrupt metadata, unsupported
manifest versions, mismatched project IDs, and linked metadata paths.
It never falls back to registering an empty project. Archive record insertion is
transactional, and invalid metadata never creates a project registration or
rewrites the exported snapshot. If publishing project-owned storage fails after
valid records have been imported, those records remain in the application database
for migration recovery. Importing a project ID already on the
current device is rejected; open that project from the project list instead.
To start a new project using ordinary files, use **New project**.

The package's workspace remains editable after import. Manifest file totals
record the original snapshot, so they are checked when exporting but do not
prevent re-importing an edited workspace. After its first import, the workspace
has a live project database. Re-importing that package uses the live database,
including edits made since export; the original `metadata/project.sqlite`
remains an unchanged export snapshot.

Both formats share progress reporting and the same export guards: the source
project stays read-only during export, while unrelated projects remain usable.
Exports are staged and validated before publication. Directory export copies
files directly without an intermediate ZIP. Source symlinks and special files
are skipped and listed in the manifest, just as for ZIP export.

## Recovering conversations from an orphaned workspace

Choose **Import project → Recover conversations from a workspace** only when the
original Wisp application database, exported project package, and sync revision are unavailable.
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
Exploration checkpoint may not appear in `.wisp/history` at all. Use project directory or ZIP
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

- **Remove from Wisp only** removes the device's project registration while
  keeping the project directory, identity metadata, and project database on disk.
  Import the project folder to register it again.
- **Delete project and local data** also permanently deletes the registered
  project directory. This choice opens a second warning that shows the exact
  directory; its final delete button stays disabled for five seconds. The data
  deletion cannot be undone.

Pressing Escape from the permanent-delete warning returns to the first choice
dialog. Pressing Escape again closes project removal without making changes.
