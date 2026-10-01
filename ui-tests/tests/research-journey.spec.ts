import { test, expect, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { tauriMock } from "./mock-tauri";
import { openSidebarEntry } from "./sidebar-nav";

const image = readFileSync(resolve(__dirname, "../fixtures/research-comparison.png")).toString("base64");
test.use({ timezoneId: "Asia/Shanghai" });
const fixedDate = new Date("2026-09-09T08:00:00Z");
test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(fixedDate);
  await page.addInitScript(tauriMock, { researchImageBase64: image });
});
async function open(page: Page, query = "") {
  await page.goto(`/${query}`);
  await page.locator(".proj-card-main").first().click();
  await openSidebarEntry(page, /Research journey|研究历程/);
  await expect(page.getByTestId("research-journey")).toBeVisible();
}

for (const route of ["sidebar", "command palette"]) {
  test(`returning home through the ${route} closes the research journey`, async ({ page }) => {
    await page.addInitScript(() => Object.defineProperty(navigator, "platform", { get: () => "Win32" }));
    await open(page);
    const journey = page.getByTestId("research-journey");
    await journey.getByTestId("journey-calendar").getByRole("button", { name: "2026-09-08", exact: true }).click();
    if (route === "sidebar") {
      await page.getByRole("button", { name: "Back to projects", exact: true }).click();
    } else {
      await page.keyboard.press("Control+p");
      const input = page.locator("#action-palette-input");
      await input.fill("Open projects");
      await expect(page.locator(".action-palette-row.active")).toContainText("Open projects");
      await input.press("Enter");
    }
    await expect(page.locator(".projects-screen")).toBeVisible();
    await expect(journey).toHaveCount(0);
    // Demo entry bypasses the normal project-switch reset. A hidden but still
    // true journey flag must not revive the old project's page here.
    await page.locator(".proj-example").click();
    await expect(page.locator(".projects-screen")).toHaveCount(0);
    await expect(journey).toHaveCount(0);
    await page.getByRole("button", { name: "Back to projects", exact: true }).click();
    await page.locator(".proj-card-main").first().click();
    await expect(journey).toHaveCount(0);
    await openSidebarEntry(page, "Research journey");
    await expect(journey).toBeVisible();
    await expect(journey.locator(".journey-day")).toHaveCount(3);
    await page.keyboard.press("Escape");
    await expect(journey).toHaveCount(0);
    await expect(page.locator("#composer-input")).toBeVisible();
  });
}

for (const platform of ["Windows NT 10.0; Win64; x64", "Macintosh; Intel Mac OS X 10_15_7"]) {
  test(`research journey respects the title bar on ${platform}`, async ({ browser }) => {
    const context = await browser.newContext({ userAgent: `Mozilla/5.0 (${platform}) AppleWebKit/537.36 Chrome/136 Safari/537.36` });
    const page = await context.newPage();
    await page.addInitScript(tauriMock);
    await open(page);
    const journey = page.getByTestId("research-journey");
    const windows = platform.startsWith("Windows");
    for (const width of [1488, 800, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(journey).toHaveCSS("top", windows ? "38px" : "0px");
      const bounds = (await journey.boundingBox())!;
      expect(bounds.y + bounds.height).toBe(900);
      if (windows) {
        const titlebar = page.locator(".window-titlebar");
        const header = (await titlebar.boundingBox())!;
        expect(bounds.y).toBeGreaterThanOrEqual(header.y + header.height);
        await page.getByRole("button", { name: "Minimize", exact: true }).click({ trial: true });
        await page.getByTestId("window-maximize").click({ trial: true });
        await page.locator(".window-close").click({ trial: true });
      } else {
        await expect(page.locator(".window-titlebar")).toHaveCount(0);
      }
    }
    if (windows) {
      await page.setViewportSize({ width: 1488, height: 900 });
      await page.getByRole("button", { name: "File", exact: true }).click();
      await expect(page.getByRole("menuitem", { name: "New session Ctrl+N", exact: true })).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.locator(".window-menu-drop")).toHaveCount(0);
      await expect(journey).toBeVisible();
    }
    await context.close();
  });
}

