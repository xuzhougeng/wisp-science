import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// A damaged project database is replaced by a backup copy or rebuilt empty
// (#1635). The outcome is recorded outside the project database and shown as
// an inline notice the next time the project is entered, until dismissed.
// Each test seeds its own outcomes; sessionStorage stands in for the
// application database so a reload plays the part of an app restart.

const restored = {
  id: 1,
  project_id: "default",
  occurred_at: 1_761_267_723,
  kind: "restored",
  copy_name: "project-default.20261024-010203.daily.sqlite",
  damaged_path: "C:\\study\\.wisp\\project.sqlite.corrupt-20261024-010203",
  backup_dir: "C:\\Users\\me\\AppData\\Roaming\\science.wisp-science\\wisp-science\\backups",
};

const rebuilt = {
  id: 2,
  project_id: "other",
  occurred_at: 1_761_267_800,
  kind: "rebuilt",
  copy_name: null,
  damaged_path: "/mock/other/.wisp/project.sqlite.corrupt-20261024-010320",
  backup_dir: null,
};

async function seed(page: Page, outcomes: unknown[]) {
  // Init scripts run again on reload; only the first run seeds.
  await page.addInitScript((seeded) => {
    if (sessionStorage.getItem("mock-database-recoveries") === null) {
      sessionStorage.setItem("mock-database-recoveries", JSON.stringify(seeded));
    }
  }, outcomes);
  await page.addInitScript(tauriMock);
}

// The mock lists "default" first and "Other project" second.
async function enterProject(page: Page, which: "default" | "other") {
  const cards = page.locator(".proj-card-main");
  await (which === "other" ? cards.filter({ hasText: "Other project" }) : cards.first()).click();
  await expect(page.locator("#composer-input")).toBeVisible();
}

function invokes(page: Page, cmd: string) {
  return page.evaluate((name) =>
    ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name), cmd);
}

test("a restored copy is announced once per entry until dismissed, then never again", async ({ page }) => {
  await seed(page, [restored, rebuilt]);
  // The guide opens in the system browser; keep the test offline.
  await page.addInitScript(() => { window.open = () => null; });
  await page.goto("/");
  // The project list alone does not show it.
  await expect(page.locator(".proj-card-main").first()).toBeVisible();
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(0);

  await enterProject(page, "default");
  const notice = page.getByTestId("database-recovery-notice");
  await expect(notice).toHaveCount(1);
  await expect(notice).toHaveAttribute("data-kind", "restored");
  await expect(notice).toHaveAttribute("role", "status");
  await expect(notice).toContainText(/Restored from a backup copy|已从备份副本恢复/);
  await expect(notice).toContainText("project-default.20261024-010203.daily.sqlite");
  await expect(notice).not.toContainText(/Rebuilt empty|已重建为空库/);
  await expect(notice.getByTestId("database-recovery-damaged")).toHaveText(restored.damaged_path);
  await expect(notice.getByTestId("database-recovery-backups")).toHaveText(restored.backup_dir);

  await notice.getByRole("button", { name: /Recovery guide|恢复说明/ }).click();
  await expect.poll(async () => (await invokes(page, "open_external_url")).length).toBe(1);
  const [guide] = await invokes(page, "open_external_url");
  const url = guide.args instanceof Map ? guide.args.get("url") : guide.args?.url;
  expect(String(url)).toMatch(/\/docs\/upgrade-backups(\.zh-CN)?\.md#/);

  // Not acknowledged yet: a restart shows it again.
  await page.reload();
  await enterProject(page, "default");
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(1);

  await page.getByRole("button", { name: /Dismiss recovery notice|关闭恢复提示/ }).click();
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(0);
  await expect.poll(async () => (await invokes(page, "acknowledge_project_database_recovery")).length).toBe(1);
  const [ack] = await invokes(page, "acknowledge_project_database_recovery");
  const id = ack.args instanceof Map ? ack.args.get("id") : ack.args?.id;
  expect(Number(id)).toBe(restored.id);

  // Acknowledged: neither a restart nor a later entry brings it back.
  await page.reload();
  await enterProject(page, "default");
  await expect.poll(async () => (await invokes(page, "list_project_database_recoveries")).length).toBeGreaterThan(0);
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(0);
});

test("an empty rebuild reads as lost history, not as a normal open", async ({ page }) => {
  await seed(page, [rebuilt]);
  await page.goto("/");
  // The first project has no outcome of its own.
  await enterProject(page, "default");
  await expect.poll(async () => (await invokes(page, "list_project_database_recoveries")).length).toBeGreaterThan(0);
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(0);

  await page.reload();
  await enterProject(page, "other");
  const notice = page.getByTestId("database-recovery-notice");
  await expect(notice).toHaveCount(1);
  await expect(notice).toHaveAttribute("data-kind", "rebuilt");
  await expect(notice).toHaveAttribute("role", "alert");
  await expect(notice).toHaveClass(/rebuilt/);
  await expect(notice).toContainText(/Rebuilt empty|已重建为空库/);
  await expect(notice).toContainText(/no usable backup copy|没有可用的备份副本/);
  await expect(notice).not.toContainText(/Restored from a backup copy|已从备份副本恢复/);
  await expect(notice.getByTestId("database-recovery-damaged")).toHaveText(rebuilt.damaged_path);
  await expect(notice.getByTestId("database-recovery-backups")).toHaveCount(0);

  await notice.getByRole("button", { name: /Dismiss recovery notice|关闭恢复提示/ }).click();
  await expect(notice).toHaveCount(0);
  await expect.poll(async () => (await invokes(page, "acknowledge_project_database_recovery")).length).toBe(1);

  await page.reload();
  await enterProject(page, "other");
  await expect.poll(async () => (await invokes(page, "list_project_database_recoveries")).length).toBeGreaterThan(0);
  await expect(page.getByTestId("database-recovery-notice")).toHaveCount(0);
});
