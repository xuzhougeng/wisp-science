import { expect, test } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

const BRIEF = "Compare method B against the baseline on the same samples.";

// An agent-opened branch (`open_branch`, src-tauri/src/branch_tool.rs) is
// realized by the backend after the turn: it creates an ordinary branch, emits
// BranchOpened to the project's windows and appends a notice to the parent.
// The mock registers the branch row when the event is emitted, since the
// backend has stored it by then; the window only refreshes and keeps the draft.
test("an agent-opened branch nests under its parent, drafts the brief, and notifies the parent", async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto("/?mockBranches=1");
  await page.locator(".proj-card-main").first().click();
  const parent = page.locator('.sidebar [data-session-id="conversation-main"]');
  await parent.click();
  await expect(page.getByText("Main current result")).toBeVisible();
  const composer = page.locator("#composer-input");
  await expect(composer).toHaveValue("");
  await expect(page.locator('[data-session-id="conversation-branch-agent"]')).toHaveCount(0);

  await page.evaluate((brief) => {
    const emit = (window as any).__tauriEmit;
    emit("agent", {
      kind: "BranchOpened",
      frame_id: "conversation-main",
      branch_id: "conversation-branch-agent",
      title: "Try method B",
      brief,
    });
    emit("agent", {
      kind: "BackgroundReply",
      frame_id: "conversation-main",
      text: `Opened branch “Try method B”. It forks from the end of this turn, is listed under this conversation in the sidebar, and its composer holds this brief for you to continue:\n\n${brief}`,
    });
  }, BRIEF);

  // The parent shows the notice as a reply of its own...
  await expect(page.getByText("Opened branch “Try method B”.")).toBeVisible();
  // ...and the sidebar nests the new conversation under it as a branch, not a subagent.
  const branch = page.getByTestId("sidebar-branches").locator('[data-session-id="conversation-branch-agent"]');
  await expect(branch).toBeVisible();
  await expect(branch).toHaveAttribute("data-session-subagent", "false");
  await expect(branch).toContainText("Try method B");
  // The parent's composer is untouched by the brief.
  await expect(composer).toHaveValue("");

  // Opening the branch puts the brief in its composer, unsent and editable.
  await branch.click();
  await expect(composer).toHaveValue(BRIEF);
  await expect(composer).toBeEnabled();

  // The draft belongs to the branch: the parent's composer stays empty and the
  // brief is still waiting when the branch is reopened.
  await parent.click();
  await expect(page.getByText("Main current result")).toBeVisible();
  await expect(composer).toHaveValue("");
  await branch.click();
  await expect(composer).toHaveValue(BRIEF);
});
