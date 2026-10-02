# Research notebook archives / 研究阶段归档

Deleting an ordinary conversation removes its stored conversation records; it
does not remove generated files from the project workspace. Use the archive
review below to preserve a completed research stage and explicitly clean up its
recorded local intermediate files.

To keep a temporary conversation out of everyday navigation, right-click its
sidebar row and choose **Shelve conversation**. Shelved conversations leave the
main list, recent sessions, default search and `#` suggestions. Use **Shelved
conversations** beside the sidebar's conversation heading to search, open or
**Restore to main list**. Existing references remain usable; messages, files,
pins and folders are retained. This display preference persists across restarts
and is independent of research archiving: restoring a sealed notebook keeps it
read-only.

Use **Archive research** in the conversation toolbar, or `/archive`, after a
research stage is complete. Wisp prepares an editable draft using the saved
notebook and recorded operations. This does not run analysis code or remove files.

The review brings together the research question, findings and limitations,
parameter comparisons and selection rationale, assembled scripts, and local
materials. Scripts are records of operations; archiving does not certify a rerun
or scientific correctness. Missing steps must remain explicit.

For each listed file, choose **Keep snapshot**, **Keep in place**, or **Permanently
delete**. Snapshots preserve the current reviewed bytes. References retain the
original file and its reviewed checksum, but its live contents can later change.
Deletion is only available for recorded creations without protected/shared use.
Unregistered files, remote files, original inputs, and uncertain ownership are
not automatically cleaned. No server connection is required.

Read the complete review and select its confirmation checkbox. Confirmation
saves the retained materials and original notebook export before locking the
conversation and deleting the selected local files immediately. There is no
recycle bin or recovery period. A changed notebook or file invalidates the
review. A snapshot failure removes the incomplete attempt and leaves the notebook
editable and original files untouched. A cleanup failure leaves the archive and notebook locked; the archive
shows per-file receipts and allows retry of the already-approved cleanup.
Changed or newly protected files are skipped, including on retry.

While the review is preparing or confirming, it can be minimized with the
header button. The review collapses into a small status pill in the corner and
the flow keeps running in the background, so other conversations stay usable.
The pill shows the archive state (running, ready for review, needs attention,
or archived); click it to bring the review back, or use its close button to
dismiss a finished review. Escape does not restore or dismiss a minimized
review.

The original conversation remains readable. Sending, compaction, undo, deletion,
and moving that notebook to another project cannot rewrite the archived record.
The milestone appears in **Research journey**, with links to retained materials,
the original notebook and **Continue research**. Continue creates a new linked
conversation. Later findings can revise earlier conclusions without overwriting
the historical milestone. New mainline conversations receive a concise archive
index and can read its reports on demand. Project exports and sync include sealed
archive metadata and workspace snapshots; unconfirmed drafts are not exported.

归档相当于完成实验记录本中的一个研究阶段：保留完整原始记录，把结论、关键代码、
必要数据和最终结果整理成一个研究历程节点。集中确认后，会话只读，确认过的本地
中间文件立即永久删除。后续研究从节点创建关联的新会话，保留旧结论及其材料。

The draft is written by the model bound to the built-in **Archivist** specialist
(Settings → Specialists → Archivist). While it is unbound, or its profile was
deleted, the archive uses the archived session's own model, as before.
归档草稿使用 **设置 → 专家 → Archivist** 绑定的模型；未绑定或绑定的模型已删除时，
沿用被归档会话自己的模型。

Current limits: preparation accepts up to 4 MiB of saved source records, split
into bounded model requests when necessary. Draft synthesis turns off extra
reasoning and uses at least a 32k output budget, clamped to the model's catalog
ceiling, and retries once at that ceiling if the first response is truncated.
Files must be regular local files
inside the project; links/junctions and directory deletion are excluded. Only
recorded file identities can be offered for cleanup. Large retained data is copied
only when the review selects a snapshot. Archive files live under
`.wisp/research-archives/<archive-id>/`; keep this directory with the project.

Validation: store/backend tests use temporary projects and fake notebook content,
without model keys, remote servers or deletion of user files. UI tests use the
mock Tauri bridge and cover the single confirmation, stale review, read-only
conversation, journey navigation, continuation, immediate Escape, Chinese and
narrow layouts. For a live smoke check, finish a disposable local analysis, run
`/archive`, review each action, confirm, reopen the milestone, and continue it in
a new conversation; verify the retained snapshot still matches its reviewed bytes.
