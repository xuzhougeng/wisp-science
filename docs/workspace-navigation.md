# Workspace navigation

By default, opening a workspace restores its most recently **used** conversation
(one that already has a user message). A named but unused draft stays visible
in the sidebar, but it is not treated as the last conversation. Opening a
specific conversation from Recent sessions or search still takes priority.

Choose another workspace from the sidebar workspace menu to switch the current
window in place. **File → New Window** opens a blank GUI on the Projects home
screen; it does not inherit the current workspace. A dedicated project window
is opened by any action labelled **Open in new window**.

Different project windows can run conversations concurrently. Live replies,
approvals, browser tab cleanup and human-verification prompts belong to the
conversation's project. Switching projects clears the old browser prompts in
that window; it does not dismiss them for the original project. Native macOS
menu actions apply only to the focused window.

The main window and dedicated project windows each restore their last selected
project, including a dedicated window that has switched workspaces. Concurrent
window changes preserve each other's restore records. Blank **New Window**
views are not restored after quitting, even after opening a project in them.

The shared Chrome profile permits one project's browser automation at a time,
from its first browser tool until its turns finish. Another project receives
an occupied message and can retry afterward; its other work can continue.
Credentials, models, themes and global settings remain application-wide.

Multi-window smoke check (Windows/macOS/Linux): open project A in the main
window and B in a dedicated window; run a tool-using conversation in each and
verify separate live replies and approvals. Switch the dedicated window to C,
then open B in a new window and confirm C stays visible in its original window.
Quit and relaunch: main restores A and the dedicated windows restore C and B.
On macOS, **File → New Window** should create exactly one blank window and
**File → Projects** should affect only the focused window. Browser cleanup or
verification prompts from A must stay out of B, including after B reloads.

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
