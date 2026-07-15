import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const repositoryRoot = resolve(__dirname, "../..");
const readRepositoryFile = (path: string) =>
  readFileSync(resolve(repositoryRoot, path), "utf8");

test("Tauri dev, UI tests, and release builds use isolated Trunk outputs", () => {
  const tauriConfig = JSON.parse(readRepositoryFile("src-tauri/tauri.conf.json"));
  const devScript = readRepositoryFile("ui/dev.ps1");
  const buildScript = readRepositoryFile("ui/build.ps1");
  const playwrightConfig = readRepositoryFile("ui-tests/playwright.config.ts");

  expect(tauriConfig.build.devUrl).toBe("http://localhost:1421");
  expect(devScript).toContain("$devPort = 1421");
  expect(devScript).toContain("--dist dist-dev");
  expect(devScript).toContain("exit $LASTEXITCODE");
  expect(buildScript).toContain("trunk build --release --dist dist");
  expect(buildScript).toContain("exit $LASTEXITCODE");
  expect(playwrightConfig).toContain('UI_TEST_PORT ?? "1422"');
  expect(playwrightConfig).toContain("--dist dist-test");
});
