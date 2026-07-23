# Transfers between SSH contexts

Wisp transfers one exact file or directory between two registered, probed SSH
execution contexts. Agent-generated free-form `ssh`, `scp`, and
`rsync -e ssh` commands remain disabled.

## Routes

`transfer_between_contexts` accepts `route=auto|direct|relay`.

- `auto` uses a previously verified directed trust edge, otherwise relay.
- `direct` requires a verified A→B edge. It runs on A, prefers rsync when it is
  installed on both servers, and falls back to scp.
- `relay` downloads into a private temporary directory on the Wisp machine and
  then uploads with B's separately configured credentials. The temporary
  directory is removed after success, failure, or cancellation.

The source and destination paths must be exact absolute or `~/` paths. Globs
and filesystem/home roots are rejected. Neither route adds `--delete`.

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
- Relay temporarily needs local free space approximately equal to the source.
- scp recursive copies follow symlinks according to the installed OpenSSH
  implementation.
- Trust removal is currently manual on the two servers.
