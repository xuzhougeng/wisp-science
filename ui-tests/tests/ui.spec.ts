import { test, expect, type Page } from "@playwright/test";
import { tauriMock, parallelMock } from "./mock-tauri";

function providerSelect(page: Page) {
  return page.getByTestId("settings-provider");
}

async function openModelsSettings(page: Page) {
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Models" }).click();
  const row = page.locator(".settings-list-row").first();
  if (await row.count()) {
    await row.click();
  } else {
    await page.getByRole("button", { name: /Add model/i }).click();
  }
  await expect(providerSelect(page)).toBeVisible();
}

async function openSettingsSection(page: Page, name: string) {
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name, exact: true }).click();
}

// The app now boots to the Projects landing screen; open a real project (not
// the "Example project" card) to reach the chat UI the tests assert against.
async function enterApp(page: Page, path = "/") {
  await page.goto(path);
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
}

function composer(page: Page) {
  return page.locator("#composer-input");
}

function newSessionButton(page: Page) {
  return page.locator(".sidebar").getByRole("button", { name: "New session" });
}

async function openAgentMenu(page: Page) {
  await page.getByRole("button", { name: "Agent options" }).click();
  return page.getByRole("menu", { name: "Agent options" });
}

async function openComputeMenu(page: Page) {
  const agentMenu = await openAgentMenu(page);
  await agentMenu.getByRole("button", { name: /^Compute/ }).click();
  return page.getByRole("menu", { name: "Compute" });
}

async function selectRemoteContext(page: Page) {
  const menu = await openComputeMenu(page);
  const server = menu.locator('[data-context-id="ssh:gpu-server"]');
  if (!(await server.getAttribute("class"))?.includes("enabled")) {
    await server.click();
    await expect(server).toHaveClass(/enabled/);
  }
  await page.keyboard.press("Escape");
}

function commandPalette(page: Page) {
  return page.locator("#command-palette-input");
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

async function invokeArgsList(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    return ((window as any).__skillInvokeLog ?? [])
      .filter((call: any) => call.cmd === name)
      .map((call: any) => plain(call.args));
  }, cmd);
}

async function setMockUpdateCheck(page: Page, value: Record<string, unknown>) {
  await page.evaluate((payload) => {
    (window as any).__setMockUpdateCheck(payload);
  }, value);
}

async function setMockUpdateCheckPending(page: Page, pending: boolean) {
  await page.evaluate((value) => {
    (window as any).__setMockUpdateCheckPending(value);
  }, pending);
}

async function resolveMockUpdateCheck(page: Page) {
  await page.evaluate(() => {
    (window as any).__resolveMockUpdateCheck();
  });
}

test.beforeEach(async ({ page }) => {
  // Install the Tauri bridge mock before the page's wasm runs.
  await page.addInitScript(tauriMock);
});

test("Example project shows bundled demos as read-only transcripts", async ({ page }) => {
  await page.goto("/");
  // The synthetic "Example project" opens a demo view whose sidebar lists the
  // bundled demos (no per-project "Open demo" button any more).
  await page.getByText("Example project").click();
  await page.getByText("Design a genome-wide CRISPR").click();

  // The demo request renders as the user turn…
  await expect(page.getByText("Design a genome-wide CRISPR knockout screen targeting all kinases.")).toBeVisible();
  // …and the agent's final report renders as the assistant turn.
  await expect(page.getByText("Human Kinome CRISPR-KO Screen")).toBeVisible();
});

test("send streams a mocked assistant reply", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  // Deltas "Hello " + "from mock wisp-science." accumulate into one assistant bubble.
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.locator(".msg.assistant").getByRole("button", { name: "Copy" }).click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
});

test("switching HTTP models confirms cache invalidation", async ({ page }) => {
  await enterApp(page);

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  const modal = page.getByTestId("model-switch-confirm");
  await expect(modal).toContainText("invalidates this conversation's model cache");
  await expect(modal).toContainText("opus-4.8");
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toBeNull();

  await modal.getByRole("button", { name: "No", exact: true }).click();
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".model-picker-label")).toHaveText("deepseek-v4-pro");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Yes, switch" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");
});

test("model switch warning can be permanently dismissed", async ({ page }) => {
  await enterApp(page);

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /opus-4\.8/ }).click();
  await page.getByTestId("model-switch-confirm")
    .getByRole("button", { name: "Switch and don't ask again" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-model-switch-warning-disabled")))
    .toBe("1");
  await expect(page.locator(".model-picker-label")).toHaveText("opus-4.8");

  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /deepseek-v4-pro/ }).click();
  await expect(page.getByTestId("model-switch-confirm")).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "default" });
});

test("Settings Models page can open ACP Agents dialog", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await expect(page.getByTestId("models-category-http")).toHaveClass(/active/);
  await page.getByTestId("open-acp-agents-from-settings").click();
  await expect(page.getByTestId("open-acp-agents-from-settings")).toHaveClass(/active/);
  await expect(page.getByTestId("acp-agents-list")).toBeVisible();
  await page.getByTestId("add-acp-agent-settings").click();
  await expect(page.getByTestId("acp-agents-settings")).toBeVisible();
  await expect(page.locator(".settings-breadcrumb")).toContainText(/ACP/);
});

test("ACP Agent settings create, test, and authenticate an installed agent", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByTestId("open-acp-agents-from-settings").click();
  await page.getByTestId("add-acp-agent-settings").click();
  const settings = page.getByTestId("acp-agents-settings");
  await expect(settings).toBeVisible();
  await expect(page.locator(".settings-breadcrumb")).toContainText(/ACP/);
  await settings.getByTestId("acp-agent-label").fill("My ACP");
  await settings.getByTestId("acp-agent-command").fill("my-acp");
  await settings.getByTestId("acp-agent-args").fill("--stdio\n  spaced  \n\n--safe");
  await settings.getByTestId("save-acp-agent").click();
  await expect(page.getByTestId("acp-agents-list")).toBeVisible();
  const row = page.getByTestId("acp-agent-row").filter({ hasText: "My ACP" });
  await expect(row).toBeVisible();
  await row.getByTestId("test-acp-agent").click();
  await expect(row.getByTestId("acp-agent-info")).toContainText("ACP v1");
  await row.getByTestId("authenticate-acp-agent").click();
  await expect.poll(() => lastInvokeArgs(page, "save_acp_agent")).toMatchObject({
    profile: { label: "My ACP", command: "my-acp", args: ["--stdio", "  spaced  ", "", "--safe"] },
  });
  await expect.poll(() => lastInvokeArgs(page, "authenticate_acp_agent")).toMatchObject({ methodId: "browser" });
});

test("selecting an ACP Agent from a populated HTTP session starts a fresh session", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("existing HTTP turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
  const firstSend = await lastInvokeArgs(page, "send_message");
  await composer(page).fill("preserved draft");

  await page.locator(".model-picker-btn").click();
  const agent = page.getByRole("button", { name: /Test ACP Agent/ });
  await expect(agent).toBeEnabled();
  await agent.click();
  await expect(page.locator(".model-picker-label")).toHaveText("Test ACP Agent");
  await expect(composer(page)).toHaveValue("preserved draft");
  await expect(page.locator(".copy-toast")).toContainText(
    "Started a new session because ACP cannot take over existing conversation history",
  );

  await composer(page).fill("continue with ACP");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    acpAgentId: "acp-test",
    message: "continue with ACP",
  });
  const secondSend = await lastInvokeArgs(page, "send_message");
  expect(secondSend.sessionId).not.toBe(firstSend.sessionId);
});

test("ACP turn maps config, overlapping tools, plan, usage, and exact permission response", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP PERMISSION");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Hello from ACP.")).toBeVisible();
  await expect(page.getByTestId("acp-tool")).toHaveCount(2);
  await expect(page.getByText("Inspect")).toBeVisible();
  const config = page.getByTestId("acp-session-config");
  await expect(config).toContainText("Agent");
  await expect(config).toContainText("Smart");
  await config.getByRole("button", { name: "Model" }).click();
  await page.getByRole("option", { name: "Fast" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_config")).toMatchObject({
    configId: "model", value: { value: "fast" },
  });
  // Session mode is now a selector (#247): switching invokes set_acp_session_mode.
  await config.getByRole("button", { name: "Session mode" }).click();
  await page.getByRole("option", { name: "Full Access" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_mode")).toMatchObject({
    modeId: "full-access",
  });
  await expect(config).toContainText("Full Access");

  const permission = page.getByTestId("acp-permission-card");
  await expect(permission).toBeVisible();
  await permission.getByRole("button", { name: "Allow once" }).click();
  await expect.poll(() => lastInvokeArgs(page, "respond_acp_permission")).toMatchObject({
    requestId: "permission-1", optionId: "allow",
  });
  await expect(permission).toHaveCount(0);
  await expect(page.getByText("ACP context: 1200 / 8000 tokens")).toBeVisible();

  await page.locator(".model-picker-btn").click();
  await expect(page.getByRole("button", { name: /deepseek-v4-pro/ })).toBeDisabled();
});

test("ACP turns retain explicitly selected Wisp skills", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("/alpha");
  await page.locator(".mention-menu .mention-item").first().click();
  await composer(page).fill("use this skill");
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    acpAgentId: "acp-test",
    references: [{ kind: "skill", name: "alphafold2" }],
  });
});

test("ACP cancellation is scoped to the active bound frame", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP LONG");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await page.getByRole("button", { name: "Stop" }).click();
  await expect.poll(() => lastInvokeArgs(page, "stop_agent")).toMatchObject({ sessionId: expect.any(String) });
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
});

test("pre-start send failures roll back optimistic rows and restore the draft", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("PRESTARTFAIL retry this draft");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(composer(page)).toHaveValue("PRESTARTFAIL retry this draft");
  await expect(page.locator(".user-bubble")).toHaveCount(0);
});

test("post-start send failures keep the persisted user row and hide the phase prefix", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("POSTSTARTFAIL keep this turn");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".user-bubble")).toContainText("POSTSTARTFAIL keep this turn");
  await expect(page.locator(".finding.err")).toContainText("execution failed after turn/start");
  await expect(page.locator(".finding.err")).not.toContainText("[turn-started]");
  await expect(composer(page)).toHaveValue("");
});

test("automatic reviewer separates the correction and resolves its finding", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEW inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  const assistants = page.locator(".msg.assistant");
  await expect(assistants).toHaveCount(2);
  await expect(assistants.nth(0)).toContainText("5 significant genes");
  await expect(assistants.nth(1)).toContainText("Correction: the analysis found 3 significant genes");

  const handoffs = page.locator(".review-transition");
  await expect(handoffs).toHaveCount(2);
  await expect(handoffs.nth(0)).toContainText("wisp-science nudged Reviewer");
  await expect(handoffs.nth(0)).toHaveAttribute("data-phase", "reviewing");
  await expect(handoffs.nth(1)).toContainText("Reviewer nudged wisp-science");
  await expect(handoffs.nth(1)).toContainText("deepseek-v4-pro");
  await expect(handoffs.nth(1)).toHaveAttribute("data-phase", "correcting");

  const review = page.locator(".review-card");
  await expect(review).toContainText("Reviewer findings");
  await expect(review.locator(".review-model")).toHaveText("claude-sonnet-5 · high");
  await expect(review).toContainText("resolved");
  await expect(review).toContainText("All findings fixed and independently rechecked.");
  await expect(review.locator(".review-finding")).toHaveCount(1);
  await review.getByRole("button", { name: "Go to transcript" }).click();
});

test("automatic reviewer visibly returns a clean response without correction", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWCLEAN inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".msg.assistant")).toHaveCount(1);
  const handoffs = page.locator(".review-transition");
  await expect(handoffs).toHaveCount(2);
  await expect(handoffs.nth(0)).toHaveAttribute("data-phase", "reviewing");
  await expect(handoffs.nth(1)).toContainText("no issues found, please continue");
  await expect(handoffs.nth(1)).toHaveAttribute("data-phase", "passed");
  await expect(page.locator(".review-card")).toContainText("No traceability problems found");
});

test("ACP review with missing tool output is unreviewable instead of passed", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWUNREVIEWABLE inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  const review = page.locator(".review-card");
  await expect(review.locator(".review-unreviewable")).toContainText("Evidence coverage is 0%");
  await expect(review).toContainText("Missing review evidence");
  await expect(review).toContainText("python analysis.py did not persist inspectable output");
  await expect(review.locator(".review-empty")).not.toContainText("No traceability problems found");
  await expect(page.locator('.review-transition[data-phase="passed"]')).toHaveCount(0);
});

test("review backend failures stay visible without failing the primary answer", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("AUTOREVIEWFAIL inspect the result");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".msg.assistant").first()).toContainText("The primary answer still completed");
  await expect(page.locator(".msg.assistant").last()).toContainText(
    "Review failed: ACP reviewer returned invalid JSON",
  );
});

test("assistant markdown table can be copied separately", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("MDTABLE");
  await page.getByRole("button", { name: "Send" }).click();
  const copyButton = page.locator(".msg.assistant .md-table-copy").first();
  await expect(copyButton).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "writeText", {
      configurable: true,
      value: async (text: string) => { (window as any).__copiedTableText = text; },
    });
  });
  await copyButton.click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedTableText)).toBe(
    "Tissue\tTPM\nVeg 0DAF\t2.62\nNotch 0DAF\t1.81",
  );
});

test("dark palettes keep rendered markdown code readable", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await enterApp(page);
  await composer(page).fill("MDCODE");
  await page.getByRole("button", { name: "Send" }).click();

  const highlightedBlocks = page.locator(".msg.assistant .md pre code[data-hl='1']");
  await expect(highlightedBlocks).toHaveCount(3);
  await expect(page.locator(".msg.assistant code.language-text")).toContainText("CAF状态 → 免疫变化");
  await expect(page.locator(".msg.assistant code.language-python .hljs-comment")).toContainText("暗色代码注释");
  await expect(page.locator(".msg.assistant code.language-diff .hljs-addition")).toContainText("CAF状态 → 免疫变化");
  await expect(page.locator(".msg.assistant code.language-diff .hljs-deletion")).toContainText("CAF状态 → 未知");

  const auditContrast = () => page.locator(".msg.assistant .md").evaluate((root) => {
    const channels = (value: string) => (value.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
    const luminance = (value: string) => {
      const rgb = channels(value).map((channel) => channel / 255)
        .map((channel) => channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4);
      return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    };
    const contrast = (foreground: string, background: string) => {
      const foregroundLuminance = luminance(foreground);
      const backgroundLuminance = luminance(background);
      return (Math.max(foregroundLuminance, backgroundLuminance) + 0.05)
        / (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
    };
    const samples = [...root.querySelectorAll("pre code.hljs")].flatMap((code) => {
      const preBackground = getComputedStyle(code.closest("pre")!).backgroundColor;
      return [code, ...code.querySelectorAll("span")]
        .filter((element) => element.textContent?.trim())
        .map((element) => {
          const style = getComputedStyle(element);
          const background = style.backgroundColor === "rgba(0, 0, 0, 0)"
            ? preBackground
            : style.backgroundColor;
          return {
            text: element.textContent?.trim().slice(0, 40),
            color: style.color,
            background,
            ratio: contrast(style.color, background),
          };
        });
    });
    return {
      minimum: Math.min(...samples.map((sample) => sample.ratio)),
      samples,
    };
  });

  await openSettingsSection(page, "Appearance");
  await page.getByTestId("theme-mode-dark").click();
  const paletteSelect = page.getByTestId("appearance-palette-select");
  for (const palette of ["charcoal", "codex", "github", "catppuccin", "gruvbox"]) {
    await paletteSelect.selectOption(palette);
    await expect(page.locator("html")).toHaveAttribute("data-dark-palette", palette);
    const audit = await auditContrast();
    expect(audit.minimum, `${palette}: ${JSON.stringify(audit.samples)}`).toBeGreaterThanOrEqual(4.5);
  }

  await page.getByTestId("theme-mode-system").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "system");
  const systemAudit = await auditContrast();
  expect(systemAudit.minimum, `system dark: ${JSON.stringify(systemAudit.samples)}`).toBeGreaterThanOrEqual(4.5);
});

test("composer @ # and / add typed context references", async ({ page }) => {
  await enterApp(page);
  const composerInput = composer(page);

  await composerInput.fill("@");
  await expect(page.locator(".mention-menu")).toContainText("nif3.treefile");
  await page.locator(".mention-menu .mention-item").first().click();

  await composerInput.fill("#");
  await expect(page.locator(".mention-menu")).toContainText("Older structure run");
  await page.locator(".mention-menu .mention-item").first().click();

  await composerInput.fill("/alpha");
  await expect(page.locator(".mention-menu")).toContainText("alphafold2");
  await page.locator(".mention-menu .mention-item").first().click();

  await composerInput.fill("use the attached context");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    references: [
      { kind: "artifact", id: "art-tree" },
      { kind: "session", id: "s-current" },
      { kind: "skill", name: "alphafold2" },
    ],
  });
  const sentContext = page.locator(".msg.user .user-context-card");
  await expect(sentContext).toHaveCount(3);
  await expect(page.locator('.msg.user [data-reference-kind="artifact"]')).toContainText("nif3.treefile");
  await expect(page.locator('.msg.user [data-reference-kind="session"]')).toContainText("Current analysis");
  await expect(page.locator('.msg.user [data-reference-kind="skill"]')).toContainText("alphafold2");
  await expect(page.locator(".msg.user .body")).not.toContainText("Selected skills:");
});

