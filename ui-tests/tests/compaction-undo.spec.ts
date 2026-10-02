import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

function compactionPayload(overrides: Record<string, unknown> = {}) {
  return {
    before: 1000,
    after: 200,
    strategy: "manual",
    epoch: 1,
    checkpoint: "[context summary checkpoint]\n\nFolded older turns.",
    kept_from_user_index: 1,
    undone: false,
    can_undo: true,
    ...overrides,
  };
}

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") {
        return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      }
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

async function openCompactedSession(page: Page, payload = compactionPayload(), locale = "en") {
  await page.addInitScript(tauriMock);
  await page.addInitScript((item) => {
    (window as any).__compactionItem = item;
  }, payload);
  await page.goto(`/?mockLocale=${locale}`);
  await expect.poll(() =>
    page.evaluate(() => Boolean((window as any).__tauriListenerReady?.("open-session"))),
  ).toBe(true);
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    const epochs = Array.from({ length: w.__compactionItem.epoch }, (_, index) => ({
      epoch: index + 1, parent_epoch: index, strategy: "manual", kind: "semantic",
      before_tokens: 1000, after_tokens: 200, initial_head_seq: 9 + index * 4,
      first_kept_seq: 4, checkpoint_seq: 7 + index * 4, has_new_turns: false,
    }));
    w.__compactionCards = epochs.map(row => ({ ...w.__compactionItem, epoch: row.epoch,
      can_undo: row.epoch === w.__compactionItem.epoch && w.__compactionItem.can_undo,
    }));
    w.__contextState = {
      head_epoch: w.__compactionItem.epoch, context_epochs: epochs,
      in_context_from_user_index: 1, compactions: w.__compactionCards, undone_epochs: [],
    };
    w.__contextView = [
      { role: "system", text: "You are wisp-science", kind: "system" },
      { role: "checkpoint", text: "[context summary checkpoint]\n\nFolded older turns.", kind: "checkpoint" },
      { role: "user", text: "second question" },
      { role: "assistant", text: "second answer" },
    ];
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd === "load_session_context_state" && arg("sessionId") === "s-compact") {
        (w.__skillInvokeLog ??= []).push({ cmd, args });
        const snapshot = JSON.parse(JSON.stringify(w.__contextState));
        await new Promise(resolve => setTimeout(resolve, w.__contextDelay ?? 0));
        return snapshot;
      }
      if (cmd === "undo_compaction" && arg("sessionId") === "s-compact") {
        (w.__skillInvokeLog ??= []).push({ cmd, args });
        const state = w.__contextState;
        const epoch = state.head_epoch;
        state.head_epoch = state.context_epochs.find(row => row.epoch === epoch).parent_epoch;
        state.context_epochs = state.context_epochs.filter(row => row.epoch !== epoch);
        state.compactions = state.compactions.filter(row => row.epoch !== epoch).map(row => ({
          ...row, can_undo: row.epoch === state.head_epoch, undo_reason: null,
        }));
        state.undone_epochs.push(epoch);
        state.in_context_from_user_index = state.head_epoch ? 1 : null;
        if (!w.__suppressUndoEvent) w.__tauriEmit("agent", { kind: "CompactionUndone", frame_id: "s-compact", epoch });
        return epoch;
      }
      if (cmd === "load_session_context_view" && (arg("sessionId") === "s-compact" || arg("id") === "s-compact")) {
        (w.__skillInvokeLog ??= []).push({ cmd, args });
        const snapshot = JSON.parse(JSON.stringify(w.__contextView));
        await new Promise(resolve => setTimeout(resolve, w.__contextViewDelay ?? 0));
        if (w.__contextViewError) throw new Error(w.__contextViewError);
        return snapshot;
      }
      if (cmd === "load_session" && arg("id") === "s-compact") {
        return {
          items: [
            { role: "user", text: "first question" },
            { role: "assistant", text: "first answer" },
            { role: "user", text: "second question" },
            { role: "assistant", text: "second answer" },
            { role: "usage", text: JSON.stringify({ input: 80, output: 20, ctx_tokens: 200, max_context: 1000 }) },
            ...w.__compactionCards.map(card => ({ role: "compaction", text: JSON.stringify(card) })),
          ],
          next_before_seq: null,
          user_offset: 0,
          outline: [
            { user_index: 0, text: "first question" },
            { user_index: 1, text: "second question" },
          ],
          head_epoch: w.__contextState.head_epoch,
          in_context_from_user_index: w.__contextState.in_context_from_user_index,
          context_epochs: w.__contextState.context_epochs,
        };
      }
      return original(cmd, args);
    };
  });
  await page.evaluate(() =>
    (window as any).__tauriEmit("open-session", { projectId: "default", sessionId: "s-compact" }),
  );
  await expect(page.getByTestId("context-compaction-flag").last()).toBeVisible();
}

