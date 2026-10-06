import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.addInitScript(() => {
    const w = window as any;
    const invoke = w.__TAURI__.core.invoke;
    w.__localUploads = [];
    w.__uploadMode = "success";
    const files: Record<string, string[]> = {};
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd === "upload_local_files") {
        const destinationDir = arg("destinationDir");
        const sourcePaths = arg("sourcePaths");
        w.__localUploads.push({ destinationDir, sourcePaths });
        await new Promise(resolve => setTimeout(resolve, 150));
        if (w.__uploadMode === "cancel") return [];
        if (w.__uploadMode === "error") throw new Error("Upload destination unavailable");
        const name = "uploaded.csv";
        (files[destinationDir] ??= []).push(name);
        const results: any[] = [{ source: "/external/uploaded.csv", path: `${destinationDir}/${name}`, error: null }];
        if (w.__uploadMode === "partial") results.push({ source: "/missing.csv", path: null, error: "File not found" });
        return results;
      }
      const result = await invoke(cmd, args);
      if (cmd === "list_dir") return [...result, ...(files[arg("path")] ?? []).map(name => ({ name, is_dir: false, size: 12 }))];
      if (cmd === "search_files" && arg("query") === "uploaded") return Object.entries(files).flatMap(([dir, names]) => names.map(name => ({ name, path: `${dir}/${name}`, is_dir: false, size: 12 })));
      return result;
    };
  });
});

async function openFiles(page: Page) {
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await expect(page.getByTestId("files-local-upload")).toBeVisible();
}

async function dropEvent(page: Page, kind: string, selector = ".rp-files", paths: string[] = []) {
  await page.locator(selector).evaluate((element, { kind, paths }) => {
    const rect = element.getBoundingClientRect();
    (window as any).__tauriEmit("native-file-drop", { kind, paths, x: (rect.left + rect.width / 2) * window.devicePixelRatio, y: (rect.top + rect.height / 2) * window.devicePixelRatio });
  }, { kind, paths });
}

test("local Upload copies to the current subdirectory and refreshes the listing", async ({ page }) => {
  await openFiles(page);
  await page.locator('.fb-row.dir[data-workspace-path="DEG"]').click();
  await page.getByTestId("files-local-upload").click();
  await expect(page.getByTestId("files-local-upload")).toBeDisabled();
  await expect(page.locator('.fb-row[data-workspace-path="DEG/uploaded.csv"]')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__localUploads)).toEqual([{ destinationDir: "DEG", sourcePaths: undefined }]);
  await expect(page.getByTestId("files-local-upload")).toBeEnabled();
});

test("native drop imports multiple paths into the local folder without attaching to chat", async ({ page }) => {
  await openFiles(page);
  const paths = [String.raw`C:\研究 数据\a.csv`, "/Users/alice/b.csv"];
  await dropEvent(page, "enter", ".rp-files", paths);
  await expect(page.locator(".rp-files")).toHaveClass(/drop-target/);
  await dropEvent(page, "drop", ".rp-files", paths);
  await expect.poll(() => page.evaluate(() => (window as any).__localUploads)).toEqual([{ destinationDir: ".", sourcePaths: paths }]);
  await expect(page.locator(".rp-files")).not.toHaveClass(/drop-target/);
  await expect(page.locator('.composer-attachment-row')).toHaveCount(0);
  await expect(page.locator('.fb-row[data-workspace-path="uploaded.csv"]')).toBeVisible();
});

test("native hover cancellation and moving outside Files clears highlight without importing", async ({ page }) => {
  await openFiles(page);
  await dropEvent(page, "enter");
  await expect(page.locator(".rp-files")).toHaveClass(/drop-target/);
  await dropEvent(page, "leave");
  await expect(page.locator(".rp-files")).not.toHaveClass(/drop-target/);
  await dropEvent(page, "enter");
  await dropEvent(page, "over", ".composer-inner");
  await expect(page.locator(".rp-files")).not.toHaveClass(/drop-target/);
  await dropEvent(page, "drop", ".composer-inner", ["/external/a.csv"]);
  await expect(page.locator(".composer-attachment-row.ready")).toContainText("a.csv");
  expect(await page.evaluate(() => (window as any).__localUploads)).toEqual([]);
});