test("Ctrl+K opens the unified command palette and Shift+Enter attaches", async ({ page }) => {
  await enterApp(page);
  await page.keyboard.press("Control+k");
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await expect(search).toHaveAttribute("type", "text");
  await expect(search).toHaveAttribute("inputmode", "search");
  await expect(search).toHaveAttribute("autocomplete", "off");
  const paletteRows = page.locator(".project-search-overlay .project-search-row");
  await expect(paletteRows.first()).toBeVisible();
  // Session glyphs use `.gi.bubble` — `.gi.chat` collides with the main `.chat` scroller
  // (`flex: 1 1 auto`) and stretches the icon, shoving labels to the right.
  await expect(page.locator(".project-search-overlay .gi.chat")).toHaveCount(0);
  const sessionIcon = page.locator(".project-search-overlay .gi.bubble").first();
  if (await sessionIcon.count()) {
    const box = await sessionIcon.boundingBox();
    expect(box?.width ?? 0).toBeLessThanOrEqual(24);
  }
  await search.press("ArrowDown");
  await expect(paletteRows.nth(1)).toHaveClass(/active/);
  await search.fill("counts");
  await expect(page.locator(".project-search-row").filter({ hasText: "counts.csv" })).toBeVisible();
  await search.press("Shift+Enter");
  await expect(search).not.toBeVisible();
  await expect(page.locator(".composer-reference-chips")).toContainText(/counts\.csv|Cross-project counts/);
});

test("Cmd+K opens search and the composer shows the macOS shortcut", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "wisp-science/Tauri",
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
  });
  await enterApp(page);
  await expect(composer(page)).toHaveAttribute("placeholder", /Cmd\+K/);
  await page.keyboard.press("Meta+k");
  await expect(commandPalette(page)).toBeVisible();
});

test("Ctrl+P command palette runs commands and switches themes", async ({ page }) => {
  await enterApp(page);
  await page.keyboard.press("Control+p");
  const palette = page.getByRole("dialog", { name: "Command Palette" });
  const input = page.locator("#action-palette-input");
  await expect(palette).toBeVisible();
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute("type", "text");
  await expect(input).toHaveAttribute("inputmode", "search");
  await expect(input).toHaveAttribute("autocomplete", "off");
  await expect(palette).toContainText("New session");

  const rows = palette.locator(".project-search-row");
  await expect(rows.first()).toHaveClass(/active/);
  await input.press("ArrowDown");
  await expect(rows.nth(1)).toHaveClass(/active/);
  await expect(rows.nth(1)).toBeInViewport();
  // Arrow past the fold must keep the active row visible (same as Ctrl+K).
  for (let i = 0; i < 12; i++) await input.press("ArrowDown");
  await expect(palette.locator(".project-search-row.active")).toBeInViewport();
  await input.press("ArrowUp");
  await expect(palette.locator(".project-search-row.active")).toBeInViewport();

  // Typing must keep focus in the input; otherwise arrow keys hit the page behind.
  await input.fill("d");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("d");
  await page.keyboard.press("ArrowDown");
  await expect(input).toBeFocused();
  await expect(palette.locator(".project-search-row.active")).toBeVisible();
  await input.fill("");

  await input.press("ArrowDown");
  await input.press("Enter");
  await expect(page.getByPlaceholder("Search this project…")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+k");
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await search.pressSequentially("c");
  await expect(search).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(search).toBeFocused();
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+p");
  await input.fill("dark theme");
  await input.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect.poll(() => page.evaluate(() => localStorage.getItem("wisp-theme"))).toBe("dark");

  await page.keyboard.press("Control+p");
  await input.fill("open files");
  await input.press("Enter");
  await expect(page.locator(".rp-files")).toBeVisible();

  await page.keyboard.press("Control+p");
  await input.fill("system theme");
  await input.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "system");

  await page.keyboard.press("Control+b");
  await expect(page.locator(".sidebar")).toHaveClass(/collapsed/);
  await page.keyboard.press("Control+,");
  await expect(page.locator(".settings-page")).toBeVisible();
  await page.keyboard.press("Escape");
  const before = await page.evaluate(() => ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "new_session").length);
  await page.keyboard.press("Control+n");
  await expect.poll(() => page.evaluate(() => ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "new_session").length)).toBeGreaterThan(before);
});

test("new session focuses the composer", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await expect(composer(page)).toBeFocused();
});

test("rename session modal autofocuses so Ctrl+A selects the title", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("rename-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:rename-me")).toBeVisible({ timeout: 10_000 });

  await page.locator(".side-item.ses", { hasText: "rename-me" }).dblclick();
  const input = page.locator("#rename-session-input");
  await expect(input).toBeVisible();
  await expect(input).toBeFocused();
  await expect.poll(async () => input.evaluate((el: HTMLInputElement) =>
    el.selectionStart === 0 && el.selectionEnd === el.value.length && el.value.length > 0
  )).toBe(true);

  // Even after clearing selection, Ctrl+A must stay inside the field.
  await input.evaluate((el: HTMLInputElement) => el.setSelectionRange(0, 0));
  await page.keyboard.press("Control+a");
  await expect.poll(async () => input.evaluate((el: HTMLInputElement) =>
    el.selectionStart === 0 && el.selectionEnd === el.value.length
  )).toBe(true);
});

test("conversation action button renames, transfers, and deletes sessions", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("actions-manage-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:actions-manage-me")).toBeVisible({ timeout: 10_000 });
  let session = page.locator(".side-item.ses", { hasText: "actions-manage-me" });
  await expect(session).toBeVisible({ timeout: 10_000 });

  const openActions = async () => {
    const row = session.locator("..");
    const actions = row.getByRole("button", { name: "Conversation actions" });
    await expect.poll(() => actions.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
    await actions.click();
  };

  await openActions();
  await expect.poll(() => page.locator(".ctx-menu").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight;
  })).toBe(true);
  await expect(page.getByRole("button", { name: "Rename", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy to another project…", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Move to another project…", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Rename", exact: true }).click();
  const renameInput = page.locator("#rename-session-input");
  await renameInput.fill("Managed analysis");
  await page.locator(".modal", { has: renameInput }).getByRole("button", { name: "Save" }).click();
  session = page.locator(".side-item.ses", { hasText: "Managed analysis" });
  await expect(session).toBeVisible();

  await openActions();
  await page.getByRole("button", { name: "Copy to another project…", exact: true }).click();
  let transfer = page.locator(".session-transfer-modal");
  await expect(transfer.locator("select")).toHaveValue("other");
  await transfer.getByRole("button", { name: "Copy", exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "transfer_session_to_project");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ targetProjectId: "other", mode: "copy" });
  await expect(session).toBeVisible();

  await openActions();
  await page.getByRole("button", { name: "Move to another project…", exact: true }).click();
  transfer = page.locator(".session-transfer-modal");
  await transfer.getByRole("button", { name: "Move", exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((call: any) => call.cmd === "transfer_session_to_project");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ targetProjectId: "other", mode: "move" });
  await expect(session).toHaveCount(0);

  await newSessionButton(page).click();
  await composer(page).fill("actions-delete-me");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:actions-delete-me")).toBeVisible({ timeout: 10_000 });
  session = page.locator(".side-item.ses", { hasText: "actions-delete-me" });
  await expect(session).toBeVisible({ timeout: 10_000 });
  await openActions();
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(session).toHaveCount(0);
  await expect.poll(() => page.evaluate(() =>
    ((window as any).__sendInvokeLog ?? []).some((call: any) => call.cmd === "delete_session")
  )).toBe(true);
});

test("folder action button visibly renames and deletes folders", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();

  await page.getByRole("button", { name: "New folder" }).click();
  const folderInput = page.locator("#folder-modal-input");
  await folderInput.fill("Figures");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();

  let folder = page.locator(".side-folder", { hasText: "Figures" });
  await expect(folder).toBeVisible();
  let actions = folder.getByRole("button", { name: "Folder actions" });
  await expect.poll(() => actions.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
  await actions.click();
  await page.getByRole("button", { name: "Rename folder" }).click();
  await folderInput.fill("Results");
  await page.locator(".modal", { has: folderInput }).getByRole("button", { name: "Save" }).click();

  folder = page.locator(".side-folder", { hasText: "Results" });
  await expect(folder).toBeVisible();
  actions = folder.getByRole("button", { name: "Folder actions" });
  await actions.click();
  await page.getByRole("button", { name: "Delete folder" }).click();
  await page.locator(".confirm-modal").getByRole("button", { name: "Delete folder", exact: true }).click();
  await expect(folder).toHaveCount(0);
});

test("user message renders before a delayed backend User event", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("DELAYUSER");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.locator(".user-bubble .body", { hasText: /^DELAYUSER$/ })).toBeVisible({ timeout: 500 });
  await expect(page.getByText("delayed reply")).toBeVisible({ timeout: 3_000 });
  await expect(page.locator(".user-bubble .body", { hasText: /^DELAYUSER$/ })).toHaveCount(1);
});

test("long unbroken user text wraps inside the chat column", async ({ page }) => {
  await enterApp(page);
  const seq = `${"MVGCHEQEAPSETTASSSSFERELVTGSSCVIDADANYSEMAVSDTAAGLTAPTARQRVSDEGKKPGPSSQHRPSPDRNYSQAVSENLQAVTSSSSEHRGISRIVQQQQPGQPFHRRHTTGATSPAMGTAEAAAVAAAASSSSAEEAALDVDCVEGHDEGLHSGREIPRCGLDNLDSSPDCGRHDASQGNSRHTCKVCKRPFSSGRALGGHMRAHGNGDPGTSSNADRKSEKQLISSSPRTQQASLHACNGVAENGIEHPGADGVARAQSLSPESRARARTREIQVRRAVGARRSKTNGKRRGSTTPKSSVEDAAALTKQQPHDEDDNAASRRQAERSSTSCSDNNSDGAHDDGAATDDAAGNICDVCREEFENEKQLNTHKKSHKPEYNLRECPRKSRRFIDQDYTEVAPPTIPTKKPPAPQEKQQSDSGCPYPGCTKKFHSSKALFGHMRCHPDRTWRGIHPPDENGASTSAAGERQHRRKKSRPNSHVPARVVSDSESEPEQKQSGKSASTEHESDTDSIEAAYIQGQEAHTNGDRQQSSTPGWWASGVTGKRSKRSRQTVRSLQAVHHGASTSSAAAPDNALEELNETAMVMMMLASNPSGAPKHEDPDEHMEDLFRNPNSADECPKDEPTEGCLEAALRAKDEEEDEEDEEEDKEEEGEDGDEKQGAAAATAAEVVEDLEQGPELVPKDEFMTAAAETAEVPMEVDEEPEASLSEDGVLQGEEAVQLEAGQQEASSSKHGQALGGHKRCHFDPTKKDAEKEGSSSNNGGKNPRSSNPAGRASYSQSRGRHESSDARGHSPRAKSDPGLQQQQQQQAAAPAESRSTGLLRPIEIDLNKPPTVTYDEEMEMAPSPASAKFSVENHEAQASASAEASSSPDDGEPMRNQPRDYQLILHLSPITLNLEDQLHAYYKRVTPA".repeat(2)} find homolog`;
  await composer(page).fill(seq);
  await page.getByRole("button", { name: "Send" }).click();
  const bubble = page.locator(".msg.user .body").first();
  await expect(bubble).toBeVisible({ timeout: 10_000 });
  const { bubbleWidth, threadWidth, scrollWidth, clientWidth } = await page.evaluate(() => {
    const body = document.querySelector(".msg.user .body") as HTMLElement | null;
    const thread = document.querySelector(".thread") as HTMLElement | null;
    const chat = document.querySelector(".chat") as HTMLElement | null;
    return {
      bubbleWidth: body?.getBoundingClientRect().width ?? 0,
      threadWidth: thread?.getBoundingClientRect().width ?? 0,
      scrollWidth: chat?.scrollWidth ?? 0,
      clientWidth: chat?.clientWidth ?? 0,
    };
  });
  expect(bubbleWidth).toBeGreaterThan(0);
  expect(bubbleWidth).toBeLessThanOrEqual(threadWidth + 1);
  // Column must not grow a horizontal scrollbar from the unbroken sequence.
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
});

test("side chat answers in a temporary side panel and can switch model", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("what did the main thread miss?");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Side chat" }).click();

  const panel = page.locator(".rightpane");
  await expect(panel).toBeVisible();
  await expect(panel.locator(".sidechat-in-pane")).toBeVisible();
  await expect(panel.getByText("Side answer: what did the main thread miss?")).toBeVisible();
  await expect(panel).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const closeBox = await panel.getByRole("button", { name: "Close tab" }).first().boundingBox();
  const panelBox = await panel.boundingBox();
  expect(closeBox && panelBox && closeBox.x + closeBox.width <= panelBox.x + panelBox.width).toBeTruthy();
  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: "what did the main thread miss?",
  });
  await expect.poll(async () => {
    const args = await lastInvokeArgs(page, "send_message");
    return args?.message ?? null;
  }).toBeNull();

  await panel.getByRole("button", { name: /deepseek-v4-pro/ }).click();
  await panel.getByRole("button", { name: "opus-4.8" }).click();
  await expect(panel.getByRole("button", { name: /opus-4.8/ })).toBeVisible();

  // Side chat can route through an ACP Agent (#250).
  await panel.getByRole("button", { name: /opus-4.8/ }).click();
  await panel.getByRole("button", { name: "Test ACP Agent" }).click();
  await expect(panel.getByRole("button", { name: /Test ACP Agent/ })).toBeVisible();
  await panel.getByPlaceholder("Follow up…").fill("acp side question");
  await panel.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "side_chat")).toMatchObject({
    question: "acp side question", acpAgentId: "acp-test",
  });
});

test("branch in new session starts a new frame from the current session", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("seed context");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("try another route");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Branch in new session" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    title: "try another route",
  });
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.stringMatching(/^branch-/),
    message: "try another route",
  });
});

test("branch on an earlier user message opens a new session from that point", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("first idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("second idea");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toBeVisible();

  const firstUser = page.locator(".msg.user", { hasText: "first idea" });
  await firstUser.getByRole("button", { name: "Branch" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    sessionId: expect.stringMatching(/^s-/),
    title: "first idea",
    userIndex: 0,
  });
  await expect(composer(page)).toHaveValue("first idea");
  await expect(page.locator(".msg.user", { hasText: "second idea" })).toHaveCount(0);

  await composer(page).fill("first idea, but normalize first");
  await page.getByRole("button", { name: "Send" }).click();
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    sessionId: expect.stringMatching(/^branch-/),
    message: "first idea, but normalize first",
  });
});

test("generic content menus do not expose session export", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByText("Hello from mock wisp-science.").click({ button: "right" });
  await expect(page.getByRole("button", { name: "Export session" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator(".rightpane").click({ button: "right", position: { x: 5, y: 100 } });
  await expect(page.locator(".ctx-menu")).toHaveCount(0);
});

test("uploaded file shows up in the artifacts panel after send", async ({ page }) => {
  await enterApp(page);
  await page.setInputFiles("#composer-file-input", {
    name: "counts.csv",
    mimeType: "text/csv",
    buffer: Buffer.from("a,b\n1,2"),
  });
  await expect(page.locator(".composer-attachment.ready")).toHaveText("counts.csv");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toMatchObject({
    message: "Uploaded files: uploads/counts.csv",
    attachments: ["uploads/counts.csv"],
  });
  // One user bubble only — attachment suffix must not spawn a duplicate turn.
  await expect(page.locator(".msg.user")).toHaveCount(1);
  await expect(page.locator(".msg.user .user-attachment-file")).toContainText("counts.csv");
  await expect(page.locator(".msg.user")).not.toContainText("Uploaded files:");
  await expect(page.locator(".center-tab.active")).not.toContainText("Uploaded files:");
  // The right panel starts collapsed; open it to see the collected artifact.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // The upload path lives in the user turn; the panel must pick it up from there.
  const tile = page.locator('.rp-tile[data-artifact-name="counts.csv"]');
  await expect(tile).toBeVisible();
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("counts.csv");
  await expect(page.locator(".center-file-preview")).toContainText("a");
  await page.locator(".center-tabs > .center-tab").click();
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({ path: "uploads/counts.csv" });
});

test("artifact category headers collapse and expand their tiles", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const tile = page.locator('.rp-tile[data-artifact-name="volcano.png"]');
  await expect(tile).toBeVisible();
  const group = page.locator(".rp-art-group").filter({ has: tile });
  const header = group.locator(".rp-art-group-label");
  await expect(header).toHaveAttribute("aria-expanded", "true");

  await header.click();
  await expect(group).toHaveClass(/collapsed/);
  await expect(header).toHaveAttribute("aria-expanded", "false");
  await expect(tile).toBeHidden();

  await header.click();
  await expect(group).not.toHaveClass(/collapsed/);
  await expect(header).toHaveAttribute("aria-expanded", "true");
  await expect(tile).toBeVisible();
});

test("dropped local file uploads and attaches to the composer", async ({ page }) => {
  await enterApp(page);
  await page.locator(".composer-inner").evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File(["gene,value\nBRCA1,2"], "dropped.csv", { type: "text/csv" }));
    el.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
    el.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
  });
  await expect(page.locator(".composer-attachment.ready")).toHaveText("dropped.csv");
});