test("Chinese research journey naming is consistent across navigation and page controls", async ({ page }) => {
  await open(page, "?mockLocale=zh&mockJourney=design");
  const journey = page.getByTestId("research-journey");
  await expect(page.locator('.sidebar .nav .side-btn[title="研究历程"]')).toHaveCount(1);
  await expect(journey).toHaveAttribute("aria-label", "研究历程");
  await expect(journey.getByRole("heading", {name:"研究历程",exact:true})).toBeVisible();
  await expect(journey.locator(".journey-breadcrumb")).toContainText("研究历程");
  await expect(journey.getByRole("button", {name:"关闭研究历程",exact:true})).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(journey).toHaveCount(0);
});

test("run record separates metadata and logs, fits long content and keeps its close button visible", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1488, height: 1058 });
  await open(page, "?mockLocale=zh&mockJourney=design");
  const command = "& 'C:\\Users\\researcher\\AppData\\Roaming\\science.wisp-science\\wisp-science\\python\\.venv\\Scripts\\python.exe' plant-sc-papers/scripts/02_filter_classify.py";
  const output = Array.from({ length: 80 }, (_, i) => `记录 ${i}: ${"normalized_counts_".repeat(12)}`).join("\n");
  await page.evaluate(({ command, output }) => {
    Object.assign((window as any).__mockRuns.find((run: any) => run.id === "run-local-002"), {
      title: "重跑文献分类并检查规则修复", status: "succeeded", context_id: "local",
      command, stdout_tail: output, stderr_tail: "  \n", exit_code: 0,
      started_at: 1788933600, ended_at: 1788933660,
    });
  }, { command, output });
  await page.locator(".journey-activity").first().click();
  const dialog = page.getByRole("dialog", { name: "运行记录", exact: true });
  await expect(dialog.locator(".journey-run-status")).toHaveText("已完成");
  await expect(dialog.locator(".journey-run-meta")).toContainText("local");
  await expect(dialog.locator(".journey-run-meta")).toContainText("2026-09-09");
  await expect(dialog.getByRole("region", { name: "执行命令", exact: true }).locator("pre")).toHaveText(command);
  await expect(dialog.getByRole("region", { name: "标准输出 · 日志尾部", exact: true }).locator("pre")).toHaveText(output);
  await expect(dialog.getByRole("region", { name: "标准错误 · 日志尾部", exact: true })).toContainText("暂无标准错误输出");
  await expect(dialog.locator("pre")).toHaveCount(2);
  expect((await dialog.boundingBox())!.width).toBeGreaterThan(800);
  const body = dialog.locator(".journey-run-body");
  expect(await body.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
  for (const width of [1488, 800, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const bounds = (await dialog.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(900);
    expect(await body.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await body.evaluate(el => { el.scrollTop = el.scrollHeight; });
    await expect(dialog.getByRole("button", { name: "关闭运行记录", exact: true })).toBeInViewport();
    await body.evaluate(el => { el.scrollTop = 0; });
    await page.screenshot({ path: testInfo.outputPath(`run-record-${width}.png`), animations: "disabled" });
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("research-journey")).toBeVisible();
});

test("run record shows failure output and explicit missing values", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    Object.assign((window as any).__mockRuns.find((run: any) => run.id === "run-local-002"), {
      status: "failed", command: null, stdout_tail: null,
      stderr_tail: "Traceback: input file missing", exit_code: 1, started_at: null, ended_at: null,
    });
  });
  await page.locator(".journey-activity").first().click();
  const dialog = page.getByRole("dialog", { name: "Run record", exact: true });
  await expect(dialog.locator(".journey-run-status")).toHaveText("Failed");
  await expect(dialog.locator(".journey-run-meta dd")).toHaveText(["local", "1", "Not recorded", "Not recorded"]);
  await expect(dialog.getByRole("region", { name: "Command", exact: true })).toContainText("No command recorded");
  await expect(dialog.getByRole("region", { name: "Standard output · log tail", exact: true })).toContainText("No standard output recorded");
  await expect(dialog.locator("pre")).toHaveText("Traceback: input file missing");
  await dialog.getByRole("button", { name: "Close run record", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("research-journey")).toBeVisible();
});

