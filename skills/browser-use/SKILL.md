---
name: browser-use
description: "Use this skill to drive Wisp Browser Runtime sessions (shared daily Chrome or workspace Chrome) — open pages, read them, click, fill and submit forms, navigate, switch tabs, or scrape content that needs the user's existing cookies and login state. Triggers when the user asks to do something in their browser, log into a site and act inside it, fill out a web form, click through a flow, or extract data from a page that requires being signed in. Tools: browser_setup (check/connect the extension), web_open_tab (open a URL), web_scan (read visible content + actionable elements with ready-made selectors), web_execute_js (click/type/navigate, or a JSON command for tabs/CDP), web_screenshot (see what the tab is showing — layout, charts, canvas, QR codes). Not for the built-in read-only web fetch — this is for interacting with a live browser."
fold_cue: "instead_of=guessing-selectors use=web_scan first — it returns a unique CSS selector and rect for every actionable element; never invent selectors"
---

# Browser Use — act inside the user's real Chrome

Wisp talks to the **Browser Runtime**. Shared mode uses the user's daily
Chrome via the unpacked extension — every action runs in their real
profile: existing cookies, logins, extensions, and normal fingerprint all
apply. Workspace mode can launch a separate Chrome profile. Omitting
`session` always uses shared, even when workspace is connected. Pass
`session: "workspace"` only when the user explicitly requests isolation. If
Settings → Browser has **Open browser automatically** enabled (the
default) and the shared extension is disconnected, Wisp may start the installed
Chrome/Chromium/Edge so the extension can reconnect. That is still the
user's profile, not Playwright or Selenium.

**Shared is the default; workspace is not a fallback.** Google Chrome 137
and later ignore `--load-extension`, so on a machine with only branded
Chrome the workspace window cannot load the Wisp extension at all.
`browser_setup {"action":"start_workspace"}` therefore returns only once
the workspace extension has connected, and otherwise closes the window
and fails with `WORKSPACE_EXTENSION_BLOCKED`. On that error: relay the
message, do not retry `start_workspace`, do not claim any workspace page
was opened or read, and get the shared session working instead.

Start with `browser_setup` without an action (an empty action is also a status
check). If the task supplies a target URL, pass it as `url` so a disconnected
daily browser opens that page directly; otherwise startup uses its new-tab page.
A connected shared browser is reused without opening another window. Startup
does not close existing tabs, including pre-existing blank or new-tab pages.

For figures/code extraction use `web_scan` with `mode: "article"` then
`web_save_assets`. For an already-logged-in in-browser chat (ChatGPT,
Gemini, or Google AI Mode at `google.com/search?udm=50`) use
`web_agent_send`, `web_agent_wait`, `web_agent_read` on that tab.

Every `web_scan` and `web_execute_js` call needs the user's approval by
design. Do not treat that as a bug to route around.

## Before anything: confirm the bridge is live

Call `browser_setup`. If `status` is not `connected` (or `live_retrieval`
is false), relay its `steps` (load the unpacked extension from
`extension_path`, verbatim) and **stop**. Do not answer live, latest,
current, or URL-specific questions from prior knowledge. Tell the user
this turn contains no live web retrieval and wait until the popup shows
*Connected to Wisp*. Only continue from memory if they explicitly ask
for a knowledge-only answer. Never invent the path.

Two fields say *why* a browser that looks connected is not usable —
never report a bare "not connected" when either is set:

- `refused_connection` — something reached the bridge port and Wisp
  refused it (usually a different extension id, or another loopback
  bridge holding the port). Its popup can still read *Connected to Wisp*.
  Relay `refused_connection.explanation`.
- `update_required` / `reload_required` — a connected extension is older than
  this build. Call `browser_setup {"action":"update_extension"}` first. If it
  returns `updated`, call `browser_setup` again and continue only when
  `update_required=false`. If it returns `manual_reload_required`, relay the
  current and bundled versions plus `extension_path` verbatim, ask the user to
  **Reload** Wisp Real Browser Bridge on `chrome://extensions`, and wait. Older
  unpacked extensions cannot accept Wisp's automatic service-worker reload.

One exception: the user says the extension is already installed. Chrome
suspends its service worker when idle and reconnects on a one-minute
alarm, so `disconnected` can just be a sleeping worker. Try `web_open_tab`
or `web_scan` once — a successful call proves the bridge is live — and
relay the install steps only if that call fails too.

## The loop

1. **`web_open_tab`** `{url}` — open the page (works even with no tab
   open yet). Waits until the document is complete, then returns the new
   tab id plus `ready`. If `ready` is false, the load timed out — call
   `web_scan` before acting.
2. **`web_scan`** — read the page (after waiting for document complete).
   Returns `page.text`, `page.title`, `page.ready_state`, and
   `page.elements[]`, where each element carries a **unique `selector`**,
   its visible `text`/`aria_label`, and a `rect` `[x,y,w,h]`. Use these
   selectors directly — do not guess. If `ready` is false, scan again;
   do not click a partial page. Use `tabs_only:true` first when you are
   unsure which tab to target; pass `switch_tab_id:<id>` to pin one.
