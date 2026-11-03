import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function enterApp(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "New session" })).toBeVisible();
}

async function emitTauriEvent(page: Page, event: string, payload: unknown) {
  await expect.poll(() => page.evaluate((name) =>
    Boolean((window as any).__tauriListenerReady?.(name)), event
  )).toBe(true);
  await page.evaluate(({ name, value }) => {
    (window as any).__tauriEmit(name, value);
  }, { name: event, value: payload });
}

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

async function invokeCount(page: Page, cmd: string) {
  return page.evaluate((name) =>
    ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name).length,
  cmd);
}

async function startLiveRetrievalTurn(page: Page) {
  await page.locator("#composer-input").fill("latest rustc version");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).not.toBeNull();
  const sessionId = String((await lastInvokeArgs(page, "send_message")).sessionId ?? "");
  expect(sessionId).not.toBe("");
  return sessionId;
}

const disconnectedScan = (sessionId: string) => ({
  kind: "ToolResult",
  frame_id: sessionId,
  name: "web_scan",
  ok: false,
  content: "real-browser bridge unavailable: browser extension is not connected. WISP_BROWSER_DISCONNECTED",
});

const disconnectedSetup = (sessionId: string) => ({
  kind: "ToolResult",
  frame_id: sessionId,
  name: "browser_setup",
  ok: true,
  content: JSON.stringify({ status: "disconnected", live_retrieval: false }),
});

const successfulScan = (sessionId: string) => ({
  kind: "ToolResult",
  frame_id: sessionId,
  name: "web_scan",
  ok: true,
  content: JSON.stringify({ tabs: [{ title: "PubMed CLEC12A" }] }),
});

test("an outdated connected extension gets a verified update and manual reload fallback", async ({ page }) => {
  await page.addInitScript(() => {
    const status = {
      connected: true,
      current_version: "0.2.1",
      bundled_version: "0.3.1",
      current_protocol: 1,
      required_protocol: 2,
      update_required: true,
      automatic_reload_available: false,
      extension_path: "/mock/wisp/browser-extension",
      extension_path_verified: true,
      integrity_verified: true,
      error: null,
    };
    (window as any).__browserExtensionStatus = status;
    (window as any).__browserExtensionUpdateResult = {
      outcome: "manual_reload_required",
      status,
      opened: true,
      error: null,
    };
  });
  await enterApp(page);

  const banner = page.getByTestId("browser-extension-update-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Connected version 0.2.1");
  await expect(banner).toContainText("0.3.1");

  await banner.getByRole("button", { name: "Update extension" }).click();
  await expect.poll(() => invokeCount(page, "update_browser_extension")).toBe(1);
  await expect(banner).toContainText("Updated files are ready");
  await expect(banner.getByTestId("browser-extension-path")).toHaveText("/mock/wisp/browser-extension");
  await expect(banner.getByTestId("browser-setup-steps").locator("li")).toHaveCount(4);
  await expect(banner.getByRole("button", { name: "Copy extension path" })).toBeVisible();
  await expect(banner.getByRole("button", { name: "Open extension page" })).toBeVisible();
  await expect(banner.getByRole("button", { name: "Recheck" })).toBeVisible();

  await banner.getByRole("button", { name: "Open extension page" }).click();
  await expect.poll(() => invokeCount(page, "open_browser_extension_page")).toBe(1);

  await page.keyboard.press("Escape");
  await expect(banner).toHaveCount(0);
  await page.waitForTimeout(2_200);
  await expect(banner).toHaveCount(0);
});

