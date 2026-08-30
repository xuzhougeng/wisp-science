> **Runtime note (0.3.1):** see [browser-runtime-architecture.md](browser-runtime-architecture.md) and [browser-runtime-acceptance.md](browser-runtime-acceptance.md). The extension is Protocol v2. Shared Chrome stays on `ws://127.0.0.1:18765`; workspace Chrome uses a dedicated profile and `18766`. Wisp prepares a verified stable extension directory and automatically reloads compatible versions; older versions receive a guided one-time Reload fallback.

# Real-browser automation

> **Acknowledgement:** this feature is inspired by GenericAgent's
> [GA Web / TMWebDriver](https://github.com/lsdefine/GenericAgent) architecture.
> Wisp's Rust bridge and Manifest V3 extension are an independent
> implementation. Detailed provenance is bundled in
> `browser-extension/NOTICE.md`.

Wisp exposes `web_scan` and `web_execute_js` against tabs in the user's existing
Chrome/Chromium profile. It does not start Playwright, Selenium, a headless
browser, or a temporary profile. The controlled pages therefore keep the
profile's cookies and login state, installed extensions, GPU/WebGL behavior,
and normal browser fingerprint.

## Install the bridge extension

The user can ask Wisp to **configure the browser**. The Agent calls the
read-only `browser_setup` tool and reports the current connection status, the
exact extension directory on that installation, and the following steps.

1. Start Wisp Science.
2. Open `chrome://extensions` in the Chrome/Chromium profile Wisp should use.
3. Enable **Developer mode**.
4. Choose **Load unpacked** and select the managed `browser-extension/`
   directory reported by `browser_setup`. Select the directory itself, not an
   individual file or archive inside it. Wisp validates the packaged files and
   synchronizes this stable application-data directory before reporting it. The
   path must be copied verbatim; Wisp never translates it between Windows, WSL,
   macOS, or Linux path formats.
5. Open the extension popup and confirm that it says **Connected to Wisp**.

If the extension is not connected, live page retrieval fails closed. Wisp
shows a chat banner that the current answer includes no live web results,
and the Agent must stop on live, latest, current, or URL-specific requests
instead of answering from memory. The banner's **Set up browser** button
opens the browser's extension page and copies the managed path so the
remaining steps are Developer mode and Load unpacked. After Chrome is open
and the popup shows **Connected to Wisp**, use **Retry after connecting**
to run the same request again.

**Settings → Browser → Open browser automatically** is on by default. When a
browser tool needs the real session and Chrome/Chromium/Edge is not running,
Wisp starts that installed browser with the existing user profile so the
unpacked extension can reconnect. Turn the setting off to keep Wisp from
launching a browser.

**Settings → Browser → Automatically close browser tabs** is off by default.
Wisp records tabs it created during the current turn (by `tab_id`, including
after in-tab navigation) and never includes tabs that were already open.
When the setting is on, those tabs are closed when the turn ends (completed,
stopped, or failed). When it is off, a confirmation lists them, all selected
by default; uncheck any to keep, then close the rest or keep all. If the
extension is disconnected at the end of the turn, the pending list is kept
until it reconnects.

The banner describes the answer on screen, not the session. It is derived from
the browser tool results of the latest turn only, and a single successful
`web_scan`, `web_open_tab`, `web_execute_js`, or `web_screenshot` clears it: the
extension's service worker sleeps and reconnects on a one-minute alarm, so a
turn can mix refused attempts with successful ones and still be a live answer.
A connected extension also counts as connected even when a build cannot verify
its own bundled `browser-extension/` copy.

The unpacked extension remains installed in that browser profile across Wisp
and browser restarts. If its version is older than the current Wisp build, an
update banner shows the current and required versions and offers **Update
extension**, **Copy extension path**, **Open extension page**, and **Recheck**.
Wisp first verifies and refreshes the managed files. Extension 0.3.1 and later
can reload automatically; older versions require one click on **Reload** in
`chrome://extensions` (or loading the managed directory if the previous install
points elsewhere). Wisp automatically rechecks the new handshake. The extension
reconnects to `ws://127.0.0.1:18765` when Wisp is running. Only loopback
connections whose WebSocket origin is a Chrome extension with Wisp's bundled,
stable extension ID are accepted.

## Human-verification handoff

When `web_scan` detects an **Are you a robot?** page together with a request to
confirm that the visitor is human or complete a CAPTCHA challenge, it returns
`human_intervention.required=true`. The Agent must stop browser automation, ask
the user to complete the challenge manually in the current visible browser tab,
and wait for confirmation. It then scans the same tab again and continues only
after the challenge is gone. The persistent browser profile keeps any clearance
cookie issued after the manual verification.

Wisp does not attempt to click, solve, or bypass CAPTCHA challenges. A page that
merely mentions the phrase **Are you a robot?** without the accompanying
human-verification prompt does not trigger the handoff.

## Downloads and native dialogs

GA Web controls web-page tabs. It cannot operate Chrome/Edge toolbar download
bubbles or native operating-system **Open**, **Save**, and **Save As** dialogs.
Page JavaScript and the Wisp extension cannot access those browser or operating
system surfaces.

For unattended browser downloads, make this one-time browser-profile change:

1. Open `chrome://settings/downloads` in Chrome, or
   `edge://settings/downloads` in Edge.
2. Turn off **Ask where to save each file before downloading**.
3. Downloads will then use the browser's configured default download directory
   without opening a native location prompt. An authorized Wisp filesystem tool
   can process or move the saved file afterward.

For unattended batches that download more than one file from the same site:

1. Before triggering the batch, Wisp explains the following browser settings
   and waits for the user to confirm that configuration is complete. Until the
   user confirms, Wisp downloads at most one file.
2. Open `chrome://settings/content/automaticDownloads` in Chrome, or
   `edge://settings/content/automaticDownloads` in Edge.
3. Add only the trusted target site to **Allowed to automatically download
   multiple files**. If the browser asks on that site's first batch, choose
   **Allow**.
4. Do not grant this permission to untrusted sites; it allows that site to
   trigger multiple successive downloads without a user gesture for each file.

These settings must be changed manually because internal settings pages such as
`chrome://settings` and `edge://settings` are not scriptable by the bridge.

## Agent tools

- `browser_setup`: report bridge status, the exact bundled extension directory,
  one-time installation steps, and the user's URL filter lists (`url_filters`).
  It does not read browser page content and does not require approval.
- `web_scan`: list real browser tabs, extract page text, or return a compact
  snapshot of visible actionable elements and selectors. It waits until the
  tab's document is `complete` (or the tool timeout) before reading. The result
  includes `ready` and `page.ready_state`; if `ready` is false, scan again
  instead of acting on a partial page.
- `web_execute_js`: execute JavaScript in the selected real tab. It also accepts
  a JSON `{ "cmd": "cdp", ... }` request for a single Chrome DevTools Protocol
  method when trusted browser input or another CDP-only action is required.
  Navigational JSON/JS that targets a blocked host is refused. The extension
  waits for document `complete` before running the script, and waits again if
  the script navigates or the tab returns to `loading`.
- `web_open_tab`: open an HTTP(S) tab. The call waits until that tab's document
  is `complete` (or times out) and returns `ready` plus the real URL/title.
  Blocked hosts from **Settings → Browser** are refused before the tab opens.
  When a prefer list is set, a successful result includes `preferred` so
  literature tasks can stay on those sites.
- `web_screenshot`: capture the visible viewport of the selected real tab as a
  JPEG and read it with the configured vision model, for rendered layout,
  charts, canvas/WebGL pages, QR codes, or a page that looks wrong. It waits
  for document `complete` before capturing. It captures the viewport only;
  scroll with `web_execute_js` to reach content below the fold. It needs a
  vision-capable model configured in **Settings → Models**, like `view_image`.
- `web_agent_send` / `web_agent_wait` / `web_agent_read`: one-shot send, wait,
  and read against an already-logged-in in-browser chat tab. Supported HTTPS
  hosts are `chatgpt.com` / `chat.openai.com`, `gemini.google.com`, and
  `google.com` with `udm=50` (Google AI Mode). Login and CAPTCHA pages stop
  for the user; Wisp does not type passwords or bypass those gates.

Both tools normally require at least one Wisp approval. The approval can be
granted once, for the session, for the project, or globally through the existing
approval card. A conversation with **Full Permission** enabled auto-approves the
same request. Treat broad grants carefully: the extension has access to every
HTTP(S) tab in that Chrome profile.

## Security and limits

- The extension asks only for `tabs`, `scripting`, `debugger`, and the alarm used
  to reconnect. It has no dedicated cookie-export API, does not remove page CSP,
  disable dialogs, or change content settings. Approved page JavaScript or raw
  CDP commands are still powerful and should be treated as access to the tab.
- Ordinary execution uses Chrome's scripting API. Wisp falls back to a temporary
  CDP attachment only when page CSP prevents that execution, or when the caller
  explicitly requests a CDP method.
- Chrome internal pages such as `chrome://settings` cannot be scripted. Only
  HTTP(S) tabs are advertised.
- JavaScript-created DOM events are not trusted events. Use an explicitly
  approved CDP `Input.*` command when a site requires trusted input.
- Wisp and GenericAgent's TMWebDriver use the same default port. Run only one
  bridge server on port `18765` at a time.
- **Settings → Browser** stores global host block and prefer lists. A blocked
  host (and its subdomains) cannot be opened or navigated to through
  `web_open_tab` or an explicit navigational `web_execute_js` script. Prefer
  hosts are advisory. Indirect JavaScript navigation can still change the
  current tab; `web_scan` of an already-open tab is not blocked.
