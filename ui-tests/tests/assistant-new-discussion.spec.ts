import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// "New discussion" (#1588): the assistant keeps one conversation; the header
// control starts a fresh model context in it and a divider marks where.

test.use({ viewport: { width: 1440, height: 960 } });
test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.addInitScript(() => {
    (window as any).__assistantHistory = [
      { role: "user", text: "Where does the RNA-seq project stand?", tool_name: null, ok: null },
      { role: "assistant", text: "QC is done; differential expression is next.", tool_name: null, ok: null },
    ];
  });
});

async function open(page: Page, locale = "en") {
  await page.goto(`/?mockLocale=${locale}`);
  await expect(page.locator(".proj-card-main").first()).toBeVisible();
  await page.getByTestId("open-research-assistant").click();
  await expect(page.getByTestId("assistant-header")).toBeVisible();
  await expect(page.locator(".chat")).toContainText("differential expression is next");
}

async function invoked(page: Page, command: string) {
  return page.evaluate(
    cmd => ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === cmd).length,
    command,
  );
}

const newDiscussion = (page: Page) => page.getByTestId("assistant-new-discussion");

test("the header starts a new discussion below the earlier one without a dialog or a model turn", async ({ page }) => {
  await open(page);
  const button = newDiscussion(page);
  await expect(button).toHaveText("New discussion");
  await expect(button).toBeEnabled();
  // Its own icon: no other header entry draws the same one.
  const icons = await page.getByTestId("assistant-header").locator("button > svg").evaluateAll(
    nodes => nodes.map(node => node.innerHTML),
  );
  const own = await button.locator("svg").innerHTML();
  expect(icons.filter(icon => icon === own)).toHaveLength(1);

  await expect(page.getByTestId("discussion-divider")).toHaveCount(0);
  await button.click();
  await expect.poll(() => invoked(page, "start_research_assistant_discussion")).toBe(1);

  const divider = page.getByTestId("discussion-divider");
  await expect(divider).toHaveCount(1);
  await expect(divider).toHaveAttribute("role", "separator");
  await expect(divider).toContainText("New discussion");
  // The earlier discussion stays readable above the divider.
  const order = await page.locator(".chat").evaluate(chat => {
    const text = chat.textContent ?? "";
    return [text.indexOf("differential expression is next"), text.indexOf("New discussion ·")];
  });
  expect(order[0]).toBeGreaterThanOrEqual(0);
  expect(order[1]).toBeGreaterThan(order[0]);
  // A divider, not a compaction card: nothing to expand or undo.
  await expect(page.getByTestId("context-compaction-flag")).toHaveCount(0);
  await expect(page.getByTestId("undo-compaction")).toHaveCount(0);
  await expect(page.locator(".confirm-modal")).toHaveCount(0);
  expect(await invoked(page, "send_message")).toBe(0);

  // Pressing again before anything new was said changes nothing.
  await button.click();
  await expect.poll(() => invoked(page, "start_research_assistant_discussion")).toBe(2);
  await expect(page.locator("#copy-toast")).toContainText("already a new discussion");
  await expect(divider).toHaveCount(1);
});

test("a refused new discussion while the assistant is replying leaves the conversation alone", async ({ page }) => {
  await open(page);
  await page.evaluate(() => { (window as any).__assistantDiscussionBusy = true; });
  await newDiscussion(page).click();
  await expect(page.locator("#copy-toast")).toContainText("The research assistant is replying");
  await expect(page.getByTestId("discussion-divider")).toHaveCount(0);
  await expect(page.locator(".chat")).toContainText("differential expression is next");
});

test("a saved divider is drawn when the assistant history is reopened", async ({ page }) => {
  await page.addInitScript(() => {
    (window as any).__assistantHistory.push(
      {
        role: "compaction",
        text: JSON.stringify({ before: 1200, after: 400, strategy: "new_discussion", epoch: 1, can_undo: false, undo_reason: "new_discussion" }),
        tool_name: null,
        ok: null,
      },
      { role: "user", text: "Plan the literature review instead.", tool_name: null, ok: null },
    );
  });
  await open(page);
  await expect(page.getByTestId("discussion-divider")).toHaveCount(1);
  await expect(page.getByTestId("context-compaction-flag")).toHaveCount(0);
  await expect(page.locator(".chat")).toContainText("Plan the literature review instead.");
});

test("the control and divider are localized", async ({ page }) => {
  await open(page, "zh");
  await expect(newDiscussion(page)).toHaveText("新开讨论");
  await newDiscussion(page).click();
  await expect(page.getByTestId("discussion-divider")).toContainText("新开讨论");
});
