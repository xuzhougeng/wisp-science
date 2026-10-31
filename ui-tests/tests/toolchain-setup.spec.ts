import { expect, test, type Locator, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Installing uv, Python, R, and Node into Wisp's own toolchain from the local
// environment panel (src-tauri/src/toolchain.rs). The backend reports progress
// through `toolchain-install` events; the mock only records the request.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function emitTauriEvent(page: Page, event: string, payload: unknown) {
  await expect.poll(() => page.evaluate((name) =>
    Boolean((window as any).__tauriListenerReady?.(name)), event
  )).toBe(true);
  await page.evaluate(({ name, value }) => {
    (window as any).__tauriEmit(name, value);
  }, { name: event, value: payload });
}

function invokes(page: Page, cmd: string) {
  return page.evaluate((name) => ((window as any).__skillInvokeLog ?? [])
    .filter((call: any) => call.cmd === name)
    .map((call: any) => {
      const args = call.args instanceof Map ? Object.fromEntries(call.args) : call.args;
      return args ?? null;
    }), cmd);
}

const ROOT = "/mock/app-data/toolchain";

function toolchain(state: Record<string, unknown>) {
  return {
    root: ROOT, installed: [], state: "idle", tools: [], step: 0, total: 0,
    label: "", log: [], error: null, ...state,
  };
}

function bootstrap(paths: Record<string, string>) {
  return {
    skills_loaded: 12, python_ok: true, local_environment: { paths, warning: null },
    mcp_catalog: 8, uv_ok: true, node_ok: true, npm_ok: true, sci_ok: true, pixi_ok: true,
    app_version: "0.29.0", os: "macos", arch: "aarch64", startup: "", workspace: "/mock",
    errors: [],
  };
}

async function openOnboardingEnvironment(page: Page, query = "") {
  // At 720 px the environment page scrolls inside the dialog, and a click
  // right after Playwright scrolls it can land on a shifted layout. Keep the
  // whole page on screen; this flow is about installing, not scrolling.
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.goto(`/?mockOnboarding=1${query}`);
  const onboard = page.locator(".onboard-overlay");
  await expect(onboard).toBeVisible();
  // Welcome → features → model key (left empty) → local environment.
  await onboard.getByRole("button", { name: "Next" }).click();
  await onboard.getByRole("button", { name: "Next" }).click();
  await onboard.getByRole("button", { name: "Set up later" }).click();
  const setup = onboard.getByTestId("toolchain-setup");
  await expect(setup).toBeVisible();
  return { onboard, setup };
}

function toolRow(setup: Locator, name: string) {
  return setup.locator(".toolchain-tool").filter({ has: setup.page().getByText(name, { exact: true }) });
}

function toolCheckbox(setup: Locator, name: string) {
  return toolRow(setup, name).locator('input[type="checkbox"]');
}

