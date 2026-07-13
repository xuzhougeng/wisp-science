# Project transfer

The Projects screen exports a project as `wisp-project-<name>.zip`. The archive
contains the workspace's regular files plus project-owned conversations, artifacts,
runs, plans, provenance, and research-graph records.

To move a project from Windows to macOS:

1. Wait for the project's active conversations and jobs to finish.
2. Open the project, then choose **File → Export current project**. The project
   card's export action is also available as a shortcut.
3. Copy the ZIP to the Mac and choose **Import project**.
4. Pick a parent folder. Wisp creates a new folder named after the project and
   opens the imported project.

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
archive manifest and are not followed.

For repeated device switching, use [Manual project sync](project-sync.md). It
uses the same portable project snapshot rules while transferring only changed
workspace files.