test("workspace file context menu attaches its path to the composer", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  const file = page.locator('.fb-row[data-workspace-path="report.csv"]');
  await expect(file).toBeVisible();
  const json = page.locator('.fb-row[data-workspace-path="config.json"]');
  await json.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview .rp-code")).toBeVisible();
  await expect(page.locator(".center-file-preview")).toContainText('"model"');
  await page.locator('.center-tab[data-center-path="config.json"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close current" }).click();
  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-file-preview")).toContainText("a");
  await expect(page.locator(".center-tab.active")).toContainText("report.csv");

  const search = page.locator(".fb-search");
  await search.fill("counts");
  const counts = page.locator('.fb-row[data-workspace-path="counts.csv"]');
  await expect(counts).toBeVisible();
  await counts.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await page.locator('.center-tab[data-center-path="report.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close tabs to the right" }).click();
  await expect(page.locator('.center-tab[data-center-path="counts.csv"]')).toHaveCount(0);
  await page.locator('.center-tab[data-center-path="report.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close current" }).click();
  await expect(page.locator(".center-file-preview")).toHaveCount(0);

  await counts.click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  await page.locator('.center-tab[data-center-path="counts.csv"]').click({ button: "right" });
  await page.getByRole("button", { name: "Close all files" }).click();
  await expect(page.locator('.center-tab[data-center-path]')).toHaveCount(0);
  await expect(composer(page)).toBeVisible();
  await search.fill("");
  await expect(file).toBeVisible();
  await file.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({ path: "report.csv" });
  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Attach to chat" }).click();
  await expect(page.locator(".composer-attachment.ready")).toHaveText("report.csv");
  await expect(composer(page)).toHaveValue("");
});

test("text-entry context menu pastes into the field that was clicked", async ({ page }) => {
  await page.addInitScript(() => {
    let clipboardText = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => clipboardText,
        writeText: async (value: string) => { clipboardText = value; },
      },
    });
  });
  await enterApp(page);
  await page.locator(".proj-switch").click();
  await page.getByRole("button", { name: "Project settings" }).click();

  const modal = page.locator(".proj-settings-modal");
  const name = modal.locator("input").first();
  const description = modal.locator("textarea").first();
  await description.fill("");
  await page.evaluate(() => navigator.clipboard.writeText("context-target"));
  await description.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Paste" }).click();
  await expect(description).toHaveValue("context-target");
  await expect(name).not.toHaveValue("context-target");

  await name.fill("");
  await page.evaluate(() => navigator.clipboard.writeText("name-target"));
  await name.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Paste" }).click();
  await expect(name).toHaveValue("name-target");
  await expect(description).toHaveValue("context-target");
});

test("center structure and FASTA previews fill the available height", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1200 });
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  const openInCenter = async (path: string) => {
    await page.locator(`[data-workspace-path="${path}"]`).click({ button: "right" });
    await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  };
  const heightRatio = (selector: string) => page.locator(".center-file-preview").evaluate((preview, childSelector) => {
    const child = preview.querySelector<HTMLElement>(childSelector);
    return child ? child.getBoundingClientRect().height / preview.getBoundingClientRect().height : 0;
  }, selector);

  await openInCenter("model.pdb");
  await expect(page.locator('.center-file-preview[data-preview-kind="structure"] .rp-3dmol')).toBeVisible();
  await expect(page.locator('.center-file-preview[data-preview-kind="structure"] .rp-3dmol canvas')).toBeVisible();
  await expect.poll(() => heightRatio(".rp-3dmol")).toBeGreaterThan(0.75);

  await openInCenter("sequences.fasta");
  await expect(page.locator('.center-file-preview[data-preview-kind="fasta"] .rp-fasta-wrap')).toBeVisible();
  await expect.poll(() => heightRatio(".rp-fasta-wrap")).toBeGreaterThan(0.75);
});

test("script previews show source while unknown file types are explicitly unsupported", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  const openInCenter = async (path: string) => {
    await page.locator(`[data-workspace-path="${path}"]`).click({ button: "right" });
    await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  };

  await openInCenter("analysis.R");
  await expect(page.locator(".center-file-preview")).toContainText("plot(1:3)");
  await expect.poll(() => lastInvokeArgs(page, "read_file")).toMatchObject({ path: "analysis.R" });

  // #307: the script rendered as one unhighlighted paragraph. It must come back
  // as R-tagged code, one line per line, with a matching line-number gutter.
  const rCode = page.locator(".center-file-preview .rp-code-body code");
  await expect(rCode).toHaveClass(/language-r/);
  await expect(rCode.locator(".hljs-string")).toHaveText('"data"');
  await expect(page.locator(".center-file-preview .rp-code-gutter")).toHaveText("1\n2\n3\n4");

  // An extension no mime claims (#307: pixi.toml) is still text — preview it.
  await openInCenter("pixi.toml");
  await expect(page.locator(".center-file-preview .rp-code-body code")).toHaveClass(/language-ini/);
  await expect(page.locator(".center-file-preview")).toContainText("[project]");

  // Genuinely binary payloads stay explicitly unsupported.
  await openInCenter("analysis.unknown");
  await expect(page.locator(".center-file-preview .rp-error")).toHaveText(
    "Preview is not supported for this file type.",
  );
});

test("Files browses registered SSH contexts without a real remote host", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();

  await page.getByRole("combobox", { name: "File location" }).selectOption("ssh:gpu-server");
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");
  await expect(page.locator('.remote-dir[data-remote-path="/home/research/projects"]')).toBeVisible();
  const remoteFile = page.locator('.remote-file[data-remote-path="/home/research/notes.txt"]');
  await expect(remoteFile).toContainText("notes.txt");
  await expect.poll(() => lastInvokeArgs(page, "list_remote_dir")).toMatchObject({
    contextId: "ssh:gpu-server",
    path: "~",
  });

  const remoteDownload = remoteFile.getByRole("button", { name: "Download" });
  await expect(remoteDownload).toBeVisible();
  await remoteDownload.click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({
    path: "ssh://gpu-server/home/research/notes.txt",
  });

  // Keep secondary-click as an alternate path, but it is no longer the only one.
  await remoteFile.click({ button: "right" });
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Download" })).toBeVisible();
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Open in center" })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.locator('.remote-dir[data-remote-path="/home/research/projects"]').click();
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research/projects");
  await expect(page.locator('.remote-file[data-remote-path="/home/research/projects/README.md"]')).toBeVisible();

  await page.getByRole("button", { name: "Parent directory" }).click();
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");
});

test("pasted image attaches to the composer", async ({ page }) => {
  await enterApp(page);
  await composer(page).evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File([new Uint8Array([137, 80, 78, 71])], "clipboard.png", { type: "image/png" }));
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", { value: data });
    el.dispatchEvent(event);
  });

  await expect(page.locator(".composer-attachment.ready")).toHaveText(/pasted_image_\d+_1\.png/);
  await expect(page.locator(".composer-attachment-row.image img")).toBeVisible();
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toMatchObject({
    message: expect.stringMatching(/^Uploaded files: uploads\/pasted_image_\d+_1\.png$/),
    attachments: [expect.stringMatching(/^uploads\/pasted_image_\d+_1\.png$/)],
  });
  await expect(page.locator(".msg.user .user-attachment-image img")).toBeVisible();
  await expect(page.locator(".msg.user")).not.toContainText("Uploaded files:");
});

test("compute menu selects remote resources per session", async ({ page }) => {
  await enterApp(page);

  const menu = await openComputeMenu(page);
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("button", { name: "Local", exact: true })).toHaveCount(0);
  const search = menu.getByRole("searchbox", { name: "Search servers" });
  await search.fill("missing");
  await expect(menu.locator('[data-context-id="ssh:gpu-server"]')).toHaveCount(0);
  await search.fill("gpu");
  const server = menu.locator('[data-context-id="ssh:gpu-server"]');
  await expect(menu.locator(".compute-resource-list")).toHaveCSS("overflow-y", "auto");
  await expect(server).toHaveCSS("display", "grid");
  await expect(menu.getByRole("button", { name: "Manage environments in Settings" })).toBeVisible();
  await expect(server).not.toHaveClass(/enabled/);
  await server.click();
  await expect.poll(() => lastInvokeArgs(page, "set_session_execution_context_enabled")).toMatchObject({
    sessionId: expect.any(String),
    contextId: "ssh:gpu-server",
    enabled: true,
  });
  const firstSession = (await lastInvokeArgs(page, "set_session_execution_context_enabled")).sessionId;
  await expect(page.locator(".composer-compute")).toHaveClass(/has-resource/);

  await page.keyboard.press("Escape");
  await newSessionButton(page).click();
  const nextMenu = await openComputeMenu(page);
  await expect(nextMenu.locator('[data-context-id="ssh:gpu-server"]')).not.toHaveClass(/enabled/);
  await expect.poll(async () => (await lastInvokeArgs(page, "list_session_execution_context_ids"))?.sessionId)
    .not.toBe(firstSession);
});

test("settings manages servers and probes them with the default environment skill", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  const server = page.locator('.environment-settings-row[data-context-id="ssh:gpu-server"]');
  const local = page.locator('.environment-settings-row[data-context-id="local"]');
  await expect(server).toBeVisible();
  await expect(local).toBeVisible();
  await expect(page.locator(".environment-resource-toggle")).toHaveCount(0);
  const rowHeights = await page.locator(".environment-settings-row").evaluateAll((rows) =>
    rows.map((row) => row.getBoundingClientRect().height),
  );
  expect(Math.max(...rowHeights) - Math.min(...rowHeights)).toBeLessThanOrEqual(1);
  const [localConfigure, serverConfigure] = await Promise.all([
    local.getByRole("button", { name: "Configure runtime interpreters" }).boundingBox(),
    server.getByRole("button", { name: "Configure runtime interpreters" }).boundingBox(),
  ]);
  expect(localConfigure?.x).toBe(serverConfigure?.x);

  await local.getByRole("button", { name: "Configure runtime interpreters" }).click();
  await expect(page.getByRole("heading", { name: "Runtime interpreters" })).toBeVisible();
  await expect(page.locator("#runtime-python-executable")).toBeVisible();
  await expect(page.locator("#runtime-rscript-executable")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toBeVisible();

  await server.getByRole("button", { name: "Probe context" }).click();
  await expect.poll(() => lastInvokeArgs(page, "probe_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
});

test("Escape closes the topmost environment modal before settings", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Environments");
  await page.getByRole("button", { name: "Add SSH host" }).click();
  await expect(page.locator(".host-modal")).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.locator(".host-modal")).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("Escape closes the compute resource menu", async ({ page }) => {
  await enterApp(page);
  await openComputeMenu(page);
  await expect(page.getByRole("menu", { name: "Compute" })).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(page.getByRole("menu", { name: "Compute" })).toHaveCount(0);
  await expect(page.getByRole("menu", { name: "Agent options" })).toHaveCount(0);
});

test("agent menu updates review, reviewer model, and memory preferences", async ({ page }) => {
  await enterApp(page);
  let menu = await openAgentMenu(page);

  await menu.locator("label.agent-menu-row", { hasText: "Auto-review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_auto_review_enabled")).toMatchObject({ enabled: true });

  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "opus-4.8" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      model_id: "opus",
      review_backend: { kind: "http_model", profile_id: "opus" },
    },
  });
  menu = await openAgentMenu(page);
  await expect(menu.getByRole("button", { name: /Reviewer model opus-4\.8/ })).toBeVisible();
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  const reviewerMenu = page.getByRole("menu", { name: "Reviewer model" });
  await expect(reviewerMenu).toBeVisible();
  await expect.poll(async () => {
    const [mainBox, reviewerBox] = await Promise.all([menu.boundingBox(), reviewerMenu.boundingBox()]);
    return mainBox && reviewerBox ? Math.round(reviewerBox.x - (mainBox.x + mainBox.width)) : null;
  }).toBeGreaterThan(5);
  await reviewerMenu.getByRole("button", { name: /Test ACP Agent/ }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  menu = await openAgentMenu(page);
  await expect(menu.getByRole("button", { name: /Reviewer model Test ACP Agent/ })).toBeVisible();
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "Follow session backend" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: { id: "reviewer", review_backend: { kind: "follow_session" } },
  });
  menu = await openAgentMenu(page);

  await menu.locator("label.agent-menu-row", { hasText: "Memory" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_memory_enabled")).toMatchObject({ enabled: false });

  await menu.locator("label.agent-menu-row", { hasText: "Auto-review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_auto_review_enabled")).toMatchObject({ enabled: false });
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await page.getByRole("menu", { name: "Reviewer model" })
    .getByRole("button", { name: "Default" }).click();
  menu = await openAgentMenu(page);
  await menu.getByRole("button", { name: /^Reviewer model/ }).click();
  await expect(page.getByRole("menu", { name: "Reviewer model" })).toBeVisible();
});

test("right panel shows execution contexts and runs", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await expect.poll(() => page.locator(".rp-tab-add-menu").evaluate((menu) => {
    const rect = menu.getBoundingClientRect();
    const hit = document.elementFromPoint(rect.left + 8, rect.top + 8);
    return hit === menu || menu.contains(hit);
  })).toBe(true);
  await page.getByRole("button", { name: "Environment" }).click();

  await expect(page.locator(".context-card", { hasText: "local" })).toBeVisible();
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toContainText("NVIDIA A100");
  const sshContext = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await sshContext.getByRole("button", { name: "Probe context" }).click();
  await expect.poll(() => lastInvokeArgs(page, "probe_execution_context")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  await sshContext.getByRole("button", { name: "Open terminal" }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_terminal")).toMatchObject({
    contextId: "ssh:gpu-server",
  });
  const terminalDock = page.getByTestId("terminal-dock");
  await expect(terminalDock).toBeVisible();
  await expect(terminalDock).toContainText("ssh:gpu-server — Terminal");
  await expect(terminalDock.locator("iframe")).toHaveCount(0);
  const firstTerminal = terminalDock.locator('.terminal-dock-frame[data-terminal-session="terminal-mock-1"]');
  await expect(firstTerminal).toHaveClass(/active/);
  await expect(firstTerminal.locator(".xterm-rows")).toContainText("terminal ready");
  await expect.poll(() => firstTerminal.locator(".xterm-viewport").evaluate((viewport) => ({
    standardWidth: getComputedStyle(viewport).scrollbarWidth,
    themedWidth: getComputedStyle(viewport, "::-webkit-scrollbar").width,
    thumbInset: getComputedStyle(viewport, "::-webkit-scrollbar-thumb").borderTopWidth,
  }))).toEqual({ standardWidth: "auto", themedWidth: "10px", thumbInset: "2px" });
  await expect.poll(async () => (await invokeArgsList(page, "resize_terminal")).some((args: any) =>
    args.sessionId === "terminal-mock-1" && args.rows > 0 && args.cols > 0,
  )).toBe(true);

  await firstTerminal.click();
  await page.keyboard.type("echo hello");
  await expect.poll(async () => (await invokeArgsList(page, "write_terminal"))
    .filter((args: any) => args.sessionId === "terminal-mock-1")
    .map((args: any) => args.data)
    .join(""),
  ).toContain("echo hello");

  await terminalDock.getByRole("button", { name: "New terminal" }).click();
  await terminalDock.getByRole("button", { name: /Local machine/ }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_terminal")).toMatchObject({
    contextId: "local",
  });
  await expect(terminalDock.getByRole("tab")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame")).toHaveCount(2);
  await expect(terminalDock.locator(".terminal-dock-frame.active"))
    .toHaveAttribute("data-terminal-session", "terminal-mock-2");

  await terminalDock.getByRole("tab", { name: "ssh:gpu-server — Terminal" }).click();
  await expect(terminalDock.locator(".terminal-dock-frame.active"))
    .toHaveAttribute("data-terminal-session", "terminal-mock-1");
  await terminalDock.getByRole("button", { name: "Close terminal panel" }).click();
  await expect(terminalDock).toBeHidden();
  await sshContext.getByRole("button", { name: "Open terminal" }).click();
  await expect(terminalDock).toBeVisible();
  await expect(terminalDock.getByRole("tab")).toHaveCount(3);
  await expect(firstTerminal.locator(".xterm-rows")).toContainText("terminal ready");
  await terminalDock.getByRole("button", { name: "Terminate" }).click();
  await expect.poll(() => lastInvokeArgs(page, "terminate_terminal")).toMatchObject({
    sessionId: "terminal-mock-3",
  });
  await expect(terminalDock.getByRole("button", { name: "Terminate" })).toBeDisabled();
  await sshContext.getByRole("button", { name: "View runs" }).click();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("succeeded");
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("ssh:gpu-server");
  await expect(page.locator(".run-card", { hasText: "Local normalization" })).toHaveCount(0);
  const remoteRun = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await expect(remoteRun).toContainText("~/.wisp-science/runs/run-kinase-001");
  await remoteRun.getByText("Latest output").click();
  await expect(remoteRun).toContainText("wrote qc table");

  await page.getByRole("button", { name: "Refresh runs" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "list_runs").length,
  )).toBeGreaterThan(1);
});

