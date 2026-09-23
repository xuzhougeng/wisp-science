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
leave the legacy records intact and defer migration. Cross-project aggregate
queries fail explicitly when a registered database is unavailable; partial-result
presentation for offline projects is a follow-up.

## Acceptance

1. Writes after project creation are readable directly from the project DB.
2. Project A and B remain isolated, including simultaneous background writes.
3. A clean global DB can register a copied project and recover complete records.
4. Legacy migration preserves conversations, branches, artifacts, runs, research
   records and local state; failures never select a partial/stale database.
5. Global settings and credentials do not leak into project packages.
6. Existing export/import, manual sync, cross-project search and native readers
   continue to work against project-owned data.
