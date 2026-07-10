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
  await page.locator(".proj-card:not(.proj-example)").first().click();
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();
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

test("send streams a mocked assistant reply", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("hello there");
  await page.getByRole("button", { name: "Send" }).click();
  // Deltas "Hello " + "from mock wisp-science." accumulate into one assistant bubble.
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
});

test("send menu supports plan first", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("draft the analysis");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Plan first" }).click();

  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: expect.stringContaining("Plan first before executing"),
  });
  await expect.poll(() => lastInvokeArgs(page, "send_message")).toMatchObject({
    message: expect.stringContaining("draft the analysis"),
  });
});

test("side chat answers in a temporary side panel and can switch model", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("what did the main thread miss?");
  await page.getByRole("button", { name: "Message options" }).click();
  await page.getByRole("button", { name: "Side chat" }).click();

  const panel = page.locator(".sidechat-pane");
  await expect(panel.getByText("Side answer: what did the main thread miss?")).toBeVisible();
  await expect(panel).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const closeBox = await panel.getByRole("button", { name: "Close side chat" }).boundingBox();
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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("seed context");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByPlaceholder(/Ask wisp-science/i).fill("try another route");
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

test("right-click export invokes active session export", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("hello there");
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
  })).toMatchObject({ attachments: ["uploads/counts.csv"] });
  // The right panel starts collapsed; open it to see the collected artifact.
  await page.getByRole("button", { name: "Toggle panel" }).click();
  // The upload path lives in the user turn; the panel must pick it up from there.
  await expect(page.locator('.rp-tile[data-artifact-name="counts.csv"]')).toBeVisible();
});

test("pasted image attaches to the composer", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").evaluate((el) => {
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
  })).toMatchObject({ attachments: [expect.stringMatching(/^uploads\/pasted_image_\d+_1\.png$/)] });
});

test("right panel shows execution contexts and runs", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
  await page.getByRole("button", { name: "Contexts" }).click();

  await expect(page.locator(".context-card", { hasText: "local" })).toContainText("Local machine");
  await expect(page.locator(".context-card", { hasText: "ssh:gpu-server" })).toContainText("NVIDIA A100");
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("succeeded");
  await expect(page.locator(".run-card", { hasText: "Kinase screen QC" })).toContainText("ssh:gpu-server");
});

test("running run can be cancelled from the contexts panel", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Toggle panel" }).click();
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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("make a volcano plot volcano.png");
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

test("artifact panel normalizes png/pdf shorthand to the previewable image", async ({ page }) => {
  await enterApp(page);
  await page
    .getByPlaceholder(/Ask wisp-science/i)
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
  await page.getByRole("button", { name: "Cancel" }).click();
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

test("skill manager filters by tag and batch disables visible skills", async ({ page }) => {
  await enterApp(page);
  await page.getByRole("button", { name: "Add to message" }).click();
  await page.getByRole("button", { name: "Manage skills" }).click();
  await expect(page.getByRole("button", { name: "Skills" })).toBeVisible();

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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("NEEDCONFIRM");
  await page.getByRole("button", { name: "Send" }).click();
  // A very long preview must not push the allow button off-screen; the card
  // scrolls the code block internally so the actions stay in view.
  const allow = page.getByRole("button", { name: "Allow once" });
  await expect(allow).toBeVisible({ timeout: 10_000 });
  await expect(allow).toBeInViewport();
});

test("inline approval scope is sent with confirmation", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("NEEDCONFIRM");
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

test("plan approval Other sends feedback (#121)", async ({ page }) => {
  await enterApp(page);
  await page.getByPlaceholder(/Ask wisp-science/i).fill("PLANOTHER");
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText("Review plan before starting?")).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Other" }).click();
  await page
    .getByPlaceholder("Tell wisp what to change in this plan.")
    .fill("Split protocol work from UI work.");
  await page.getByRole("button", { name: "Send feedback" }).click();

  await expect.poll(async () => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).map((c: any) => ({
      cmd: c.cmd,
      args: c.args instanceof Map ? Object.fromEntries(c.args) : (c.args ?? {}),
    }));
    return calls.find((c: any) => c.cmd === "confirm_response") ?? null;
  })).toMatchObject({
    cmd: "confirm_response",
    args: {
      approved: false,
      feedback: "Split protocol work from UI work.",
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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("SCROLLTEST");
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
  const search = page.getByPlaceholder("Search projects, artifacts, sessions...");
  await expect(search).toBeVisible();
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.fill("file");
  await expect(page.locator(".project-search-row", { hasText: "nif3.treefile" })).toBeVisible();
  await search.press("Enter");
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await expect(page.locator(".am-name")).toHaveText("nif3.treefile");
  await page.locator(".artifact-modal .icon-btn", { hasText: "×" }).click();

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
  await page.locator(".proj-card:not(.proj-example)").first().click();
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();

  // Start conversation A. The mock streams "echo:alpha" at once but delays Done,
  // so A stays "running".
  await page.getByPlaceholder(/Ask wisp-science/i).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });

  // While A is still running, open a fresh session. The composer must be usable
  // (per-session busy: A running does NOT block B).
  await page.getByRole("button", { name: "New session" }).click();
  await expect(page.getByPlaceholder(/Ask wisp-science/i)).toBeEmpty();
  await page.getByPlaceholder(/Ask wisp-science/i).fill("beta");
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
  await page.locator(".proj-card:not(.proj-example)").first().click();
  await expect(page.getByRole("button", { name: "New session" })).toBeVisible();

  await page.getByPlaceholder(/Ask wisp-science/i).fill("alpha");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("echo:alpha")).toBeVisible({ timeout: 10_000 });

  await page.getByPlaceholder(/Ask wisp-science/i).fill("queued");
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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("MDLIST");
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
  await page.getByPlaceholder(/Ask wisp-science/i).fill("STEPSDEMO");
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

  await page.getByPlaceholder(/Ask wisp-science/i).fill("hello there");
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
