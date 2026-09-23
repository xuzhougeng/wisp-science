# Manual project sync

[中文说明](project-sync.zh-CN.md)

Wisp synchronizes projects only when you press **Sync now**. There is no
background timer, file watcher, WebSocket, or automatic merge. A sync takes an
exclusive project gate and is rejected while any conversation, ACP turn,
approval, review, persistence flush, or structured run is active. New project
tasks are also rejected until that sync finishes.

The **Sync now** and **Copy device code** actions appear on project cards only
after the selected sync backend is fully configured in Settings. Relay mode
requires both a relay URL and token; shared-folder mode requires a folder path.

Each revision contains:

- a complete, filtered project SQLite snapshot;
- an encrypted workspace manifest using `/`-separated relative paths; and
- encrypted content-addressed blobs for changed workspace files.

Project-owned storage does not change this revision protocol. Snapshots are
exported from the live `.wisp/project.sqlite`, and pulls replace that project's
rows. The workspace scanner excludes the live database, its WAL/SHM/journal files
and project identity metadata; copying those as ordinary blobs would produce
inconsistent database copies. Device sync configuration stays in the application
database. A pull also commits a pending cursor with the project rows so an
interrupted device-cursor update can recover the existing workspace journal.

Project directories are now self-contained, but a cloud-drive client cannot
copy an open SQLite database consistently. Close Wisp before copying a live
project directory, use **Sync now** / project export, or keep the project folder
in a cloud drive with the mode below.

## Project folder inside a cloud drive

When the whole project folder lives in Nutstore, Baidu Netdisk, OneDrive,
iCloud Drive or Dropbox, open **Project Settings** and press **Save safely for
cloud-drive sync**. Enable it while Wisp is closed on other devices. Wisp then:

- moves the live database out of the folder into this device's application
  data (`project-cache/`), where WAL writes never reach the drive client;
- publishes complete versions into `.wisp/revisions/`: an immutable database
  snapshot (`<revision>.sqlite`) plus a small descriptor (`<revision>.json`)
  naming its parents, size and SHA-256. Each file is written beside its target
  and renamed, so the drive only ever sees finished files;
- marks the folder with `.wisp/project.json` version 2. Older Wisp versions
  reject it instead of opening a missing database.

Workspace files are moved by the drive client itself; this mode uploads no
workspace blobs and needs no relay, sync key or device code.

**Saving.** After a conversation turn finishes, Wisp publishes within about 20
seconds. An unchanged project creates no revision. The project card shows the
state: *Saved to folder*, *Changes not yet saved to folder*, *Newer version in
folder*, *Waiting for the cloud drive* (a descriptor arrived but its snapshot
is missing or incomplete), or *Folder version conflict*. **Sync now** publishes
immediately.

**Switching devices.** On the second device, wait for the drive client, then
use **Import project → Import project folder** and select the folder. Wisp
verifies the latest snapshot's checksum, builds its own local cache and binds
workspace paths to that device. Later, opening the project adopts a newer
version published elsewhere when this device has nothing unpublished; runs
that were in flight on the other device are recorded as not resumed.

**Conflicts.** A cloud drive's file lock is not a cross-device lock, so Wisp
never trusts one. Revisions form a history through their parents and there is
no mutable head file. If both devices changed the project from the same
version, or both published concurrently (two newest revisions), Wisp overwrites
nothing and shows the conflict dialog: **Use this device** publishes this
device's records as a revision that supersedes every competing one; **Use
remote version** adopts the newest complete folder version. A snapshot whose
size or checksum does not match is treated as still downloading, never applied.

**Retention.** The newest version and its parents keep their snapshots; older
snapshots are deleted, and descriptors are kept for 64 generations so a device
that fell behind can still prove it is not diverging. Files without a
descriptor are never deleted, because they may be another device's version
still in transit. Project export and **Sync now** relay sync skip
`.wisp/revisions/`.

Limits: the mode cannot currently be turned off, and removing the project from
Wisp leaves its local cache file in the application data directory.

The metadata snapshot is small and complete on every revision. Unchanged
workspace files reuse their previous encrypted blob, so only changed files are
uploaded. The backend sees project/revision identifiers and opaque blob hashes,
not project names, conversations, file names, file contents, or the project
encryption key. Revision descriptors are authenticated with that key, and a
normal pull verifies that the remote history descends from the device's last
known revision, preventing silent descriptor tampering or rollback.

## Choose a backend

Open **Settings → General → Manual project sync**.

### Self-hosted relay

