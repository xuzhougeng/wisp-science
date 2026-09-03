# Workspace navigation

By default, opening a workspace restores its most recently **used** conversation
(one that already has a user message). A named but unused draft stays visible
in the sidebar, but it is not treated as the last conversation. Opening a
specific conversation from Recent sessions or search still takes priority.

Choose another workspace from the sidebar workspace menu to switch the current
window in place. A separate window is opened only by an action explicitly
labelled **Open in new window**. Each window remembers its own last project;
opening a workspace in a second window does not change what the first window
restores after a restart.

Each window title includes the open workspace name
(`wisp science — my-project`) so the taskbar, Alt-Tab, and macOS title bar can
tell windows apart. The Projects home screen uses the app name alone.

On the Projects home screen, each project card has a **Project settings**
button. It opens name, description, and Agent Context for that project without
entering the workspace first. The home screen does not use the browser
right-click menu.

To open workspaces on a blank conversation instead, turn off **Resume the last
conversation when opening a workspace** in **Settings → General**. Starting a
new conversation manually is always available from the sidebar. A newly
created conversation appears there immediately as **Untitled session**, even
before its first message is sent.

Use the magnifying-glass button beside **Sessions** to search conversation
titles in the current workspace. Search includes older conversations that have
not been loaded into the paginated sidebar yet. Clear the field or press Escape
to restore the normal grouped conversation list.

## Project rules changes and existing conversations

A conversation's system prompt — including `AGENTS.md` and the project **Agent
context** (`.wisp/WISP.md`) — is assembled once when the conversation starts
and kept stable for its lifetime, so edits apply only to new conversations.
When the files on disk no longer match a conversation's persisted prompt,
right-click that conversation and choose **Reload project rules…**. A
confirmation dialog explains the prompt-cache cost; there is no toast.
The reload takes effect on the next turn and leaves the chat history
untouched; because the prompt prefix changes, the provider's prompt cache
for that conversation is invalidated once, so the next turn costs a bit more.

Editing **Agent Context** in Project Settings writes `.wisp/WISP.md`. Saving
a changed context asks for confirmation first: new conversations pick it up
automatically, while existing conversations keep the old prompt until you
reload project rules for that session.
