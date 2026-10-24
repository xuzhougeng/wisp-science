# Automatic database copies

[简体中文](upgrade-backups.zh-CN.md)

Wisp's database migrations only go forward. So that an upgrade which goes
wrong can be undone, Wisp keeps a copy of each database before a different
app version migrates it. It also keeps a copy a day of each database it
opens, and a project database that SQLite reports as damaged is restored
from the newest copy automatically.

## What is copied, and when

Each database records the app version that last migrated it. When another
version, newer or older, is about to open it, Wisp first copies it. Otherwise
it copies the database when it opens it and the newest copy is older than a
day:

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
before 1.18.0 opened it. The day's copy is named
`<database>.<UTC time>.daily.sqlite`. Project copies are named
`project-<project id>.…`; the id is the `project_id` in the project's
`.wisp/project.json`.

The three most recent copies of each kind are kept per database. The folder
can be deleted at any time to free disk space. A copy that cannot be written
(a full disk, a read-only folder) is logged and does not stop Wisp from
starting. A database that fails SQLite's `quick_check` is not copied, so a
damaged database never replaces the good copies it will be restored from.

Installing an older version is not a rollback by itself: the older version
opens the database as the newer one left it. It does take its own copy first,
so the newer state is kept too.

## A damaged project database is restored by itself

When opening a project fails because SQLite reports its `.wisp/project.sqlite`
as damaged (`database disk image is malformed`, `file is not a database`),
typically after the app was force-quit or the disk lost power, Wisp:

1. renames the damaged file to `project.sqlite.corrupt-<UTC time>` in the
   same `.wisp` folder, together with any `-wal`, `-shm` or `-journal` file
   beside it, so nothing is lost for a manual salvage;
2. copies the newest copy from `backups/` that opens into its place, or, when
   no copy exists, creates an empty database that still lists the project;
3. shows which of the two happened and where the damaged file is, and opens
   the project on the next attempt.

Conversations recorded after the restored copy was taken are not in the
database. Workspace files, `.wisp/artifacts/` and git history are never
touched. The damaged file is kept until you delete it.

## Restoring a copy by hand

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
