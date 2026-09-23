# Wisp Science Basics: Import, Export, and Sharing

After an analysis, you may want to continue the whole project on another computer, hand one conversation to a colleague, or share a few results with your lab group.

Wisp Science provides separate entry points for these needs. Choosing the right scope makes importing and reading easier. This tutorial goes from the whole project to one conversation and then selected messages, explaining what each option carries.

> Screenshots use the real frontend in English with demonstration projects and conversations. Numbers, paths, and contact names are examples, not real experimental results or evidence of completed migration.

**Choose what you want to deliver.**

| Need | Export | How the recipient uses it |
| --- | --- | --- |
| Continue the whole project on another computer | Project directory or ZIP | Restore through Import project in Wisp |
| Hand off one complete conversation and exportable associated artifacts | Session ZIP | Use Import session archive in the target project |
| Show a few messages | PNG long image or HTML sharing export | Read with an image viewer or browser |
| Inspect the execution process | Trajectory HTML export | Review calls, results, time, and usage |

Project ZIPs and session ZIPs share an extension but have different formats and import entry points. Sharing HTML and trajectory HTML are reading materials, not archives that can be imported to resume a conversation.

**Export a whole project for migration or a complete handoff.**

Wait for active conversations and tasks in the project to finish. Click **Export project** on its project card, or open it and choose **File → Export current project**.

![Project export dialog offering a full ZIP archive](../../assets/tutorials/en/transfer/02-project-export.png)

*Figure 1 shows the previous interface. The current dialog offers Export directory and Export ZIP. Both include files, conversations and project records; directory export skips compression.*

Choose **Export directory** or **Export ZIP** and a destination. A progress card at the lower right reports stage, file count, bytes, and current path. Wait for completion before copying the complete directory or ZIP.

The source project is temporarily read-only during export; other projects remain usable. Do not send a destination file that is unfinished or still empty.

The package includes regular workspace files and project-owned conversations, artifacts, Runs, plans, provenance, and research-graph records. It does not carry:

- Model API keys, other keyring secrets, or global model configuration.
- This computer's SSH/WSL environment configuration.
- Running external ACP agent process state.
- Entire server datasets behind remote references.

The recipient must configure their own models and servers. A remote path may survive as a reference, but importing the ZIP does not grant access to it.

**Choose the matching project folder or ZIP import entry.**

Click **Import project** on the Projects screen to see the choices.

![Project import choices: open a folder in place, import a ZIP, or recover workspace conversations](../../assets/tutorials/en/transfer/01-project-import.png)

*Figure 2 shows the previous interface. Import project folder now replaces Open a folder in place and requires an exported project package. Workspace conversation recovery remains a separate fallback.*

For a complete migration:

1. Choose **Import a ZIP archive** and select the exported Wisp project ZIP.
2. Select a local parent directory; Wisp creates the project directory inside it.
3. Wait for the progress card to finish, then open the project from the Projects screen.
4. Check conversations, scripts, figures, and key data paths, then configure models and environments on the destination computer.

For an exported directory, choose **Import project folder** and select the package root containing `manifest.json`, `metadata/project.sqlite` and `workspace`. Wisp validates metadata, restores records and uses the packaged workspace in place. Ordinary folders without metadata are rejected instead of creating a blank project.

Metadata is a snapshot taken at export time. It does not automatically update as you work; export again before the next transfer.

Use **Recover conversations from a workspace** when the original app database and complete archive are lost but the workspace remains. It attempts to recover saved history snapshots; not every ordinary conversation necessarily has one.

Importing the same project ID twice on a device is rejected rather than merged. For frequent switching between devices, see [Manual Project Sync](../../project-sync.md).

**Export one conversation from the sidebar.**

Inside a project, find the conversation on the left. Right-click it, or open its conversation action menu, and select **Export session**.

![Export session in the conversation action menu](../../assets/tutorials/en/transfer/03-session-export.png)

*Figure 3: This menu belongs to the sidebar conversation entry. Right-clicking ordinary transcript text does not open the same export menu.*

Choose a destination to save a session ZIP. It contains message records, readable Markdown, tool-call records, and associated artifacts and information that can be collected.

