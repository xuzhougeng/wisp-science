# Remote web access

Open the Wisp running on your computer from a phone or any browser (#1460).
The desktop dials out to your own `wisp-relay`; no inbound port is opened on
the computer. One relay serves any number of computers: each computer has its
own connection code, and browsers reach the computer whose code they hold.

```
browser ──HTTPS/WSS──> wisp-relay (ciphertext only) <──outbound WSS── desktop Wisp
```

## Deploy the relay

Remote access uses the same `wisp-relay` binary as [project sync](project-sync.md#self-hosted-relay);
one deployment serves both.

```bash
export WISP_RELAY_TOKEN="replace-with-a-long-random-token"
export WISP_RELAY_ROOT="/var/lib/wisp-relay"
export WISP_RELAY_BIND="127.0.0.1:8787"
cargo run -p wisp-sync --bin wisp-relay --release
```

Browsers only allow the page's encryption (WebCrypto) on HTTPS, so put the
relay behind a TLS reverse proxy that forwards WebSocket upgrades. Caddy does
this by default:

```
relay.example.com {
    reverse_proxy 127.0.0.1:8787
}
```

With nginx, forward the upgrade headers:

```nginx
location / {
    proxy_pass http://127.0.0.1:8787;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_read_timeout 120s;
}
```

The relay pings both sides every 25 seconds, under nginx's default idle
timeout. A path prefix works (`https://example.com/wisp/` → page at
`/wisp/remote`) as long as the proxy strips it.

## Connect a computer

1. Desktop: **Settings → Remote Access → Remote web access**.
2. Enter the relay URL and the relay's `WISP_RELAY_TOKEN`, then enable it.
   The token is stored in the OS keyring. HTTP is accepted only for `localhost`.
3. Wisp shows a **connection code** (`xxxx-xxxx-…`, 128 random bits, kept in
   the OS keyring) and a link `https://relay.example.com/remote#<code>`.
4. Open the link on the phone, or open `/remote` and type the code.

The badge shows *Running* while the tunnel is up and how many browsers are
connected. **Reset code** generates a new code: every old link stops working
and connected browsers are disconnected. The code survives restarts until
reset.

## Use the page

The page is laid out like the desktop workspace, for the moments you are away
from it rather than as a full client:

- **Projects and conversations.** Pick a project, then a conversation from the
  list on the left; the project picker above the list switches projects without
  leaving the page. Rows show *Running* and *Needs you*, and opening a finished
  conversation clears its *Needs you* flag on the desktop too.
- **Conversation.** The latest transcript page with replies rendered as
  Markdown (headings, lists, tables, code, links), pictures shown inline, live
  tool calls, and pending approvals with the full command or diff. You can
  start a conversation, send a message, stop a turn and approve or reject a
  request.
- **Files.** The folder button opens the conversation's project folder beside
  it. Folders can be browsed; text files open as text, Markdown is rendered,
  and images open as pictures. PDF and Office documents show the text the
  desktop extracts from them, not their original layout. A file link in a
  reply opens the same preview.

On a phone the three panes are shown one at a time and the back arrow steps
out of the current one. The address keeps track of where you are
(`#<code>/p/<project>/s/<conversation>/f/<file>`), so the browser's Back and
reload work and a bookmark returns to the same conversation. That address
contains the connection code: treat a copied address like the link itself.

The page polls every 1.5 s while a turn runs and every 4 s otherwise, and an
open folder every 8 s. It keeps unsent text after a failed send and never
retries a mutation by itself.

## Security model

- **The relay never sees the code.** The code lives in the URL fragment, which
  browsers never send to servers. The relay sees only
  `sid = SHA-256("wisp-remote/sid/v1" ‖ code)[..16]`; frames are AES-256-GCM
  with key `SHA-256("wisp-remote/key/v1" ‖ code)`. Relay operators, proxies and
  logs only see ciphertext and timing.
- **Frames are direction-bound and fresh.** Each direction has its own AAD
  label, so the relay cannot reflect a frame back. On every browser connection
  the host sends a random nonce; each request must carry it and a strictly
  increasing sequence number, so a recorded request cannot be replayed or
  reordered.
- **Only the relay's token holder can register a computer**, so a public relay
  cannot be used by strangers as a free tunnel.
- **Small allowlist.** Remote browsers can list projects and conversations,
  read snapshots, create conversations, send, stop, approve and mark a
  conversation as seen. Terminal input, kernel execution, file save and file
  actions, attachments, queued follow-ups and ACP conversations are not
  available remotely. Approvals apply once; a remote browser cannot grant
  session/project/global permissions.
- **Files are read-only and stay on the computer.** A browser can list folders
  inside a conversation's project folder and read text previews (the first
  1 MB of a large file). Pictures are sent as a re-encoded thumbnail of at most
  1024 px. Other file bytes are never sent, so nothing can be downloaded, and
  files on remote execution contexts are not reachable. Markdown is rendered from
  text into a fixed set of elements; HTML inside a reply or a file is shown as
  text, never interpreted.
- **One oversized reply cannot drop the tunnel.** A reply that would exceed the
  relay's frame limit is answered with an error for that request only.
- **Remote turns always ask.** Messages sent from the web run with the same
  origin as IM channels: writing files, editing and running shell commands
  require approval even if the desktop default is Allow.
- **The loopback native broker is unchanged.** It still rejects any request
  with a browser `Origin`; the web page never talks to the computer directly.

Anyone with the link can use the computer's projects: treat it like a password.

## Not yet included

The page is deliberately not a full client. Rendered PDF and Office previews,
downloads, earlier transcript pages, model and plan-mode controls, run cards,
the terminal and notebooks stay on the desktop.

Tracked in #1460: chunked attachment upload, command result lookup after a
dropped reply, actor columns in the store, per-connection delivery tiers,
push notifications, read-only/operator roles for guests, and proxy support for
the desktop's outbound connection. The desktop host name comes from
`COMPUTERNAME`/`HOSTNAME`; macOS shows `Wisp` until a hostname API is linked.

## Internals (for contributors)

| Piece | Location |
| --- | --- |
| Relay rendezvous, page routes, code derivation, frame sealing | `crates/wisp-sync/src/remote.rs` |
| Web client (vanilla JS, `textContent` only, strict CSP) | `crates/wisp-sync/src/remote.{html,js}` |
| Browser test of that page against a fake sealed-frame host | `ui-tests/tests/remote-web.spec.ts` |
| Desktop tunnel, allowlist, settings commands | `src-tauri/src/channels/remote.rs` |
| Remote actor stamp and IM-origin turns | `Broker::remote` in `native_settings.rs`, `remote_turn` in `native_conversations.rs` |

Relay ↔ host envelopes are plain JSON (`{"t":"open"|"msg"|"close","c":<browser>,"d":<sealed>}`).
Sealed frames are `base64(iv ‖ ciphertext ‖ tag)`. The host's first sealed
frame to a browser is `{"type":"hello","nonce","name","version"}`; requests are
`{"nonce","seq","id","command","project_id","args"}`; replies are
`{"type":"response","response":<native settings Response>}`.
