import { settleRendering } from "./async-observers";
import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Trajectory (轨迹) modal: a split inspector over the chat, opened from the
// topbar or `/trajectory`. Data comes from `load_session_trajectory` (mocked
// in mock-tauri.ts with two turns, tool details, usage cells, and stats).

test.beforeEach(async ({ page }) => {
  // Install the Tauri bridge mock before the page's wasm runs.
  await page.addInitScript(tauriMock);
});

async function enterApp(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeVisible();
}

async function openTrajectory(page: Page) {
  await page.getByTestId("trajectory-topbar").click();
  const overlay = page.getByTestId("trajectory-overlay");
  await expect(overlay).toBeVisible();
  return overlay.getByTestId("trajectory-view");
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

async function runOneTurn(page: Page) {
  await enterApp(page);
  await page.locator("#composer-input").fill("analyze ESR1");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
}

// A 20-turn session: tall enough that the list has to scroll.
async function seedTallTrajectory(page: Page) {
  await page.evaluate(() => {
    const cell = (kind: string, summary: string, index: number, extra: Record<string, unknown> = {}) => ({
      kind,
      summary,
      detail_input: kind === "tool" ? `{"n":${index}}` : null,
      detail_output: summary,
      ok: kind === "tool" ? true : null,
      is_error: false,
      ts: 1755000000000 + index * 1000,
      duration_ms: kind === "tool" ? 200 : 50,
      usage: null,
      ...extra,
    });
    const turns = Array.from({ length: 20 }, (_, i) => {
      const n = i + 1;
      return {
        index: n,
        started_at: 1755000000000 + n * 4000,
        cells: [
          cell("user", `User turn ${n}`, n),
          cell("assistant", `Assistant turn ${n}`, n),
          cell("tool", `late_tool_${n}`, n),
        ],
      };
    });
    (window as any).__trajectorySnapshot = {
      frame_id: "",
      model: "deepseek-v4-pro",
      turns,
      stats: {
        turns: 20,
        steps: 40,
        llm_ms: 1000,
        tool_ms: 4000,
        input_tokens: 1000,
        output_tokens: 1000,
        cached_input_tokens: 0,
        cache_hit_pct: null,
        tokens_per_sec: 10,
      },
    };
  });
}

test("trajectory modal renders turns, inspector tabs, usage lines, and stats", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("analyze ESR1");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("thread-tabs")).toHaveCount(0);

  const view = await openTrajectory(page);
  const inspector = view.getByTestId("traj-inspector");

  // Turn groups and their headers.
  await expect(view.getByText("Turn 1", { exact: true })).toBeVisible();
  await expect(view.getByText("Turn 2", { exact: true })).toBeVisible();
  await expect(view.getByTestId("traj-gantt")).toBeVisible();
  await expect(view.getByTestId("traj-axis-duration")).toBeVisible();

  // Auto-selected first row: user message summary in the inspector.
  await expect(inspector.getByTestId("traj-meta-source")).toHaveText("User");
  await expect(inspector.getByTestId("traj-meta-status")).toHaveText("Completed");
  await inspector.getByTestId("traj-tab-preview").click();
  await expect(inspector.getByTestId("traj-preview")).toContainText("Analyze the ESR1 dataset");
  await inspector.getByTestId("traj-tab-raw").click();
  await expect(inspector.getByTestId("traj-raw")).toContainText('"kind": "user"');
  await inspector.getByTestId("traj-tab-source").click();
  await expect(inspector.getByTestId("traj-source")).toContainText("Analyze the ESR1 dataset");

  // Inspector open: the narrow list uses icons instead of USER/TOOL badges.
  const toolRow = view.getByTestId("traj-row-tool").first();
  await expect(toolRow.getByTestId("traj-row-icon")).toBeVisible();
  await expect(toolRow.locator(".traj-badge")).toBeHidden();
  await expect(toolRow).toContainText("python · df.describe()");
  await expect(toolRow).toContainText("3s");
  await expect(view.getByTestId("traj-row-user").first()).toContainText("Analyze the ESR1 dataset");

  // Usage rows stay compact single lines.
  await expect(view.getByText("round 1 · in 12.3k · out 1.4k · cached 75%")).toBeVisible();

  // Tool rows open full args JSON + full result in the inspector.
  await toolRow.click();
  await inspector.getByTestId("traj-tab-preview").click();
  await expect(inspector.getByTestId("traj-detail-input")).toContainText('"code": "df.describe()"');
  await expect(inspector.getByTestId("traj-detail-output")).toContainText("count  612.0");

  // Error rows carry the red accent class.
  await expect(view.getByTestId("traj-row-tool").nth(1)).toHaveClass(/error/);

  // Footer stats line.
  const footer = view.getByTestId("trajectory-footer");
  await expect(footer).toContainText("2 turns · 4 steps");
  await expect(footer).toContainText("LLM 3s · Tools 4s");
  await expect(footer).toContainText("12.5 tok/s");
  await expect(footer).toContainText("cache hit 75%");
  await expect(footer).toContainText("in 27.3k tok · out 2.3k tok");

  // Client-side search filters cells (Turn 1 has no "volcano" cell).
  await view.getByPlaceholder("Search events").fill("volcano");
  await expect(view.getByText("Turn 1", { exact: true })).toHaveCount(0);
  await expect(view.getByText("Turn 2", { exact: true })).toBeVisible();
  await view.getByPlaceholder("Search events").fill("");
  await expect(view.getByText("Turn 1", { exact: true })).toBeVisible();

  // Closing the inspector (not the modal) expands the list; badges return.
  await page.getByTestId("traj-inspector-close").click();
  await expect(view.getByTestId("traj-inspector")).toHaveCount(0);
  await expect(toolRow.getByText("TOOL", { exact: true })).toBeVisible();
  await expect(toolRow.getByTestId("traj-row-icon")).toBeHidden();
  await expect(view.getByText("Turn 1", { exact: true })).toBeVisible();

  // Clicking a row reopens the inspector.
  await toolRow.click();
  await expect(view.getByTestId("traj-inspector")).toBeVisible();

  // Closing the modal restores the chat thread.
  await page.getByTestId("trajectory-close").click();
  await expect(page.getByTestId("trajectory-overlay")).toHaveCount(0);
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
});

