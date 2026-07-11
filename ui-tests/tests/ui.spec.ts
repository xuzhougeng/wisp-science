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

  const review = page.locator(".review-card");
  await expect(review).toContainText("Reviewer findings");
  await expect(review.locator(".review-model")).toHaveText("claude-sonnet-5 · high");
  await expect(review).toContainText("resolved");
  await expect(review).toContainText("All findings fixed and independently rechecked.");
  await expect(review.locator(".review-finding")).toHaveCount(1);
  await review.getByRole("button", { name: "Go to transcript" }).click();
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
  await expect(page.locator(".settings-modal")).toBeVisible();
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

test("right-click export invokes active session export", async ({ page }) => {
  await enterApp(page);
  await composer(page).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByText("Hello from mock wisp-science.").click({ button: "right" });
  await page.getByRole("button", { name: "Export session" }).click();

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "export_session") ?? null;
  })).toMatchObject({
    cmd: "export_session",
    args: {
      sessionId: expect.stringMatching(/^s-/),
      artifactPaths: expect.any(Array),
    },
  });
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
  await expect(page.locator(".msg.user")).toContainText("Uploaded files: uploads/counts.csv");
  // The right panel starts collapsed; open it to see the collected artifact.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // The upload path lives in the user turn; the panel must pick it up from there.
  const tile = page.locator('.rp-tile[data-artifact-name="counts.csv"]');
  await expect(tile).toBeVisible();
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
  await file.click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Download" }).click();
  await expect.poll(() => lastInvokeArgs(page, "download_file")).toMatchObject({ path: "report.csv" });
  await file.click({ button: "right" });
  await page.getByRole("button", { name: "Attach to chat" }).click();
  await expect(page.locator(".composer-attachment.ready")).toHaveText("report.csv");
  await expect(composer(page)).toHaveValue("");
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
  // Code tab renders the recorded source (from get_artifact_provenance).
  await page.locator(".am-tab", { hasText: "Code" }).click();
  await expect(page.locator(".artifact-modal")).toContainText("savefig");
  // Environment tab renders the captured package list.
  await page.locator(".am-tab", { hasText: "Environment" }).click();
  await expect(page.locator(".am-env")).toContainText("matplotlib");
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

test("settings modal shows the saved provider", async ({ page }) => {
  await enterApp(page);
  await openModelsSettings(page);
  await expect(providerSelect(page)).toHaveValue("openai");
  await expect(page.locator("label.settings-check", { hasText: "Supports image input" })).toHaveCSS("flex-direction", "row");
  await expect(page.locator("label.settings-check", { hasText: "Use for image analysis" })).toHaveCSS("flex-direction", "row");
  await page.locator(".settings-footer").getByRole("button", { name: "Cancel" }).click();
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
  await expect(page.locator(".settings-modal")).toBeVisible();
  await page.locator(".settings-head-close").click();

  await page.getByRole("button", { name: "Search" }).click();
  const search = commandPalette(page);
  await expect(search).toBeVisible();
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
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

  await page.getByRole("button", { name: "File" }).click();
  await expect(page.getByRole("menuitem", { name: "Open projects" })).toBeVisible();
  await page.getByRole("menuitem", { name: "Open projects" }).click();
  await expect(page.locator(".projects-screen")).toBeVisible();

  await page.getByRole("button", { name: "Help" }).click();
  await page.getByRole("menuitem", { name: "Documentation" }).click();
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? [])
      .filter((c: any) => c.cmd === "open_external_url")
      .map((c: any) => (c.args instanceof Map ? c.args.get("url") : c.args?.url))
  )).toContain("https://github.com/xuzhougeng/wisp-science#readme");

  await context.close();
});

test("macOS uses the integrated title bar but keeps native traffic lights", async ({ browser }) => {
  const context = await browser.newContext({
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15",
  });
  const page = await context.newPage();
  await page.addInitScript(tauriMock);
  await page.goto("/");

  await expect(page.locator(".window-titlebar.mac")).toBeVisible();
  // Native traffic lights (Overlay title bar) replace our own window controls.
  await expect(page.locator(".window-controls")).toHaveCount(0);

  await page.getByRole("button", { name: "File" }).click();
  await page.getByRole("menuitem", { name: "Open projects" }).click();
  await expect(page.locator(".projects-screen")).toBeVisible();

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
  await expect(page.locator(".settings-modal")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".settings-modal")).toHaveCount(0);

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
  await expect(page.locator(".user-bubble .body", { hasText: /^queued$/ })).toBeVisible({ timeout: 500 });

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
  await expect(page.locator(".settings-modal")).toHaveCount(0);
  await expect.poll(async () => page.evaluate(() =>
    ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "send_message").length,
  )).toBeGreaterThan(0);
});
