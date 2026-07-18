# Remote file browser

The Files panel can switch between the current local project and registered
SSH execution contexts. In the local project, its toolbar can create an empty
file, create a folder, and refresh the current directory. Right-click a local
file or folder to rename or delete it. These operations are constrained to the
project root, reject path separators in names, and never overwrite an existing
entry; deleting a non-empty folder requires an explicit confirmation.

Selecting an SSH context opens the remote user's home directory and supports:

- entering an absolute path (or `~` / `~/...`) and pressing Enter;
- moving to the parent directory;
- opening child directories;
- viewing non-hidden file names and sizes;
- downloading a remote file through its right-click menu and a native save
  dialog.

Remote browsing uses the existing `ssh:<alias>` `ExecutionContext` connection
snapshot and the system OpenSSH client. It honors the configured SSH alias,
user, port, identity-file path, SSH config, and agent. No private-key contents
are stored in SQLite or copied by the browser. Batch SSH/SCP always enables
OpenSSH `IdentitiesOnly` so unrelated agent keys are not offered to the server;
agent-only users with a non-default key must name its `IdentityFile` in Wisp or
SSH config.

Remote mutation remains intentionally out of scope: SSH files can be previewed
or downloaded, but cannot be created, renamed, or deleted from Files. Downloads
are explicit user actions and do not otherwise synchronize large remote data
into the project.

## Manual smoke test

1. Register or import an SSH host and confirm its `ssh:<alias>` context appears
   in the Contexts panel.
2. Open Files on the local project and smoke-test new file, new folder, rename,
   delete, and refresh. Confirm duplicate names show an error instead of
   overwriting an entry.
3. Select the SSH host in **File location**.
4. Confirm the remote home directory loads, then open a child directory, use
   the parent button, and enter an absolute path.
5. Right-click a remote file, choose **Download**, and confirm the selected
   file is copied to the destination chosen in the native save dialog.
6. Disconnect the host or enter an inaccessible path and confirm Files shows a
   retryable error without blocking the rest of the app.

Automated tests use a fake remote-directory runner and a mocked Tauri command;
they never require a real SSH host or network access.