async function publishCompaction(page: Page, automatic = false) {
  await page.evaluate(({ automatic }) => {
    const w = window as any;
    const state = w.__contextState;
    const epoch = state.head_epoch + 1;
    const strategy = automatic ? "auto" : "manual";
    const event = { kind: "Compaction", frame_id: "s-compact", before: 1000, after: 200,
      strategy, epoch: automatic ? null : epoch };
    if (automatic) w.__tauriEmit("agent", event);
    state.context_epochs.push({ ...state.context_epochs.at(-1), epoch, parent_epoch: state.head_epoch, strategy });
    state.head_epoch = epoch;
    state.compactions = state.compactions.map(card => ({ ...card, can_undo: false, undo_reason: "not_head" }));
    state.compactions.push({ ...w.__compactionItem, epoch, strategy,
      checkpoint: "[context summary checkpoint]\n\nNewly folded turns.", can_undo: true });
    if (!automatic) w.__tauriEmit("agent", event);
    else w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "end_turn" });
  }, { automatic });
}

async function wireManualCompaction(page: Page, options: { checkpoint?: string; manualCompletion?: boolean } = {}) {
  await page.evaluate(({ checkpoint, manualCompletion }) => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd !== "send_message") return original(cmd, args);
      const message = String(arg("message") ?? "");
      (w.__skillInvokeLog ??= []).push({ cmd, args });
      w.__compactInstruction = message;
      const complete = () => {
        const state = w.__contextState;
        const epoch = state.head_epoch + 1;
        const compactedCheckpoint = checkpoint ?? "[context summary checkpoint]\n\nNew context after manual compaction.";
        state.context_epochs.push({
          epoch, parent_epoch: state.head_epoch, strategy: "manual", kind: "semantic",
          before_tokens: 1000, after_tokens: 150, initial_head_seq: 20,
          first_kept_seq: 8, checkpoint_seq: 18, has_new_turns: false,
        });
        state.head_epoch = epoch;
        state.compactions = state.compactions.map((card: any) => ({ ...card, can_undo: false, undo_reason: "not_head" }));
        state.compactions.push({ ...w.__compactionItem, epoch, before: 1000, after: 150,
          checkpoint: compactedCheckpoint, can_undo: true });
        w.__contextView = [
          { role: "system", text: "You are wisp-science", kind: "system" },
          { role: "checkpoint", text: compactedCheckpoint, kind: "checkpoint" },
          { role: "user", text: "second question" },
          { role: "assistant", text: "second answer" },
        ];
        w.__tauriEmit("agent", { kind: "CompactionStarted", frame_id: "s-compact", strategy: "manual" });
        w.__tauriEmit("agent", { kind: "Compaction", frame_id: "s-compact", before: 1000, after: 150, strategy: "manual", epoch });
        w.__tauriEmit("agent", { kind: "Usage", frame_id: "s-compact", round: 0, model: "mock", created_at: 1,
          input: 0, output: 0, reasoning: 0, cached: 0, ctx_tokens: 150, max_context: 1000,
          context_usage: { system_prompt: 40, tool_definitions: 20, rules: 10, skills: 10,
            mcp_dynamic_tools: 0, subagent_definitions: 0, conversation: 70 } });
        w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "compact" });
      };
      if (manualCompletion) w.__completeManualCompaction = complete;
      else setTimeout(complete, 40);
      return "s-compact";
    };
  }, options);
}