test("Escape immediately after opening the trajectory modal closes only that layer", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("analyze ESR1");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible({ timeout: 10_000 });

  await page.getByTestId("trajectory-topbar").click();
  await expect(page.getByTestId("trajectory-overlay")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("trajectory-overlay")).toHaveCount(0);
  await expect(page.getByText("Hello from mock wisp-science.")).toBeVisible();
});

test("/trajectory slash command opens the inspector", async ({ page }) => {
  await enterApp(page);
  await page.locator("#composer-input").fill("/trajectory");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("trajectory-overlay")).toBeVisible();
  await expect(page.locator("#composer-input")).toHaveValue("");
});

test("trajectory modal shows the empty state when a session has no turns", async ({ page }) => {
  await enterApp(page);
  await page.evaluate(() => {
    (window as any).__trajectorySnapshot = {
      frame_id: "",
      model: null,
      turns: [],
      stats: {
        turns: 0,
        steps: 0,
        llm_ms: 0,
        tool_ms: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cache_hit_pct: null,
        tokens_per_sec: null,
      },
    };
  });
  const view = await openTrajectory(page);
  await expect(view).toContainText("No trajectory yet");
});

test("clicking a Gantt segment selects that event and scrolls it to the top of the list", async ({ page }) => {
  await runOneTurn(page);
  await seedTallTrajectory(page);
  const view = await openTrajectory(page);
  const list = view.getByTestId("traj-list");
  await expect(list.getByText("User turn 1", { exact: true })).toBeVisible();

  await view.locator('.traj-gantt [data-traj-key="20:2"]').click();
  const selected = list.locator(".traj-row.selected");
  await expect(selected).toContainText("late_tool_20");
  await expect(view.getByTestId("traj-meta-source")).toHaveText("Tool");
  await expect
    .poll(async () => {
      const listBox = await list.boundingBox();
      const rowBox = await selected.boundingBox();
      if (!listBox || !rowBox) return 9999;
      return rowBox.y - listBox.y;
    })
    .toBeLessThan(72);
});

test("selecting a row leaves the list scrolled where the reader left it", async ({ page }) => {
  await runOneTurn(page);
  await seedTallTrajectory(page);
  const view = await openTrajectory(page);
  const list = view.getByTestId("traj-list");
  await expect(list.getByText("User turn 1", { exact: true })).toBeVisible();

  // Scroll down to the newest turns, then pick whichever row the reader would
  // see in the middle of the viewport. Hit testing the live layout avoids
  // depending on row heights, which vary with the platform's fonts.
  await list.evaluate((el) => {
    el.scrollTop = el.scrollHeight - el.clientHeight * 1.5;
  });
  const key = await list.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    const x = rect.left + rect.width / 2;
    for (const frac of [0.5, 0.35, 0.65, 0.2, 0.8]) {
      const row = document
        .elementFromPoint(x, rect.top + rect.height * frac)
        ?.closest("[data-traj-key]");
      if (row) return row.getAttribute("data-traj-key");
    }
    return null;
  });
  expect(key).toBeTruthy();
  const target = list.locator(`[data-traj-key="${key}"]`);
  const before = await list.evaluate((el) => el.scrollTop);
  expect(before).toBeGreaterThan(0);

  // A raw mouse click: `locator.click` scrolls the row into view on its own
  // terms (it honours `scroll-margin-top`) and would move the list for us.
  // Coordinates are only trustworthy once the modal's entry animation ends.
  await view.evaluate((el) =>
    Promise.all(el.closest(".modal")!.getAnimations().map((a) => a.finished)).then(() => undefined),
  );
  const box = (await target.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(target).toHaveClass(/selected/);
  await expect(view.getByTestId("traj-inspector")).toBeVisible();
  await settleRendering(page);
  expect(await list.evaluate((el) => el.scrollTop)).toBe(before);

  // Closing the inspector must not throw the reader back to the top either.
  // Rows grow a little when badges replace icons, so assert the weaker property
  // that actually matters: the row stays where the reader can see it.
  await view.getByTestId("traj-inspector-close").click();
  await expect(view.getByTestId("traj-inspector")).toHaveCount(0);
  await settleRendering(page);
  const stillVisible = await target.evaluate((el) => {
    const listEl = el.closest("[data-testid='traj-list']")!;
    const row = el.getBoundingClientRect();
    const bounds = listEl.getBoundingClientRect();
    return row.top >= bounds.top - 4 && row.bottom <= bounds.bottom + 4;
  });
  expect(stillVisible).toBe(true);
});

