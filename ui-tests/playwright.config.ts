import { defineConfig } from "@playwright/test";

const port = process.env.UI_TEST_PORT ?? "1420";

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
    command: `NO_COLOR=false trunk serve --port ${port}`,
    url: `http://localhost:${port}`,
    reuseExistingServer: true,
    timeout: 180_000,
    cwd: "../ui",
  },
});