test("SSH failures show that automatic retry was stopped", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();

  await page.evaluate(() => {
    const context = (window as any).__mockExecutionContexts.find(
      (item: any) => item.id === "ssh:gpu-server",
    );
    context.last_probe_status = "error";
    context.last_probe_error = "Permission denied (publickey).";
  });
  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.getByRole("button", { name: "Probe context" }).click();
  await expect(page.locator(".copy-toast-warning")).toHaveText(
    "SSH probe failed. Automatic retry was stopped to protect the server; check the connection and retry manually.",
  );
  await expect(page.locator(".copy-toast-warning")).toBeHidden({ timeout: 3_000 });

  await page.evaluate(() => {
    const run = (window as any).__mockRuns.find((item: any) => item.id === "run-kinase-001");
    run.status = "failed";
    run.exit_code = 69;
    run.last_poll_error =
      "SSH automatic retry stopped after the first failed attempt to protect the server. Manual retry is required. Connection reset by peer.";
  });
  await remote.getByRole("button", { name: "View runs" }).click();
  await page.getByRole("dialog", { name: "Runs" })
    .getByRole("button", { name: "Refresh runs" }).click();
  await expect(page.locator(".copy-toast-warning")).toHaveText(
    "SSH failed. Automatic retry was stopped to protect the server; check the connection and retry manually.",
  );
});

test("context cards open machine, runtime, and runs details in modals", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();

  await expect(page.locator(".context-detail-pane")).toHaveCount(0);
  await expect(page.locator(".runtime-card")).toHaveCount(0);
  await expect(page.locator(".run-card")).toHaveCount(0);
  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.locator(".context-card-select").click();
  await expect(page.getByRole("dialog", { name: "Machine information" })).toContainText("gpu-server");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Machine information" })).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();

  await remote.getByRole("button", { name: "View runtimes" }).click();
  const runtimeDialog = page.getByRole("dialog", { name: "Runtimes" });
  await expect(runtimeDialog).toBeVisible();
  await expect(page.locator('.runtime-card[data-runtime-context="ssh:gpu-server"]')).toHaveCount(2);
  await runtimeDialog.evaluate((dialog) => dialog.setAttribute("data-refresh-stable", "true"));
  await runtimeDialog.getByRole("button", { name: "Refresh runtimes" }).click();
  await expect(runtimeDialog).toHaveAttribute("data-refresh-stable", "true");
  await page.getByRole("button", { name: "Close details" }).click();

  await remote.getByRole("button", { name: "View runs" }).click();
  const runsDialog = page.getByRole("dialog", { name: "Runs" });
  await expect(runsDialog).toBeVisible();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toBeVisible();
  await runsDialog.evaluate((dialog) => dialog.setAttribute("data-refresh-stable", "true"));
  await runsDialog.getByRole("button", { name: "Refresh runs" }).click();
  await expect(runsDialog).toHaveAttribute("data-refresh-stable", "true");
});

test("execution contexts remember Python and R interpreter paths", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();

  const remote = page.locator(".context-card", { hasText: "ssh:gpu-server" });
  await remote.getByRole("button", { name: "Configure runtime interpreters" }).click();
  const runtimeModal = page.locator(".runtime-config-modal");
  await expect.poll(() => runtimeModal.locator(".ps-close").evaluate((button) => ({
    headDisplay: getComputedStyle(button.parentElement!).display,
    buttonDisplay: getComputedStyle(button).display,
    width: getComputedStyle(button).width,
    border: getComputedStyle(button).borderTopWidth,
  }))).toEqual({ headDisplay: "flex", buttonDisplay: "flex", width: "30px", border: "0px" });
  const python = page.locator("#runtime-python-executable");
  const rscript = page.locator("#runtime-rscript-executable");
  const pastedPython = String.raw`C:\Tools\Python\python.exe`;
  await runtimeModal.evaluate((modal) => modal.setAttribute("data-paste-stable", "true"));
  await python.evaluate((element, value) => {
    const input = element as HTMLInputElement;
    const clipboard = new DataTransfer();
    clipboard.setData("text/plain", value);
    input.focus();
    input.dispatchEvent(new ClipboardEvent("paste", {
      bubbles: true,
      cancelable: true,
      clipboardData: clipboard,
    }));
    input.value = value;
    input.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      data: value,
      inputType: "insertFromPaste",
    }));
  }, pastedPython);
  await expect(python).toHaveValue(pastedPython);
  await expect(python).toBeFocused();
  await expect(runtimeModal).toHaveAttribute("data-paste-stable", "true");
  await rscript.fill(String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`);
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect.poll(() => lastInvokeArgs(page, "update_execution_context_interpreters")).toMatchObject({
    contextId: "ssh:gpu-server",
    pythonExecutable: String.raw`C:\Tools\Python\python.exe`,
    rscriptExecutable: String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`,
  });
  await expect(page.getByRole("heading", { name: "Runtime interpreters" })).toBeHidden();

  await remote.getByRole("button", { name: "Configure runtime interpreters" }).click();
  await expect(python).toHaveValue(String.raw`C:\Tools\Python\python.exe`);
  await expect(rscript).toHaveValue(String.raw`C:\Program Files\R\R-4.5.2\bin\Rscript.exe`);
});

test("runtime panel shows lifecycle state and controls start stop restart", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();

  await expect(page.locator(".runtime-card")).toHaveCount(0);
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runtimes" }).click();

  const localPython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="local"]');
  const localR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="local"]');
  const remotePython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="ssh:gpu-server"]');
  const remoteR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="ssh:gpu-server"]');

  await expect(localPython).toHaveCount(0);
  await expect(localR).toHaveCount(0);
  await expect(remotePython).toContainText("Busy");
  await expect(remotePython).toContainText("10.0 GB");
  await expect(remoteR).toContainText("Not started");

  await remoteR.getByRole("button", { name: "Configure path" }).click();
  await page.locator("#runtime-rscript-executable").fill("/data/apps/R/4.5/bin/Rscript");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "update_execution_context_interpreters")).toMatchObject({
    contextId: "ssh:gpu-server",
    rscriptExecutable: "/data/apps/R/4.5/bin/Rscript",
  });

  await remoteR.getByRole("button", { name: "Start" }).click();
  await expect(remoteR).toContainText("Ready");
  await expect.poll(() => lastInvokeArgs(page, "start_runtime")).toMatchObject({
    contextId: "ssh:gpu-server",
    language: "r",
  });
});

test("runtime inspector lists object metadata without loading object contents", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runtimes" }).click();

  const runtime = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="ssh:gpu-server"]');
  await runtime.getByRole("button", { name: "Stop" }).click();
  await runtime.getByRole("button", { name: "Start" }).click();
  await runtime.getByRole("button", { name: "View Python environment" }).click();

  const environment = page.getByRole("region", { name: "Python Environment" });
  await expect(environment).toBeVisible();
  const runtimeDialog = page.getByRole("dialog", { name: "Runtimes" });
  const runtimeList = runtimeDialog.locator(".context-modal-section");
  await expect.poll(async () => {
    const [listBox, environmentBox] = await Promise.all([
      runtimeList.boundingBox(),
      environment.boundingBox(),
    ]);
    return listBox && environmentBox
      ? Math.round(environmentBox.x - listBox.x - listBox.width)
      : -1;
  }).toBeGreaterThan(0);
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("DataFrame");
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("12000000 × 48");
  await expect(environment.locator(".runtime-environment-row", { hasText: "counts" })).toContainText("4.0 GB");
  await expect(environment.locator(".runtime-environment-row", { hasText: "model" })).toContainText("RandomForestClassifier");
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "ssh:gpu-server",
    language: "python",
  });

  await environment.getByRole("button", { name: "Close runtime environment" }).click();
  const rRuntime = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="ssh:gpu-server"]');
  await rRuntime.getByRole("button", { name: "Start" }).click();
  await rRuntime.getByRole("button", { name: "View R environment" }).click();
  const rEnvironment = page.getByRole("region", { name: "R Environment" });
  await expect(rEnvironment).toBeVisible();
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "ssh:gpu-server",
    language: "r",
  });

  await rEnvironment.getByRole("button", { name: "Pin environment to conversation" }).click();
  await expect(runtimeDialog).toHaveCount(0);
  await expect(rEnvironment).toBeVisible();
  await expect(rEnvironment.getByRole("button", { name: "Unpin environment" }))
    .toHaveAttribute("aria-pressed", "true");

  const beforeDrag = await rEnvironment.boundingBox();
  await rEnvironment.locator(".runtime-environment-title").evaluate((handle) => {
    const rect = handle.getBoundingClientRect();
    const startX = rect.left + rect.width / 2;
    const startY = rect.top + rect.height / 2;
    handle.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 7,
      clientX: startX,
      clientY: startY,
    }));
    handle.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      buttons: 1,
      pointerId: 7,
      clientX: startX - 120,
      clientY: startY + 48,
    }));
    handle.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 7,
      clientX: startX - 120,
      clientY: startY + 48,
    }));
  });
  await expect.poll(async () => {
    const afterDrag = await rEnvironment.boundingBox();
    return beforeDrag && afterDrag ? Math.round(beforeDrag.x - afterDrag.x) : 0;
  }).toBeGreaterThan(100);
  await expect.poll(async () => {
    const afterDrag = await rEnvironment.boundingBox();
    return beforeDrag && afterDrag ? Math.round(afterDrag.y - beforeDrag.y) : 0;
  }).toBeGreaterThan(30);

  await page.keyboard.press("Escape");
  await expect(rEnvironment).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
});

test("Windows environment settings imports installed WSL distributions", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    });
  });
  await enterApp(page);
  await openSettingsSection(page, "Environments");

  await page.getByRole("button", { name: "Import WSL" }).click();

  await expect.poll(() => lastInvokeArgs(page, "import_wsl_contexts")).not.toBeNull();
});

test("environment panel shows runs only for the selected context", async ({ page }) => {
  await enterApp(page);
  await selectRemoteContext(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Environment" }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: "View runs" }).click();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toBeVisible();
  await expect(page.locator(".run-card", { hasText: "Local normalization" })).toHaveCount(0);
});

test("clicking a figure opens the artifact modal with provenance", async ({ page }) => {
  await enterApp(page);
  // A file path in the user turn is collected as an artifact; a .png name maps
  // to the "image" kind, which opens directly in the modal viewer on click.
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // Clicking an image artifact opens the modal viewer directly (no expand step).
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await expect(page.locator(".artifact-modal")).toBeVisible();
  const overlay = page.locator(".overlay", { has: page.locator(".artifact-modal") });
  await expect.poll(async () => overlay.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      top: Math.round(rect.top),
      left: Math.round(rect.left),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  })).toEqual({ top: 0, left: 0, width: 1280, height: 720 });
  await expect.poll(() => page.evaluate(() =>
    document.elementFromPoint(innerWidth - 4, innerHeight / 2)?.closest(".overlay") !== null,
  )).toBe(true);
  const modalBoundsAt100 = await page.locator(".artifact-modal").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  const modalFigure = page.locator(".artifact-modal .am-figure");
  const figureHeightAt100 = await modalFigure.evaluate((el) =>
    Math.round(el.getBoundingClientRect().height),
  );
  const modalImage = page.locator(".artifact-modal .rp-img");
  const modalWidthAt100 = await modalImage.evaluate((el) => el.getBoundingClientRect().width);
  for (let i = 0; i < 3; i += 1) {
    await page.getByRole("button", { name: "Zoom out" }).click();
  }
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("25%");
  const modalBoundsAt25 = await page.locator(".artifact-modal").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  expect(Math.abs(modalBoundsAt25.width - modalBoundsAt100.width)).toBeLessThanOrEqual(12);
  expect(Math.abs(modalBoundsAt25.height - modalBoundsAt100.height)).toBeLessThanOrEqual(12);
  await expect.poll(async () => Math.abs(
    await modalFigure.evaluate((el) => Math.round(el.getBoundingClientRect().height))
      - figureHeightAt100,
  )).toBeLessThanOrEqual(12);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  for (let i = 0; i < 8; i += 1) {
    await page.getByRole("button", { name: "Zoom in" }).click();
  }
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("300%");
  await expect.poll(() => modalImage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(modalWidthAt100);
  await expect.poll(() => page.locator(".artifact-modal").evaluate((el) =>
    Math.round(el.getBoundingClientRect().width),
  )).toBeGreaterThan(0);
  const modalBoundsAt300 = await page.locator(".artifact-modal").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  });
  expect(Math.abs(modalBoundsAt300.width - modalBoundsAt100.width)).toBeLessThanOrEqual(12);
  expect(Math.abs(modalBoundsAt300.height - modalBoundsAt100.height)).toBeLessThanOrEqual(12);
  const modalViewport = page.locator(".artifact-modal .file-preview-zoom-viewport");
  await modalViewport.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const y = rect.top + rect.height * 0.5;
    const startX = rect.left + rect.width * 0.7;
    const endX = rect.left + rect.width * 0.25;
    el.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 1,
      clientX: startX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      button: 0,
      buttons: 1,
      pointerId: 1,
      clientX: endX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 1,
      clientX: endX,
      clientY: y,
    }));
  });
  await expect.poll(() => modalViewport.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("100%");
  // Code tab renders the recorded source (from get_artifact_provenance).
  await page.locator(".am-tab", { hasText: "Code" }).click();
  await expect(page.locator(".artifact-modal")).toContainText("savefig");
  const codeScrollOwners = await page.locator(".artifact-modal .am-panel").evaluate((panel) => {
    const code = panel.querySelector<HTMLElement>(".rp-code")!;
    code.querySelector("code")!.textContent = Array.from({ length: 200 }, (_, i) => `line ${i + 1}`).join("\n");
    const scrollsVertically = (element: HTMLElement) => {
      const overflow = getComputedStyle(element).overflowY;
      return (overflow === "auto" || overflow === "scroll") && element.scrollHeight > element.clientHeight;
    };
    return {
      panel: scrollsVertically(panel as HTMLElement),
      code: scrollsVertically(code),
    };
  });
  expect(codeScrollOwners).toEqual({ panel: true, code: false });
  // Environment tab renders the captured package list.
  await page.locator(".am-tab", { hasText: "Environment" }).click();
  await expect(page.locator(".am-env")).toContainText("matplotlib");
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".artifact-modal")).toHaveCount(0);
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");
  const centerImage = page.locator(".center-file-preview .rp-img");
  const centerWidthAt100 = await centerImage.evaluate((el) => el.getBoundingClientRect().width);
  await centerImage.hover();
  await page.mouse.wheel(0, -100);
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("125%");
  await expect.poll(() => centerImage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(centerWidthAt100);
  const centerViewport = page.locator(".center-file-preview .file-preview-zoom-viewport");
  await centerViewport.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const y = rect.top + rect.height * 0.5;
    const startX = rect.left + rect.width * 0.7;
    const endX = rect.left + rect.width * 0.3;
    el.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 2,
      clientX: startX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true,
      button: 0,
      buttons: 1,
      pointerId: 2,
      clientX: endX,
      clientY: y,
    }));
    el.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      pointerId: 2,
      clientX: endX,
      clientY: y,
    }));
  });
  await expect.poll(() => centerViewport.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
});