for (const locale of ["en", "zh"]) {
  test(`transcript switch adapts to pane width and preserves keyboard and panel state (${locale})`, async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 900 });
    await openCompactedSession(page, compactionPayload(), locale);
    const toggle = page.getByTestId("transcript-view-toggle");
    const full = page.getByTestId("transcript-view-full");
    const model = page.getByTestId("transcript-view-model");
    const label = full.locator(".transcript-view-label");
    await expect(label).toBeVisible();
    await expect(full).toHaveAccessibleName(locale === "zh" ? "完整记录" : "Full transcript");
    await expect(model).toHaveAccessibleName(locale === "zh" ? "模型视角" : "Model view");
    await expect(full).toHaveAttribute("aria-pressed", "true");
    await toggle.screenshot({ path: test.info().outputPath(`switch-${locale}-wide.png`) });

    // The pane can shrink while the desktop window remains wide (e.g. a split).
    await page.locator(".center").evaluate(el => { (el as HTMLElement).style.maxWidth = "680px"; });
    await expect(label).toBeHidden();
    expect((await toggle.boundingBox())!.width).toBeLessThanOrEqual(72);
    await page.locator(".center").evaluate(el => { (el as HTMLElement).style.removeProperty("max-width"); });
    await expect(label).toBeVisible();

    for (const width of [1100, 760]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(label).toBeHidden();
      await expect(full).toHaveAttribute("aria-pressed", "true");
      const toolbar = await page.locator(".topbar").boundingBox();
      const bounds = await toggle.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(toolbar!.x);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(toolbar!.x + toolbar!.width);
    }
    await full.focus();
    await page.keyboard.press("Tab");
    await expect(model).toBeFocused();
    await page.keyboard.press("Space");
    await expect(model).toHaveAttribute("aria-pressed", "true");
    await expect(full).toHaveAttribute("aria-pressed", "false");
    await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "true");
    await toggle.screenshot({ path: test.info().outputPath(`switch-${locale}-compact.png`) });
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
    await toggle.screenshot({ path: test.info().outputPath(`switch-${locale}-compact-dark.png`) });

    await page.getByTestId("context-usage-trigger").click();
    const panelToggle = page.getByTestId("context-usage-view-toggle");
    await expect(panelToggle.locator(".transcript-view-label").first()).toBeVisible();
    await expect(page.getByTestId("context-usage-view-model")).toHaveAttribute("aria-pressed", "true");
    await page.getByTestId("context-usage-view-full").click();
    await expect(full).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "false");
    await page.setViewportSize({ width: 1600, height: 900 });
    await expect(label).toBeVisible();
    await expect(full).toHaveAttribute("aria-pressed", "true");
  });
}

test("compaction row expands the checkpoint and Escape closes only that layer", async ({ page }) => {
  await openCompactedSession(page);
  await page.getByTestId("conversation-outline-toggle").click();
  await expect(page.getByTestId("conversation-outline")).toBeVisible();
  await page.getByTestId("context-compaction-expand").click();
  const details = page.getByTestId("context-compaction-details");
  await expect(details).toBeVisible();
  await expect(details).toContainText("Folded older turns.");
  await expect(details).toContainText("Epoch 1");
  await expect(details).toContainText("Kept from turn 2");
  await page.keyboard.press("Escape");
  await expect(details).toHaveCount(0);
  await expect(page.getByTestId("conversation-outline")).toBeVisible();
});