3. **`web_execute_js`** — act, then re-scan to confirm the effect. The
   extension waits for complete before running the script, and again if
   the script navigates.

## Recipes (`web_execute_js` `script`)

| Goal | script |
|---|---|
| Click | `document.querySelector('<selector>').click()` |
| Type into a field | `const e=document.querySelector('<sel>'); e.value='text'; e.dispatchEvent(new Event('input',{bubbles:true})); e.dispatchEvent(new Event('change',{bubbles:true}))` |
| Submit a form | click the submit control by its selector, then re-scan |
| Navigate current tab | `location.href='https://example.com'` |
| Read a value | `document.querySelector('<sel>').textContent` |

`script` may instead be a **JSON command**:

| Goal | JSON command |
|---|---|
| Switch to & focus a tab (so the user sees it) | `{"cmd":"tabs","method":"switch","tabId":<id>}` |
| List tabs | `{"cmd":"tabs"}` (or just `web_scan tabs_only`) |
| Close tabs you opened | `{"cmd":"tabs","method":"close","tabIds":[<id>,...]}` — returns `closed` + `remaining` |
| Trusted click when `.click()` is ignored | `{"cmd":"cdp","method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":<x>,"y":<y>,"button":"left","clickCount":1}}` then the same with `"type":"mouseReleased"` — use the element's `rect` centre from `web_scan` |

Prefer plain JS. Reach for `cmd:cdp` only when a page blocks synthetic
events or you truly need trusted input.

## In-browser chat — `web_agent_send` / `web_agent_wait` / `web_agent_read`

Use these on an **already signed-in** tab. They are not a new Wisp agent;
they drive the chat composer in the user's Chrome.

Supported tabs (HTTPS, exact host, no lookalikes):

- ChatGPT: `chatgpt.com` / `chat.openai.com`
- Gemini: `gemini.google.com`
- Google AI Mode: `google.com/search?udm=50` (plain Google Search without
  `udm=50` is refused)

Flow: `web_agent_send {prompt}` → `web_agent_wait` → `web_agent_read`. The
read result is `{answer_text, citations, status, site}`. If the page is
login or CAPTCHA, stop and let the user finish it in that tab.

## Seeing the page — `web_screenshot`

`web_scan` gives text and elements; **`web_screenshot`** gives sight. Use it
when structure isn't enough: rendered layout, a chart or diagram, a
canvas/WebGL page, a QR code, a PDF or image viewer, or a page that looks
broken. It captures the **visible viewport** of the tab — to see below the
fold, scroll first (`web_execute_js` `scrollTo(0, 1200)`) and capture again.
Pass `question` to say what to read out of it, e.g.
`{"question":"is the login QR code visible and not expired?"}`.

It goes through the configured vision model, so `web_scan` stays the cheaper
default — screenshot when you need eyes, not for every step.

## Tab hygiene — the app tracks what you open

Browsing tasks (searching papers, opening a dozen results) used to leave the
user with a pile of tabs. **Do not ask in chat whether to close them.** The
desktop records every tab `web_open_tab` (and tab-create commands) opened in
this turn, including after URL changes, and never includes tabs that were
already open.

- If Settings → Browser has **Automatically close browser tabs** on
  (`browser_setup.auto_close_tabs=true`), the app closes this turn's tabs
  when the turn ends. Do not also close them yourself unless the user asks
  mid-task.
- If that setting is off, the app shows a confirmation after the turn
  (default: close all this-turn tabs; the user can uncheck pages to keep).
- You may still close a tab **mid-task** with
  `{"cmd":"tabs","method":"close","tabIds":[...]}` if a later step does not
  need it, or if the user explicitly asks now.

Close **only ids you opened yourself**. Tabs the user had open, or ones
they opened during the task, are theirs.

## Stop conditions (do not automate through these)

- **Human verification / CAPTCHA:** if `web_scan` returns
  `human_intervention.required=true`, a Wisp prompt has already been shown.
  Stop browser automation. Do not open another in-app question about the
  challenge. Do not click, solve, or bypass it. End your turn. The user
  confirms in the app after completing it in the visible tab; the next
  message is their confirmation.
- **Credentials:** never type passwords, card numbers, or one-time codes
  yourself. If a step needs a password, have the user sign in directly in
  the browser and continue once they confirm.
- **Irreversible / outward actions** (send, pay, post, delete): confirm
  with the user before clicking the control.
- **Downloads:** for multiple-file downloads, first surface the browser
  settings from `browser_setup` (`download_automation`) and wait for the
  user to confirm; until then trigger at most one download.
- **Blocked sites:** if `web_open_tab` or a navigational `web_execute_js`
  fails with `blocked by user URL filter`, do not retry that site. Read
  `browser_setup.url_filters.block` for the current list. Prefer entries in
  `url_filters.prefer` for literature search and similar retrieval; other
  sites are still allowed.
