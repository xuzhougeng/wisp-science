# Remote file browser

The Files panel can switch between the current local project and registered
SSH execution contexts. In the local project, its toolbar can create an empty
file, create a folder, refresh the current directory, and sort entries by
name, size, or modification time. Sorting by size shows file sizes on the
right; sorting by time shows each entry's update time there instead. Right-click a local
file or folder to rename or delete it. These operations are constrained to the
project root, reject path separators in names, and never overwrite an existing
entry; deleting a non-empty folder requires an explicit confirmation.
Right-clicking a local folder can also add its path to chat, allowing the agent
to inspect its files and directory structure with the normal project tools.

Selecting an SSH context opens the remote user's home directory and supports:

- entering an absolute path (or `~` / `~/...`) and pressing Enter;
- moving to the parent directory;
- opening child directories;
- viewing non-hidden file names, sizes, and modification times;
- sorting the current directory by name, size, or modification time;
- uploading local files into the current remote folder with the **Upload**
  button or by dropping them onto the Files panel;
- downloading a remote file through its right-click menu and a native save
  dialog.

Remote browsing uses the existing `ssh:<alias>` `ExecutionContext` connection
snapshot and the system OpenSSH client. It honors the configured SSH alias,
user, port, identity-file path, SSH config, and agent. No private-key contents
are stored in SQLite or copied by the browser. Batch SSH/SCP always enables
OpenSSH `IdentitiesOnly` so unrelated agent keys are not offered to the server;
agent-only users with a non-default key must name its `IdentityFile` in Wisp or
SSH config.

Uploads use the same managed `scp` `file_transfer` Run as
`transfer_between_contexts`: they land in the folder currently shown in Files,
never overwrite an existing remote path, and appear in the composer transfer
tray. Creating, renaming, or deleting arbitrary remote files from Files remains
out of scope. Ledgered project files can still be removed from the Environment
panel's Remote files list, including harvest-persisted outputs that were
too large to pull back. Dropping the SSH host from Settings marks those
remote artifact references as source-discarded and blocks later download
or preview of the abandoned URIs. Downloads are explicit user actions and do not
otherwise synchronize large remote data into the project.

Remote PDF, DOCX, XLSX, and PPTX previews use the same raw-byte IPC and bounded
OOXML validation as local previews. The remote size check runs before transfer;
Office archives are then checked locally for entry count, expanded size,
compression ratio, unsafe paths, macros, ActiveX, and embedded OLE content.
Other supported rich-document formats (legacy Word/PowerPoint/Excel,
OpenDocument, RTF, and EPUB) are converted locally to Markdown with AnyDoc for
preview and agent reading. Text-based PDFs are extractable by the agent; scanned
PDFs still require OCR or a vision-capable model.

Large text, code, CSV, and log previews (local or remote) load only a bounded
head — about 1 MiB by default, and at most 8 000 rendered lines in the UI —
instead of the whole file. Remote text uses an SSH `head`/`dd` sample plus the
real file size so a multi-GB MEDLINE dump never crosses the wire just for a
preview. The UI shows a short note when content is truncated. Binary and office
previews still require a complete under-budget payload.

## Manual smoke test

1. Register or import an SSH host, **Probe** it successfully with the configured
   connection settings, and confirm its `ssh:<alias>` context appears
   in the Contexts panel.
2. Open Files on the local project and smoke-test new file, new folder, rename,
   delete, and refresh. Confirm duplicate names show an error instead of
   overwriting an entry.
3. Select the SSH host in **File location**.
4. Confirm the remote home directory loads, then open a child directory, use
   the parent button, and enter an absolute path.
5. Click **Upload**, choose a local file, and confirm a transfer Run starts
   and the file appears in the current remote folder after refresh. Dropping a
   local file onto the remote Files panel should start the same upload.
6. Right-click a remote file, choose **Download**, and confirm the selected
   file is copied to the destination chosen in the native save dialog.
7. Disconnect the host or enter an inaccessible path and confirm Files shows a
   retryable error without blocking the rest of the app.

Automated tests use a fake remote-directory runner and a mocked Tauri command;
they never require a real SSH host or network access.