test("undo compaction invokes the command and marks the row undone", async ({ page }) => {
  await openCompactedSession(page);
  await page.getByTestId("context-compaction-expand").click();
  await page.getByTestId("undo-compaction").click();
  await expect.poll(() => lastInvokeArgs(page, "undo_compaction")).toMatchObject({
    sessionId: "s-compact",
  });
  const flag = page.getByTestId("context-compaction-flag");
  await expect(flag).toHaveAttribute("data-undone", "true");
  await expect(flag).toContainText("Compaction undone");
});

test("new turns disable undo and show the backend reason", async ({ page }) => {
  await openCompactedSession(page, compactionPayload({
    can_undo: false,
    undo_reason: "has_new_turns",
  }));
  await page.getByTestId("context-compaction-expand").click();
  const undo = page.getByTestId("undo-compaction");
  await expect(undo).toBeDisabled();
  await expect(undo).toHaveAttribute("title", "Conversation continued after compaction");
});

test("compacted bubbles are marked out of context and model view is read-only", async ({ page }) => {
  await openCompactedSession(page);
  const first = page.locator("[data-user-index='0']").first();
  const second = page.locator("[data-user-index='1']").first();
  await expect(first).toHaveAttribute("data-in-context", "false");
  await expect(first).toHaveAttribute("title", "Not in the current context; represented by the summary");
  await expect(second).toHaveAttribute("data-in-context", "true");
  await expect(
    page.locator("[data-testid='transcript-item']").filter({
      has: page.getByTestId("context-compaction-flag"),
    }),
  ).toHaveAttribute("data-in-context", "true");

  await page.getByTestId("transcript-view-model").click();
  await expect.poll(() => lastInvokeArgs(page, "load_session_context_view")).toMatchObject({
    sessionId: "s-compact",
  });
  await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "true");
  const thread = page.locator(".thread");
  await expect(page.getByTestId("context-system-row")).toBeVisible();
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("Folded older turns.");
  await expect(thread.getByText("second question")).toBeVisible();
  await expect(thread.getByText("first question")).toHaveCount(0);
  await expect(thread.getByRole("button", { name: "Rewind" })).toHaveCount(0);
  await expect(thread.getByRole("button", { name: "Branch" })).toHaveCount(0);

  await page.getByTestId("transcript-view-full").click();
  await expect(thread).toHaveAttribute("data-model-view", "false");
  await expect(thread.getByText("first question")).toBeVisible();
  await expect(page.getByTestId("context-system-row")).toHaveCount(0);

  await page.getByTestId("context-usage-trigger").click();
  const panel = page.getByTestId("context-usage-panel");
  await expect(panel.getByTestId("context-usage-epoch")).toHaveText(
    "Epoch 1 · system + checkpoint + 1 kept turns",
  );
});

for (const emitEvent of [true, false]) {
  test(`undo restores the actual parent epoch and its undo action (event=${emitEvent})`, async ({ page }) => {
    await openCompactedSession(page, compactionPayload({ epoch: 2 }));
    await page.evaluate(emitEvent => { (window as any).__suppressUndoEvent = !emitEvent; }, emitEvent);
    await page.getByTestId("context-compaction-expand").last().click();
    await page.getByTestId("undo-compaction").click();
    await expect(page.getByTestId("context-compaction-flag").last()).toHaveAttribute("data-undone", "true");
    await page.getByTestId("context-usage-trigger").click();
    await expect(page.getByTestId("context-usage-epoch")).toContainText("Epoch 1");
    await expect(page.locator("[data-user-index='0']").first()).toHaveAttribute("data-in-context", "false");
    await page.getByTestId("context-compaction-expand").first().click();
    await expect(page.getByTestId("context-compaction-details").first().getByTestId("undo-compaction")).toBeEnabled();
    await page.getByTestId("context-compaction-details").first().getByTestId("undo-compaction").click();
    await expect(page.getByTestId("context-usage-epoch")).toHaveCount(0);
    await expect(page.locator("[data-user-index='0']").first()).toHaveAttribute("data-in-context", "true");
  });
}