for (const [width, locale] of [[1280, "en"], [820, "zh"], [1440, "zh"]] as const) {
  test(`long paths and actions fit at ${width}px in ${locale}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const windowsPath = "C:\\Users\\xuzhougeng\\AppData\\Roaming\\science.wisp-science\\wisp-science\\browser-extension\\unpacked\\very-long-managed-path-segment";
    const longPath = (locale === "en" ? windowsPath : "/Users/xuzhougeng/Library/Application Support/science.wisp-science/wisp-science/browser-extension/unpacked/very-long-managed-path-segment")
      + (locale === "en" ? "\\nested-managed-directory" : "/nested-managed-directory").repeat(8);
    await page.addInitScript((path) => {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
        writeText: async (value: string) => { (window as any).__copiedExtensionPath = value; },
      } });
      const status = {
        connected: true,
        current_version: "0.3.0",
        bundled_version: "0.3.1",
        current_protocol: 1,
        required_protocol: 2,
        update_required: true,
        automatic_reload_available: false,
        extension_path: path,
        extension_path_verified: true,
        integrity_verified: true,
        error: null,
      };
      (window as any).__browserExtensionStatus = status;
      (window as any).__browserExtensionUpdateResult = {
        outcome: "manual_reload_required",
        status,
        opened: true,
        error: null,
      };
    }, longPath);
    await page.goto(`/?mockLocale=${locale}`);
    await page.locator(".proj-card-main").first().click();

    const banner = page.getByTestId("browser-extension-update-banner");
    await expect(banner).toBeVisible();

    const path = banner.getByTestId("browser-extension-path");
    await expect(path).toHaveText(longPath);

    const checkLayout = async () => {
      const layout = await page.evaluate(() => {
        const bannerEl = document.querySelector("[data-testid='browser-extension-update-banner']") as HTMLElement | null;
        const pathEl = document.querySelector("[data-testid='browser-extension-path']") as HTMLElement | null;
        const composer = document.querySelector(".composer") as HTMLElement | null;
        const inner = document.querySelector(".composer-inner") as HTMLElement | null;
        const strip = document.querySelector(".session-runtime-strip") as HTMLElement | null;
        if (!bannerEl || !pathEl || !composer || !inner) return null;
        const bannerBox = bannerEl.getBoundingClientRect();
        const pathBox = pathEl.getBoundingClientRect();
        const composerBox = composer.getBoundingClientRect();
        const innerBox = inner.getBoundingClientRect();
        const stripBox = strip?.getBoundingClientRect();
        const pathStyle = getComputedStyle(pathEl);
        return {
          bannerWidth: bannerBox.width,
          bannerRight: bannerBox.right,
          bannerBottom: bannerBox.bottom,
          composerRight: composerBox.right,
          innerWidth: innerBox.width,
          innerTop: innerBox.top,
          stripTop: stripBox?.top ?? innerBox.top,
          pathRight: pathBox.right,
          pathWhiteSpace: pathStyle.whiteSpace,
          pathOverflow: pathStyle.overflow,
          pathTextOverflow: pathStyle.textOverflow,
          actionsFit: [...bannerEl.querySelectorAll("button")].every(button => {
            const box = button.getBoundingClientRect();
            return box.left >= bannerBox.left && box.right <= bannerBox.right;
          }),
          pathClipped: pathEl.scrollWidth > pathEl.clientWidth,
          bannerOverflows: bannerEl.scrollWidth > bannerEl.clientWidth + 1,
        };
      });
      expect(layout).not.toBeNull();
      expect(layout!.bannerOverflows).toBe(false);
      expect(layout!.actionsFit).toBe(true);
      expect(layout!.pathClipped).toBe(true);
      expect(layout!.bannerWidth).toBeLessThanOrEqual(layout!.innerWidth + 2);
      expect(layout!.bannerRight).toBeLessThanOrEqual(layout!.composerRight + 1);
      expect(layout!.pathRight).toBeLessThanOrEqual(layout!.bannerRight + 1);
      expect(layout!.bannerBottom).toBeLessThanOrEqual(layout!.stripTop + 1);
      expect(layout!.pathWhiteSpace).toBe("nowrap");
      expect(layout!.pathOverflow).toBe("hidden");
      expect(layout!.pathTextOverflow).toBe("ellipsis");
    };
    await checkLayout();
    await banner.getByRole("button", { name: locale === "zh" ? "复制扩展路径" : "Copy extension path", exact: true }).click();
    expect(await page.evaluate(() => (window as any).__copiedExtensionPath)).toBe(longPath);
    await page.screenshot({ path: testInfo.outputPath("extension-update-path.png"), animations: "disabled" });
    await banner.getByRole("button", { name: locale === "zh" ? "更新扩展" : "Update extension", exact: true }).click();
    await expect(banner).toContainText(locale === "zh" ? "新版文件已就绪" : "Updated files are ready");
    await checkLayout();
  });
}

test("disconnected browser retrieval shows a banner that Escape dismisses without moving focus", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));

  const banner = page.getByTestId("browser-offline-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("This answer has no live web results");
  await expect(banner).toContainText("based only on the model's existing knowledge");

  await page.keyboard.press("Escape");
  await expect(banner).toBeHidden();
  await expect(page.locator("#composer-input")).toBeVisible();
});

const macUserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const windowsUserAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

async function openSetupSteps(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: async (value: string) => { (window as any).__copiedExtensionPath = value; },
    } });
  });
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);
  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));
  const banner = page.getByTestId("browser-offline-banner");
  await expect(banner).toBeVisible();
  await expect(banner.getByTestId("browser-setup-steps")).toHaveCount(0);

  await banner.getByRole("button", { name: "Set up browser" }).click();
  await expect.poll(() => invokeCount(page, "open_browser_extension_page")).toBe(1);
  await expect(banner.getByTestId("browser-offline-extension-path")).toHaveText("/mock/wisp/browser-extension");
  expect(await page.evaluate(() => (window as any).__copiedExtensionPath)).toBe("/mock/wisp/browser-extension");
  const steps = banner.getByTestId("browser-setup-steps").locator("li");
  await expect(steps).toHaveCount(4);
  await expect(steps.nth(0)).toContainText("Developer mode");
  await expect(steps.nth(1)).toContainText("Load unpacked");
  return { banner, steps };
}

test.describe("load-unpacked steps on macOS", () => {
  test.use({ userAgent: macUserAgent });

  test("Set up browser keeps the ⌘⇧G walkthrough and the path in the banner", async ({ page }) => {
    const { banner, steps } = await openSetupSteps(page);
    await expect(steps.nth(2)).toContainText("⌘⇧G");
    await expect(steps.nth(3)).toContainText("do not click any file");

    await banner.getByRole("button", { name: "Dismiss" }).click();
    await expect(banner).toBeHidden();
  });
});

test.describe("load-unpacked steps on Windows", () => {
  test.use({ userAgent: windowsUserAgent });

  test("Set up browser explains the address-bar paste", async ({ page }) => {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "platform", { configurable: true, value: "Win32" });
    });
    const { steps } = await openSetupSteps(page);
    await expect(steps.nth(2)).toContainText("address bar");
    await expect(steps.nth(3)).toContainText("Select Folder");
  });
});

test("browser offline banner stays under Settings in the Escape stack and can retry", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", disconnectedScan(sessionId));

  const banner = page.getByTestId("browser-offline-banner");
  await expect(banner).toBeVisible();

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("button", { name: "Back to app" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: "Back to app" })).toHaveCount(0);
  await expect(banner).toBeVisible();

  await banner.getByRole("button", { name: "Retry after connecting" }).click();
  await expect(banner).toBeHidden();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: "latest rustc version",
  });
});

test("a later connected browser_setup clears the offline banner", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));
  const banner = page.getByTestId("browser-offline-banner");
  await expect(banner).toBeVisible();

  await emitTauriEvent(page, "agent", {
    kind: "ToolResult",
    frame_id: sessionId,
    name: "browser_setup",
    ok: true,
    content: JSON.stringify({ status: "connected", live_retrieval: true, connected_tabs: 1 }),
  });
  await expect(banner).toHaveCount(0);
});

test("a live extension recheck clears a stale offline verdict", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);
  await page.evaluate(() => {
    (window as any).__extensionConnected = true;
  });

  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));

  await expect.poll(() => invokeCount(page, "extension_connected")).toBe(1);
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);
});

test("successful live retrieval survives a stream disconnect error", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));
  await expect(page.getByTestId("browser-offline-banner")).toBeVisible();

  await emitTauriEvent(page, "agent", successfulScan(sessionId));
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);

  await emitTauriEvent(page, "agent", {
    kind: "Error",
    frame_id: sessionId,
    message: "api: 200 stream error: stream disconnected before completion: stream closed before response.completed",
  });
  await expect(page.getByText(/stream disconnected before completion/)).toBeVisible();
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);
});

test("a reconnecting extension after a successful scan keeps the turn marked live", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", successfulScan(sessionId));
  await emitTauriEvent(page, "agent", disconnectedScan(sessionId));
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);
});

test("the offline banner does not carry over to the next turn", async ({ page }) => {
  await enterApp(page);
  const sessionId = await startLiveRetrievalTurn(page);

  await emitTauriEvent(page, "agent", disconnectedSetup(sessionId));
  await expect(page.getByTestId("browser-offline-banner")).toBeVisible();

  await emitTauriEvent(page, "agent", {
    kind: "User",
    frame_id: sessionId,
    text: "read this page for me",
  });
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);
});

test("reopening a session does not revive a stale disconnected presentation", async ({ page }) => {
  await page.goto("/?mockBrowserRestore=1");
  await page.locator(".proj-card-main").first().click();
  const session = page.locator('[data-session-id="browser-restore-session"]');
  await expect(session).toBeVisible();
  await session.click();
  await expect(page.getByText("PubMed currently lists live hits for CLEC12A.")).toBeVisible();
  await expect(page.getByTestId("browser-offline-banner")).toHaveCount(0);
});
