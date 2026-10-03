# IM Channels (Feishu / WeChat)

Settings → Remote Access connects IM bots to the workspace agent: messages you send
from Feishu or WeChat drive normal agent sessions (visible in the desktop app),
and the final answer of each turn is sent back to the chat.

The **Research assistant → Remote access** entry is a separate, WeChat-only
connection to the assistant's persistent conversation. It manages visible
projects through natural-language requests and does not use the project route
or project-switching commands described below. Bind and enable it separately;
the same bot cannot be bound to both entries. Its approval replies are `yes`
(approve once), `no` (reject), and `full` (approve and enable Full Permission
for the assistant conversation until Wisp restarts or `full off` is sent).
`/approval` repeats the assistant's pending request; `/status` shows permission
state. Dispatched project work keeps its own approvals. See
[Research assistant](research-assistant.md#微信远程接入).

The assistant can list a project's conversations, continue a selected one, and
bind a requested execution server before sending work. It acknowledges actual
startup and ends its coordinating turn. When that project turn finishes, a
background callback reviews the result and appends a summary to assistant history;
WeChat-originated work also returns the summary to the original owner/bot binding.
Project approvals are also supervised in the background: the assistant approves clear operations within the original user request once, and forwards uncertain requests with their details and reason to the assistant conversation and original WeChat binding. Reply yes/no to resolve the exact project request. Explicit project Full Permission is respected for ordinary confirmations; assistant Full Permission is not inherited. Keep Wisp running for these callbacks. If iLink's reply window has expired, the
summary remains on the desktop and waits in the active connection until a fresh
owner message permits delivery. Pending remote notices do not survive disconnect
or restart. Failures and interrupted turns are reported without claiming success.

Desktop, Feishu, and WeChat share one durable **IM target project**. Ordinary
IM messages continue that project's current IM session. Starting work on the
desktop in another project does **not** move Feishu or WeChat to that project —
it only records that project's last session. Use `/project` from either bot to
choose the IM destination.

The IM target can be inspected and changed from either Feishu or WeChat:

- `/status` shows the IM project and last IM session.
- `/project` lists projects; `/project <number|name|id>` switches the IM
  project and prepares a new session there.
- `/session` lists recent sessions in the IM project;
  `/session <number|title|id>` makes one the IM target.
- `/new` prepares a fresh IM session in the selected project.
- `/stop` cancels the shared target's running turn; `/help` shows the command
  list.
- WeChat additionally supports `/approval`, `/approve <code>`, and
  `/reject <code> [feedback]` for text-only tool approval.

List numbers and unique ID prefixes are accepted, so a UUID does not normally
need to be typed in full. Route resolution and first-session creation are
serialized across both channels, so simultaneous first messages cannot split
into separate sessions. A desktop send only records that project's last
session; it does not move the IM target. On upgrade, a legacy
`{project_id, session_id}` route becomes the IM destination and is copied into
`last_session_by_project`. If no IM target is set, `/status`, `/session`, and
`/new` ask the user to `/project` first.

Only plain text input is supported in v1 (WeChat voice messages arrive as
transcripts and work too). Approval prompts still appear in the desktop app. A
WeChat turn that reaches a native Wisp confirmation or an ACP permission request
also receives a bounded plain-text summary and a one-time approval code; Feishu
approvals still require the desktop app. Project IM turns additionally force Ask on
write/edit/shell and other mutating tools even when the desktop policy defaults
to Allow, including Full Permission.

## Feishu bot

Uses a **self-built app** over Feishu's official long connection, so no public
callback URL is needed. The recommended setup is **Settings → Remote Access →
Create by QR code**. Choose Feishu China or Lark International first, scan with
the matching mobile app, and finish the app setup in the page opened by Feishu.
Wisp stores the returned App Secret directly in the OS keyring; the device code
and secret are never exposed to the webview or written to SQLite.

An existing app can still be configured manually on
[open.feishu.cn](https://open.feishu.cn) or
[open.larksuite.com](https://open.larksuite.com):

1. Create a self-built app (企业自建应用); copy its App ID / App Secret.
2. Events & callbacks → subscription mode **Long connection (长连接)**; subscribe
   to `im.message.receive_v1`.
3. Permissions: `im:message`, `im:message.p2p_msg`, `im:message.group_at_msg`
   (or `im:message.group_msg`), plus "get bot info".
4. Paste App ID / App Secret in Settings → Remote Access, select the matching
   region, save, then toggle on. The secret is stored in the OS keyring.

Only the **bound owner** can drive agent turns. Scanning to create the app
binds that Feishu account when the registration response includes an
`open_id`. Otherwise bind an owner in Settings by confirming a pending pairing
request or pasting their `open_id`. The first person to message the bot is
never made owner automatically. Direct (p2p) and group @-mentions from anyone
else are rejected before they enter the agent.

Duplicate event delivery is deduped by `event_id`. Normal agent turns appear
as a single CardKit card: the card shows a safe, coarse view of tool progress
and partial answer text, then becomes the final answer. Raw model reasoning,
tool output, and command output are never copied into the external progress
card. Slash-command replies remain plain text.

If CardKit creation or delivery is unavailable (for example because the app is
missing CardKit permissions), Wisp falls back to one plain-text final reply.
Current limits: text input only; files/images and interactive approval buttons
are not yet supported. Use `/stop` to cancel a running turn.

## WeChat bot (iLink)

Uses WeChat's official iLink bot API (`ilinkai.weixin.qq.com`). Click **Scan to
bind** in Settings → Remote Access and confirm in WeChat. The scanning account
becomes the owner — only its 1:1 messages are handled; group messages are
ignored. The bot token lives in the OS keyring; unbind removes it.

Notes:

- The login cannot be refreshed programmatically. When the server reports the
  session expired (errcode −14) the channel disables itself; re-scan to rebind.
- Replies must be sent within ~30 minutes of your message (server-side
  `context_token` window). Wisp keeps polling while an agent turn is running and
  uses the newest owner message token for approval acknowledgements and final
  replies.
- Native Wisp confirmations and ACP permission requests are sent as plain text.
  Reply with the exact
  `/approve <code>` or `/reject <code> [feedback]` command, or use `/approval`
  to list pending requests again. Codes are single-use and accept a unique
  prefix of at least six hexadecimal characters.
- For ACP, Wisp selects only the protocol's `AllowOnce` or `RejectOnce` option.
  If the ACP agent offers only persistent choices, the text command fails
  closed and that request must be handled in the desktop UI. ACP rejection
  options cannot carry free-form feedback; Wisp names that limitation in its
  acknowledgement instead of silently claiming the feedback was delivered.
- Project-bot remote approval means **once**. Its commands cannot enable Full
  Permission or persistent session/project/global grants. The separate research
  assistant binding supports `full` only for its own assistant conversation.

## Internals (for contributors)

`src-tauri/src/channels/`: `feishu.rs` (regional endpoint discovery → WSS →
pbbp2 frames → ACK ≤3s → events; REST token cache + CardKit stream),
`feishu_registration.rs` (OAuth device-flow QR creation and polling),
`feishu_card.rs` (pure CardKit/progress projection), `pbbp2.rs` (hand-rolled
protobuf frame codec, round-trip tested), `weixin.rs` (QR bind, non-blocking
`getupdates` pump, sequential agent-turn worker, immediate slash-command
control tasks, send), and `mod.rs` (ChannelManager, turn-scoped progress and
approval observer, shared last-message route in the
`channel_last_message_route` setting, Tauri commands). The route contains
the IM `project_id`, an optional IM `session_id`, and `last_session_by_project`.
An empty IM session is the intentional pending state created by `/project` or
`/new`. Desktop `send_message` only updates that project's last session. Inbound
text reuses the same `send_message` path as the UI with `TurnOrigin::Im`, so
history is shared, while mutating-tool approval is stricter for IM.
Protocol shapes follow phantty's tested implementations and the official
`larksuite/oapi-sdk-go`.