for (const mode of ["cancel", "error"]) {
  test(`local upload ${mode} restores button and leaves directory unchanged`, async ({ page }) => {
    await openFiles(page);
    await page.evaluate(mode => { (window as any).__uploadMode = mode; }, mode);
    await page.getByTestId("files-local-upload").click();
    await expect.poll(() => page.evaluate(() => (window as any).__localUploads.length)).toBe(1);
    await expect(page.getByTestId("files-local-upload")).toBeEnabled();
    await expect(page.locator('.fb-row[data-workspace-path="uploaded.csv"]')).toHaveCount(0);
    if (mode === "error") await expect(page.locator("#copy-toast")).toContainText("Upload destination unavailable");
  });
}

test("partial upload reports failed paths and refreshes active search", async ({ page }) => {
  await openFiles(page);
  await page.locator(".fb-search").fill("uploaded");
  await page.evaluate(() => { (window as any).__uploadMode = "partial"; });
  await page.getByTestId("files-local-upload").click();
  await expect(page.locator("#copy-toast")).toContainText("Uploaded 1 file(s)");
  await expect(page.locator("#copy-toast")).toContainText("/missing.csv: File not found");
  await expect(page.locator('.fb-row[data-workspace-path="./uploaded.csv"]')).toBeVisible();
});


test("native drop on SSH Files retains remote upload routing", async ({ page }) => {
  await openFiles(page);
  await page.getByRole("combobox", { name: "File location" }).selectOption("ssh:gpu-server");
  await expect(page.getByRole("textbox", { name: "Remote path" })).toHaveValue("/home/research");
  await expect(page.getByTestId("files-local-upload")).toHaveCount(0);
  await dropEvent(page, "drop", ".rp-files", ["/external/a.csv"]);
  await expect.poll(() => page.evaluate(() => { const args = (window as any).__skillInvokeLog?.filter((entry: any) => entry.cmd === "upload_to_context").at(-1)?.args; return args instanceof Map ? Object.fromEntries(args) : args; })).toMatchObject({
    contextId: "ssh:gpu-server", destinationDir: "/home/research", sourcePaths: ["/external/a.csv"],
  });
  expect(await page.evaluate(() => (window as any).__localUploads)).toEqual([]);
});


test.describe("scaled desktop native drops", () => {
  test.use({ deviceScaleFactor: 1.5 });
  test("physical coordinates target Files and keep composer drops out of the project", async ({ page }) => {
    await openFiles(page);
    await dropEvent(page, "enter");
    await expect(page.locator(".rp-files")).toHaveClass(/drop-target/);
    // Control only the hit rectangles, independent of sidebar transforms.
    // A physical point in Files maps to a CSS point in the composer at 150%.
    await page.evaluate(() => {
      document.querySelector(".composer-inner")!.getBoundingClientRect = () => new DOMRect(120, 120, 400, 150);
      document.querySelector(".rp-files")!.getBoundingClientRect = () => new DOMRect(600, 120, 300, 450);
    });
    // Pick a composer point whose unscaled physical coordinates land in Files.
    await page.locator(".composer-inner").evaluate(element => {
      const composer = element.getBoundingClientRect();
      const files = document.querySelector(".rp-files")!.getBoundingClientRect();
      const scale = window.devicePixelRatio;
      const rawX = (files.left + files.right) / 2;
      const rawY = Math.max(composer.top * scale + 2, files.top + 2);
      if (rawX / scale < composer.left || rawX / scale > composer.right ||
          rawY / scale > composer.bottom || rawY > files.bottom) {
        throw new Error("Fixture must exercise overlapping scaled and unscaled targets");
      }
      (window as any).__tauriEmit("native-file-drop", { kind: "drop", paths: ["/external/scaled.csv"], x: rawX, y: rawY });
    });
    await expect(page.locator(".composer-attachment-row.ready")).toContainText("scaled.csv");
    expect(await page.evaluate(() => (window as any).__localUploads)).toEqual([]);
    await dropEvent(page, "drop", ".rp-files", ["/external/local.csv"]);
    await expect.poll(() => page.evaluate(() => (window as any).__localUploads)).toEqual([{ destinationDir: ".", sourcePaths: ["/external/local.csv"] }]);
  });
});
