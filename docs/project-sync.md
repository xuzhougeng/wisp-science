# Manual project sync

Wisp synchronizes projects only when you press **Sync now**. There is no
background timer, file watcher, WebSocket, or automatic merge. A sync takes an
exclusive project gate and is rejected while any conversation, ACP turn,
approval, review, persistence flush, or structured run is active. New project
tasks are also rejected until that sync finishes.

Each revision contains:

- a complete, filtered project SQLite snapshot;
- an encrypted workspace manifest using `/`-separated relative paths; and
- encrypted content-addressed blobs for changed workspace files.

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
2. Press **Join synced project** and paste the device code.
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