test("PDF artifacts render inside the app without a browser PDF plugin", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  // Single-page viewer: one page is rendered at a time, navigated with controls.
  await expect(modal.locator('.rp-pdf[data-page-count="2"][data-current-page="1"]')).toBeVisible();
  const renderedPage = modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]');
  await expect(renderedPage).toBeVisible();
  await expect(modal.locator(".rp-pdf-page")).toHaveCount(1);
  const canvas = renderedPage.locator("canvas");
  await expect(canvas).toBeVisible();
  await expect.poll(() => canvas.evaluate(
    (el: HTMLCanvasElement) => el.width * el.height,
  )).toBeGreaterThan(0);
  const pageWidthAt100 = await renderedPage.evaluate((el) => el.getBoundingClientRect().width);
  const textSpan = renderedPage.locator(".rp-pdf-textlayer span").first();
  const textWidthAt100 = await textSpan.evaluate((el) => el.getBoundingClientRect().width);
  await page.getByRole("button", { name: "Zoom in" }).click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("150%");
  await expect.poll(() => renderedPage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(pageWidthAt100 * 1.4);
  await expect.poll(() => textSpan.evaluate((el) => el.getBoundingClientRect().width))
    .toBeGreaterThan(textWidthAt100 * 1.4);
  await page.getByRole("button", { name: "Reset zoom" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();
  await page.getByRole("button", { name: "Zoom out" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("50%");
  await expect.poll(() => renderedPage.evaluate((el) => el.getBoundingClientRect().width))
    .toBeLessThan(pageWidthAt100 * 0.6);
  await expect.poll(() => textSpan.evaluate((el) => el.getBoundingClientRect().width))
    .toBeLessThan(textWidthAt100 * 0.6);
  await expect(page.getByRole("button", { name: "Previous page" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Next page" }).locator("svg")).toBeVisible();
  await expect(modal.locator('embed[type="application/pdf"]')).toHaveCount(0);
  // A selectable text layer sits over the canvas so PDF text can be added to chat.
  await expect(renderedPage.locator(".rp-pdf-textlayer")).toContainText("PDF preview works");
});

test("PDF artifacts switch pages with toolbar buttons, arrow keys, and Page Up/Down", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  await page.getByRole("button", { name: "Zoom in" }).click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByRole("button", { name: "Reset zoom" })).toHaveText("150%");

  // Toolbar button steps forward.
  await page.getByRole("button", { name: "Next page" }).click();
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  const secondPage = modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]');
  await expect(secondPage).toBeVisible();
  await expect.poll(() => secondPage.evaluate((el) => Math.abs(
    el.getBoundingClientRect().width
      - el.querySelector(".rp-pdf-textlayer")!.getBoundingClientRect().width,
  ))).toBeLessThan(2);
  await expect(page.getByRole("button", { name: "Next page" })).toBeDisabled();

  // Page Up steps back. Wait for the page to finish rendering (rendered="true")
  // before the next key — stepPage is a no-op while a render is in flight.
  await page.keyboard.press("PageUp");
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  // Arrow keys also navigate: Right → next, Left → previous.
  await page.keyboard.press("ArrowRight");
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]')).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();
});

test("PDF text can be selected and added to chat", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const layer = page.locator(".artifact-modal .rp-pdf-textlayer");
  await expect(layer).toContainText("PDF preview works");

  // Select a text-layer span and raise the shared quote popup (the modal
  // figure's data-file-path ancestor drives it — same path as md/docx).
  await layer.locator("span").first().evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  const popup = page.locator(".selection-popup");
  await expect(popup).toBeVisible();
  await popup.getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("PDF preview works");
});

test("DOCX text in the modal (Files browser) can be added to chat", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path*="manuscript.docx"]').click();

  const docx = page.locator(".artifact-modal .rp-docx");
  await expect(docx).toContainText("Differential expression of FX-cell markers");
  const heading = docx.getByText("Differential expression of FX-cell markers").first();
  // Modal preview text must stay selectable despite the overlay's user-select:none.
  await heading.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
  await page.locator(".selection-popup").getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("Differential expression");
});

test("DOCX artifacts render offline with headings, tables, and equations", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open manuscript.docx");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="manuscript.docx"] .rp-tile-main').click();

  // docx-preview renders a `.docx-wrapper` of `section.docx` pages, fully offline.
  const docx = page.locator(".rp-docx");
  await expect(docx.locator(".docx-wrapper section.docx").first()).toBeVisible();
  await expect(docx).toContainText("Differential expression of FX-cell markers");
  await expect(docx).toContainText("FOXA2"); // a table cell
  // The OMML equations convert to MathML — this is the #274 formula concern.
  await expect(docx.locator("math").first()).toBeAttached();
  // The wrapping preview carries data-file-path so P2 selection/annotate works here too.
  await expect(page.locator('.rp-file-preview[data-file-path*="manuscript.docx"]')).toBeVisible();

  // #274: a tall docx must be scrollable in the right pane (not trapped by a
  // fixed-height .rp-docx). The .rp-view container owns the scroll.
  const view = page.locator(".rp-view");
  await docx.locator(".docx-wrapper").evaluate((el) => {
    (el as HTMLElement).style.minHeight = "4000px";
  });
  await expect.poll(() => view.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(100);
  await view.evaluate((el) => { el.scrollTop = 500; });
  await expect.poll(() => view.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
});

test("DOCX opened from the Files browser scrolls inside the modal (#274)", async ({ page }) => {
  await enterApp(page);
  // Files browser → docx opens in the artifact modal (like the tester's flow).
  await page.getByRole("button", { name: "Files" }).click();
  await page.locator('.fb-row[data-workspace-path*="manuscript.docx"]').click();

  const docx = page.locator(".artifact-modal .rp-docx");
  await expect(docx.locator(".docx-wrapper section.docx").first()).toBeVisible();
  // A tall document must scroll inside .rp-docx — the modal figure clips, so the
  // bounded height has to reach .rp-docx (the #274 "can't scroll down" bug).
  await docx.locator(".docx-wrapper").evaluate((el) => {
    (el as HTMLElement).style.minHeight = "4000px";
  });
  await expect.poll(() => docx.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(100);
  await docx.evaluate((el) => { el.scrollTop = 800; });
  await expect.poll(() => docx.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
});

test("Markdown center preview can be edited in place and saved", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open report.md");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();

  // Right-click the file tile → "Open in center" opens the real workspace file.
  await page.locator('.rp-tile[data-artifact-name="report.md"]').click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator('.center-file-preview[data-file-path="report.md"]');
  await expect(preview.locator("h1")).toHaveText("Draft manuscript");

  // Enter edit mode: the raw Markdown loads into a textarea.
  await preview.getByRole("button", { name: "Edit" }).click();
  const editor = preview.locator(".center-file-editor");
  await expect(editor).toHaveValue(/Original body paragraph/);

  // Rewrite and save → write_file is invoked and the preview reloads the new text.
  await editor.fill("# Revised manuscript\n\nRewritten body paragraph.\n");
  await preview.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "write_file"))
    .toMatchObject({ path: "report.md", content: "# Revised manuscript\n\nRewritten body paragraph.\n" });
  await expect(editor).toHaveCount(0);
  await expect(preview.locator("h1")).toHaveText("Revised manuscript");
  await expect(preview.getByRole("button", { name: "Edit" })).toBeVisible();
});

test("center split keeps the same conversation beside the open document", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open report.md");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="report.md"]').click({ button: "right" });
  await page.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator('.center-file-preview[data-file-path="report.md"]');
  await expect(preview.locator("h1")).toHaveText("Draft manuscript");

  // Opening a document hides the conversation by default.
  const chat = page.locator(".chat");
  await expect(chat).toBeHidden();

  // Split → the conversation comes back beside the document and the right pane
  // folds away so the two share its width.
  await preview.locator("[data-center-split]").click();
  await expect(chat).toBeVisible();
  await expect(composer(page)).toBeVisible();
  await expect(page.locator(".rightpane")).toHaveCount(0);

  // Really side by side, not stacked: the chat starts past the document's right edge.
  const doc = (await preview.boundingBox())!;
  const box = (await chat.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(doc.x + doc.width - 1);
  expect(box.y).toBeLessThan(doc.y + doc.height);

  // Same session, not a new one — the sent message is still in the thread.
  await expect(chat.getByText("open report.md")).toBeVisible();

  // Toggling off restores the document-only view.
  await preview.locator("[data-center-split]").click();
  await expect(chat).toBeHidden();
});

test("artifact modal switches between images with left and right arrows", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make plots first.png second.png third.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator('.rp-tile[data-artifact-name="second.png"]')).toBeVisible();

  await page.locator('.rp-tile[data-artifact-name="second.png"] .rp-tile-main').click();
  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("second.png");
  await expect(page.getByRole("button", { name: "Previous image" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Next image" })).toBeEnabled();

  await page.keyboard.press("ArrowRight");
  await expect(modal.locator(".am-name")).toHaveText("third.png");
  await expect(page.getByRole("button", { name: "Next image" })).toBeDisabled();

  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator(".am-name")).toHaveText("second.png");
  await page.keyboard.press("ArrowLeft");
  await expect(modal.locator(".am-name")).toHaveText("first.png");
  await expect(page.getByRole("button", { name: "Previous image" })).toBeDisabled();
});

test("center file tabs are restored per conversation", async ({ page }) => {
  await enterApp(page);

  await page.keyboard.press("Control+K");
  const search = commandPalette(page);
  await search.fill("Current analysis");
  await search.press("Enter");

  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  await page.getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");

  await page.keyboard.press("Control+K");
  await search.fill("Older structure run");
  await search.press("Enter");
  await expect(page.locator(".center-tab-wrap")).toHaveCount(0);
  await expect(page.locator(".center-tabs > .center-tab")).toHaveClass(/active/);

  await page.keyboard.press("Control+K");
  await search.fill("Current analysis");
  await search.press("Enter");
  await expect(page.locator(".center-tab-wrap")).toHaveCount(1);
  await expect(page.locator(".center-tab.active")).toContainText("volcano.png");
});

test("image preview context menu copies the image", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const image = page.locator(".artifact-modal .rp-img");
  await expect(image).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, "write", {
      configurable: true,
      value: async (items: ClipboardItem[]) => { (window as any).__copiedImageTypes = items.flatMap((item) => item.types); },
    });
  });
  await image.click({ button: "right" });
  await page.getByRole("button", { name: "Copy image" }).click();
  await expect(page.locator(".copy-toast")).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => (window as any).__copiedImageTypes)).toContain("image/png");
});

test("image region can be cropped and attached to the composer", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();
  const image = page.locator(".artifact-modal .rp-img");
  await expect(image).toBeVisible();

  // Toggle crop mode → the capture layer appears.
  await page.getByRole("button", { name: "Select a region to ask about" }).click();
  const layer = page.locator(".file-preview-crop-layer");
  await expect(layer).toBeVisible();

  // Rubber-band a rectangle inside the image.
  const box = (await image.boundingBox())!;
  await page.mouse.move(box.x + 20, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + 100, { steps: 4 });
  await expect(page.locator(".file-preview-crop-rect")).toBeVisible();
  await page.mouse.up();

  // The crop uploads as a PNG and attaches to the composer (region_*.png).
  await expect.poll(() => lastInvokeArgs(page, "upload_file"))
    .toMatchObject({ filename: expect.stringMatching(/^region_.*\.png$/) });
  await expect(page.locator(".composer-attachments .composer-attachment.ready")).toContainText("region_");
  // Crop mode auto-exits after a successful crop.
  await expect(layer).toHaveCount(0);
});

test("artifact panel normalizes png/pdf shorthand to the previewable image", async ({ page }) => {
  await enterApp(page);
  await page
    .locator("#composer-input")
    .fill("show `figures/panel_I_heatmap_4genes_median.png/.pdf`");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();

  const tile = page.locator('.rp-tile[data-artifact-name="panel_I_heatmap_4genes_median.png"]');
  await expect(tile).toBeVisible();
  await expect(tile.locator(".rp-badge")).toHaveText("image");
  await expect(page.locator('.rp-tile[data-artifact-name="panel_I_heatmap_4genes_median.png/.pdf"]')).toHaveCount(0);
});

test("settings page shows the saved provider", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await expect(providerSelect(page)).toHaveValue("openai");
  await expect(page.locator("label.settings-check", { hasText: "Supports image input" })).toHaveCSS("flex-direction", "row");
  await expect(page.locator("label.settings-check", { hasText: "Use for image analysis" })).toHaveCSS("flex-direction", "row");
  await page.locator(".settings-footer").getByRole("button", { name: "Cancel" }).click();
});

test("model settings updates activation and confirms removal", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Models");

  const opus = page.locator(".settings-list-row").filter({ hasText: "opus-4.8" });
  await opus.getByRole("button", { name: "Use" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_active_model")).toMatchObject({ id: "opus" });
  await expect(opus).toHaveClass(/settings-list-row-active/);

  const deepseek = page.locator(".settings-list-row").filter({ hasText: "deepseek-v4-pro" });
  await deepseek.getByTitle("Remove model").click();
  const confirm = page.getByTestId("model-delete-confirm");
  await expect(confirm).toContainText("Remove deepseek-v4-pro? This cannot be undone.");
  await expect.poll(() => lastInvokeArgs(page, "remove_model")).toBeNull();

  await confirm.getByRole("button", { name: "Remove model" }).click();
  await expect.poll(() => lastInvokeArgs(page, "remove_model")).toMatchObject({ id: "default" });
  await expect(deepseek).toHaveCount(0);
});

test("appearance settings persist separate light and dark palettes and font sizes", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Appearance");

  await page.getByTestId("theme-mode-light").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByTestId("appearance-palette-select").selectOption("catppuccin");
  await expect(page.getByTestId("appearance-palette-select")).toHaveValue("catppuccin");
  await expect(page.locator("html")).toHaveAttribute("data-light-palette", "catppuccin");

  await page.getByTestId("theme-mode-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByTestId("appearance-palette-select").selectOption("gruvbox");
  await expect(page.getByTestId("appearance-palette-select")).toHaveValue("gruvbox");
  await expect(page.locator("html")).toHaveAttribute("data-dark-palette", "gruvbox");

  await page.getByRole("slider", { name: "UI font size" }).fill("16");
  await page.getByRole("slider", { name: "Code font size" }).fill("15");
  await expect.poll(() => page.evaluate(() => ({
    theme: localStorage.getItem("wisp-theme"),
    light: localStorage.getItem("wisp-light-palette"),
    dark: localStorage.getItem("wisp-dark-palette"),
    ui: localStorage.getItem("wisp-ui-font-size"),
    code: localStorage.getItem("wisp-code-font-size"),
  }))).toEqual({ theme: "dark", light: "catppuccin", dark: "gruvbox", ui: "16", code: "15" });

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("html")).toHaveAttribute("data-light-palette", "catppuccin");
  await expect(page.locator("html")).toHaveAttribute("data-dark-palette", "gruvbox");
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--ui-font-size").trim())).toBe("16px");
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement)
    .getPropertyValue("--code-font-size").trim())).toBe("15px");
});

test("vision assignment keeps model fields and stored key placeholder untouched", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);

  const effort = page.getByLabel("Reasoning effort");
  const key = page.getByLabel("API key (stored in OS keyring)");
  const useForVision = page.getByLabel("Use for image analysis");

  await providerSelect(page).selectOption("openai_responses");
  await page.getByLabel("API URL").fill("https://api.openai-proxy.org/v1");
  await page.getByLabel("Model").fill("gpt-5.6-luna");
  await effort.selectOption("medium");
  await expect(key).toHaveValue("");
  await expect(key).toHaveAttribute("placeholder", "(stored — leave blank to keep)");

  if (await useForVision.isChecked()) {
    await useForVision.uncheck();
  }
  await useForVision.check();

  await expect(providerSelect(page)).toHaveValue("openai_responses");
  await expect(effort).toHaveValue("medium");
  await expect(key).toHaveValue("");

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => page.evaluate(() => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "save_model");
    const args = plain(calls.at(-1)?.args ?? null);
    return args ? { ...args, key: args.key ?? null } : null;
  })).toMatchObject({
    key: null,
    useForVision: true,
    profile: {
      provider: "openai_responses",
      reasoning_effort: "medium",
      use_for_vision: true,
    },
  });

  await page.locator(".settings-list-row").first().click();
  await expect(providerSelect(page)).toHaveValue("openai_responses");
  await expect(effort).toHaveValue("medium");
  await expect(page.getByLabel("Use for image analysis")).toBeChecked();
});

test("settings normalizes a blank stored provider to openai", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await expect(providerSelect(page)).toHaveValue("openai");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validated openai with deepseek-v4-pro");
});

test("editing API URL keeps provider state and display aligned", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByLabel("API URL").fill("https://api.deepseek.com");
  await expect(providerSelect(page)).toHaveValue("openai");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validated openai with deepseek-v4-pro");
});

test("settings can validate current API config", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validated openai with deepseek-v4-pro");
});

test("editing a saved model validates with that model profile id", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Models" }).click();
  await page.locator(".settings-list-row", { hasText: "opus-4.8" }).click();
  await expect(providerSelect(page)).toBeVisible();
  await expect(page.getByLabel("Model ID")).toHaveValue("opus-4.8");

  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validated openai with deepseek-v4-pro");
  await expect.poll(() => lastInvokeArgs(page, "validate_settings")).toMatchObject({
    profileId: "opus",
    key: "",
    settings: {
      model: "opus-4.8",
    },
  });
});

test("check for updates shows an up-to-date modal", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("You're up to date");
  await expect(modal).toContainText("Wisp 0.9.0 is already the latest version.");
  await modal.getByRole("button", { name: "OK" }).click();
  await expect(modal).toHaveCount(0);
});

test("check for updates shows an available-update modal before opening releases", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "1.2.3",
    update_available: true,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v1.2.3",
  });

  await page.getByRole("button", { name: "Check for updates" }).click();
  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("Update available");
  await expect(modal).toContainText("Wisp 1.2.3 is available.");
  await expect(await lastInvokeArgs(page, "open_external_url")).toBeNull();
  await page.getByTestId("update-check-open-releases").click();
  await expect(modal).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: "https://github.com/xuzhougeng/wisp-science/releases/tag/v1.2.3",
  });
});

