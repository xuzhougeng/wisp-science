# IM Channels (Feishu / WeChat)

Settings → Remote Access connects IM bots to the workspace agent: messages you send
from Feishu or WeChat drive normal agent sessions (visible in the desktop app),
and the final answer of each turn is sent back to the chat.

Desktop, Feishu, and WeChat share one durable **last-message session**. Every
ordinary IM message is sent to the session that most recently accepted a user
message on any of those three surfaces. This makes it possible to start work in
the desktop app and continue it from either bot without rebuilding context.
Merely viewing or navigating to a session does not change the route.

The shared target can be inspected and changed from either Feishu or WeChat:

- `/status` shows the shared project and last-message session.
- `/project` lists projects; `/project <number|name|id>` switches project and
  prepares the shared route for a new session there.
- `/session` lists recent sessions in the selected project;
  `/session <number|title|id>` makes one the shared target.
- `/new` prepares a fresh shared session in the selected project.
- `/stop` cancels the shared target's running turn; `/help` shows the command
  list.

List numbers and unique ID prefixes are accepted, so a UUID does not normally
need to be typed in full. Route resolution and first-session creation are
serialized across both channels, so simultaneous first messages cannot split
into separate sessions. The last-message pointer is updated before waiting for
a busy session, so a queued desktop follow-up becomes the Feishu/WeChat target
immediately instead of remaining stuck behind the running turn. On upgrade,
Wisp recovers the latest persisted user message once, then records accepted
sends directly going forward.

Only plain text is supported in v1 (WeChat voice messages arrive as transcripts
and work too). Tool-approval prompts still appear in the desktop app —
unattended turns that need approval wait until you click there.

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

Direct (p2p) messages are handled as-is; in group chats the bot only reacts
when @-mentioned. Duplicate event delivery is deduped by `event_id`. Normal
agent turns appear as a single CardKit card: the card shows a safe, coarse
view of tool progress and partial answer text, then becomes the final answer.
Raw model reasoning, tool output, and command output are never copied into the
external progress card. Slash-command replies remain plain text.

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
  `context_token` window); if a turn runs longer, send another message to get
  the result.

## Internals (for contributors)

`src-tauri/src/channels/`: `feishu.rs` (regional endpoint discovery → WSS →
pbbp2 frames → ACK ≤3s → events; REST token cache + CardKit stream),
`feishu_registration.rs` (OAuth device-flow QR creation and polling),
`feishu_card.rs` (pure CardKit/progress projection), `pbbp2.rs` (hand-rolled
protobuf frame codec, round-trip tested), `weixin.rs` (QR bind, `getupdates`
long-poll with cursor, send), and `mod.rs` (ChannelManager, live progress
observer, shared last-message route in the `channel_last_message_route` setting,
Tauri commands). The route contains `project_id` and an optional `session_id`;
an empty session is the intentional pending state created by `/project` or
`/new`. Inbound text reuses the same `send_message` path as the UI, so desktop
and both channels update the same route and history/tools/approvals behave
identically.
Protocol shapes follow phantty's tested implementations and the official
`larksuite/oapi-sdk-go`.
