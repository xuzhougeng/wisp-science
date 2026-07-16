# IM Channels (Feishu / WeChat)

Settings → Channels connects IM bots to the workspace agent: messages you send
from Feishu or WeChat drive normal agent sessions (visible in the desktop app),
and the final answer of each turn is sent back to the chat.

Each chat maps to one session; send `/new` to start a fresh session, `/stop` to
cancel the running turn, `/help` for the command list. Only plain text is
supported in v1 (WeChat voice messages arrive as transcripts and work too).
Tool-approval prompts still appear in the desktop app — unattended turns that
need approval wait until you click there.

## Feishu bot

Uses a **self-built app** over Feishu's official long connection, so no public
callback URL is needed. One-time setup on
[open.feishu.cn](https://open.feishu.cn):

1. Create a self-built app (企业自建应用); copy its App ID / App Secret.
2. Events & callbacks → subscription mode **Long connection (长连接)**; subscribe
   to `im.message.receive_v1`.
3. Permissions: `im:message`, `im:message.p2p_msg`, `im:message.group_at_msg`
   (or `im:message.group_msg`), plus "get bot info".
4. Paste App ID / App Secret in Settings → Channels, toggle on. The secret is
   stored in the OS keyring.

Direct (p2p) messages are handled as-is; in group chats the bot only reacts
when @-mentioned. Duplicate event delivery is deduped by `event_id`.

Current limits: Feishu CN domain only (no Lark International), plain-text
replies (no cards/files yet).

## WeChat bot (iLink)

Uses WeChat's official iLink bot API (`ilinkai.weixin.qq.com`). Click **Scan to
bind** in Settings → Channels and confirm in WeChat. The scanning account
becomes the owner — only its 1:1 messages are handled; group messages are
ignored. The bot token lives in the OS keyring; unbind removes it.

Notes:

- The login cannot be refreshed programmatically. When the server reports the
  session expired (errcode −14) the channel disables itself; re-scan to rebind.
- Replies must be sent within ~30 minutes of your message (server-side
  `context_token` window); if a turn runs longer, send another message to get
  the result.

## Internals (for contributors)

`src-tauri/src/channels/`: `feishu.rs` (endpoint discovery → WSS → pbbp2
frames → ACK ≤3s → events; REST token cache + send), `pbbp2.rs` (hand-rolled
protobuf frame codec, round-trip tested), `weixin.rs` (QR bind, `getupdates`
long-poll with cursor, send), `mod.rs` (ChannelManager, chat↔session map in the
`channel_sessions` setting, Tauri commands). Inbound text reuses the same
`send_message` path as the UI, so history/tools/approvals behave identically.
Protocol shapes follow phantty's tested implementations and the official
`larksuite/oapi-sdk-go`.
