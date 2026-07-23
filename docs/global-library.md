# Global Library

The Library keeps immutable copies of code cells and image artifacts across all
projects in the local Wisp installation.

- Click the SVG star beside a Notebook cell to save its language and source.
- Click the SVG star in an image artifact header to save the image bytes and,
  when provenance exists, the code that generated it.
- In an image artifact viewer, open **Code** and use **Copy code** to copy the
  exact recorded source. Provenance code remains read-only.
- Open **Library** from the Projects screen or a project sidebar. Items can be
  searched, filtered, removed, or traced back to their source project/session.

Library data lives in `library.sqlite` beside the main `wisp.sqlite` app
database. Project and session IDs/names are stored as source snapshots without
cross-database foreign keys. Deleting the source project, session, or workspace
therefore does not delete or alter a saved Library item; the source link may no
longer open, but the saved code/image remains available.

The first version accepts code up to 2 MiB and PNG, JPEG, GIF, WebP, SVG, or BMP
images up to 32 MiB. PDFs and arbitrary workspace files are not Library items.
