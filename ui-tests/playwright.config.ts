import { defineConfig } from "@playwright/test";

const port = process.env.UI_TEST_PORT ?? "1420";
const { NO_COLOR: _noColor, TRUNK_NO_COLOR: _trunkNoColor, ...serverEnv } = process.env;
const serveCommand = process.platform === "win32"
  ? `powershell -NoProfile -Command "Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue; Remove-Item Env:TRUNK_NO_COLOR -ErrorAction SilentlyContinue; trunk serve --port ${port}"`
  : `trunk serve --port ${port}`;

// E2E for the wisp-science Leptos UI. A `trunk serve` dev server hosts the frontend
// at :1420; Playwright injects a mocked `window.__TAURI__` before the page
// loads so the UI's invoke/listen calls resolve against canned data — no Rust
// backend or API key required.
export default defineConfig({
  testDir: "./tests",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  use: {
    baseURL: `http://localhost:${port}`,
    browserName: "chromium",
    trace: "on-first-retry",
  },
  webServer: {
    command: serveCommand,
    env: serverEnv,
    url: `http://localhost:${port}`,
    reuseExistingServer: true,
    timeout: 180_000,
    cwd: "../ui",
  },
});