test("command palette check for updates also shows the result modal", async ({ page }) => {
  await enterApp(page);
  await setMockUpdateCheck(page, {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
  });

  await page.keyboard.press("Control+p");
  const input = page.locator("#action-palette-input");
  await input.fill("check for updates");
  await input.press("Enter");

  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("You're up to date");
});

test("command palette click shows checking feedback immediately", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".proj-card-main")).not.toHaveCount(0);
  await setMockUpdateCheckPending(page, true);

  await page.keyboard.press("Control+p");
  await page.getByRole("button", { name: "Check for updates" }).click();

  const modal = page.getByTestId("update-check-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("Checking for updates");
  await expect(modal).toContainText("Contacting GitHub Releases");
  await resolveMockUpdateCheck(page);
  await expect(modal).toContainText("You're up to date", { timeout: 2_000 });
});

test("credentials settings include SCIMaster and save its key", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Credentials");
  const field = page.locator("label", { hasText: "SCIMaster API key" });
  await expect(field).toContainText("Not configured");
  await field.locator("input").fill("sk-sci-123");
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_credential")).toMatchObject({
    id: "scimaster_api_key",
    value: "sk-sci-123",
  });
  await expect(page.locator(".settings-status")).toHaveText("Saved. Applies to new sessions.");
  await expect(field).toContainText("Configured");
});

test("skill manager filters by tag and batch disables visible skills", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Add to message" }).click();
  await page.getByRole("button", { name: "Manage skills" }).click();
  await expect(page.getByRole("button", { name: "Skills" })).toBeVisible();
  await expect(page.locator(".settings-search")).toHaveAttribute("type", "text");
  await expect(page.locator(".settings-search")).toHaveAttribute("inputmode", "search");
  await expect(page.locator(".settings-search")).toHaveAttribute("autocomplete", "off");
  await expect(page.locator(".settings-filter")).toContainText(/visible.*enabled/);
  await expect(page.locator(".skill-tags-editor").first()).not.toHaveAttribute("open", "");

  await page.getByRole("button", { name: "Disabled", exact: true }).click();
  await expect(page.getByText("No skills match the current filters.")).toBeVisible();
  await expect(page.locator("[data-skill-name]")).toHaveCount(0);

  await page.getByRole("button", { name: "Enabled", exact: true }).click();
  await expect(page.getByText("alphafold2")).toBeVisible();
  await expect(page.getByText("remote-compute-modal")).toBeVisible();

  await page.getByRole("button", { name: "compute", exact: true }).click();
  await expect(page.getByText("remote-compute-modal")).toBeVisible();
  await expect(page.getByText("alphafold2")).not.toBeVisible();

  await page.getByRole("button", { name: "Disable visible" }).click();
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "set_skills_enabled");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  })).toEqual({ names: ["remote-compute-modal"], enabled: false });
  await expect(page.locator('[data-skill-name="remote-compute-modal"] input[type="checkbox"]')).not.toBeChecked();
});

test("custom MCP row opens tools while edit uses a dedicated button", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Connections" }).click();
  await expect(page.getByRole("button", { name: "Connect Notion" })).toHaveCount(0);

  const row = page.locator(".settings-list-row", { hasText: "wolai_cmp" });
  await row.click();
  await expect(page.getByText("wolai_search")).toBeVisible();
  await expect(page.getByText("Search Wolai pages")).toBeVisible();

  await page.locator(".settings-head-back").click();
  await row.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByLabel("Name")).toHaveValue("wolai_cmp");
  await expect(page.getByPlaceholder("https://host/mcp")).toHaveValue("https://api.wolai.com/v1/mcp/");
});

test("Notion uses the generic Remote URL OAuth connection flow", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Connections" }).click();

  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toBeNull();
  await expect.poll(() => lastInvokeArgs(page, "test_oauth_mcp_connection")).toBeNull();
  await page.getByRole("button", { name: "Add connection" }).click();
  const type = page.getByLabel("Type");
  await expect(type.locator("option")).toHaveCount(2);
  await expect(type.locator('option[value="notion"]')).toHaveCount(0);

  await page.getByLabel("Name").fill("Notion");
  await type.selectOption("http");
  await page.getByPlaceholder("https://host/mcp").fill("https://mcp.notion.com/mcp");
  await page.getByLabel("Authentication").selectOption("oauth");
  await expect(page.getByText("Testing does not save the connection.")).toBeVisible();

  await page.getByRole("button", { name: "Test" }).click();
  await expect.poll(() => lastInvokeArgs(page, "test_oauth_mcp_connection")).toMatchObject({
    conn: {
      name: "Notion",
      transport: {
        kind: "http",
        url: "https://mcp.notion.com/mcp",
        auth: "oauth",
      },
    },
  });
  await expect(page.locator(".settings-status")).toHaveText("OK — 2 tools");
  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toBeNull();
  await expect(page.getByLabel("Name")).toHaveValue("Notion");

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "authorize_http_connection")).toMatchObject({
    conn: {
      name: "Notion",
      enabled: true,
      transport: {
        kind: "http",
        url: "https://mcp.notion.com/mcp",
        auth: "oauth",
      },
    },
  });
  const row = page.locator(".settings-list-row", { hasText: "Notion" });
  await expect(row).toContainText("https://mcp.notion.com/mcp");
  await expect(row).toContainText("OAuth");
  await expect(row).toContainText("Enabled");

  await row.click();
  await expect(page.getByText("Service", { exact: true })).toBeVisible();
  await expect(page.getByText("https://mcp.notion.com/mcp", { exact: true })).toBeVisible();
  await expect(page.getByText("Status", { exact: true })).toBeVisible();
  await expect(page.getByText("Enabled", { exact: true })).toBeVisible();
  await expect(page.getByText("Authentication", { exact: true })).toBeVisible();
  await expect(page.getByText("OAuth", { exact: true })).toBeVisible();
});

test("OAuth authorization keeps Cancel available and clears form status", async ({ page }) => {
  await enterApp(page, "/?mockOAuthPending=1");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Connections" }).click();
  await page.getByRole("button", { name: "Add connection" }).click();
  await page.getByLabel("Name").fill("Hosted MCP");
  await page.getByLabel("Type").selectOption("http");
  await page.getByPlaceholder("https://host/mcp").fill("https://example.com/mcp");
  await page.getByLabel("Authentication").selectOption("oauth");
  await expect(page.getByPlaceholder("X-Custom-Header: value")).toBeVisible();

  await page.getByRole("button", { name: "Test" }).click();
  await expect(page.getByText("Complete authorization in your browser…")).toBeVisible();
  const cancel = page.getByRole("button", { name: "Cancel" });
  await expect(cancel).toBeEnabled();
  await cancel.click();
  await expect(page.getByRole("button", { name: "Add connection" })).toBeVisible();
  await expect.poll(async () => (await invokeArgsList(page, "cancel_oauth_authorization")).length).toBe(1);

  await page.evaluate(() => (window as any).__resolveMockOAuth());
  await expect(page.locator(".settings-status")).toHaveCount(0);
});

test("settings validation rejects blank required fields", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await page.getByLabel("API URL").fill("");
  await page.getByRole("button", { name: "Valid" }).click();
  await expect(page.locator(".settings-status")).toHaveText("Validation failed: API URL is required.");
});

test("provider switch fills current API defaults", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await providerSelect(page).selectOption("openai_responses");
  await expect(page.getByLabel("API URL")).toHaveValue("https://api.openai.com/v1");
  await expect(page.getByLabel("Model")).toHaveValue("gpt-5.5");
  await providerSelect(page).selectOption("anthropic");
  await expect(page.getByLabel("API URL")).toHaveValue("https://api.anthropic.com");
  await expect(page.getByLabel("Model")).toHaveValue("claude-sonnet-5");
  await providerSelect(page).selectOption("openai");
  await expect(page.getByLabel("API URL")).toHaveValue("https://api.deepseek.com");
  await expect(page.getByLabel("Model")).toHaveValue("deepseek-v4-pro");
});

test("model form input keeps focus while typing (#62)", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  const model = page.getByLabel("Model");
  await model.fill("");
  // Type character-by-character. The bug: the form pane was gated on the whole
  // model_form signal, so each keystroke rebuilt the inputs and dropped focus —
  // only the first character survived. After the fix the field stays mounted.
  await model.pressSequentially("gpt-5.5-x");
  await expect(model).toHaveValue("gpt-5.5-x");
  await expect(model).toBeFocused();
});

test("inline approval card keeps its buttons reachable with a long preview (#63)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  // A very long preview must not push the allow button off-screen; the card
  // scrolls the code block internally so the actions stay in view.
  const allow = page.getByRole("button", { name: "Allow once" });
  await expect(allow).toBeVisible({ timeout: 10_000 });
  await expect(allow).toBeInViewport();
});

test("inline approval scope is sent with confirmation", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByRole("button", { name: "Allow once" })).toBeVisible({ timeout: 10_000 });
  await page.getByLabel("Approval scope").selectOption("project");
  await page.getByRole("button", { name: "Allow for this project" }).click();

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "confirm_response") ?? null;
  })).toMatchObject({
    cmd: "confirm_response",
    args: {
      approved: true,
      scope: "project",
    },
  });
});

test("R execution uses the language-specific approval label", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("NEEDRCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Run R code?")).toBeVisible({ timeout: 10_000 });
  await expect(page.locator(".approval-code code.language-r")).toContainText("summary(dataset)");
});

test("settings permissions lists and revokes remembered approvals", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Permissions");

  await expect(page.getByText("Shell commands")).toBeVisible();
  await expect(page.getByText("Global")).toBeVisible();
  await page.getByRole("button", { name: "Revoke all" }).click();
  await expect(page.getByText("No remembered approvals.")).toBeVisible();
});

test("chat stays pinned to the bottom while streaming a long reply (#61)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("SCROLLTEST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("line 79")).toBeVisible({ timeout: 15_000 });
  // The per-delta re-render used to clamp scrollTop toward the top and unfollow,
  // stranding the view at the top mid-stream. The scroller must end at the bottom.
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const el = document.getElementById("chat-scroller");
          if (!el) return 9999;
          return el.scrollHeight - el.clientHeight - el.scrollTop;
        }),
      { timeout: 5000 },
    )
    .toBeLessThan(8);
});

test("recent sessions show only title and status badge", async ({ page }) => {
  await page.goto("/");
  const cards = page.locator('[data-testid="recent-session-card"]');
  await expect(cards).toHaveCount(2);

  const first = cards.first();
  await expect(first.locator(".pc-name")).toHaveText("帮我找一篇单细胞的文章");
  await expect(first.locator(".sess-status-needs-you")).toBeVisible();
  await expect(first.locator(".pc-hint")).toHaveCount(0);
  await expect(first.locator(".pc-when")).toHaveCount(0);
  await expect(first.locator(".pc-meta-row")).toHaveCount(0);

  const second = cards.nth(1);
  await expect(second.locator(".pc-name")).toHaveText("Enumerate MCP bio-tools databases");
  await expect(second.locator(".sess-status-complete")).toBeVisible();
});

test("session history loads older pages with a stable cursor", async ({ page }) => {
  await page.goto("/?mockManySessions=1");
  await page.locator(".proj-card-main").first().click();

  await expect(page.getByRole("button", { name: "Paged session 1", exact: true })).toBeVisible();
  expect(await page.getByRole("button", { name: "Paged session 101", exact: true }).count()).toBe(0);
  await page.getByRole("button", { name: "Load earlier sessions" }).click();
  await expect(page.getByRole("button", { name: "Paged session 101", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Load earlier sessions" })).toHaveCount(0);
  await expect.poll(() => lastInvokeArgs(page, "list_sessions_page")).toMatchObject({
    cursor: { id: "session-100", ts: 1901 },
  });
});

test("home search opens artifacts, sessions, and settings", async ({ page }) => {
  await page.goto("/");

  await page.getByRole("button", { name: "Settings" }).click();
  const settingsPage = page.locator(".settings-page");
  await expect(settingsPage).toBeVisible();
  await expect(page.locator(".overlay", { has: settingsPage })).toHaveCount(0);
  const expectedSettingsTop = await page.locator(".window-titlebar").count() === 1 ? 38 : 0;
  await expect.poll(() => settingsPage.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      top: Math.round(rect.top),
      left: Math.round(rect.left),
      right: Math.round(rect.right),
      bottom: Math.round(rect.bottom),
    };
  })).toEqual({ top: expectedSettingsTop, left: 0, right: 1280, bottom: 720 });
  await expect(page.getByRole("button", { name: "Back to app" })).toBeVisible();
  await page.locator(".settings-head-close").click();

  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.fill("update");
  await expect(page.locator(".project-search-row", { hasText: "Check for updates" })).toBeVisible();
  await search.fill("star");
  await expect(page.locator(".project-search-row", { hasText: "Star us on GitHub" })).toBeVisible();
  await search.fill("file");
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.press("Enter");
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await expect(page.locator(".am-name")).toHaveText("nif3.treefile");
  await page.locator(".artifact-modal").getByRole("button", { name: "Close panel" }).click();

  await page.getByRole("button", { name: "Search" }).click();
  await search.fill("Enumerate");
  await expect(page.locator(".project-search-row", { hasText: "Enumerate MCP bio-tools databases" })).toBeVisible();
  await search.press("Enter");
  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "load_session") ?? null;
  })).toMatchObject({ cmd: "load_session", args: { id: "s-complete" } });
});

test("long transcripts load earlier turns without jumping to the new top", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await page.getByText("Long transcript", { exact: true }).click();

  await expect(page.getByText("Newest page first question", { exact: true })).toBeVisible();
  const scroller = page.locator("#chat-scroller");
  const loadEarlier = page.getByRole("button", { name: "Load earlier messages" });
  await expect(loadEarlier).toBeVisible();
  await loadEarlier.click();

  await expect(page.getByText("Oldest loaded question", { exact: true })).toBeAttached();
  await expect(loadEarlier).toHaveCount(0);
  await expect.poll(() => scroller.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => page.evaluate(() => (window as any).__transcriptPageCalls)).toEqual([
    null,
    41,
  ]);
});

test("long transcript rendering keeps a bounded turn window", async ({ page }) => {
  const pageCount = Number(process.env.TRANSCRIPT_SOAK_PAGES ?? 8);
  test.setTimeout(Math.max(30_000, pageCount * 2_000));
  await page.goto(`/?mockLongPages=${pageCount}`);
  await page.locator(".proj-card-main").first().click();
  await page.getByText("Long transcript", { exact: true }).click();

  for (let loaded = 1; loaded < pageCount; loaded += 1) {
    await page.getByRole("button", { name: "Load earlier messages" }).click();
    await expect.poll(() => page.evaluate(() =>
      ((window as any).__transcriptPageCalls ?? []).length,
    )).toBe(loaded + 1);
  }

  await expect(page.locator(".msg.user")).toHaveCount(40);
  const oldestRow = new RegExp(`Window page ${pageCount - 1} row 0`);
  await expect(page.getByText(oldestRow)).toBeVisible();
  const newerSteps = Math.ceil(Math.max(0, pageCount * 10 - 40) / 20);
  for (let step = 0; step < newerSteps; step += 1) {
    await page.getByRole("button", { name: "Show newer messages" }).click();
  }
  await expect(page.locator(".msg.user")).toHaveCount(40);
  await expect(page.getByText(/Window page 0 row 0/)).toBeVisible();
  await expect(page.getByText(oldestRow)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Show earlier loaded messages" })).toBeVisible();
});

test("branching from a paged transcript uses the global user-turn index", async ({ page }) => {
  await page.goto("/?mockLongSession=1");
  await page.locator(".proj-card-main").first().click();
  await page.getByText("Long transcript", { exact: true }).click();
  const firstLoadedUser = page.locator(".msg.user", { hasText: "Newest page first question" });
  await firstLoadedUser.getByRole("button", { name: "Branch" }).click();

  await expect.poll(() => lastInvokeArgs(page, "branch_session")).toMatchObject({
    sessionId: "long-session",
    userIndex: 10,
  });
});

test("HTML artifact modal uses a desktop preview viewport", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("dashboard");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal.html-preview");
  await expect(modal).toBeVisible();
  const frame = modal.locator("iframe.rp-html");
  await expect(frame).toBeVisible();
  await expect.poll(() => frame.evaluate((el) => el.clientWidth)).toBeGreaterThanOrEqual(1190);
  await expect.poll(() => frame.evaluate((el: HTMLIFrameElement) => {
    const mode = el.contentDocument?.querySelector("#mode");
    return mode ? getComputedStyle(mode, "::after").content : "";
  })).toBe('"Desktop"');
});

test("Markdown artifact modal opens its rendered preview in center", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("analysis-report");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await expect(modal.locator(".am-name")).toHaveText("analysis-report.md");
  await expect(modal.locator(".am-figure h1")).toHaveText("Differential expression report");
  await modal.getByRole("button", { name: "Open in center" }).click();

  await expect(modal).toHaveCount(0);
  await expect(page.locator('.center-tab[data-center-path="artifact:art-markdown"]')).toContainText("analysis-report.md");
  await expect(page.locator(".center-file-preview h1")).toHaveText("Differential expression report");
  await expect(page.locator(".center-file-preview")).toContainText("Rendered Markdown body.");
});

