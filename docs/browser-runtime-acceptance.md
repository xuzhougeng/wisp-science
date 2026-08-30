# Browser Runtime acceptance

Use a Wisp build that bundles extension **0.3.1**. Load the unpacked extension from `browser_setup.extension_path` before testing.

1. Connect extension 0.2.1 and confirm the app shows current 0.2.1 / bundled 0.3.1, the verified managed path, and the update actions. **Update extension** must return the manual fallback, open the extension page, and clear only after Reload reconnects as 0.3.1. With an extension that advertises `runtime_reload`, the same action must reconnect automatically. `browser_setup` then shows `update_required=false`, `extension_version` 0.3.1, and `required_protocol` 2.
2. Open a WeChat article, `web_scan` with `mode=article`, confirm `images[]` includes the body figures, then `web_save_assets` copies them under the project `browser-assets/` with SHA-256. Do not use page `fetch`.
3. `web_open_tab` on a GitHub repo and a Zenodo DOI returns a non-empty final `tab.url` / `tab.title`.
4. `browser_setup` `action=start_workspace` opens a second Chrome. Both sessions stay connected. Omitting `session` after both are live returns `SESSION_REQUIRED`.
5. In an already-logged-in ChatGPT, Gemini, or Google AI Mode
   (`google.com/search?udm=50`) tab: `web_agent_send` → `web_agent_wait` →
   `web_agent_read` returns the assistant text and source links. Captcha/login
   pages stop for the user. Ordinary Google Search without `udm=50` is refused.

Popup **Pause control** must fail later automations with `USER_CONTROLLING`.

6. Open several tabs with `web_open_tab` in one turn. With **Settings → Browser → Automatically close browser tabs** off, the turn-end dialog lists only those tabs (not ones already open). Unchecking one and confirming closes the rest. Enabling the setting closes this turn's tabs without a dialog.
