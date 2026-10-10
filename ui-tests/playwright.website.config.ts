import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./website-tests",
  outputDir: "./website-test-results",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  use: { baseURL: "http://127.0.0.1:1433", browserName: "chromium" },
  webServer: {
    command: `${process.platform === "win32" ? "python" : "python3"} -m http.server 1433 --bind 127.0.0.1 --directory ../docs/dist-cloudflare`,
    url: "http://127.0.0.1:1433",
    reuseExistingServer: !process.env.CI,
  },
});