test("the inspector scrolls when a call has more content than fits", async ({ page }) => {
  await runOneTurn(page);
  await page.evaluate(() => {
    const long = Array.from({ length: 300 }, (_, i) => `web_scan result line ${i}`).join("\n");
    (window as any).__trajectorySnapshot = {
      frame_id: "",
      model: "deepseek-v4-pro",
      turns: [
        {
          index: 1,
          started_at: 1755000000000,
          cells: [
            {
              kind: "tool",
              summary: "web_scan · long result",
              detail_input: '{"url":"https://example.org"}',
              detail_output: long,
              ok: true,
              is_error: false,
              ts: 1755000000000,
              duration_ms: 200,
              usage: null,
            },
          ],
        },
      ],
      stats: {
        turns: 1,
        steps: 1,
        llm_ms: 0,
        tool_ms: 200,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cache_hit_pct: null,
        tokens_per_sec: null,
      },
    };
  });

  const view = await openTrajectory(page);
  const inspector = view.getByTestId("traj-inspector");
  await inspector.getByTestId("traj-tab-preview").click();
  await expect(inspector.getByTestId("traj-detail-output")).toContainText("web_scan result line 299");

  // The overflow belongs to the inspector body: it stays inside the split
  // instead of growing past it, and it scrolls all the way to the last line.
  const splitBox = (await view.getByTestId("traj-split").boundingBox())!;
  const body = inspector.locator(".traj-inspector-body");
  const bodyBox = (await body.boundingBox())!;
  expect(bodyBox.y + bodyBox.height).toBeLessThanOrEqual(splitBox.y + splitBox.height + 1);

  const scrolled = await body.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
    return { top: el.scrollTop, overflow: el.scrollHeight - el.clientHeight };
  });
  expect(scrolled.overflow).toBeGreaterThan(0);
  expect(scrolled.top).toBe(scrolled.overflow);
});

test("trajectory title bar exports HTML from the backend, not the search filter", async ({ page }) => {
  await runOneTurn(page);
  const view = await openTrajectory(page);
  const exportBtn = page.getByTestId("trajectory-export");
  await expect(exportBtn).toBeVisible();
  await expect(exportBtn).toHaveAttribute("aria-label", "Export HTML");

  await view.getByPlaceholder("Search events").fill("volcano");
  await expect(view.getByText("Turn 1", { exact: true })).toHaveCount(0);
  await expect(view.getByText("Turn 2", { exact: true })).toBeVisible();

  await exportBtn.click();
  await expect.poll(() => lastInvokeArgs(page, "export_session_trajectory")).toMatchObject({
    frameId: expect.any(String),
    locale: "en",
  });
  const args = await lastInvokeArgs(page, "export_session_trajectory");
  expect(String(args.frameId)).not.toHaveLength(0);
  expect(args).not.toHaveProperty("query");
  expect(args).not.toHaveProperty("html");
  await expect(page.locator("#copy-toast")).toContainText("/mock/wisp-trajectory.html");
  await expect(page.getByTestId("trajectory-overlay")).toBeVisible();
});

test("cancelling the trajectory HTML save dialog does not toast", async ({ page }) => {
  await runOneTurn(page);
  await page.evaluate(() => {
    (window as any).__trajectoryExportCancel = true;
  });
  await openTrajectory(page);
  await page.getByTestId("trajectory-export").click();
  await expect.poll(() => lastInvokeArgs(page, "export_session_trajectory")).toMatchObject({
    locale: "en",
  });
  await expect(page.locator("#copy-toast")).toHaveCount(0);
  await expect(page.locator(".copy-toast-warning")).toHaveCount(0);
  await expect(page.getByTestId("trajectory-overlay")).toBeVisible();
});
