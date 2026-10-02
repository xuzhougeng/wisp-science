# Project-owned persistence, phase 1

The project database is the durable owner of project records. The application
database owns machine configuration and a registry of project locations. This
phase does not implement cloud-drive watching, concurrent-device editing, or
automatic synchronization of live SQLite files.

## Storage contract

- Existing workspaces keep their paths and files. Their database lives at
  `.wisp/project.sqlite`; a small versioned `.wisp/project.json` identifies it.
- Export packages keep the existing manifest/metadata/workspace format. Opening
  a registered project routes reads and writes to its project database, including
  background turns, artifact capture, research records and runs.
- Global configuration, credentials, execution contexts and device sync cursors
  remain local. Cross-project listings/search aggregate project stores; indexes
  and registration must not be the only copy of conversations.
- Missing or inaccessible project databases produce an error. Never silently
  create an empty replacement, restore an older export, or fall back to a stale
  global copy.
- Removing a project registration preserves its database. Reopening its folder
  on a new profile can reconstruct registration without the original global DB.

## Compatibility and migration

Preserve the existing Store API through explicit routing at the persistence
boundary. Project-scoped operations resolve the project directly; entity-scoped
operations resolve ownership; cross-project operations aggregate results with
their original ordering/limit semantics. Transactions remain within a project.
Cross-project copy/move needs an explicit transfer boundary.

The implementation uses a location registry and cached project pools. Project-ID
methods select the pool directly; entity-only methods cache ownership after
looking it up in registered databases. Aggregate queries combine project results
and preserve their ranking/limits. Native read-only clients use the same routing
without migrating schemas. The existing CLI-only `.wisp/wisp.sqlite` mode remains
compatible; pointing a client at an application database with a location registry
also enables routing.

A cross-project move commits the destination transcript before deleting the
source. An interruption between commits may leave both copies, but cannot delete
the only transcript. Automatic deduplication of interrupted moves is a follow-up.

Legacy migration must preserve records without applying portable-export filters
to the original project. Build and verify a project database in staging before
publishing and switching registration. Retain a recovery copy until the cutover
is durable. Failed migration leaves the legacy project usable and retryable.

Migration stages its unfiltered source snapshot beside the application database,
removes other projects and global configuration there, then publishes only the
verified project copy. A durable publication marker distinguishes an interrupted
cutover from an unknown pre-existing database. The project manifest is published
after registration commits. Missing/unwritable legacy workspaces retain their
records and are retried at startup; registered missing databases fail explicitly.

Publication currently requires filesystem hard-link support. Unsupported volumes
leave the legacy records intact and defer migration. Cross-project searches and
safety checks (execution-context deletion, remote-file GC) fail explicitly when a
registered database is unavailable. Project listing, stars, recent sessions,
usage statistics and background loops (schedules, run monitoring, startup
recovery, workflow deliveries, retention) skip that project with a warning: one
unplugged or moved folder must not hide every other project or stop their
automation. The registration row keeps the offline project listed, and opening
it still fails explicitly.

## Acceptance

1. Writes after project creation are readable directly from the project DB.
2. Project A and B remain isolated, including simultaneous background writes.
3. A clean global DB can register a copied project and recover complete records.
4. Legacy migration preserves conversations, branches, artifacts, runs, research
   records and local state; failures never select a partial/stale database.
5. Global settings and credentials do not leak into project packages.
6. Existing export/import, manual sync, cross-project search and native readers
   continue to work against project-owned data.

# Phase 2: project folders inside a cloud drive

A drive client copies files, not transactions: it may upload a SQLite file
without its WAL, or deliver a half-written file. A project folder in a drive
therefore never contains a live database.

- Opt-in per project (`enable_folder_snapshots`). The live database moves to
  `<app data>/project-cache/<id>-<uuid>.sqlite`; `project_locations` points at
  it, so all phase-1 routing is unchanged. The device cursor is a
  `project_sync_state` row with transport `workspace`; the project cannot also
  use relay/shared-folder device sync.
- Publishing reuses the portable export and `portable_project_database_hash`.
  `.wisp/revisions/<uuid>.sqlite` is written first, then `<uuid>.json`
  (parents, device, size, SHA-256, state hash), each by write-and-rename. An
  unchanged state hash publishes nothing. `.wisp/project.json` becomes version 2
  and the stale folder database is removed after the first publish.
- There is no mutable head. Tips are descriptors that no other descriptor names
  as a parent. One tip equal to the cursor: publish if dirty. One tip that
  descends from the cursor: adopt it if clean (fast-forward when the state hash
  already matches), else conflict. Several tips, or no descent: conflict.
  Resolution publishes over all tips (`local`) or adopts the newest complete tip
  and closes the fork (`remote`). A tip whose snapshot is missing, short or fails
  its checksum is "waiting" and is never applied.
- Adopting a version uses `replace_project_database`, which rebinds workspace
  paths, retires other devices' in-flight runs and commits the cursor with the
  rows. Registering a version-2 folder on a new device materializes a fresh
  cache from the single verified tip.
- The desktop publishes changed projects every 20 seconds while no turn is
  running (hashing once per launch, then only after a cache write), adopts newer
  versions when a project is opened, and routes **Sync now** and the conflict
  dialog to the folder. The project card reports `saved`, `unpublished`,
  `remote-newer`, `waiting` or `conflict` from descriptors and cache file
  metadata without exporting.
- Retention: snapshots for the head and its parents, descriptors for 64
  generations, and never a file without a known descriptor.

Follow-ups: turning the mode off, cleanup of cache files left by removed
registrations, and suggesting the mode automatically for known drive folders.
