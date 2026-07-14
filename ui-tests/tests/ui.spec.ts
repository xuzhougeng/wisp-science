import { test, expect, type Page } from "@playwright/test";
import { tauriMock, parallelMock } from "./mock-tauri";

function providerSelect(page: Page) {
  return page.getByTestId("settings-provider");
}

function terminalTauriMock(): void {
  class Channel {
    onmessage: ((message: any) => void) | null = null;
  }
  (window as any).__terminalInvokeLog = [];
  (window as any).__terminalPinned = false;
  (window as any).__TAURI__ = {
    core: {
      Channel,
      invoke: async (cmd: string, args: any) => {
        (window as any).__terminalInvokeLog.push({ cmd, args });
        if (cmd === "attach_terminal") {
          setTimeout(() => args.onEvent.onmessage?.({
            event: "output",
            data: { base64: btoa("terminal ready\r\n") },
          }), 0);
          return {
            id: "term-1",
            projectId: "default",
            contextId: "ssh:gpu-server",
            title: "gpu-server — Terminal",
            kind: "ssh",
            displayCwd: "~",
            processId: 1234,
            running: true,
          };
        }
        return null;
      },
    },
    window: {
      getCurrentWindow: () => ({
        setAlwaysOnTop: async (value: boolean) => {
          (window as any).__terminalPinned = value;
        },
      }),
    },
  };
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
async function enterApp(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();
}

function composer(page: Page) {
  return page.locator("#composer-input");
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

test.beforeEach(async ({ page }, testInfo) => {
  // Install the Tauri bridge mock before the page's wasm runs.
  if (!testInfo.title.startsWith("terminal window")) {
    await page.addInitScript(tauriMock);
  }
});

test("terminal window attaches, accepts input, pins, and terminates", async ({ page }) => {
  await page.addInitScript(terminalTauriMock);
  await page.goto("/terminal.html?session=term-1");

  await expect(page.locator("#terminal-title")).toHaveText("gpu-server — Terminal");
  await expect(page.locator("#terminal-context")).toHaveText("ssh:gpu-server");
  await expect(page.locator(".xterm-rows")).toContainText("terminal ready");

  await page.locator("#terminal-container").click();
  await page.keyboard.type("echo hello");
  await expect.poll(() => page.evaluate(() =>
    (window as any).__terminalInvokeLog
      .filter((call: any) => call.cmd === "write_terminal")
      .map((call: any) => call.args.data)
      .join("")
      .includes("echo hello"),
  )).toBe(true);

  await page.locator("#terminal-pin").click();
  await expect.poll(() => page.evaluate(() => (window as any).__terminalPinned)).toBe(true);

  page.once("dialog", (dialog) => dialog.accept());
  await page.locator("#terminal-terminate").click();
  await expect.poll(() => page.evaluate(() =>
    (window as any).__terminalInvokeLog.some((call: any) => call.cmd === "terminate_terminal"),
  )).toBe(true);
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
  await page.locator(".model-picker-btn").click();
  await page.getByTestId("add-acp-agent").click();
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
  await page.getByRole("button", { name: "New session" }).click();
  await page.locator(".model-picker-btn").click();
  await page.getByRole("button", { name: /Test ACP Agent/ }).click();
  await composer(page).fill("ACP PERMISSION");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Hello from ACP.")).toBeVisible();
  await expect(page.getByTestId("acp-tool")).toHaveCount(2);
  await expect(page.getByText("Inspect")).toBeVisible();
  const config = page.getByTestId("acp-session-config");
  await expect(config).toContainText("code");
  await expect(config).toContainText("Smart");
  await config.getByRole("button", { name: "Model" }).click();
  await page.getByRole("option", { name: "Fast" }).click();
  await expect.poll(() => lastInvokeArgs(page, "set_acp_session_config")).toMatchObject({
    configId: "model", value: { value: "fast" },
  });

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
  await page.getByRole("button", { name: "New session" }).click();
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
  await page.getByRole("button", { name: "New session" }).click();
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
  await page.getByRole("button", { name: "New session" }).click();
  await expect(composer(page)).toBeFocused();
});

test("rename session modal autofocuses so Ctrl+A selects the title", async ({ page }) => {
  await page.addInitScript(parallelMock);
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();

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
  await expect(page.locator(".center-title")).not.toContainText("Uploaded files:");
  // The right panel starts collapsed; open it to see the collected artifact.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // The upload path lives in the user turn; the panel must pick it up from there.
  const tile = page.locator('.rp-tile[data-artifact-name="counts.csv"]');
  await expect(tile).toBeVisible();
  await tile.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Open in center" }).click();
  await expect(page.locator(".center-tab.active")).toContainText("counts.csv");
  await expect(page.locator(".center-file-preview")).toContainText("a");
  await page.getByRole("button", { name: "Conversation" }).click();
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

  await remoteFile.click({ button: "right" });
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Download" })).toBeVisible();
  await expect(page.locator(".ctx-menu").getByRole("button", { name: "Open in center" })).toHaveCount(0);
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({
    path: "ssh://gpu-server/home/research/notes.txt",
  });

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

test("right panel shows execution contexts and runs", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await expect.poll(() => page.locator(".rp-tab-add-menu").evaluate((menu) => {
    const rect = menu.getBoundingClientRect();
    const hit = document.elementFromPoint(rect.left + 8, rect.top + 8);
    return hit === menu || menu.contains(hit);
  })).toBe(true);
  await page.getByRole("button", { name: "Contexts" }).click();

  await expect(page.locator(".context-card", { hasText: "local" })).toContainText("Local machine");
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
  await expect(terminalDock.locator("iframe")).toHaveAttribute("src", /terminal\.html\?session=terminal-mock&embedded=1/);
  await expect(terminalDock.locator("iframe").contentFrame().locator(".xterm-rows")).toContainText("terminal ready");
  await terminalDock.getByRole("button", { name: "Close terminal panel" }).click();
  await expect(terminalDock).toBeHidden();
  await sshContext.getByRole("button", { name: "Open terminal" }).click();
  await expect(terminalDock).toBeVisible();
  await terminalDock.getByRole("button", { name: "Terminate" }).click();
  await expect.poll(() => lastInvokeArgs(page, "terminate_terminal")).toMatchObject({
    sessionId: "terminal-mock",
  });
  await expect(terminalDock.getByRole("button", { name: "Terminate" })).toBeDisabled();
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("succeeded");
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("ssh:gpu-server");
  const remoteRun = page.locator(".run-card", { hasText: "Kinase screen QC" });
  await expect(remoteRun).toContainText("~/.wisp-science/runs/run-kinase-001");
  await remoteRun.getByText("Latest output").click();
  await expect(remoteRun).toContainText("wrote qc table");

  await page.getByRole("button", { name: "Refresh runs" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "list_runs").length,
  )).toBeGreaterThan(1);
});

test("runtime panel shows lifecycle state and controls start stop restart", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Contexts" }).click();

  const localPython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="local"]');
  const localR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="local"]');
  const remotePython = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="ssh:gpu-server"]');
  const remoteR = page.locator('.runtime-card[data-runtime-language="r"][data-runtime-context="ssh:gpu-server"]');

  await expect(localPython).toContainText("Ready");
  await expect(localPython).toContainText("512.0 MB");
  await expect(remotePython).toContainText("Busy");
  await expect(remotePython).toContainText("10.0 GB");
  await expect(localR).toContainText("Dead");
  await expect(remoteR).toContainText("Not started");

  await localPython.getByRole("button", { name: "Stop" }).click();
  await expect(localPython).toContainText("Dead");
  await expect.poll(() => lastInvokeArgs(page, "stop_runtime")).toMatchObject({
    projectId: "default",
    contextId: "local",
    language: "python",
  });

  await localR.getByRole("button", { name: "Restart" }).click();
  await expect(localR).toContainText("Ready");
  await expect.poll(() => lastInvokeArgs(page, "restart_runtime")).toMatchObject({
    projectId: "default",
    contextId: "local",
    language: "r",
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
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Contexts" }).click();

  const runtime = page.locator('.runtime-card[data-runtime-language="python"][data-runtime-context="local"]');
  await runtime.getByRole("button", { name: "Refresh objects" }).click();

  await expect(runtime.locator(".runtime-object-row", { hasText: "counts" })).toContainText("DataFrame");
  await expect(runtime.locator(".runtime-object-row", { hasText: "counts" })).toContainText("12000000 × 48");
  await expect(runtime.locator(".runtime-object-row", { hasText: "counts" })).toContainText("4.0 GB");
  await expect(runtime.locator(".runtime-object-row", { hasText: "model" })).toContainText("RandomForestClassifier");
  await expect.poll(() => lastInvokeArgs(page, "inspect_runtime")).toMatchObject({
    projectId: "default",
    contextId: "local",
    language: "python",
  });
});

