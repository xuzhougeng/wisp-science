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
4. Choose **Load unpacked** and select the bundled `browser-extension/`
   directory. In a source checkout this is the repository's
   `browser-extension/` directory. An installed build reports its exact bundled
   path through `browser_setup`. Select the directory itself, not an individual
   file or archive inside it. The Agent must quote the reported picker path
   verbatim; under WSL this is the `\\wsl.localhost\...` path that a Windows
   Chrome folder picker can open.
5. Open the extension popup and confirm that it says **Connected to Wisp**.

The unpacked extension remains installed in that browser profile across Wisp
and browser restarts. It reconnects to `ws://127.0.0.1:18765` when Wisp is
running. Only loopback connections whose WebSocket origin is a Chrome extension
with Wisp's bundled, stable extension ID are accepted.

## Agent tools

- `browser_setup`: report bridge status, the exact bundled extension directory,
  and one-time installation steps. It does not read browser page content and
  does not require approval.
- `web_scan`: list real browser tabs, extract page text, or return a compact
  snapshot of visible actionable elements and selectors.
- `web_execute_js`: execute JavaScript in the selected real tab. It also accepts
  a JSON `{ "cmd": "cdp", ... }` request for a single Chrome DevTools Protocol
  method when trusted browser input or another CDP-only action is required.

Both tools always require at least one Wisp approval. The approval can be
granted once, for the session, for the project, or globally through the existing
approval card. Treat broad grants carefully: the extension has access to every
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