for (const automatic of [false, true]) {
  test(`live compaction refreshes checkpoint and epoch without replacing the transcript (auto=${automatic})`, async ({ page }) => {
    await openCompactedSession(page);
    await publishCompaction(page, automatic);
    await page.getByTestId("context-compaction-expand").last().click();
    await expect(page.getByTestId("context-compaction-details")).toContainText("Newly folded turns.");
    await expect(page.getByTestId("context-compaction-details")).toContainText("Epoch 2");
    await expect(page.locator(".thread").getByText("first question", { exact: true })).toBeVisible();
    await page.getByTestId("context-usage-trigger").click();
    await expect(page.getByTestId("context-usage-epoch")).toContainText("Epoch 2");
  });
}

test("a visible model view refreshes after compaction without falling back to full history", async ({ page }) => {
  await openCompactedSession(page);
  await page.getByTestId("transcript-view-model").click();
  await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "true");
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("Folded older turns.");
  await publishCompaction(page);
  await page.waitForTimeout(300);
  await expect(page.getByTestId("transcript-view-model")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "true");
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("Folded older turns.");
  await expect(page.locator(".thread").getByText("first question", { exact: true })).toHaveCount(0);
});

test("a late compaction snapshot cannot overwrite a later undo", async ({ page }) => {
  await openCompactedSession(page, compactionPayload({ epoch: 2 }));
  await page.evaluate(() => {
    const w = window as any;
    w.__contextDelay = 800;
    w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "compact" });
  });
  await expect.poll(() => lastInvokeArgs(page, "load_session_context_state")).toMatchObject({ sessionId: "s-compact" });
  await page.evaluate(() => { (window as any).__contextDelay = 0; });
  await page.getByTestId("context-compaction-expand").last().click();
  await page.getByTestId("undo-compaction").click();
  await page.getByTestId("context-usage-trigger").click();
  await expect(page.getByTestId("context-usage-epoch")).toContainText("Epoch 1");
  await page.waitForTimeout(900);
  await expect(page.getByTestId("context-usage-epoch")).toContainText("Epoch 1");
});

test("a late context refresh cannot alter another conversation", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => { (window as any).__contextDelay = 800; });
  await publishCompaction(page);
  await expect.poll(() => lastInvokeArgs(page, "load_session_context_state")).toMatchObject({ sessionId: "s-compact" });
  await page.evaluate(() => (window as any).__tauriEmit("open-session", { projectId: "default", sessionId: "s1" }));
  await expect(page.locator(".thread").getByText("first question", { exact: true })).toHaveCount(0);
  await page.waitForTimeout(900);
  await expect(page.getByTestId("context-compaction-flag")).toHaveCount(0);
  await expect(page.getByTestId("context-usage-epoch")).toHaveCount(0);
});

