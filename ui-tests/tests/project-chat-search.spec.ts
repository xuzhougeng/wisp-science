import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

const dialog = (page: Page) => page.getByRole("dialog", { name: "Project chat history", exact: true });
const search = (page: Page) => dialog(page).getByRole("searchbox");

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.addInitScript(() => {
    const w = window as any;
    const invoke = w.__TAURI__.core.invoke;
    w.__chatSearchCalls = [];
    w.__chatSearchLoads = [];
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd === "locate_project_chat_match") {
        return { user_index: 0, before_seq: 101 };
      }
      if (cmd === "search_project_chats") {
        const query = String(arg("query")).trim().toLowerCase();
        const rawCursor = arg("cursor");
        const cursor = rawCursor instanceof Map ? Object.fromEntries(rawCursor) : rawCursor;
        w.__chatSearchCalls.push({ query, projectId: arg("projectId"), cursor });
        if (w.__chatSearchFail) throw new Error("Search database unavailable");
        if (query === "slow") await new Promise<void>(resolve => { w.__releaseSlowSearch = resolve; });
        if (!["needle", "slow", "%_"].includes(query)) return { items: [], next_cursor: null };
        const literal = query === "%_";
        const count = literal || query === "slow" ? 1 : 51;
        const start = cursor ? 50 : 0;
        const items = Array.from({ length: Math.min(50, count - start) }, (_, index) => ({
          session_id: "old-history", session_title: query === "slow" ? "Obsolete result" : "Saved research discussion",
          message_seq: 2, role: "assistant", ts: 100 - start - index,
          before: literal ? "<script>" : `Historical result ${start + index + 1}: `,
          matched: literal ? "%_" : "Needle", after: literal ? "</script>" : " in an older reply",
        }));
        return {
          items,
          next_cursor: !cursor && count > 50 ? { ts: 51, session_id: "old-history", message_seq: 2 } : null,
        };
      }
      if (cmd === "load_session" && arg("id") === "old-history") {
        const before = arg("beforeSeq");
        w.__chatSearchLoads.push(before ?? null);
        return {
          items: [
            { role: "user", text: before ? "Original research question" : "Latest unrelated question" },
            { role: "assistant", text: before ? "Needle in the saved older reply" : "Latest unrelated answer" },
          ],
          next_before_seq: before ? null : 101,
          user_offset: before ? 0 : 1,
          outline: before ? [] : [
            // Event-backed outlines deliberately expose no message cursor.
            { user_index: 0, seq: null, text: "Original research question", sent_at: 100 },
            { user_index: 1, seq: null, text: "Latest unrelated question", sent_at: 200 },
          ],
        };
      }
      return invoke(cmd, args);
    };
  });
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await expect(page.locator("#composer-input")).toBeVisible();
});