test("bound Markdown resources use immutable versions and a scrollable center preview", async ({ page }) => {
  await page.goto("/?mockResourceSession=1");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("Enumerate");
  await search.press("Enter");

  await page.getByRole("link", { name: "Open bound report" }).click();
  const tab = page.locator('.center-tab[data-center-path="artifact-version:resource-version-markdown"]');
  await expect(tab).toContainText("report.md");
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator("h1")).toHaveText("Bound report");
  await expect(preview).toContainText("Scrollable row 120");
  await expect.poll(() => preview.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }))).toMatchObject({ clientHeight: expect.any(Number), scrollHeight: expect.any(Number) });
  const dimensions = await preview.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);
  await preview.evaluate((element) => { element.scrollTop = element.scrollHeight; });
  await expect.poll(() => preview.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await expect.poll(() => lastInvokeArgs(page, "read_artifact_version"))
    .toMatchObject({ versionId: "resource-version-markdown" });
});

// Programmatically select the rendered body of the center file preview and
// raise the quote popup (Playwright has no direct "select text" gesture).
async function selectCenterPreviewText(page: Page) {
  await page.evaluate(() => {
    const host = document.querySelector(".center-file-preview .md")
      ?? document.querySelector(".center-file-preview");
    if (!host) throw new Error("no center preview to select");
    const range = document.createRange();
    range.selectNodeContents(host);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true }));
  });
}

test("selecting preview text quotes it into chat and saves a review annotation", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await search.fill("analysis-report");
  await search.press("Enter");

  const modal = page.locator(".artifact-modal");
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Open in center" }).click();
  const preview = page.locator(".center-file-preview");
  await expect(preview.locator("h1")).toHaveText("Differential expression report");
  await expect(preview).toHaveAttribute("data-file-path", "artifact:art-markdown");

  // Selecting inside the preview raises the quote popup with all three actions.
  await selectCenterPreviewText(page);
  const popup = page.locator(".selection-popup");
  await expect(popup).toBeVisible();
  await expect(popup.getByRole("button", { name: "Add to chat" })).toBeVisible();
  await expect(popup.getByRole("button", { name: "Add to review" })).toBeVisible();

  // "Add to chat" attaches the selection as a composer quote chip (#274).
  await popup.getByRole("button", { name: "Add to chat" }).click();
  await expect(page.locator(".composer-reference-chips .quote")).toContainText("Differential expression report");

  // "Add to review" appends the passage to the reviews/ sidecar the agent reads.
  await selectCenterPreviewText(page);
  await page.locator(".selection-popup").getByRole("button", { name: "Add to review" }).click();
  await expect.poll(() => lastInvokeArgs(page, "append_review_note"))
    .toMatchObject({ sourcePath: "artifact:art-markdown" });
  await expect(page.locator(".topbar .hint")).toContainText("reviews/");
});

test("projects landing stays centered on wide windows", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/");
  await expect(page.locator(".projects-head")).toBeVisible();
  await expect.poll(async () => page.locator(".projects-head").evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return Math.round(rect.width);
  })).toBeLessThanOrEqual(1200);
});

test("Windows uses the integrated title bar without covering the project landing", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.locator(".window-titlebar")).toBeVisible();
  await expect(page.getByRole("button", { name: "Minimize" })).toBeVisible();
  await expect.poll(async () => page.locator(".projects-screen").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(38);

  await page.getByRole("button", { name: "Settings" }).click();
  await expect.poll(async () => page.locator(".settings-page").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(38);
  await page.getByRole("button", { name: "Back to app" }).click();

  await page.getByRole("button", { name: "File", exact: true }).click();
  await expect(page.getByRole("menuitem", { name: "Open projects" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Export current project" })).toBeDisabled();
  await page.getByRole("menuitem", { name: "Open projects" }).click();
  await expect(page.locator(".projects-screen")).toBeVisible();

  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await page.getByRole("button", { name: "File", exact: true }).click();
  const exportCurrentProject = page.getByRole("menuitem", { name: "Export current project" });
  await expect(exportCurrentProject).toBeEnabled();
  await exportCurrentProject.click();
  await expect.poll(() => lastInvokeArgs(page, "export_project")).toMatchObject({ id: "default" });

  await page.getByRole("button", { name: "Help" }).click();
  await page.getByRole("menuitem", { name: "Documentation" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_external_url")
      .map((c: any) => (c.args instanceof Map ? c.args.get("url") : c.args?.url))
  )).toContain("https://github.com/xuzhougeng/wisp-science#readme");

  await context.close();
});

test("macOS uses the native title bar without the integrated header", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.locator(".window-titlebar")).toHaveCount(0);
  await expect(page.locator(".window-controls")).toHaveCount(0);
  await expect(page.locator(".projects-screen")).toBeVisible();

  await page.getByRole("button", { name: "Settings" }).click();
  await expect.poll(async () => page.locator(".settings-page").evaluate((el) =>
    Math.round(el.getBoundingClientRect().top)
  )).toBe(0);
  await page.getByRole("button", { name: "Back to app" }).click();

  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();
  await openComputeMenu(page);
  await expect(page.locator('.compute-resource-row[data-context-id="ssh:gpu-server"]')).toBeVisible();

  await context.close();
});

test("project cards use semantic buttons for keyboard access", async ({ page }) => {
  await page.goto("/");
  const project = page.locator(".proj-card-main").first();
  await expect(project).toBeVisible();
  await expect(project.evaluate((el) => el.tagName)).resolves.toBe("BUTTON");
});

test("Escape closes settings and unwinds the composer picker before the right pane", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator(".settings-page")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator(".rightpane")).toBeVisible();
  await composer(page).fill("@");
  await expect(page.locator(".mention-menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".mention-menu")).toHaveCount(0);
  await expect(page.locator(".rightpane")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".rightpane")).toHaveCount(0);
});

test("Windows titlebar menus close on Escape", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/136 Safari/537.36",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await page.getByRole("button", { name: "File" }).click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);

  await context.close();
});

test("compact workspace keeps the conversation usable and opens Inspector as a drawer", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 720 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  await expect(page.locator(".rightpane-backdrop")).toBeVisible();
  await expect(page.locator(".rightpane")).toHaveCSS("position", "fixed");
  await expect.poll(async () => page.locator(".center").evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(700);

  await page.locator(".rightpane-backdrop").click({ position: { x: 16, y: 16 } });
  await expect(page.locator(".rightpane")).toHaveCount(0);
});

test("default Tauri workspace opens Inspector as a split pane", async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 760 });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();

  await expect(page.locator(".rightpane-backdrop")).toBeHidden();
  await expect(page.locator(".rightpane")).not.toHaveCSS("position", "fixed");
  await expect(page.locator(".resizer")).toBeVisible();
  await expect.poll(async () => page.locator(".center").evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(400);
});

test("project switcher does not show a stale fallback name while opening", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => (window as any).__delayNextProjectOpen("default", 250));
  await page.locator(".proj-card-main").first().click();

  await expect(page.locator(".proj-name")).toHaveText("Opening project…");
  await expect(page.locator(".proj-name")).toHaveText("wisp-science");
});

test("default workspace keeps history labels and compact navigation keeps hover labels", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 800 });
  await enterApp(page);

  const sidebar = page.locator(".sidebar");
  const resizer = page.locator(".sidebar-resizer");
  await expect(resizer).toBeVisible();
  const before = await sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width));
  const box = await resizer.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + 80);
  await page.mouse.down();
  await page.mouse.move(box!.x + 160, box!.y + 80);
  await page.mouse.up();
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(before + 140);

  // 1100px is the default Tauri window width. It must keep the history area
  // readable rather than hiding all session text behind an icon-only rail.
  await page.setViewportSize({ width: 1100, height: 760 });
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThan(200);
  await expect(page.locator(".side-hint")).toBeVisible();

  await page.setViewportSize({ width: 800, height: 720 });
  await expect.poll(async () => sidebar.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeLessThanOrEqual(64);
  await expect(newSessionButton(page)).toHaveAttribute("title", "New session");
  await expect(page.locator(".proj-switch")).toHaveAttribute("title", /.+/);

  await page.locator(".proj-switch").click();
  const menu = page.locator(".proj-menu");
  await expect(menu).toBeVisible();
  await expect.poll(async () => menu.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(220);
  await expect(page.getByRole("button", { name: /Project settings|项目设置/ })).toBeVisible();
});

test("new project form enables Create after name and folder are set", async ({ page }) => {
  // Stay on the Projects landing screen (don't enter a project).
  await page.goto("/");
  await page.getByRole("button", { name: "New project" }).click();
  const overlay = page.locator(".overlay", { has: page.locator("#new-project-name") });
  await expect.poll(() => overlay.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      viewportWidth: innerWidth,
      viewportHeight: innerHeight,
    };
  })).toMatchObject({ x: 0, y: 0, width: 1280, height: 720, viewportWidth: 1280, viewportHeight: 720 });
  await expect.poll(() => overlay.locator(".modal").evaluate((el) => el.getBoundingClientRect().top)).toBeGreaterThanOrEqual(20);
  const create = page.getByRole("button", { name: "Create" });
  await expect(create).toBeDisabled();
  // Typing the name must register in the signal — a wrong event-target cast
  // used to panic in the input handler, leaving the name empty and Create
  // permanently disabled.
  await page.getByPlaceholder("Project name").pressSequentially("My Project");
  await page.locator(".pn-dir .btn-ghost").click(); // Choose folder → mock path
  await expect(page.locator(".pn-dir .path")).toHaveText("/mock/root/new-project");
  await expect(create).toBeEnabled();
});

test("projects can be exported and imported from the landing screen", async ({ page }) => {
  await page.goto("/");
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  const exportProject = projectCard.getByRole("button", { name: "Export project" });
  await expect.poll(() => exportProject.evaluate((el) => Number.parseFloat(getComputedStyle(el).opacity))).toBeGreaterThan(0);
  await exportProject.click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "export_project"),
  )).toBe(true);

  await page.getByRole("button", { name: "Import project" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "import_project"),
  )).toBe(true);
});

test("projects sync manually, copy a device code, and join on another device", async ({ page }) => {
  await page.goto("/");
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Sync now" }).click();
  await expect(page.locator(".projects-sync-notice")).toContainText("Uploaded 1 changed workspace file");
  await expect(projectCard.locator(".pc-sync-state")).toContainText("Synced");
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "sync_project"),
  )).toBe(true);

  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Copy device code" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "project_sync_code"),
  )).toBe(true);

  await expect(page.getByRole("button", { name: "Join synced project" })).toHaveCount(0);
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "General", exact: true }).click();
  await page.getByRole("button", { name: "Join synced project" }).click();
  const joinDialog = page.getByRole("dialog", { name: "Join a synced project" });
  const deviceCode = page.getByTestId("sync-device-code");
  await expect(joinDialog).toBeVisible();
  await expect(deviceCode).toBeFocused();
  await expect(page.getByText("Secret device code", { exact: true })).toBeVisible();
  await expect.poll(async () => joinDialog.evaluate((el) => Math.round(el.getBoundingClientRect().width))).toBeGreaterThanOrEqual(520);
  await expect.poll(async () => joinDialog.getByRole("button", { name: "Cancel" }).first().evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return [Math.round(rect.width), Math.round(rect.height)];
  })).toEqual([34, 34]);

  await joinDialog.getByRole("button", { name: "Read sync guide" }).click();
  await expect.poll(() => lastInvokeArgs(page, "open_external_url")).toMatchObject({
    url: expect.stringContaining("docs/project-sync.md"),
  });

  await deviceCode.fill("wisp-sync:mock-secret-code");
  await page.getByRole("button", { name: "Choose destination and join" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((call: any) => call.cmd === "join_synced_project"),
  )).toBe(true);
});

test("general settings configure a cloud-drive sync folder", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "General");
  await page.getByTestId("sync-backend").selectOption("folder");
  await page.locator(".settings-path-row").getByRole("button", { name: "Choose folder" }).click();
  await expect(page.getByTestId("sync-folder")).toHaveValue("/mock/root/new-project");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { sync_backend: "folder", sync_folder: "/mock/root/new-project" },
  });
});

test("general settings save the maximum agent iterations", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "General");
  await page.getByTestId("max-iter").fill("0");
  await page.locator(".settings-footer").getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: { max_iter: 0 },
  });
});

test("pet stays off until the user explicitly configures its directory", async ({ page }) => {
  await page.goto("/");
  await openSettingsSection(page, "Pet");

  await expect(page.getByTestId("pet-enabled")).not.toBeChecked();
  await expect(page.getByTestId("pet-directory")).toHaveValue("");
  await page.getByTestId("pet-directory").fill("C:\\Users\\tester\\.codex\\pets\\wispy");
  await page.locator(".pet-settings-pane .toggle").click();
  await page.locator(".pet-settings-pane .settings-footer").getByRole("button", { name: "Save" }).click();

  await expect.poll(() => lastInvokeArgs(page, "set_settings")).toMatchObject({
    settings: {
      pet_enabled: true,
      pet_directory: "C:\\Users\\tester\\.codex\\pets\\wispy",
    },
  });

  await page.goto("/?pet=desktop&mockPet=1");
  const pet = page.getByTestId("wisp-pet");
  await expect(pet).toBeVisible();
  await expect.poll(() => pet.getAttribute("data-state")).toMatch(/^(idle|looking)$/);
  await pet.click();
  await expect(pet).toHaveAttribute("data-state", "waving");
});

test("desktop pet remains independent and reflects global agent state", async ({ page }) => {
  await page.goto("/?pet=desktop&mockPet=1");

  const pet = page.getByTestId("wisp-pet");
  await expect(page.getByTestId("pet-window-root")).toBeVisible();
  await expect(pet).toBeVisible();
  await expect(pet).toHaveAttribute("data-tauri-drag-region", "deep");
  await expect.poll(() => page.evaluate(() => (window as any).__petWindowVisible)).toBe(true);

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "User", frame_id: "pet-frame", text: "run" });
  });
  await expect(pet).toHaveAttribute("data-state", "running");
  await expect(pet.getByText("Working")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("confirm-request", { frame_id: "pet-frame", message: "Approve?" });
  });
  await expect(pet).toHaveAttribute("data-state", "waiting");
  await expect(pet.getByText("Needs you")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Text", frame_id: "pet-frame", delta: "continuing" });
    (window as any).__tauriEmit("agent", { kind: "ReviewStarted", frame_id: "pet-frame" });
  });
  await expect(pet).toHaveAttribute("data-state", "review");
  await expect(pet.getByText("Reviewing")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Error", frame_id: "pet-frame", message: "failed" });
  });
  await expect(pet).toHaveAttribute("data-state", "failed");
  await expect(pet.getByText("Failed")).toBeVisible();

  await page.evaluate(() => {
    (window as any).__tauriEmit("agent", { kind: "Done", frame_id: "pet-frame" });
  });
  await expect(pet).toHaveAttribute("data-state", "jumping");
});

test("a sync conflict requires an explicit authoritative device choice", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => { (window as any).__failSyncConflict = true; });
  const projectCard = page.locator(".proj-card:not(.proj-example)").first();
  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Sync now" }).click();
  await expect(page.getByRole("dialog", { name: "Both devices changed this project" })).toBeVisible();
  await page.getByRole("button", { name: "Use remote version" }).click();
  await expect.poll(() => lastInvokeArgs(page, "resolve_project_sync")).toMatchObject({
    id: "default", strategy: "remote",
  });
});

test("a second conversation can run in parallel without interleaving transcripts", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  // Start conversation A. The mock streams "echo:alpha" at once but delays Done,
  // so A stays "running".
  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });

  // While A is still running, open a fresh session. The composer must be usable
  // (per-session busy: A running does NOT block B).
  await newSessionButton(page).click();
  await expect(composer(page)).toBeEmpty();
  await composer(page).fill("beta");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:beta")).toBeVisible({ timeout: 10_000 });

  // A's transcript must not leak into B's view.
  await expect(page.getByText("echo:alpha")).toHaveCount(0);

  // A is still running → its sidebar entry shows the running indicator.
  await expect(page.locator(".side-item.ses.running")).toBeVisible();

  // Switch back to A: the cached (live) transcript renders, B's does not.
  await page.locator(".side-item.ses", { hasText: "alpha" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("echo:beta")).toHaveCount(0);
});

test("a running conversation accepts another message for queueing", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(newSessionButton(page)).toBeVisible();

  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });

  await composer(page).fill("queued");
  const send = page.getByRole("button", { name: "Queue" });
  await expect(send).toBeEnabled({ timeout: 500 });
  await send.click();
  const queued = page.locator(".msg.user.queued .body", { hasText: /^queued$/ });
  await expect(queued).toBeVisible({ timeout: 500 });

  // The first turn keeps streaming after the second is queued. Its tail must
  // stay attached to the first assistant row instead of leaking into a hidden
  // placeholder after the queued user message (#143).
  await expect(page.getByText("echo:alpha:tail", { exact: true })).toBeVisible({ timeout: 3_000 });
  await expect(queued).toBeVisible();
  await expect(page.getByText("echo:queued")).toHaveCount(0);

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__sendInvokeLog ?? []).filter((c: any) => c.cmd === "send_message");
    return calls.map((c: any) => c.args?.message);
  })).toEqual(["alpha", "queued"]);
});