test("slash compact opens a guided locked flow and reveals the compacted model context", async ({ page }) => {
  await openCompactedSession(page);
  await wireManualCompaction(page, { manualCompletion: true });
  const composer = page.locator("#composer-input");
  await composer.fill("/compact preserve the QC thresholds and blockers");
  await composer.press("Enter");
  const modal = page.getByTestId("compact-modal");
  await expect(modal).toBeVisible();
  await expect(page.getByTestId("compact-instruction")).toHaveValue("preserve the QC thresholds and blockers");
  await page.getByTestId("compact-start").click({ force: true });
  await expect(page.getByTestId("compact-progress")).toBeVisible();
  await expect(page.getByTestId("compact-close")).toHaveCount(0);
  await expect(page.getByTestId("compact-cancel")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(modal).toBeVisible();
  await expect(page.getByTestId("compact-mode-semantic")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("compact-instruction")).toHaveValue("preserve the QC thresholds and blockers");
  await expect.poll(() => page.evaluate(() => (window as any).__compactInstruction)).toBe(
    "/compact --semantic preserve the QC thresholds and blockers",
  );
  // Keep the mock operation running until the locked-state assertions finish.
  // A 40 ms timer can complete between Playwright calls and hide the modal.
  await page.evaluate(() => (window as any).__completeManualCompaction());
  await expect(modal).toHaveCount(0);
  await expect(page.locator(".thread")).toHaveAttribute("data-model-view", "true");
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("New context after manual compaction.");
  await page.getByTestId("context-usage-trigger").click({ force: true });
  await expect(page.getByTestId("context-usage-panel")).toContainText("150");
  await expect(page.getByTestId("context-usage-epoch")).toContainText("Epoch 2");
});

test("context usage compact button opens the same instruction dialog", async ({ page }) => {
  await openCompactedSession(page, compactionPayload(), "zh");
  await page.getByTestId("context-usage-trigger").click();
  await page.getByTestId("context-usage-compact-header").click();
  await expect(page.getByTestId("compact-modal")).toBeVisible();
  await expect(page.getByTestId("compact-mode-regular")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("compact-instruction")).toBeHidden();
  await expect(page.getByTestId("compact-modal")).toContainText("压缩上下文");
  await page.getByTestId("compact-mode-semantic").click();
  await expect(page.getByTestId("compact-instruction")).toBeVisible();
  await expect(page.getByTestId("compact-instruction")).toHaveValue("");
  await page.getByTestId("compact-cancel").click();
  await expect(page.getByTestId("compact-modal")).toHaveCount(0);
});

test("regular compact sends /compact and semantic compact sends --semantic", async ({ page }) => {
  await openCompactedSession(page);
  await wireManualCompaction(page);
  await page.getByTestId("context-usage-trigger").click({ force: true });
  await page.getByTestId("context-usage-compact-header").click({ force: true });
  await expect(page.getByTestId("compact-mode-regular")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("compact-instruction")).toBeHidden();
  await page.getByTestId("compact-start").click({ force: true });
  await expect.poll(() => page.evaluate(() => (window as any).__compactInstruction)).toBe("/compact");
});

for (const locale of ["en", "zh"]) {
  test(`expanded compaction keeps horizontal counts and stacks details (${locale})`, async ({ page }) => {
    await page.setViewportSize({ width: 1600, height: 1000 });
    await openCompactedSession(page, compactionPayload({ before: 569400, after: 245500 }), locale);
    const flag = page.getByTestId("context-compaction-flag");
    await page.getByTestId("context-compaction-expand").click();
    for (const width of [1600, 760]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.evaluate(dark => document.documentElement.setAttribute("data-theme", dark ? "dark" : "light"), width === 760);
      const toggle = (await page.getByTestId("context-compaction-expand").boundingBox())!;
      const details = (await page.getByTestId("context-compaction-details").boundingBox())!;
      const counts = (await flag.locator(".context-compaction-count").boundingBox())!;
      const reduction = (await flag.locator(".context-compaction-reduction").boundingBox())!;
      expect(toggle.height).toBeLessThan(100);
      expect(counts.height).toBeLessThan(30);
      expect(reduction.height).toBeLessThan(30);
      expect(details.y).toBeGreaterThanOrEqual(toggle.y + toggle.height - 1);
      expect(await flag.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      await flag.screenshot({ path: test.info().outputPath(`expanded-${locale}-${width}.png`) });
    }
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("context-compaction-details")).toHaveCount(0);
  });
}

test("compaction immediately replaces old occupancy before another Usage event", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__tauriEmit("agent", { kind: "Usage", frame_id: "s-compact", input: 5350300, output: 1500,
      ctx_tokens: 92160, max_context: 128000, context_usage: { conversation: 92160 } });
  });
  await expect(page.getByTestId("context-usage-trigger")).toContainText("72%");
  await page.evaluate(() => (window as any).__tauriEmit("agent", {
    kind: "Compaction", frame_id: "s-compact", before: 92160, after: 29440, strategy: "auto",
  }));
  await expect(page.getByTestId("context-usage-trigger")).toContainText("23%");
  await page.getByTestId("context-usage-trigger").click();
  await expect(page.getByTestId("context-usage-panel")).toContainText("29.4K");
  // A later real usage estimate must supersede the compaction estimate.
  await page.evaluate(() => (window as any).__tauriEmit("agent", { kind: "Usage", frame_id: "s-compact",
    input: 31000, output: 30, ctx_tokens: 32000, max_context: 128000,
    context_usage: { system_prompt: 1000, conversation: 31000 } }));
  await expect(page.getByTestId("context-usage-trigger")).toContainText("25%");
});