Set **Storage backend** to **Self-hosted relay server**, then enter its HTTPS
URL and bearer token. The token is stored in the operating-system keyring and
is not included in project data or device codes. Plain HTTP is accepted only
for `localhost`, for local development.

Run the bundled relay with:

```bash
export WISP_RELAY_TOKEN="replace-with-a-long-random-token"
export WISP_RELAY_ROOT="/var/lib/wisp-relay"
export WISP_RELAY_BIND="127.0.0.1:8787"
cargo run -p wisp-sync --bin wisp-relay --release
```

Put an HTTPS reverse proxy in front of the relay for access from other devices.
The relay stores files under `WISP_RELAY_ROOT`, uses atomic file replacement,
and compares the submitted base revision with the current project head before
committing. Back up this directory like any other application data.

### Baidu Netdisk, Nutstore, or another shared folder

Set **Storage backend** to **Shared cloud-drive folder** and choose a directory
managed by the provider's desktop sync client. Wisp creates a `Wisp Sync`
subdirectory containing the same encrypted revision/blob layout as the relay.
No provider API or account credential is given to Wisp.

The absolute folder is device-local. For example, Windows may use
`D:\BaiduNetdisk\Wisp` while macOS uses `/Users/me/Nutstore/Wisp`. Those paths
are never placed in the project or device code. Before pressing **Sync now**,
wait for the provider client to finish downloading; after a push, wait for it
to finish uploading before switching devices. Do not synchronize the same
project simultaneously from two devices. A relay gives stronger cross-device
compare-and-swap behavior than a cloud-synchronized folder.

## First device

1. Finish or stop every task and run in the project.
2. Press **Sync now** on its project card. The first sync creates the project
   encryption key and revision.
3. Press **Copy device code**. Treat this code like a password: it contains the
   project encryption key. The relay bearer token is deliberately not included.

## Additional device

1. Configure the relay token, or select that device's local cloud-drive folder.
2. Open **Settings → General → Manual project sync**, press
   **Join synced project**, and paste the device code.
3. Choose a new local parent directory. Wisp downloads into staging, verifies
   every encrypted blob and plaintext checksum, then imports the project under
   a device-local workspace path.

After working on one device, press **Sync now**, wait for the relay/cloud client,
then press **Sync now** on the next device. A clean device pulls; a dirty device
pushes if its base revision is still current.

## Conflicts and recovery

If both devices changed after the same revision, Wisp reports a conflict and
does not overwrite either side. The dialog offers two explicit choices:

- **Use remote version** replaces the synchronized metadata and eligible local
  files with the current remote revision.
- **Use this device** publishes this device's complete state as a new revision
  whose parent is the current remote head.

Export the local project ZIP first if you want a safety copy before choosing.
Revision and blob files are immutable, so the previous remote revision remains
in relay storage even when a new authoritative revision is selected.

Workspace application uses a same-volume staging and backup directory. A small
recovery marker records the target revision. If the app exits during a pull,
the next manual sync either completes cleanup when SQLite committed or restores
the previous files when it did not.

## Paths and exclusions

- Workspace paths are relative and slash-separated. Windows drive letters and
  macOS absolute paths never cross devices.
- Artifact and run paths inside the workspace are rebound to the destination
  workspace. Source-local absolute paths outside the workspace become
  unavailable references instead of being guessed.
- Runtime handles, process IDs, lifecycle leases, remote work directories,
  private-key paths, API credentials, global settings, SSH/WSL configuration,
  and ACP process bindings are excluded.
- Symlinks, special files, Windows-incompatible names, and individual files
  larger than 64 MiB remain local and are reported after sync. Large scientific
  datasets should be represented as remote references/checksummed assets rather
  than copied through progress sync.
- A workspace scan accepts at most 100,000 entries, and one push accepts at most
  128 MiB of changed workspace blobs. The filtered SQLite metadata snapshot is
  limited to 192 MiB. Split or reference larger datasets instead of syncing them.
- Empty directories are not materialized unless a synchronized file needs them.
- Both devices should run the same Wisp version for protocol v1 snapshots.
- Protocol v1 does not garbage-collect old remote revisions or blobs. Deleting
  a local project removes its local sync key/cursor but does not erase relay or
  cloud-folder history; remove that backend data separately when appropriate.
- Protocol v1 does not rotate a project key or revoke an already copied device
  code. If a code is exposed, move the project to fresh sync storage and remove
  the old backend history.

Manual ZIP export/import remains available for offline transfer, archival
backup, and preserving a local copy before conflict resolution. See
[Project transfer](project-transfer.md).