test("deleting a project uses an in-app confirm modal, not native confirm (#96)", async ({ page }) => {
  // Native window.confirm() is a no-op in this webview (wry's WKUIDelegate has
  // no JS confirm panel), so the ✕ silently did nothing. Deletion now goes
  // through an in-app modal.
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example) .pc-del").first().click();
  const modal = page.locator(".confirm-modal");
  await expect(modal).toBeVisible();
  await modal.locator("button.primary").click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((c: any) => c.cmd === "delete_project"),
  )).toBe(true);
});

test("external links open in the system browser, not the app webview (#97)", async ({ page }) => {
  // A reference link in rendered markdown used to navigate the whole webview
  // away from the UI with no way back. Any external <a> must now be intercepted
  // and handed to the system browser instead.
  await enterApp(page);
  await page.evaluate(() => {
    const a = document.createElement("a");
    a.id = "ext-link-probe";
    a.href = "https://example.com/paper.pdf";
    a.textContent = "open paper";
    document.body.appendChild(a);
  });
  await page.click("#ext-link-probe");
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_external_url")
      // serde_wasm_bindgen passes args as a JS Map, not a plain object.
      .map((c: any) => (c.args instanceof Map ? c.args.get("url") : c.args?.url)),
  )).toContain("https://example.com/paper.pdf");
  // The app itself must still be on screen — the click was intercepted, not
  // followed as a top-level navigation.
  await expect(newSessionButton(page)).toBeVisible();
});

test("assistant markdown uses normal whitespace (no phantom blank lines)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("MDLIST");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("FX细胞")).toBeVisible({ timeout: 10_000 });
  const whiteSpace = await page.locator(".msg.assistant .body.md").first().evaluate(
    (el) => getComputedStyle(el).whiteSpace,
  );
  expect(whiteSpace).toBe("normal");
});

test("a thinking + tool run folds into one collapsible steps panel (#82)", async ({ page }) => {
  // Instead of a wall of separate tool cards, consecutive thinking/tool activity
  // collapses into a single foldable "Ran N steps" panel, collapsed by default.
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  // The assistant answer renders as a normal message…
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  // …and the 3 tool calls collapse into exactly one steps panel, closed by default.
  const steps = page.locator(".steps");
  await expect(steps).toHaveCount(1);
  await expect(steps).not.toHaveClass(/open/);
  await expect(page.locator(".step-body:visible")).toHaveCount(0);
  // Expanding reveals the individual steps (3 tools + folded thinking).
  await page.locator(".steps-head").click();
  await expect(steps).toHaveClass(/open/);
  await expect(page.locator(".steps .step-name")).toContainText(["thinking", "shell", "python", "write"]);
});

test("live step disclosure choices survive tool updates and completion (#172)", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSLIVE");
  await page.getByRole("button", { name: "Send" }).click();

  const steps = page.locator(".steps");
  const shell = page.locator(".steps .step", { hasText: "shell" }).first();
  await expect(steps).toHaveClass(/open/, { timeout: 2_000 });
  await expect(shell).toHaveClass(/open/);

  // Record explicit user choices rather than relying on the automatic live
  // defaults. Each following event changes the row fingerprint and remounts
  // its rendered content.
  await page.locator(".steps-head").click();
  await expect(steps).not.toHaveClass(/open/);
  await page.locator(".steps-head").click();
  await expect(steps).toHaveClass(/open/);
  await shell.locator(".step-head").click();
  await expect(shell).not.toHaveClass(/open/);
  await shell.locator(".step-head").click();
  await expect(shell).toHaveClass(/open/);

  await expect(shell.locator(".tool-output")).toContainText("shell output line", { timeout: 4_000 });
  await expect(steps).toHaveClass(/open/);
  await expect(shell).toHaveClass(/open/);

  await expect(page.getByText("Live steps finished.")).toBeVisible({ timeout: 4_000 });
  await expect(steps).toHaveClass(/open/);
  await expect(shell).toHaveClass(/open/);
});

test("ACP thinking folds into the steps panel instead of dangling under the reply", async ({ page }) => {
  await enterApp(page);
  await newSessionButton(page).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACPTHINK");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Let me search the literature first.")).toBeVisible({ timeout: 4_000 });

  // Thinking + the ACP tool coalesce into exactly one steps panel (order within
  // the panel follows flush timing, so assert membership, not position)…
  const steps = page.locator(".steps");
  await expect(steps).toHaveCount(1);
  await steps.locator(".steps-head").click();
  await expect(steps.getByText("thinking")).toBeVisible();
  await expect(steps.getByText("web_search")).toBeVisible();
  // …and there is no lone thinking row stranded outside the panel (the bug).
  await expect(page.locator(".msg.reasoning")).toHaveCount(0);

  // The panel sits above the reply, not below it.
  const stepsY = await steps.evaluate((el) => el.getBoundingClientRect().top);
  const replyY = await page
    .getByText("Let me search the literature first.")
    .evaluate((el) => el.getBoundingClientRect().top);
  expect(stepsY).toBeLessThan(replyY);
});

test("code lives in Notebook instead of Artifacts", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });

  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Notebook (2)", exact: true }).click();

  const cells = page.locator(".notebook-cell");
  await expect(cells).toHaveCount(2);
  await expect(cells.nth(0).locator(".notebook-language")).toHaveText("bash");
  await expect(cells.nth(1).locator(".notebook-language")).toHaveText("python");
  await expect(cells.nth(1)).toContainText("import pandas as pd");
  await cells.nth(1).locator(".notebook-output summary").click();
  await expect(cells.nth(1).locator(".notebook-output pre")).toContainText("col_0: ok");

  await page.getByRole("button", { name: "Artifacts", exact: true }).click();
  await expect(page.locator(".rp-badge.code")).toHaveCount(0);
});

test("R tool calls project into a highlighted Notebook cell", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("RNOTEBOOK");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("R summary complete.")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Notebook (1)", exact: true }).click();

  const cell = page.locator(".notebook-cell");
  await expect(cell.locator(".notebook-language")).toHaveText("r");
  await expect(cell.locator("code.language-r")).toContainText("summary(dataset)");
  await expect(cell.locator("code.language-r")).not.toContainText("ssh:gpu-server");
  await cell.locator(".notebook-output summary").click();
  await expect(cell.locator(".notebook-output pre")).toContainText("Length Class Mode");
});

test("an SVG star saves a Notebook cell in the global library", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("STEPSDEMO");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText(/60,675 genes/)).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Notebook (2)", exact: true }).click();

  const cell = page.locator(".notebook-cell").first();
  const star = cell.getByRole("button", { name: "Add to library" });
  await expect(star.locator("svg path")).toHaveCount(1);
  await expect(star).toHaveText("");
  const copy = cell.getByRole("button", { name: "Copy code" });
  await expect.poll(() => copy.evaluate((node) =>
    node.previousElementSibling?.classList.contains("notebook-star") ?? false,
  )).toBe(true);
  await star.click();
  await expect(cell.getByRole("button", { name: "Remove from library" })).toHaveAttribute("aria-pressed", "true");

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await expect(page.getByTestId("library-screen")).toBeVisible();
  await expect(page.locator('.library-card[data-library-kind="code"]')).toContainText("zcat counts.txt.gz");
  await expect(page.locator('.library-card[data-library-kind="code"]')).toContainText("wisp-science / Current analysis");
});

test("a starred figure keeps its image and generating code", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("make a volcano plot volcano.png");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="volcano.png"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  const star = modal.getByRole("button", { name: "Add to library" });
  await expect(star.locator("svg path")).toHaveCount(1);
  const openCenter = modal.getByRole("button", { name: "Open in center" });
  await expect.poll(() => openCenter.evaluate((node) =>
    node.previousElementSibling?.getAttribute("aria-label"),
  )).toBe("Add to library");
  await star.click();
  await expect(modal.getByRole("button", { name: "Remove from library" })).toHaveAttribute("aria-pressed", "true");
  await modal.getByRole("button", { name: "Close panel" }).click();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  const figure = page.locator('.library-card[data-library-kind="figure"]');
  await expect(figure).toContainText("volcano.png");
  await figure.locator(".library-card-main").click();
  const detail = page.locator(".library-detail");
  await expect(detail.locator(".library-figure img")).toBeVisible();
  await expect(detail).toContainText("Generating code");
  await expect(detail).toContainText("savefig");

  await detail.getByRole("button", { name: "Remove from library" }).click();
  await expect(page.locator('.library-card[data-library-kind="figure"]')).toHaveCount(0);
});

test("a project card can open its project in a new window (#52)", async ({ page }) => {
  await page.goto("/");
  await page.locator(".proj-card:not(.proj-example) .pc-window").first().click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_project_window")
      .map((c: any) => (c.args instanceof Map ? c.args.get("id") : c.args?.id)),
  )).toContain("default");
});

test("a ?project window opens straight into the project, skipping the landing (#52)", async ({ page }) => {
  // A dedicated project window carries ?project=<id>; it must open that project
  // directly (per-window active) instead of showing the projects landing.
  await page.goto("/?project=default");
  await expect(newSessionButton(page)).toBeVisible({ timeout: 10_000 });
  // The landing (project cards) must NOT be shown in a dedicated project window.
  await expect(page.locator(".proj-card")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((c: any) => c.cmd === "open_project"),
  )).toBe(true);
});

test("specialists page lists builtin reviewer without a delete affordance and saves a custom specialist", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await expect(page.getByText("Reviewer")).toBeVisible();
  // Only the builtin specialist exists so far: its list row has no remove button.
  await expect(page.locator(".settings-list-remove")).toHaveCount(0);

  // builtin row: open it and verify instructions are disabled
  await page.getByText("Reviewer").click();
  await expect(page.getByLabel("Instructions")).toBeDisabled();
  await page.locator(".settings-head-back").click();

  await page.getByText("Add specialist").click();
  await page.getByText("Write from scratch").click();
  await page.getByLabel("Name").fill("Paper hunter");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Paper hunter")).toBeVisible();
});

test("Reviewer settings select, test, and persist an ACP backend", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Reviewer").click();

  const backend = page.getByTestId("reviewer-backend-select");
  await expect(backend.locator('option[value="acp:acp-test"]')).toHaveCount(1);
  await backend.selectOption("acp:acp-test");
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText("Test ACP Agent");
  await expect(backend).toHaveValue("acp:acp-test");
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText("ACP");

  await page.getByTestId("test-reviewer-backend").click();
  await expect.poll(() => lastInvokeArgs(page, "test_reviewer_backend")).toMatchObject({
    reviewer: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  await expect(page.locator(".settings-status")).toContainText(
    "valid review JSON via ACP / Test ACP Agent",
  );

  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_specialist_cmd")).toMatchObject({
    spec: {
      id: "reviewer",
      review_backend: { kind: "acp_agent", profile_id: "acp-test" },
    },
  });
  await expect(page.locator(".settings-status")).toContainText("Specialist saved");
  await expect(backend).toHaveValue("acp:acp-test");

  await page.locator(".settings-head-back").click();
  await page.getByText("Reviewer").click();
  await expect(page.getByTestId("reviewer-backend-select")).toHaveValue("acp:acp-test");
});

test("a deleted ACP reviewer remains visibly selected as missing", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Reviewer").click();
  await page.getByTestId("reviewer-backend-select").selectOption("acp:acp-test");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.locator(".settings-status")).toContainText("Specialist saved");

  const nav = page.locator(".settings-nav");
  await nav.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByTestId("open-acp-agents-from-settings").click();
  const row = page.getByTestId("acp-agent-row").filter({ hasText: "Test ACP Agent" });
  await row.locator(".settings-list-remove").click();
  await expect(row).toHaveCount(0);

  await nav.getByRole("button", { name: "Specialists", exact: true }).click();
  await page.getByText("Reviewer").click();
  const backend = page.getByTestId("reviewer-backend-select");
  await expect(backend).toHaveValue("acp:acp-test");
  await expect(page.getByTestId("reviewer-missing-acp-option")).toHaveText(
    "Missing ACP Agent · acp-test",
  );
  await expect(page.getByTestId("reviewer-selected-backend")).toContainText(
    "Missing ACP Agent · acp-test",
  );

  await page.getByTestId("test-reviewer-backend").click();
  await expect(page.locator(".settings-status")).toContainText(
    "Reviewer backend test failed: The Reviewer ACP Agent profile no longer exists.",
  );
});

test("new session can pick a specialist and it locks after the first message", async ({ page }) => {
  await enterApp(page);
  // Create the custom specialist through the settings flow, as above.
  await openSettingsSection(page, "Specialists");
  await page.getByText("Add specialist").click();
  await page.getByText("Write from scratch").click();
  await page.getByLabel("Name").fill("Paper hunter");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("Paper hunter")).toBeVisible();
  await page.locator(".settings-head-close").click();

  // Picking a specialist requires an active session (set lazily on first send
  // otherwise), so start one explicitly via "New session".
  await newSessionButton(page).click();
  let agentMenu = await openAgentMenu(page);
  await agentMenu.getByRole("button", { name: /^Specialist/ }).click();
  await page.getByRole("menu", { name: "Specialist" }).getByRole("button", { name: "Paper hunter" }).click();
  await expect(page.locator(".session-specialist")).toHaveText("Paper hunter");

  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  agentMenu = await openAgentMenu(page);
  await expect(agentMenu.getByRole("button", { name: /^Specialist/ })).toBeDisabled();
});

test("chat-with-claude creation opens a new session with the interview prompt", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Specialists");
  await page.getByText("Add specialist").click();
  await page.getByText("Chat with Claude").click();
  // settings closed, a session is active, and send_message was invoked with the template
  await expect(page.locator(".settings-page")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message").length,
  )).toBeGreaterThan(0);
});

test("remote access settings: Feishu QR/manual setup and WeChat QR binding", async ({ page }) => {
  await enterApp(page);
  await openSettingsSection(page, "Remote Access");

  // List page: routing note plus one row per bot, toggles disabled until bound.
  await expect(page.getByTestId("channel-routing-help")).toBeVisible();
  await expect(page.getByTestId("channel-routing-help").getByText("/project", { exact: true })).toBeVisible();
  await expect(page.getByTestId("channel-routing-help").getByText("/session", { exact: true })).toBeVisible();
  await expect(page.getByTestId("feishu-channel-row")).toBeVisible();
  await expect(page.getByTestId("weixin-channel-row")).toBeVisible();
  await expect(page.getByTestId("feishu-enabled")).toBeDisabled();
  await expect(page.getByTestId("weixin-enabled")).toBeDisabled();

  // Feishu subpage: existing applications still have a manual, keyring-backed
  // setup path.
  await page.getByTestId("feishu-channel-row").click();
  await expect(page.getByTestId("feishu-channel-card")).toBeVisible();
  await page.getByTestId("feishu-international").check();
  await page.getByTestId("feishu-app-id").fill("cli_test123");
  await page.getByTestId("feishu-app-secret").fill("secret-xyz");
  await page.getByTestId("feishu-save").click();
  await expect.poll(() => lastInvokeArgs(page, "set_feishu_channel")).toMatchObject({
    enabled: false,
    international: true,
    appId: "cli_test123",
    appSecret: "secret-xyz",
  });

  // Removing local credentials does not claim to delete the remote app. The
  // one-click path then shows a real QR lifecycle and stores credentials in
  // the backend without exposing the secret to the webview.
  await page.getByTestId("feishu-unbind").click();
  await expect(page.getByTestId("feishu-bind")).toBeVisible();
  await page.getByTestId("feishu-bind").click();
  await expect(page.getByTestId("feishu-qr")).toBeVisible();
  await expect(page.getByTestId("feishu-unbind")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("feishu-app-id")).toHaveValue("cli_scan_created");

  // Back on the list the bound bot's toggle is now enabled.
  await page.locator(".settings-head-back").click();
  await expect(page.getByTestId("feishu-enabled")).toBeEnabled();

  // WeChat subpage: QR binding. The 2s poll hits the mock's immediate
  // "confirmed": QR goes away and the bind button flips to unbind.
  await page.getByTestId("weixin-channel-row").click();
  await page.getByTestId("weixin-bind").click();
  await expect(page.getByTestId("weixin-qr")).toBeVisible();
  await expect(page.getByTestId("weixin-unbind")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("weixin-qr")).toHaveCount(0);

  await page.locator(".settings-head-back").click();
  await expect(page.getByTestId("weixin-enabled")).toBeEnabled();

  await page.getByTestId("weixin-channel-row").click();
  await page.getByTestId("weixin-unbind").click();
  await expect(page.getByTestId("weixin-bind")).toBeVisible({ timeout: 10_000 });
});