test("model view renders prune tombstones as archived rows instead of assistant bubbles", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    const tombstone = "[compacted; full content archived at wisp-history:550b23285f0042bfab0f08fb95bf12e7 — retrieve only narrow ranges with read/grep; do not load the whole archive back into context]";
    w.__contextView = [
      { role: "system", text: "You are wisp-science", kind: "system" },
      { role: "user", text: "plan the preprint" },
      { role: "assistant", text: "We will rewrite the outline." },
      { role: "tool", tool_name: "attempt_completion", text: tombstone, kind: "tombstone", ok: true },
      { role: "user", text: "title and dataset" },
      { role: "tool", tool_name: "read", text: tombstone, kind: "tombstone", ok: true },
      { role: "assistant", text: "Which server should we use?" },
    ];
  });
  await page.getByTestId("transcript-view-model").click();
  const thread = page.locator(".thread");
  await expect(page.getByTestId("context-system-row")).toBeVisible();
  await expect(thread.getByText("We will rewrite the outline.")).toBeVisible();
  await expect(thread.getByText("Which server should we use?")).toBeVisible();
  await expect(page.getByTestId("context-tombstone-row")).toHaveCount(2);
  await expect(page.getByTestId("context-tombstone-row").first()).toContainText("Archived tool result");
  await expect(page.getByTestId("context-tombstone-row").last()).toContainText("Archived read");
  await expect(page.getByTestId("context-tombstone-row").first()).toContainText("wisp-history:550b23285f0042bfab0f08fb95bf12e7");
  const answers = thread.locator(".msg.assistant");
  await expect(answers).toHaveCount(2);
  await expect(answers.nth(0)).not.toContainText("[compacted;");
  await expect(answers.nth(1)).not.toContainText("[compacted;");
  await expect(thread.locator(".activity-summary")).toHaveCount(0);
  await page.getByTestId("context-tombstone-row").first().locator("summary").click();
  await expect(page.getByTestId("context-tombstone-row").first()).toContainText("retrieve only narrow ranges");
});

test("model view renders epoch tool rows and streams continued chat from the same source", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__contextView.splice(3, 0, { role: "tool", tool_name: "shell", text: "EPOCH TOOL OUTPUT", input: "echo epoch", ok: true });
  });
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
  const thread = page.locator(".thread");
  await thread.screenshot({ path: test.info().outputPath("model-epoch-start.png"), animations: "disabled" });
  await thread.locator(".steps-head").first().click();
  await thread.locator(".step-head").first().click();
  await expect(thread).toContainText("EPOCH TOOL OUTPUT");
  await expect(thread).not.toContainText("first answer");
  await page.evaluate(() => {
    const w = window as any;
    const original = w.__TAURI__.core.invoke;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => cmd === "send_message"
      ? new Promise(resolve => { w.__finishEpochSend = () => resolve("s-compact"); })
      : original(cmd, args);
  });
  await page.locator("#composer-input").fill("continue in this epoch");
  await page.locator("#composer-input").press("Enter");
  await page.evaluate(() => {
    const w = window as any;
    w.__tauriEmit("agent", { kind: "User", frame_id: "s-compact", text: "continue in this epoch" });
    w.__tauriEmit("agent", { kind: "Text", frame_id: "s-compact", delta: "NEW EPOCH STREAM" });
  });
  await expect(thread.getByText("continue in this epoch", { exact: true })).toBeVisible();
  await expect(thread).toContainText("NEW EPOCH STREAM");
  await expect(thread.locator(".streaming-markdown")).toContainText("NEW EPOCH STREAM");
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__contextView.push({ role: "user", text: "continue in this epoch" }, { role: "assistant", text: "NEW EPOCH STREAM persisted" });
    w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "end_turn" });
    w.__finishEpochSend();
  });
  await expect(thread).toContainText("NEW EPOCH STREAM persisted");
  await expect(thread.getByText("continue in this epoch", { exact: true })).toHaveCount(1);
  await expect(thread.getByText("first question", { exact: true })).toHaveCount(0);
  await thread.screenshot({ path: test.info().outputPath("model-epoch-continued.png"), animations: "disabled" });
});

