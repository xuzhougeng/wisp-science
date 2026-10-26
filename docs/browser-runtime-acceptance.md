# Browser Runtime acceptance

Use a Wisp build that bundles extension **0.5.0**. Load the unpacked extension from `browser_setup.extension_path` before testing.

For #1107, test on Windows, macOS, and Linux with automatic launch enabled:
close daily Chrome, call `browser_setup` with no action, and verify its normal
profile opens a new-tab page and the installed extension reconnects. Repeat with
`url: "https://example.com"` and verify that target opens directly. With shared
already connected, neither call should open a second window or modify existing
tabs. Let the extension worker sleep and retry to verify reconnection. Without
the extension, expect installation guidance and no workspace fallback. An empty
action must behave like an omitted action. Existing blank/new-tab pages are never
cleaned up by startup; ordinary per-turn cleanup still applies only to recorded
tool-created tabs.

1. Connect extension 0.2.1 and confirm the app shows current 0.2.1 / bundled 0.5.0, the verified managed path, and the update actions. **Update extension** must return the manual fallback, open the extension page, and clear only after Reload reconnects as 0.5.0. With an extension that advertises `runtime_reload`, the same action must reconnect automatically. `browser_setup` then shows `update_required=false`, `extension_version` 0.5.0, and `required_protocol` 2.
   With the extension connected, quit Wisp without closing Chrome, wait two
   minutes, and restart Wisp: the extension must reconnect on its own within
   about two minutes (keepalive staleness plus the one-minute alarm) without a
   Reload in `chrome://extensions`.
2. Open a WeChat article, `web_scan` with `mode=article`, confirm `images[]` includes the body figures, then `web_save_assets` copies them under the project `browser-assets/` with SHA-256. Do not use page `fetch`.
3. `web_open_tab` on a GitHub repo and a Zenodo DOI returns a non-empty final `tab.url` / `tab.title`.
4. `browser_setup` `action=start_workspace` opens a second Chrome only when the user explicitly requests isolation. Both sessions stay connected, and omitting `session` still uses `shared`; pass `session=workspace` to target the isolated browser.
5. Hold a browser turn open in project A on `shared`. Project B can use
   `session=workspace` concurrently, but another project targeting `shared`
   receives an occupancy error naming the session and the occupying project.
5. In an already-logged-in ChatGPT, Gemini, or Google AI Mode
   (`google.com/search?udm=50`) tab: `web_agent_send` → `web_agent_wait` →
   `web_agent_read` returns the assistant text and source links. Captcha/login
   pages stop for the user. Ordinary Google Search without `udm=50` is refused.

Popup **Pause control** must fail later automations with `USER_CONTROLLING`.

6. Open several tabs with `web_open_tab` in one turn. With **Settings → Browser → Automatically close browser tabs** off, the turn-end dialog lists only those tabs (not ones already open). Unchecking one and confirming closes the rest. Enabling the setting closes this turn's tabs without a dialog.
7. Open ScienceDirect (or any Cloudflare **Are you a robot?** page). `web_scan` must show the in-app **Human verification needed** prompt, keep that tab open when auto-close is on, and refuse click/JS on that tab until **I completed verification** succeeds.
8. Side panel. With Wisp running in the background and a project open, open an
   article tab, choose **Ask about this page** in the popup and ask a question.
   The answer streams into the panel and a conversation titled after the page
   appears in that project. A follow-up shows only the page title under the
   question (the text is not resent). Select a paragraph and ask again: the
   line shows the selection length. Open a PDF URL and ask: the line says there
   is no readable text, and an approval card appears when the agent reaches
   for a tool; **Allow once** continues the turn and **Deny** makes the agent
   answer without it. **Stop** cancels a running turn. The context menu and
   `Alt+Shift+W` open the same panel. Quit Wisp and ask: the panel reports that
   Wisp is not running.

## Project tab scope (#1617)

Shared Chrome with extension 0.5.0 and two projects A and B open in Wisp.

1. In A, `web_open_tab` twice. Both tabs appear in one native group titled
   `Wisp · <A's name>` in the same window. Focus a different Chrome window and
   open a third tab from A: it still lands in A's window and group.
   `web_scan tabs_only` in A lists the three tabs under `tabs` and reports
   `scope.window_id`, `scope.group_id` and `window_pinning: pinned`.
2. In B (after A's turn ends), `web_open_tab` once. B's tab gets its own group
   and colour. `web_scan tabs_only` in B lists only B's tab under `tabs`; A's
   tabs are absent from both `tabs` and `unscoped_tabs`. Passing one of A's
   tab ids as `switch_tab_id` to `web_scan`, `web_execute_js` or
   `web_screenshot`, or inside `{"cmd":"tabs","method":"close","tabIds":[…]}`
   or a `cdp` command, fails with `TAB_OUT_OF_SCOPE` and no prompt.
3. Open a page yourself. It appears under `unscoped_tabs`. A `web_scan` with
   that id shows the **Share browser tab …** approval. Deny: the tool fails
   and the tab stays unscoped. Allow: the scan runs, the tab is listed under
   `tabs` with `granted: true`, and the window binding is unchanged.
4. With A's tab focused in Chrome, run `web_scan` from B without
   `switch_tab_id`: B reads its own tab, not A's. With no tab in scope, B is
   told to open one or ask for one instead of being handed the active tab.
5. Drag one of A's tabs into another window. A's next listing drops it and
   reports it once under `detached`; it reappears under `unscoped_tabs`.
6. Close A's window and open a tab from A: the extension opens a new window for
   A (not B's or yours) and `scope.window_id` changes to it. Quit and restart
   Chrome, reconnect, and list again: every scope is empty and
   `window_pinning` is `unbound`; nothing is reused from the old process.
7. Load extension 0.4.2 instead: the same refusals apply, but
   `window_pinning` reads `unsupported_extension` and no groups are created.
8. Click a link that opens a popup from one of A's tabs: the popup is listed in
   A's scope and in A's turn-end cleanup prompt. The shared cross-project
   occupancy error from step 5 of the general list still appears unchanged.

## Project workspace isolation (#1325)

1. Start a workspace in projects A and B. Confirm different profile directories,
   endpoints and lane identities in `browser_setup`, with matching project ids.
2. Open different URLs in both projects using `session=workspace`; scan both
   concurrently. Equal numeric tab ids must still return the correct pages.
3. Hold shared Chrome from A. B's workspace still works; B targeting `shared`
   receives an occupancy error naming A. Default routing stays shared.
4. Start A again: it reuses its instance. Start a third workspace; a fourth
   fails with actionable stop instructions. Stop B and retry the fourth.
5. Stop/delete A while B has a pending request and a verification prompt.
   Only A's process, requests and prompts are cleared; B remains usable.
6. Restart Wisp, then start A and B. Their profiles stay separate and their
   connection identities change. Old workspace tab-cleanup and verification
   prompts do not reappear or act on another instance.

Automated coverage uses fake processes/connections and temporary files; no
real browser, external network, or logged-in session is needed.
