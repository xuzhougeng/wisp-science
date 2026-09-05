# Browser Runtime acceptance

Use a Wisp build that bundles extension **0.3.1**. Load the unpacked extension from `browser_setup.extension_path` before testing.

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

1. Connect extension 0.2.1 and confirm the app shows current 0.2.1 / bundled 0.3.1, the verified managed path, and the update actions. **Update extension** must return the manual fallback, open the extension page, and clear only after Reload reconnects as 0.3.1. With an extension that advertises `runtime_reload`, the same action must reconnect automatically. `browser_setup` then shows `update_required=false`, `extension_version` 0.3.1, and `required_protocol` 2.
2. Open a WeChat article, `web_scan` with `mode=article`, confirm `images[]` includes the body figures, then `web_save_assets` copies them under the project `browser-assets/` with SHA-256. Do not use page `fetch`.
3. `web_open_tab` on a GitHub repo and a Zenodo DOI returns a non-empty final `tab.url` / `tab.title`.
4. `browser_setup` `action=start_workspace` opens a second Chrome only when the user explicitly requests isolation. Both sessions stay connected, and omitting `session` still uses `shared`; pass `session=workspace` to target the isolated browser.
5. In an already-logged-in ChatGPT, Gemini, or Google AI Mode
   (`google.com/search?udm=50`) tab: `web_agent_send` → `web_agent_wait` →
   `web_agent_read` returns the assistant text and source links. Captcha/login
   pages stop for the user. Ordinary Google Search without `udm=50` is refused.

Popup **Pause control** must fail later automations with `USER_CONTROLLING`.

6. Open several tabs with `web_open_tab` in one turn. With **Settings → Browser → Automatically close browser tabs** off, the turn-end dialog lists only those tabs (not ones already open). Unchecking one and confirming closes the rest. Enabling the setting closes this turn's tabs without a dialog.
7. Open ScienceDirect (or any Cloudflare **Are you a robot?** page). `web_scan` must show the in-app **Human verification needed** prompt, keep that tab open when auto-close is on, and refuse click/JS on that tab until **I completed verification** succeeds.