test("daily history groups sessions, opens exact versions and preserves Escape layers", async ({ page }) => {
  await open(page);
  const journey = page.getByTestId("research-journey");
  await expect(journey.locator(".journey-day")).toHaveCount(3);
  const today = journey.locator('[data-day="2026-09-09"]');
  await expect(today).toContainText("2 experiments · 3 outputs · 4 notes · 1 conversations");
  await expect(today.locator(".journey-session-links button")).toHaveCount(1);
  // A run that started and finished today is one row, carrying its outputs.
  const compare = today.locator(".journey-activity").filter({ hasText: "Completed normalization comparison" });
  await expect(compare).toHaveCount(1);
  await expect(compare).toContainText("1 outputs · Completed");
  await today.getByRole("button", { name: "normalized_counts.csv", exact: true }).click();
  const source = journey.getByTestId("journey-source");
  await expect(source).toContainText("Version 2");
  await expect(source).toContainText("counts_matrix.csv");
  await source.getByRole("button", { name: "Open output", exact: true }).click();
  await expect(page.locator(".artifact-modal")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".artifact-modal")).toHaveCount(0);
  await expect(journey).toBeVisible();
  await source.getByRole("button", { name: "View run record", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", {name: "Run record", exact: true})).toHaveCount(0);
  await expect(journey).toBeVisible();
  await journey.getByRole("button", { name: "Add entry", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Add research entry", exact: true })).toHaveCount(0);
  await expect(journey).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(journey).toHaveCount(0);
});

test("calendar selects days and months; search and empty dates are explicit", async ({ page }) => {
  await open(page);
  const journey = page.getByTestId("research-journey");
  const calendar = journey.getByTestId("journey-calendar");
  await calendar.getByRole("button", { name: "2026-09-08", exact: true }).click();
  await expect(journey.locator(".journey-day")).toHaveCount(1);
  await expect(journey.locator(".journey-day")).toHaveAttribute("data-day", "2026-09-08");
  await journey.getByRole("button", { name: "Show full month" }).click();
  await expect(journey.locator(".journey-day")).toHaveCount(3);
  const search = journey.getByRole("searchbox", { name: "Search research records" });
  await search.fill("normalized_counts");
  await expect(journey.locator(".journey-day")).toHaveCount(1);
  await search.fill("not-present");
  await expect(journey).toContainText("No records in this view");
  await journey.getByRole("button", { name: "Previous month" }).click();
  await expect(calendar).toContainText("2026 / 08");
  await expect(journey).toContainText("No records in this view");
  await journey.getByRole("button", { name: "Back to today" }).click();
  await expect(journey.locator(".journey-day")).toHaveCount(3);
  await calendar.getByRole("button", { name: "2026-09-06", exact: true }).click();
  await expect(journey).toContainText("No records in this view");
});

test("manual backdated notes persist across reopening and show recording dates", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Add entry", exact: true }).click();
  const editor=page.getByRole("dialog", {name: "Add research entry", exact: true});
  await editor.getByLabel("Research date").fill("2026-08-31");
  await editor.getByRole("combobox", {name:"Category", exact:true}).selectOption("finding");
  await editor.getByLabel("Title", {exact:true}).fill("Baseline sensitivity observed");
  await editor.getByLabel("Details and evidence").fill("Observed in comparison run; repeat required.");
  await editor.getByRole("button", {name:"Save entry", exact:true}).click();
  await expect(editor).toHaveCount(0);
  const journey=page.getByTestId("research-journey");
  await expect(journey.getByTestId("journey-calendar")).toContainText("2026 / 08");
  await journey.locator(".journey-note-text").filter({hasText:"Baseline sensitivity observed"}).click();
  await expect(journey.getByTestId("journey-source")).toContainText("Added on 2026-09-09");
  await expect(journey.getByTestId("journey-source")).not.toContainText("12:00");
  await page.keyboard.press("Escape");
  await openSidebarEntry(page, "Research journey");
  await page.getByRole("button", {name:"Previous month"}).click();
  await expect(page.getByTestId("journey-feed")).toContainText("Baseline sensitivity observed");
});

test("read and save errors stay recoverable without losing entry text", async ({ page }) => {
  await open(page, "?mockJourney=error");
  await expect(page.getByRole("alert")).toContainText("Research store unavailable");
  await page.goto("/");
  await page.locator(".proj-card-main").first().click();
  await openSidebarEntry(page, "Research journey");
  await page.getByRole("button",{name:"Add entry",exact:true}).click();
  await page.getByLabel("Title",{exact:true}).fill("Do not lose this note");
  await page.evaluate(()=>{(window as any).__journeySaveError=true;});
  await page.getByRole("button",{name:"Save entry",exact:true}).click();
  await expect(page.getByRole("alert")).toContainText("Journal write failed");
  await expect(page.getByLabel("Title",{exact:true})).toHaveValue("Do not lose this note");
  await page.evaluate(()=>{(window as any).__journeySaveError=false;});
  await page.getByRole("button",{name:"Save entry",exact:true}).click();
  await expect(page.getByRole("dialog",{name:"Add research entry",exact:true})).toHaveCount(0);
});

test("research journey design matches the selected desktop layout and fits narrow screens", async ({ page }) => {
  const errors: string[]=[]; page.on("pageerror",e=>errors.push(e.message));
  await page.setViewportSize({width:1488,height:1058});
  await open(page,"?mockLocale=zh&mockJourney=design");
  const journey=page.getByTestId("research-journey");
  await expect(journey.locator(".journey-output-preview img").first()).toBeVisible();
  await journey.getByRole("button",{name:"normalization_comparison.png",exact:true}).click();
  await expect(journey.getByTestId("journey-source")).toContainText("归一化方法比较");
  await page.evaluate(()=>document.fonts.ready);
  await expect(journey.locator(".journey-headline").first()).toHaveCSS("font-size","18px");
  await page.screenshot({path:test.info().outputPath("desktop.png")});
  await page.setViewportSize({width:800,height:900});
  let bounds=await journey.boundingBox();expect(bounds!.x+bounds!.width).toBeLessThanOrEqual(800);
  await expect.poll(async()=>Math.round((await page.locator(".sidebar").boundingBox())!.width)).toBe(56);
  expect(bounds!.x).toBe(56);
  await expect(page.locator(".sidebar .side-btn.active")).toHaveCSS("color","rgba(0, 0, 0, 0)");
  expect(await journey.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
  await page.screenshot({path:test.info().outputPath("narrow.png")});
  await page.setViewportSize({width:390,height:844});
  bounds=await journey.boundingBox();expect(bounds!.x).toBe(0);expect(bounds!.width).toBeLessThanOrEqual(390);
  expect(await journey.evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
  await expect(journey.locator(".journey-day").first()).toBeInViewport();
  await page.screenshot({path:test.info().outputPath("mobile.png")});
  await page.setViewportSize({width:1488,height:1058});
  await expect.poll(async()=>Math.abs((await page.locator(".sidebar").boundingBox())!.width-(await journey.boundingBox())!.x)).toBeLessThan(1);
  await page.evaluate(()=>document.documentElement.setAttribute("data-theme","dark"));
  await page.screenshot({path:test.info().outputPath("dark.png")});
  expect(errors).toEqual([]);
});

test("window dialogs stay above the research page in the Escape stack", async ({ page }) => {
  await open(page);
  const journey=page.getByTestId("research-journey");
  await page.getByRole("button",{name:"Settings",exact:true}).click();
  await page.keyboard.press("Escape");
  await expect(journey).toBeVisible();
  await page.getByRole("button",{name:"Add entry",exact:true}).click();
  await page.keyboard.press("Escape");
  await expect(journey).toBeVisible();
});

test("local calendar day bounds include the extra hour at daylight-saving end", async ({ browser }) => {
  const context=await browser.newContext({timezoneId:"America/New_York"});
  const page=await context.newPage();
  await page.clock.setFixedTime(new Date("2026-11-01T17:00:00Z"));
  await page.addInitScript(tauriMock,{researchImageBase64:image});
  await open(page);
  await page.getByTestId("journey-calendar").getByRole("button",{name:"2026-11-01",exact:true}).click();
  await expect(page.locator('.journey-day[data-day="2026-11-01"]')).toBeVisible();
  const duration=await page.evaluate(()=>{
    const args=((window as any).__skillInvokeLog as any[]).filter(c=>c.cmd==="get_research_journey").at(-1).args;
    return args instanceof Map ? args.get("until")-args.get("from") : args.until-args.from;
  });
  expect(duration).toBe(25*60*60);
  await context.close();
});

test("closing during a history request does not resurrect the page", async ({ page }) => {
  await open(page);
  const errors:string[]=[];page.on("pageerror",error=>errors.push(error.message));
  await page.evaluate(()=>{(window as any).__journeyDelay=200;});
  await page.getByRole("button",{name:"Previous month"}).click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("research-journey")).toHaveCount(0);
  await openSidebarEntry(page, "Research journey");
  await expect(page.getByTestId("journey-calendar")).toContainText("2026 / 09");
  await expect(page.locator(".journey-day")).toHaveCount(3);
  expect(errors).toEqual([]);
});


test("large output days load previews in small groups", async ({ page }) => {
  await open(page,"?mockJourney=many");
  const outputs=page.locator('.journey-day[data-day="2026-09-09"] .journey-output');
  await expect(outputs).toHaveCount(3);
  await page.getByRole("button",{name:"Show more outputs",exact:true}).click();
  await expect(outputs).toHaveCount(8);
  await expect(page.getByRole("button",{name:"Show more outputs",exact:true})).toHaveCount(0);
});

test("research journey does not reserve classic scrollbar gutters", async ({ page }) => {
  await page.setViewportSize({ width: 1488, height: 900 });
  await open(page);
  const metrics = (el: Element) => ({
    gutter: getComputedStyle(el).scrollbarGutter,
    width: getComputedStyle(el, "::-webkit-scrollbar").width,
    thumb: getComputedStyle(el, "::-webkit-scrollbar-thumb").backgroundColor,
  });
  await expect.poll(() => page.getByTestId("journey-feed").evaluate(metrics)).toEqual({
    gutter: "auto",
    width: "10px",
    thumb: "rgba(0, 0, 0, 0)",
  });
  await expect.poll(() => page.locator(".journey-aside").evaluate(metrics)).toEqual({
    gutter: "auto",
    width: "10px",
    thumb: "rgba(0, 0, 0, 0)",
  });
});

test("relationship list scrolls under the wheel over a middle column", async ({ page }) => {
  await page.setViewportSize({ width: 1488, height: 900 });
  await open(page, "?mockLocale=zh&mockGraph=dense");
  const journey = page.getByTestId("research-journey");
  await journey.getByRole("tab", { name: "关系图", exact: true }).click();
  const list = journey.getByTestId("research-graph-list");
  const board = journey.getByTestId("journey-relationships");
  await expect(list.locator(".graph-node")).toHaveCount(77);
  expect(await list.evaluate((el) => el.scrollHeight > el.clientHeight + 40)).toBe(true);
  expect(await board.evaluate((el) => {
    const overflowY = getComputedStyle(el).overflowY;
    return (overflowY === "auto" || overflowY === "scroll") && el.scrollHeight > el.clientHeight + 1;
  })).toBe(false);
  const runCard = list.locator(".control-section")
    .filter({ has: page.locator(".control-section-head", { hasText: "运行" }) })
    .locator(".graph-node").first();
  await runCard.hover();
  const before = await list.evaluate((el) => el.scrollTop);
  await page.mouse.wheel(0, 600);
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBeGreaterThan(before + 40);
  await journey.getByRole("button", { name: "图谱", exact: true }).click();
  const canvas = journey.getByTestId("research-graph-canvas");
  await expect(canvas).toBeVisible();
  expect(await canvas.evaluate((el) => el.scrollHeight > el.clientHeight + 40)).toBe(true);
  const canvasBox = (await canvas.boundingBox())!;
  await page.mouse.move(canvasBox.x + canvasBox.width / 2, canvasBox.y + canvasBox.height / 2);
  const canvasBefore = await canvas.evaluate((el) => el.scrollTop);
  await page.mouse.wheel(0, 600);
  await expect.poll(() => canvas.evaluate((el) => el.scrollTop)).toBeGreaterThan(canvasBefore + 40);
});