test("a delayed epoch read cannot lose its checkpoint when Done refreshes the same epoch", async ({ page }) => {
  await openCompactedSession(page);
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__contextViewDelay = 500;
    w.__contextView[1].text = "[context summary checkpoint]\n\nEPOCH TWO CHECKPOINT";
  });
  await publishCompaction(page);
  await expect(page.getByTestId("context-view-loading")).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__contextViewDelay = 0;
    w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "compact" });
  });
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("EPOCH TWO CHECKPOINT");
  await page.waitForTimeout(600);
  await expect(page.getByTestId("context-checkpoint-row")).toContainText("EPOCH TWO CHECKPOINT");
  await expect(page.locator(".thread").getByText("first question", { exact: true })).toHaveCount(0);
});

test("loading or failing model view never presents full history and can be retried", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__contextViewDelay = 500;
    w.__contextViewError = "model context unavailable";
  });
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-view-loading")).toBeVisible();
  await expect(page.locator(".thread").getByText("first question", { exact: true })).toHaveCount(0);
  await expect(page.getByTestId("context-view-error")).toContainText("model context unavailable");
  await page.evaluate(() => { (window as any).__contextViewError = null; (window as any).__contextViewDelay = 0; });
  await page.getByTestId("context-view-error").getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
});

test("a long head epoch keeps its checkpoint at the beginning", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__contextView = w.__contextView.slice(0, 2).concat(Array.from({ length: 65 }, (_, i) => [
      { role: "user", text: `epoch question ${i}` }, { role: "assistant", text: `epoch answer ${i}` },
    ]).flat());
  });
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
  await expect(page.locator(".thread").getByText("epoch question 0", { exact: true })).toHaveCount(1);
  await expect(page.locator(".thread").getByText("epoch question 64", { exact: true })).toHaveCount(1);
});

test("finishing a model read does not undo a switch back to full transcript", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => { (window as any).__contextViewDelay = 500; });
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-view-loading")).toBeVisible();
  await page.getByTestId("transcript-view-full").click();
  await page.waitForTimeout(650);
  await expect(page.getByTestId("transcript-view-full")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".thread").getByText("first question", { exact: true })).toBeVisible();
  await expect(page.getByTestId("context-checkpoint-row")).toHaveCount(0);
});

test("a turn sent during model loading remains after the checkpoint", async ({ page }) => {
  await openCompactedSession(page);
  await page.evaluate(() => {
    const w = window as any;
    w.__contextViewDelay = 500;
    const original = w.__TAURI__.core.invoke;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => cmd === "send_message"
      ? new Promise(resolve => { w.__finishEpochSend = () => resolve("s-compact"); })
      : original(cmd, args);
  });
  await page.getByTestId("transcript-view-model").click();
  await expect(page.getByTestId("context-view-loading")).toBeVisible();
  await page.locator("#composer-input").fill("sent while loading");
  await page.locator("#composer-input").press("Enter");
  await expect(page.getByTestId("context-view-loading")).toHaveCount(0);
  await expect(page.getByTestId("context-checkpoint-row")).toBeVisible();
  await expect(page.locator(".thread").getByText("sent while loading", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__contextView.push({ role: "user", text: "sent while loading" });
    w.__tauriEmit("agent", { kind: "Done", frame_id: "s-compact", stop_reason: "end_turn" });
    w.__finishEpochSend();
  });
  await expect(page.locator(".thread").getByText("sent while loading", { exact: true })).toHaveCount(1);
});