test("onboarding suggests installing only missing tools, with progress, stop, and retry", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__mockLocalEnvironment = {
      paths: { node_executable: "/usr/local/bin/node" }, warning: null,
    };
  });
  const { onboard, setup } = await openOnboardingEnvironment(page);
  await expect(setup).toContainText(ROOT);
  // Missing tools are suggested and preselected; the detected Node is used
  // as it is and offers no install.
  await expect(toolCheckbox(setup, "uv")).toBeChecked();
  await expect(toolCheckbox(setup, "Python 3.12")).toBeChecked();
  await expect(toolCheckbox(setup, "R")).toBeChecked();
  await expect(toolRow(setup, "R")).toContainText("Not found · install suggested");
  await expect(toolCheckbox(setup, "Node.js")).toHaveCount(0);
  await expect(toolRow(setup, "Node.js")).toContainText("Found · used as is");
  await expect(setup.locator(".toolchain-later")).toContainText("install later from Settings → General → Local environment");

  await toolCheckbox(setup, "R").uncheck();
  await setup.getByRole("button", { name: "Install selected" }).click();
  await expect.poll(async () => (await invokes(page, "install_toolchain")).at(-1))
    .toEqual({ tools: ["python", "uv"] });
  const stop = setup.getByRole("button", { name: "Stop installing" });
  await expect(stop).toBeVisible();
  await expect(toolCheckbox(setup, "uv")).toBeDisabled();

  await emitTauriEvent(page, "toolchain-install", toolchain({
    state: "running", tools: ["uv", "python"], step: 3, total: 6, label: "install:python",
    log: ["$ pixi global install --environment python python=3.12", "✔ The python environment has been installed."],
  }));
  await expect(setup).toContainText("Step 3 of 6: installing Python 3.12");
  await expect(setup.locator(".toolchain-log")).toContainText("python environment has been installed");
  // Installing never blocks finishing onboarding.
  await expect(onboard.getByRole("button", { name: "Get started" })).toBeEnabled();

  await stop.click();
  await expect.poll(async () => (await invokes(page, "cancel_toolchain_install")).length).toBe(1);
  await emitTauriEvent(page, "toolchain-install", toolchain({
    state: "cancelled", tools: ["uv", "python"], step: 3, total: 6, label: "install:python",
  }));
  await expect(setup).toContainText("Installation stopped. Finished tools stay installed.");
  await setup.getByRole("button", { name: "Retry" }).click();
  await expect.poll(async () => (await invokes(page, "install_toolchain")).length).toBe(2);
  await expect(stop).toBeVisible();

  // Completion refreshes the Local paths through the shared bootstrap event.
  const python = `${ROOT}/../python/.venv/bin/python`;
  await emitTauriEvent(page, "bootstrap-status", bootstrap({
    python_executable: python, uv_executable: `${ROOT}/envs/uv/bin/uv`, node_executable: "/usr/local/bin/node",
  }));
  await emitTauriEvent(page, "toolchain-install", toolchain({
    state: "done", installed: ["uv", "python"], tools: ["uv", "python"], step: 6, total: 6, label: "save",
  }));
  await expect(setup.locator(".settings-status.ok")).toHaveText("Tools installed; paths saved to Local.");
  await expect(page.locator(".copy-toast")).toContainText("Tools installed");
  await expect(toolRow(setup, "Python 3.12")).toContainText("Installed by Wisp");
  await expect(toolCheckbox(setup, "Python 3.12")).toHaveCount(0);
  await expect(onboard.getByTestId("local-environment").locator(".environment-paths")).toContainText(python);
  // Only the R the user skipped is still offered, and it stays unticked.
  await expect(toolCheckbox(setup, "R")).not.toBeChecked();
  await expect(setup.getByRole("button", { name: "Install selected" })).toBeDisabled();
  expect(await invokes(page, "send_message")).toEqual([]);
});

test("detected tools are used as they are and nothing is offered", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name: "General", exact: true }).click();
  // The default mock finds Python, Rscript, uv, and Node: the panel lists
  // their paths and offers no install section at all.
  const environment = page.getByTestId("local-environment");
  await expect(environment).toContainText("/mock/bin/Rscript");
  await expect.poll(async () => (await invokes(page, "get_toolchain_status")).length).toBeGreaterThan(0);
  await expect(page.getByTestId("toolchain-setup")).toHaveCount(0);
  await expect(environment.locator('input[type="checkbox"]')).toHaveCount(0);
  expect(await invokes(page, "install_toolchain")).toEqual([]);
});

test("a failed install explains itself in Chinese and keeps settings dismissible", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__mockLocalEnvironment = {
      paths: { python_executable: "/usr/bin/python3", uv_executable: "/usr/local/bin/uv", node_executable: "/usr/local/bin/node" },
      warning: null,
    };
    (window as any).__mockToolchainStatus = {
      root: "/mock/app-data/toolchain", installed: [], state: "failed", tools: ["r"], step: 1, total: 3,
      label: "pixi", log: ["Downloading https://github.com/prefix-dev/pixi/releases/download/v0.83.0/pixi-aarch64-apple-darwin.tar.gz"],
      error: "Could not download pixi from GitHub: connection refused. Check the Code requests proxy in Settings → Network, then retry.",
    };
  });
  await page.goto("/?mockLocale=zh");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name: "常规", exact: true }).click();
  const setup = page.getByTestId("toolchain-setup");
  await expect(setup).toContainText("安装到 Wisp");
  await expect(setup.getByRole("alert")).toContainText("无法从 GitHub 下载 pixi（connection refused）");
  await expect(setup.getByRole("alert")).toContainText("代码请求");
  await expect(setup.locator(".toolchain-log")).toContainText("pixi-aarch64-apple-darwin");
  await expect(toolRow(setup, "R")).toContainText("未检测到 · 建议安装");
  await expect(toolRow(setup, "uv")).toContainText("已检测到 · 直接使用");
  await expect(setup.locator(".toolchain-later")).toContainText("现在不装也可以");
  await expect(toolCheckbox(setup, "R")).toBeChecked();

  // A refusal from the backend is shown in place, not as a dialog.
  await page.evaluate(() => { (window as any).__failInstallToolchain = true; });
  await setup.getByRole("button", { name: "重试" }).click();
  await expect.poll(async () => (await invokes(page, "install_toolchain")).at(-1)).toEqual({ tools: ["r"] });
  await expect(setup).toContainText("已有安装正在进行。");

  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toBeHidden();
});