test("Search entry finds project-wide message snippets, pages results and opens older history", async ({ page }) => {
  await page.locator(".sidebar").getByRole("button", { name: "Search sessions", exact: true }).click();
  await page.getByRole("button", { name: /Search this project's chat history/ }).click();
  await expect(search(page)).toBeFocused();
  expect(await page.evaluate(() => (window as any).__chatSearchCalls)).toEqual([]);
  await search(page).fill("NEEDLE");
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(50);
  await expect(dialog(page).locator("mark").first()).toHaveText("Needle");
  expect(await page.evaluate(() => (window as any).__chatSearchCalls[0].projectId)).toBe("default");
  await dialog(page).getByRole("button", { name: "Load more matches" }).click();
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(51);
  await expect(dialog(page).getByRole("button", { name: "Load more matches" })).toHaveCount(0);
  await dialog(page).locator(".project-chat-search-result").first().click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(page.locator(".center-tabs > .center-tab").first()).toContainText("Saved research discussion");
  await expect(page.getByText("Needle in the saved older reply", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__chatSearchLoads)).toEqual([null, 101]);
});

test("literal snippets are safe text; empty results and failed searches can be retried", async ({ page }) => {
  await page.keyboard.press("Control+Shift+f");
  await search(page).fill("%_");
  await expect(dialog(page).locator("mark")).toHaveText("%_");
  await expect(dialog(page).locator(".project-chat-search-snippet")).toHaveText("<script>%_</script>");
  await expect(dialog(page).locator("script")).toHaveCount(0);
  await search(page).fill("absent");
  await expect(dialog(page)).toContainText("No matching messages in this project.");
  await page.evaluate(() => { (window as any).__chatSearchFail = true; });
  await search(page).fill("needle");
  await expect(dialog(page).getByRole("alert")).toContainText("Search database unavailable");
  await page.evaluate(() => { (window as any).__chatSearchFail = false; });
  await dialog(page).getByRole("button", { name: "Retry search" }).click();
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(50);
  await search(page).fill("   ");
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(0);
  await expect(dialog(page)).toContainText("Search questions, replies and tool output.");
});

test("stale responses cannot replace a new query or a reopened search", async ({ page }) => {
  await page.keyboard.press("Control+Shift+f");
  await search(page).fill("slow");
  await expect.poll(() => page.evaluate(() => typeof (window as any).__releaseSlowSearch)).toBe("function");
  await search(page).fill("needle");
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(50);
  await page.evaluate(async () => {
    (window as any).__releaseSlowSearch();
    await new Promise(requestAnimationFrame);
  });
  await expect(dialog(page).getByText("Obsolete result")).toHaveCount(0);

  await page.evaluate(() => { delete (window as any).__releaseSlowSearch; });
  await search(page).fill("slow");
  await expect.poll(() => page.evaluate(() => typeof (window as any).__releaseSlowSearch)).toBe("function");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+Shift+f");
  await page.evaluate(async () => {
    (window as any).__releaseSlowSearch();
    await new Promise(requestAnimationFrame);
  });
  await expect(search(page)).toHaveValue("");
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(0);
});

for (const modifier of ["Control", "Meta"]) {
  test(`${modifier}+Shift+F participates in the window Escape stack without preparing focus`, async ({ page }) => {
    await page.getByRole("button", { name: "Shelved conversations", exact: true }).click();
    const parent = page.getByRole("dialog", { name: "Shelved conversations", exact: true });
    await page.keyboard.press(`${modifier}+Shift+f`);
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);
    await expect(parent).toBeVisible();
    await page.keyboard.press(`${modifier}+Shift+f`);
    await page.keyboard.press(`${modifier}+p`);
    await expect(page.locator(".action-palette")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".action-palette")).toHaveCount(0);
    await expect(dialog(page)).toBeVisible();
    await page.keyboard.press(`${modifier}+k`);
    await expect(page.locator(".conversation-search-dialog")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".conversation-search-dialog")).toHaveCount(0);
    await expect(dialog(page)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(parent).toBeVisible();
  });
}

test("chat search fits a narrow viewport and Enter opens the selected result", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 700 });
  await page.keyboard.press("Control+Shift+f");
  await search(page).fill("needle");
  await expect(dialog(page).locator(".project-chat-search-result")).toHaveCount(50);
  expect(await dialog(page).evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  await search(page).press("ArrowDown");
  await expect(dialog(page).locator(".project-chat-search-result.active")).toContainText("Historical result 2");
  await search(page).press("Enter");
  await expect(page.getByText("Needle in the saved older reply", { exact: true })).toBeVisible();
});

test("a dialog above chat search consumes Escape before search", async ({ page }) => {
  // The default mock project has no saved conversations.
  await page.locator(".sidebar").getByRole("button", { name: "New session", exact: true }).click();
  await page.locator(".sidebar .ses").first().click({ button: "right" });
  await page.locator(".ctx-menu").getByRole("button", { name: "Rename", exact: true }).click();
  await expect(page.locator("#rename-session-input")).toBeVisible();
  await page.keyboard.press("Control+Shift+f");
  await page.keyboard.press("Escape");
  await expect(page.locator("#rename-session-input")).toHaveCount(0);
  await expect(dialog(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);
});