This is a conversation archive, not an archive of every project file. A file mentioned in prose, located remotely, or deleted locally is not guaranteed to be included.

Before handoff, ask for a checklist:

> From this conversation, prepare a handoff checklist of inputs actually read, scripts and outputs generated, and data that must be provided separately. Identify each local or remote path, and mark files whose existence cannot be confirmed.

Compare the checklist with the archive. Supply other input files separately or export the complete project if that better matches the handoff.

**Open the receiving project before importing a session.**

In the target project, choose **Edit → Import session archive**. Alternatively, press **Ctrl+P**, or **Cmd+P** on macOS, and search for **Import session archive**.

![Command palette finding Import session archive](../../assets/tutorials/en/transfer/04-session-import.png)

*Figure 4: This imports into the current project. Select a ZIP produced by Export session, not a project ZIP or sharing HTML.*

On first import, the conversation appears in the `imported` group. Open it, check the messages, and verify that associated artifacts are readable.

Artifacts are restored to their original relative paths where those paths are safe and unoccupied. Conflicts may place files under `imports/<session-id>/`; files that cannot be restored are reported. Inspect actual destination paths rather than running old absolute paths from the transcript.

Single-session import primarily restores messages and extractable artifacts. It does not reconstruct all project-level records or turn exported provenance into destination-database execution records. It also does not restore running Python/R memory, SSH terminals, or ACP processes.

Reimporting the same source session may update its existing import or skip it. This is not two-way merging. If both copies have continued differently, keep backups of both before deciding how to hand them off.

**Use Share as image for selected messages.**

For an explanation, result table, or a few turns, use the topbar share button or send `/share` in the message box.

![Sharing dialog with selected messages, redaction keywords, and PNG or HTML options](../../assets/tutorials/en/transfer/05-share.png)

*Figure 5: Only the first two messages are selected. The contact name “Dr. Lee” is entered as a redaction keyword; the later discussion is excluded.*

A practical sequence is:

1. Select the messages you want and deselect the rest.
2. Enter names or internal project identifiers as redaction keywords and inspect the preview.
3. Choose **PNG** or **HTML**.
4. Adjust the image width for PNG, or save HTML as a webpage file.
5. Open the export and check text, tables, and equations.

Selection is by whole message, not an arbitrary sentence selection. User messages and assistant replies are selected by default; thinking is not. Tool calls and usage records are not ordinary sharing rows.

| Format | Useful for | Check before sharing |
| --- | --- | --- |
| PNG long image | Group chats and meeting materials | Reasonable length and legible small text |
| HTML webpage | Longer discussions and result tables in a browser | The file opens and any external links or images remain accessible |

Keyword redaction changes the sharing copy, not the saved conversation. It replaces supplied text; it does not automatically recognize every sensitive detail or erase words embedded in screenshots. Inspect the exported result.

Export saves a file. It does not automatically publish to a public account, group chat, or other platform. You choose the recipient afterward.

**Share conclusions and retain the process together.**

Export key messages for a colleague who needs the conclusion. Add [trajectory HTML](wisp-science-trajectory.md) for debugging. To continue analysis in Wisp, supply a session or project ZIP and any required input data and environment information.

Practice in a small project with two conversation turns: export a project, export and import one session, and share just one question-and-answer pair. Check how each file opens before relying on it for a real handoff.

**Check archive type and actual contents when troubleshooting.**

| Symptom | Check first |
| --- | --- |
| ZIP import rejects the format | Project archive sent to session import, or session archive sent to project import |
| Conversations are missing after copying a folder | Whether only files were copied, leaving database records behind |
| An imported file cannot be opened | Remote reference, missing archive entry, or a conflict that moved it into `imports/` |
| Analysis cannot resume after import | Models, interpreters, dependencies, and servers on the destination machine |
| Trying to import sharing HTML as a session | HTML is for reading; resuming a conversation requires a session ZIP |
| Shared output lacks tool-call detail | Use trajectory HTML to inspect execution |
| Shared image is too long or too small | Select fewer messages, change the width, or use HTML |

> See [Project Transfer](../../project-transfer.md) for migration rules and [Basic Configuration](../../basic-configuration.md) for sharing entry points. This tutorial reflects the implementation when written; labels may vary. Example prompts do not represent completed file inspections.
