# Generated files in assistant replies

Each assistant reply with generated artifacts has a collapsed **Generated · N**
entry (**已生成 · N** in Chinese). Open it to browse a directory hierarchy built
from the files associated with that reply, using the same workspace-location
rules as the right-side artifact panel. This does not list unrelated project files.

Folders start collapsed and show their total descendant artifact count. Root
files and inline outputs appear directly in the list. Open nested folders to
find files with identical names in different directories. Clicking a file keeps
the existing artifact preview behavior, including its Open in center action;
superseded outputs remain disabled and labelled. The expanded list scrolls
within a bounded height.

Use Tab to focus a disclosure and Enter or Space to expand/collapse it. These
are inline disclosures, not overlays. Closing and reopening the Generated
section preserves folder expansion while the reply remains mounted. Artifact
updates also preserve those choices, including when a later reply regenerates
one of the files and changes this reply's Generated count.

Manual smoke: generate many outputs across nested directories, open Generated,
check counts, expand individual folders, preview a file, and use Open in center.
Check a narrow window, keyboard toggling, root files and duplicate basenames.

An open artifact preview stays in place while background tools produce or update
other outputs. Its zoom, provenance tab, and unsent code edits are preserved;
image navigation updates as new images become available. Selecting another image
loads that image and its provenance.

Session artifact move/delete previews resolve absolute references through the
registered workspace root, including macOS `/var` and `/tmp` aliases. Files are
validated under the physical root while preserving their relative suffix;
symlinks inside the workspace, external paths and project-state files remain
ineligible. This also keeps shared snapshot ownership and crash recovery tied
to the exact physical workspace.

Manual smoke: open a figure while analysis continues, zoom in, switch provenance
tabs or edit the recorded code, and let tools save more outputs. Check that the
preview does not flash or reset, new images are reachable with the navigation
buttons and arrow keys, and Escape closes the preview immediately after opening.