test("Windows contexts panel imports installed WSL distributions", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", {
      configurable: true,
      value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    });
  });
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Contexts" }).click();

  await page.getByRole("button", { name: "Import WSL" }).click();

  await expect.poll(() => lastInvokeArgs(page, "import_wsl_contexts")).not.toBeNull();
  await expect(page.locator(".context-card", { hasText: "wsl:Ubuntu-24.04" })).toContainText("Ubuntu-24.04");
});

test("running run can be cancelled from the contexts panel", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Add panel" }).click();
  await page.getByRole("button", { name: "Contexts" }).click();

  const run = page.locator(".run-card", { hasText: "Local normalization" });
  await run.getByRole("button", { name: "Cancel run" }).click();
  await expect(run).toContainText("cancelled");
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).some((c: any) => c.cmd === "cancel_run"),
  )).toBe(true);
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
  await expect(modal.locator('.rp-pdf[data-page-count="2"][data-current-page="1"]')).toBeVisible();
  const renderedPage = modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]');
  await expect(renderedPage).toBeVisible();
  await expect(modal.locator(".rp-pdf-page")).toHaveCount(1);
  const canvas = renderedPage.locator("canvas");
  await expect(canvas).toBeVisible();
  await expect.poll(() => canvas.evaluate(
    (el: HTMLCanvasElement) => el.width * el.height,
  )).toBeGreaterThan(0);
  await expect(page.getByRole("button", { name: "Previous page" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Next page" }).locator("svg")).toBeVisible();
  await expect(modal.locator('embed[type="application/pdf"]')).toHaveCount(0);
});

test("PDF artifacts switch pages with toolbar buttons and Page Up or Page Down", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("open paper.pdf");
  await page.getByRole("button", { name: "Send" }).click();
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.locator('.rp-tile[data-artifact-name="paper.pdf"] .rp-tile-main').click();

  const modal = page.locator(".artifact-modal");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  await page.getByRole("button", { name: "Next page" }).click();
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]')).toBeVisible();
  await expect(page.getByRole("button", { name: "Next page" })).toBeDisabled();

  await page.keyboard.press("PageUp");
  await expect(modal.locator('.rp-pdf[data-current-page="1"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="1"][data-rendered="true"]')).toBeVisible();

  await page.keyboard.press("PageDown");
  await expect(modal.locator('.rp-pdf[data-current-page="2"]')).toBeVisible();
  await expect(modal.locator('.rp-pdf-page[data-page="2"][data-rendered="true"]')).toBeVisible();
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

  await expect(providerSelect(page)).toHaveValue("openai");
  await expect(effort).toHaveValue("default");
  await expect(key).toHaveValue("");
  await expect(key).toHaveAttribute("placeholder", "(stored — leave blank to keep)");

  if (await useForVision.isChecked()) {
    await useForVision.uncheck();
  }
  await useForVision.check();

  await expect(providerSelect(page)).toHaveValue("openai");
  await expect(effort).toHaveValue("default");
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
      provider: "openai",
      reasoning_effort: "",
      use_for_vision: true,
    },
  });

  await page.locator(".settings-list-row").first().click();
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

  const row = page.locator(".settings-list-row", { hasText: "wolai_cmp" });
  await row.click();
  await expect(page.getByText("wolai_search")).toBeVisible();
  await expect(page.getByText("Search Wolai pages")).toBeVisible();

  await page.locator(".settings-head-back").click();
  await row.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByLabel("Name")).toHaveValue("wolai_cmp");
  await expect(page.getByPlaceholder("https://host/mcp")).toHaveValue("https://api.wolai.com/v1/mcp/");
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
  await expect.poll(() => frame.evaluate((el: HTMLIFrameElement) =>
    getComputedStyle(el.contentDocument!.querySelector("#mode")!, "::after").content,
  )).toBe('"Desktop"');
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
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();
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

  await context.close();
});

