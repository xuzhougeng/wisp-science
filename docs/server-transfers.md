# Transfers between local and SSH contexts

Wisp transfers one exact file or directory from a registered, probed SSH
execution context either to another SSH context or to the local machine, and
uploads one exact local file or directory to a selected SSH context.
Agent-generated free-form `ssh`, `scp`, and `rsync -e ssh` commands remain
disabled.

## Routes

`transfer_between_contexts` accepts `route=auto|direct|relay`.

- `auto` uses a previously verified directed trust edge, otherwise relay.
- `direct` requires a verified A→B edge. It runs on A, prefers rsync when it is
  installed on both servers, and falls back to scp.
- `relay` downloads into a private temporary directory on the Wisp machine and
  then uploads with B's separately configured credentials. The temporary
  directory is removed after success, failure, or cancellation.

For SSH-to-SSH transfers, the source and destination paths must be exact
absolute or `~/` paths. Globs and filesystem/home roots are rejected. Neither
route adds `--delete`.

## Download to the local machine

Set `destination_context_id` to `local` and provide the exact new absolute
local file or directory path. If the user has not chosen that path, ask before
calling the tool. Local downloads accept `route=auto|relay` and
`transport=auto|scp`.

Wisp authenticates through the selected source context once and downloads with
scp into a private staging directory beside the destination. After a complete
download, it renames the staged item to the requested path. Existing
destinations are rejected and partial downloads are removed after failure,
cancellation, or timeout.

The composer tray shows transfer progress while work is active. Transports
that report bytes show the transferred size, percentage, speed, and ETA. The
external scp/rsync paths used for local uploads, downloads, and SSH relay do not
currently report byte counts while copying (scp disables its meter with piped
output). These phases show an animated indeterminate bar, the known total size
when available, and elapsed time refreshed every second, instead of a false 0%.
The animation indicates an active Run, not proof that bytes are moving; byte
speed and ETA are unavailable on these paths. Relay switches from Downloading
to Uploading after staging completes and shows 100% only after upload succeeds.

Use **Collapse transfers** to reduce all cards to one small **Transfers (N)**
row; **Expand transfers** restores the details and cancellation controls. The
collapsed state survives progress refreshes. Escape collapses an expanded tray
after any higher dialog or menu has been dismissed. Cards use a thinner progress
bar below their details to leave more room for the conversation. A completed,
failed, or cancelled transfer remains there for three seconds so its final
state can be confirmed, then dismisses automatically without covering the
conversation.

When checking an existing transfer, its Run card keeps the last recorded
status visible. If refreshing that status takes more than ten seconds, the
card warns that the displayed status may be outdated. A failed refresh shows
the error and retries automatically; a failure to read status does not mark
the transfer itself failed. When no Run record is available, the card shows
**Status unavailable**. Run-detail read failures also appear on the card and
retry after five seconds. Slow status and detail requests are coalesced so
repeated refresh ticks do not accumulate concurrent reads.

## Upload from the local machine

From the Files panel, select an SSH context, open the destination folder, and
use **Upload** or drop local files onto the panel. That path does not require
the agent: it submits the same `file_transfer` Run used by the tool.

The agent can still call `transfer_between_contexts`. Set `source_context_id`
to `local`, provide an exact existing absolute local path, and select an SSH
destination. Local uploads accept `route=auto|relay` and `transport=auto|scp`.
Globs, roots, missing paths, symbolic links, and special files are rejected.

Before uploading, Wisp checks through the configured SSH connection that the
exact remote destination does not exist. It then uploads with scp as a
persisted `file_transfer` Run, preserving cancellation, timeout, progress, and
audit records. The destination is ledgered when the attempt starts, so a
failed or cancelled partial stays visible and can be deleted. A successful
upload stays active on that server (it is project data, not sweep fodder).
Existing remote destinations are never silently overwritten. Restarting Wisp
retries a persisted transfer handle instead of marking the Run lost.

## User-approved trust

`configure_ssh_trust` always requires approval.

With `action=install`, Wisp:

1. Generates a dedicated Ed25519 key on A under `~/.ssh/`.
2. Reads only the public key through the managed A connection.
3. Adds that public key idempotently to B's `authorized_keys` through the
   managed B connection.
4. Verifies A→B non-interactive authentication.
5. Records only the directed edge and remote key path in settings.

The private key never leaves A and no password is written to SQLite or a
command. A→B and B→A are separate edges.

With `action=verify`, Wisp verifies and records trust the user configured
themselves; it does not generate or copy a key.

## Current limitations

- Direct rsync is resumable at rsync's file-transfer level; scp relay is not.
- SSH-to-local downloads use scp and are not resumable yet.
- Local-to-SSH uploads use scp and are not resumable yet.
- Relay temporarily needs local free space approximately equal to the source.
- scp recursive copies follow symlinks according to the installed OpenSSH
  implementation.
- Trust removal is currently manual on the two servers.
