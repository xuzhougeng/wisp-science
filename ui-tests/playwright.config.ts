import { defineConfig } from "@playwright/test";

const port = process.env.UI_TEST_PORT ?? "1422";
const serveCommand = process.platform === "win32"
  ? `powershell -NoProfile -Command "Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue; Remove-Item Env:TRUNK_NO_COLOR -ErrorAction SilentlyContinue; node sync-vendor.mjs; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; trunk serve --no-autoreload --address 127.0.0.1 --port ${port} --dist dist-test"`
  : `node sync-vendor.mjs && env -u NO_COLOR -u TRUNK_NO_COLOR trunk serve --no-autoreload --address 127.0.0.1 --port ${port} --dist dist-test`;

// E2E for the wisp-science Leptos UI. A `trunk serve` dev server hosts the frontend
// at :1422 by default; Playwright injects a mocked `window.__TAURI__` before the page
// loads so the UI's invoke/listen calls resolve against canned data — no Rust
// backend or API key required.
//
// Every test boots its own page (addInitScript + goto) against the shared dev
// server, so tests are independent and run fully parallel, including within one
// file: ui.spec.ts alone holds half the suite and would otherwise pin one worker
// for the whole run. CI (4 vCPUs) uses 4 workers and splits the suite across
// runners with `--shard`; locally Playwright picks half the cores. Pass
// `--workers=1` to debug in file order. No `retries`: a failing test is a bug.
export default defineConfig({
  testDir: "./tests",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  workers: process.env.CI ? 4 : undefined,
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