test("project cards use semantic buttons for keyboard access", async ({ page }) => {
  await page.goto("/");
  const project = page.locator(".proj-card-main").first();
  await expect(project).toBeVisible();
  await expect(project.evaluate((el) => el.tagName)).resolves.toBe("BUTTON");
});

test("Escape closes settings on the projects landing and the right pane from the composer", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator(".settings-page")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-page")).toHaveCount(0);

  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await expect(page.locator(".rightpane")).toBeVisible();
  await composer(page).focus();
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
  await expect(page.getByRole("button", { name: "New session" })).toHaveAttribute("title", "New session");
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
  await projectCard.hover();
  await projectCard.getByRole("button", { name: "Export project" }).click();
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
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();

  // Start conversation A. The mock streams "echo:alpha" at once but delays Done,
  // so A stays "running".
  await composer(page).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });

  // While A is still running, open a fresh session. The composer must be usable
  // (per-session busy: A running does NOT block B).
  await page.getByRole("button", { name: "New session" }).click();
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
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();

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
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();
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
  await page.getByRole("button", { name: "New session" }).click();
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
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible({ timeout: 10_000 });
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
  await page.getByRole("button", { name: "New session" }).click();
  await page.getByRole("button", { name: "Specialist" }).click();
  await page.getByRole("button", { name: "Paper hunter" }).click();
  await expect(page.locator(".session-specialist")).toHaveText("Paper hunter");

  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByRole("button", { name: "Specialist" }).click();
  await expect(page.getByRole("button", { name: "Paper hunter" })).toBeDisabled();
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
