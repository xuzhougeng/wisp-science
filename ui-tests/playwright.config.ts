import { defineConfig } from "@playwright/test";

const port = process.env.UI_TEST_PORT ?? "1422";
const serveCommand = process.platform === "win32"
  ? `powershell -NoProfile -Command "Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue; Remove-Item Env:TRUNK_NO_COLOR -ErrorAction SilentlyContinue; trunk serve --address 127.0.0.1 --port ${port} --dist dist-test"`
  : `env -u NO_COLOR -u TRUNK_NO_COLOR trunk serve --address 127.0.0.1 --port ${port} --dist dist-test`;

// E2E for the wisp-science Leptos UI. A `trunk serve` dev server hosts the frontend
// at :1422 by default; Playwright injects a mocked `window.__TAURI__` before the page
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
  webServer: process.env.PLAYWRIGHT_REUSE_SERVER ? undefined : {
    command: serveCommand,
    url: `http://localhost:${port}`,
    reuseExistingServer: true,
    timeout: 180_000,
    cwd: "../ui",
  },
});
