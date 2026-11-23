# Generated files in assistant replies

Each assistant reply with generated artifacts has a **Generated · N** entry
(**已生成 · N** in Chinese). Open it to browse a directory hierarchy built
from the files associated with that reply, using the same workspace-location
rules as the right-side artifact panel. This does not list unrelated project files.

When a reply generated four outputs or fewer, the entry starts expanded with
its folders open, and each current image file is shown as an inline thumbnail
(the whole figure, scaled to fit) that opens the preview when clicked. Longer
lists start collapsed, with images as rows in the tree. Either form can be
collapsed or expanded from the entry's summary.

In a collapsed list, folders start collapsed and show their total descendant
artifact count. Root
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

In the GPUI desktop the list holds the project files the turn recorded as
artifacts: those written with `write` or `edit` and by Python, R or shell code
the model ran. It is under the turn's last answer. Tables and equations inside
the answer, uploaded files and outputs a run harvested outside a turn are not
listed, and turns that ended before the host started keeping which answer its
artifacts belong to have no list (their files are in the Artifacts tab).
Clicking a file opens it in the Files tab; there is no Open in center yet.

The artifact preview opens an image fitted to its viewport (the zoom label reads
**Fit** / **适应**), so a tall figure is shown whole. Clicking the label switches
to 100% (full width); the zoom buttons and the wheel step from 100%; the **Fit**
button returns to the fitted view. The center and side-panel previews keep
opening at 100%.

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
