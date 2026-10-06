# Backups taken before an upgrade

[简体中文](upgrade-backups.zh-CN.md)

Wisp's database migrations only go forward. So that an upgrade which goes
wrong can be undone, Wisp keeps a copy of each database before a different
app version migrates it.

## What is copied, and when

Each database records the app version that last migrated it. When another
version, newer or older, is about to open it, Wisp first copies it:

| Database | Copied when |
| --- | --- |
| `wisp.sqlite` (application database) | at startup |
| `library.sqlite` (global library) | at startup |
| a project's `.wisp/project.sqlite` | the first time that project is opened |

Copies are written to `backups/` in the application data directory, never into
a project folder:

- Windows: `%APPDATA%\science.wisp-science\wisp-science\backups\`
- macOS: `~/Library/Application Support/science.wisp-science/wisp-science/backups/`
- Linux: `~/.local/share/science.wisp-science/wisp-science/backups/`

A file is named `<database>.<UTC time>.pre-<version>.sqlite`, for example
`wisp.20261006-120301.pre-1.18.0.sqlite`: the state of `wisp.sqlite` just
before 1.18.0 opened it. Project copies are named `project-<project id>.…`;
the id is the `project_id` in the project's `.wisp/project.json`.

The three most recent copies of each database are kept. The folder can be
deleted at any time to free disk space. A copy that cannot be written (a full
disk, a read-only folder) is logged and does not stop Wisp from starting.

Installing an older version is not a rollback by itself: the older version
opens the database as the newer one left it. It does take its own copy first,
so the newer state is kept too.

## Restoring a copy

Restoring discards everything recorded after the copy was taken. Nothing below
deletes a file, so the step can be undone by moving the files back.

1. Quit Wisp completely.
2. In the application data directory, move `wisp.sqlite` to another folder,
   together with `wisp.sqlite-wal` and `wisp.sqlite-shm` if they exist. Leaving
   those two beside a restored database can corrupt it.
3. Copy the chosen file from `backups/` into the application data directory and
   name it `wisp.sqlite`.
4. Start the Wisp version you intend to keep using.

For a project database, do the same in the project's `.wisp` folder with
`project.sqlite` and any `project.sqlite-journal`, `-wal` or `-shm` beside it.

Projects created after the copy was taken are no longer listed once
`wisp.sqlite` is restored. Those that keep their own `.wisp/project.sqlite` are
untouched and can be added again with **Import project → Import project
folder**.

Projects kept in a cloud folder with snapshots ([project sync](project-sync.md))
keep their live database under `project-cache/` rather than in the project
folder. It is copied the same way, but putting a copy back by hand is not
supported yet; open an issue instead of replacing files there.

If sessions are missing but nothing was upgraded in between, see the
[project-database recovery steps](project-database-recovery.zh-CN.md) first;
a copy does not help when the data is intact but attached to another project.
