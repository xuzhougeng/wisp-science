import { chromium, expect, test, type Page, type TestInfo } from "@playwright/test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { tauriMock } from "./mock-tauri";

const locales = process.env.WISP_TUTORIAL_LOCALE === "en" ? ["en"] as const
  : process.env.WISP_TUTORIAL_LOCALE === "zh" ? ["zh"] as const : ["zh", "en"] as const;
for (const locale of locales) {
const tr = <T>(zh: T, en: T): T => locale === "en" ? en : zh;
test.describe(`tutorial screenshots (${locale})`, () => {

// Real UI with deterministic teaching data; no model API, SSH host, or browser bridge.
test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Linux x86_64" });
  });
  await page.addInitScript(tauriMock);
  await page.addInitScript((locale) => {
    const tr = <T>(zh: T, en: T): T => locale === "en" ? en : zh;
    const core = (window as any).__TAURI__.core;
    const invoke = core.invoke;
    core.invoke = async (cmd: string, args: any) => {
      const quickStart = new URLSearchParams(location.search).has("mockQuickStartTutorial");
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (quickStart && cmd === "send_message") {
        ((window as any).__skillInvokeLog ??= []).push({ cmd, args });
        const frame = String(arg("sessionId"));
        (window as any).__tauriEmit("agent", { kind: "User", frame_id: frame, text: arg("message") });
        (window as any).__tauriEmit("agent", {
          kind: "Text", frame_id: frame,
          delta: tr("根据你提供的数字：\n\n| 样本 | 数值 |\n| --- | --- |\n| A | 10 |\n| B | 20 |\n| C | 30 |\n\n**样本数：3。平均值：20。**\n\n计算方式：(10 + 20 + 30) ÷ 3 = 20。\n\n本次只使用消息中的数字，没有读取文件、运行代码或访问网页。", "From the supplied numbers:\n\n| Sample | Value |\n| --- | --- |\n| A | 10 |\n| B | 20 |\n| C | 30 |\n\n**Sample count: 3. Mean: 20.**\n\nCalculation: (10 + 20 + 30) / 3 = 20.\n\nI used only your message; I did not read files, run code, or visit websites."),
        });
        (window as any).__tauriEmit("agent", { kind: "Done", frame_id: frame, stop_reason: "end_turn" });
        return frame;
      }
      const result = await invoke(cmd, args);
      if (quickStart) {
        if (cmd === "list_projects" || cmd === "list_recent_sessions") return [];
        if (cmd === "create_project") return { ...result, name: arg("name"), workspace_dir: "/mock/root/new-project" };
        if (cmd === "open_project") return { ...result, name: tr("第一次使用 Wisp", "My first Wisp project"), workspace_dir: "/mock/root/new-project" };
        if (cmd === "get_project_info") return { ...result, name: tr("第一次使用 Wisp", "My first Wisp project"), root: "/mock/root/new-project" };
        if (cmd === "get_settings") return { ...result, follow_up_questions: false };
        return result;
      }
      if (locale === "en" && cmd === "list_recent_sessions") {
        return result.map((row: any) => ({ ...row, title: row.id === "s-needs-you" ? "Find a single-cell paper" : row.title }));
      }
      if (cmd === "list_sessions_page") {
        result.items = [{ id: "s1", title: tr("示例：检查差异分析结果", "Example: inspect differential expression"), ts: 1788998400, folder_id: null, has_user_turn: true, model_id: "default" }];
      }
      if (cmd === "load_session") return {
        items: [
          { role: "user", text: tr("请检查 results/differential_expression.csv 的列名和缺失值，暂时不要修改文件。项目联系人：李老师。", "Check the columns and missing values in results/differential_expression.csv without changing the file. Project contact: Dr. Lee."), tool_name: null, ok: null },
          { role: "assistant", text: tr("## 数据检查记录（教学示例）\n\n表格包含 `gene`、`log2FoldChange`、`padj` 三列。\n\n| 检查项 | 示例结果 |\n| --- | --- |\n| 行数 | 1,200 |\n| log2FoldChange 缺失值 | 3 |\n| padj 缺失值 | 8 |\n\n尚未删除缺失值，也没有改写原文件。下一步可以先确认过滤规则，再绘制火山图。", "## Data inspection (teaching example)\n\nColumns: `gene`, `log2FoldChange`, and `padj`.\n\n| Check | Example result |\n| --- | --- |\n| Rows | 1,200 |\n| Missing log2FoldChange | 3 |\n| Missing padj | 8 |\n\nNo missing values were removed and the source file was not modified. Confirm filtering rules before plotting a volcano plot."), tool_name: null, ok: null },
          { role: "user", text: tr("请先说明过滤规则，我确认后再画图。", "Explain the filtering rules first. Wait for my confirmation before plotting."), tool_name: null, ok: null },
          { role: "assistant", text: tr("建议先标记缺失值，并保留原始表格。作图时单独建立有效行的副本；阈值与保留的行数一并记录。这里还没有执行绘图。", "Mark missing values and preserve the original table. Plot from a separate copy of valid rows, recording thresholds and retained row counts. No plot has been generated yet."), tool_name: null, ok: null },
        ], next_before_seq: null, user_offset: 0, branches: [],
      };
      return result;
    };
  }, locale);
});

async function enter(page: Page) {
  await page.goto(`/?mockLocale=${locale}`);
  await page.locator(".proj-card-main").first().click();
  await page.locator('[data-session-id="s1"]').click();
  await expect(page.locator("#composer-input")).toBeVisible();
}
async function settings(page: Page, name: string) {
  await page.getByRole("button", { name: tr("设置", "Settings"), exact: true }).click();
  await page.locator(".settings-nav").getByRole("button", { name, exact: true }).click();
}
async function shot(page: Page, info: TestInfo, name: string) {
  await page.evaluate(async () => { await document.fonts.ready; });
  if (locale === "en") {
    expect(await page.locator("body").innerText()).not.toMatch(/[\p{Script=Han}]/u);
    for (const field of await page.locator("input:visible, textarea:visible").all()) {
      expect(await field.inputValue()).not.toMatch(/[\p{Script=Han}]/u);
    }
  }
  const relative = `${locale === "en" ? "en/" : ""}${name}.png`;
  const path = process.env.WISP_TUTORIAL_SHOTS
    ? resolve(process.env.WISP_TUTORIAL_SHOTS, relative)
    : info.outputPath(relative);
  mkdirSync(dirname(path), { recursive: true });
  await page.screenshot({ path, animations: "disabled" });
}
async function emit(page: Page, name: string, payload: unknown) {
  await expect.poll(() => page.evaluate((event) => (window as any).__tauriListenerReady?.(event), name)).toBe(true);
  await page.evaluate(({ name, payload }) => (window as any).__tauriEmit(name, payload), { name, payload });
}

test("quick start walks through onboarding, project creation, and a first conversation", async ({ page }, info) => {
  await page.goto(`/?mockLocale=${locale}&mockOnboarding=1&mockQuickStartTutorial=1`);
  const modal = page.locator(".onboard");
  const titles = [tr("欢迎使用 Wisp Science", "Welcome to Wisp Science"), tr("Wisp Science 能做什么", "What Wisp Science can do"), tr("配置模型", "Set up your model"), tr("本地环境（可选）", "Local environment (optional)")];
  const names = ["01-welcome", "02-features", "03-model", "04-environment"];
  for (let step = 0; step < 4; step++) {
    await expect(modal.getByRole("heading")).toHaveText(titles[step]);
    await expect(modal.locator(".onboard-dot").nth(step)).toHaveClass(/active/);
    if (step === 3) await expect(modal.getByTestId("local-environment")).toBeVisible();
    await shot(page, info, `quick-start/${names[step]}`);
    if (step === 2) await modal.locator('input[type="password"]').fill("demo-key-not-valid");
    await modal.locator(".row > .primary").click();
  }
  await expect(modal).toBeHidden();
  await expect(page.getByRole("button", { name: tr(/新建项目/, /New project/) })).toBeVisible();
  await shot(page, info, "quick-start/05-projects");
  await page.getByRole("button", { name: tr(/新建项目/, /New project/) }).click();
  await page.locator("#new-project-name").fill(tr("第一次使用 Wisp", "My first Wisp project"));
  await page.locator(".pn-dir .btn-ghost").click();
  await expect(page.locator(".pn-dir .path")).toHaveText("/mock/root/new-project");
  await shot(page, info, "quick-start/06-create-project");
  await page.getByRole("button", { name: tr("创建", "Create"), exact: true }).click();
  const composer = page.locator("#composer-input");
  await expect(composer).toBeVisible();
  await expect(page.locator(".sidebar")).toContainText(tr("第一次使用 Wisp", "My first Wisp project"));
  await expect.poll(() => page.evaluate(() => (window as any).__tauriListenerReady?.("agent"))).toBe(true);
  const prompt = tr("这是我的第一次对话测试。请把样本 A=10、B=20、C=30 整理成表格，并告诉我样本数和平均值。只根据这些数字回答，不读取文件、不运行代码、不访问网页。", "This is my first conversation test. Put samples A=10, B=20, and C=30 in a table, then report the sample count and mean. Answer only from these numbers. Do not read files, run code, or visit websites.");
  await composer.fill(prompt);
  await page.getByRole("button", { name: tr("发送", "Send"), exact: true }).click();
  await expect(page.locator(".msg.user")).toContainText(prompt);
  await expect(page.locator(".msg.assistant")).toContainText(tr("样本数：3。平均值：20。", "Sample count: 3. Mean: 20."));
  await expect(page.locator(".msg.assistant table tbody tr")).toHaveCount(3);
  await expect(composer).toHaveValue("");
  await shot(page, info, "quick-start/07-first-conversation");
});

test("model tutorial shows API access and conversation selection", async ({ page }, info) => {
  await enter(page);
  await settings(page, tr("模型", "Models"));
  await expect(page.getByRole("button", { name: tr("添加 API 接入", "Add API access"), exact: true })).toBeVisible();
  await shot(page, info, "models/01-overview");
  await page.getByRole("button", { name: tr("添加 API 接入", "Add API access"), exact: true }).click();
  await page.getByLabel(/^Base URL/).fill("https://api.deepseek.com");
  await expect(page.getByTestId("provider-model-row").first()).toBeVisible();
  await page.getByTestId("provider-model-row").first().getByLabel(tr("模型 ID", "Model ID"), { exact: true }).fill("deepseek-v4-flash");
  await page.getByTestId("provider-model-row").first().getByLabel(tr("显示名称（别名）", "Display name"), { exact: true }).fill(tr("实验室主模型（示例）", "Laboratory primary model (example)"));
  await page.getByTestId("provider-model-row").nth(1).getByLabel(tr("模型 ID", "Model ID"), { exact: true }).fill("deepseek-v4-pro");
  await shot(page, info, "models/02-api-access");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.locator(".model-picker-btn").click();
  await expect(page.locator(".model-menu")).toBeVisible();
  await shot(page, info, "models/03-picker");
});

test("browser tutorial shows settings and turn-end tab selection", async ({ page }, info) => {
  await enter(page);
  await settings(page, tr("浏览器", "Browser"));
  await page.getByTestId("browser-prefer-host").fill("pubmed.ncbi.nlm.nih.gov");
  await page.getByTestId("browser-prefer-add").click();
  await expect(page.getByTestId("browser-prefer-list")).toContainText("pubmed.ncbi.nlm.nih.gov");
  await shot(page, info, "browser/01-settings");
  await page.keyboard.press("Escape");
  await emit(page, "browser-tab-cleanup", {
    turn_id: "tutorial-turn", frame_id: "s1", tabs: [
      { session: "shared", tab_id: 11, url: "https://pubmed.ncbi.nlm.nih.gov/", title: tr("PubMed 检索页（教学示例）", "PubMed search (teaching example)"), initial_url: "https://pubmed.ncbi.nlm.nih.gov/" },
      { session: "shared", tab_id: 12, url: "https://example.com/", title: tr("Example Domain（教学示例）", "Example Domain (teaching example)"), initial_url: "https://example.com/" },
    ],
  });
  await expect(page.getByTestId("browser-tab-cleanup")).toBeVisible();
  await page.getByTestId("browser-tab-cleanup-check-11").uncheck();
  await shot(page, info, "browser/02-tabs");
  await page.keyboard.press("Escape");
  await emit(page, "browser-needs-human", { tabs: [{ session: "shared", tab_id: 12, url: "https://example.com/", title: tr("需要人工验证（教学示例）", "Human verification required (teaching example)"), reason: "captcha_challenge", frame_id: "s1", turn_id: "tutorial-human" }] });
  await expect(page.getByTestId("browser-needs-human")).toBeVisible();
  await shot(page, info, "browser/03-human-check");
});

test("server tutorial shows SSH settings, attachment, and terminal dock", async ({ page }, info) => {
  await enter(page);
  await settings(page, tr("环境", "Environments"));
  await page.getByRole("button", { name: tr("添加 SSH 主机", "Add SSH host") }).click();
  await page.locator("#add-host-alias").fill("gpu-lab");
  await page.locator("#host-hostname").fill("gpu.example.org");
  await page.locator("#host-user").fill("researcher");
  await page.locator("#host-identity").fill("~/.ssh/id_ed25519");
  await page.locator("#host-notes").fill(tr("教学示例：长任务提交到调度器；数据保留在远程目录；运行前确认项目路径。", "Teaching example: submit long tasks to the scheduler, keep data remote, and confirm the project path before running."));
  await shot(page, info, "servers/01-add-ssh");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: tr("Agent 选项", "Agent options") }).click();
  await page.getByRole("button", { name: tr(/^计算环境/, /^Compute/) }).click();
  const server = page.getByRole("menu", { name: tr("计算环境", "Compute") }).locator('[data-context-id="ssh:gpu-server"]');
  if (!(await server.getAttribute("class"))?.includes("enabled")) await server.click();
  await expect(server).toHaveClass(/enabled/);
  await shot(page, info, "servers/02-context");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: tr("切换面板", "Toggle panel") }).click();
  await page.locator(".rightpane").getByRole("button", { name: tr("环境", "Environment"), exact: true }).click();
  await page.locator(".context-card", { hasText: "ssh:gpu-server" }).getByRole("button", { name: tr("打开终端", "Open terminal") }).click();
  await expect(page.getByTestId("terminal-dock").locator(".xterm-rows")).toContainText("terminal ready");
  await shot(page, info, "servers/03-terminal");
});

test("transfer tutorial shows project choices, session archive, and selective sharing", async ({ page }, info) => {
  await page.goto(`/?mockLocale=${locale}`);
  await page.getByRole("button", { name: tr("导入项目", "Import project"), exact: true }).click();
  await expect(page.getByTestId("project-import-options")).toBeVisible();
  await shot(page, info, "transfer/01-project-import");
  await page.keyboard.press("Escape");
  await page.locator(".proj-card:not(.proj-example)").first().getByRole("button", { name: tr("导出项目", "Export project") }).click();
  await expect(page.getByTestId("project-export-options")).toBeVisible();
  await shot(page, info, "transfer/02-project-export");
  await page.keyboard.press("Escape");
  await page.locator(".proj-card-main").first().click();
  const session = page.locator('[data-session-id="s1"]');
  await session.click();
  await session.click({ button: "right" });
  await expect(page.locator(".ctx-menu").getByRole("button", { name: tr("导出会话", "Export session"), exact: true })).toBeVisible();
  await shot(page, info, "transfer/03-session-export");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+p");
  await page.locator("#action-palette-input").fill(tr("导入会话归档", "Import session archive"));
  await expect(page.locator(".action-palette-row", { hasText: tr("导入会话归档", "Import session archive") })).toBeVisible();
  await shot(page, info, "transfer/04-session-import");
  await page.keyboard.press("Escape");
  await page.getByTestId("share-topbar").click();
  const share = page.getByTestId("share-overlay");
  await expect(share).toBeVisible();
  await share.locator("#share-redact-input").fill(tr("李老师", "Dr. Lee"));
  await share.locator(".share-row").nth(2).locator("input").uncheck();
  await share.locator(".share-row").nth(3).locator("input").uncheck();
  await expect(share.locator(".share-count")).toContainText("2/4");
  await shot(page, info, "transfer/05-share");
});

if (locale === "en") {
  test("ACP tutorial shows the agent list and adapter form in English", async ({ page }, info) => {
    await enter(page);
    await settings(page, "Models");
    await page.getByTestId("open-acp-agents-from-settings").click();
    await expect(page.getByTestId("acp-agents-list")).toBeVisible();
    await shot(page, info, "acp/01-overview");
    await page.getByTestId("add-acp-agent-settings").click();
    const form = page.getByTestId("acp-agents-settings");
    await form.getByTestId("acp-agent-label").fill("Codex ACP");
    await form.getByTestId("acp-agent-command").fill("npx");
    await form.getByTestId("acp-agent-args").fill("-y\n@agentclientprotocol/codex-acp");
    await shot(page, info, "acp/02-add-agent");
  });

  test("trajectory tutorial shows the documented data inspection and KeyError in English", async ({ page }, info) => {
    await enter(page);
    await page.evaluate(() => {
      const cell = (kind: string, summary: string, extra: Record<string, unknown> = {}) => ({
        kind, summary, detail_input: null, detail_output: summary, ok: null,
        is_error: false, ts: 1788998400000, duration_ms: null, usage: null, ...extra,
      });
      (window as any).__trajectorySnapshot = {
        model: "deepseek-v4-pro",
        turns: [
          { index: 1, started_at: 1788998400000, cells: [
            cell("user", "Check the columns and missing values in results/differential_expression.csv."),
            cell("assistant", "I will read the table and check its columns without changing the file.", { ts: 1788998400500, duration_ms: 1200 }),
            cell("tool", "python · inspect differential_expression.csv", {
              ts: 1788998402000, duration_ms: 3400, ok: true,
              detail_input: JSON.stringify({ code: "import pandas as pd\ndf = pd.read_csv('results/differential_expression.csv')\nprint(df.columns.tolist())\nprint(df.isna().sum())" }, null, 2),
              detail_output: "Columns: ['gene', 'log2FoldChange', 'padj']\n\nMissing values:\ngene                0\nlog2FoldChange      3\npadj              128\n\nThe original file has not been modified.",
            }),
            cell("usage", "", { ts: 1788998405600, detail_output: null, usage: { round: 1, model: "deepseek-v4-pro", input_tokens: 12300, output_tokens: 1400, reasoning_tokens: 300, cached_input_tokens: 9225 } }),
          ] },
          { index: 2, started_at: 1788998460000, cells: [
            cell("user", "Now draw a volcano plot from this table.", { ts: 1788998460000 }),
            cell("tool", "python · plot volcano using log2fc", {
              ts: 1788998460500, duration_ms: 800, ok: false, is_error: true,
              detail_input: JSON.stringify({ code: "import numpy as np\nimport matplotlib.pyplot as plt\nplt.scatter(df['log2fc'], -np.log10(df['padj']))" }, null, 2),
              detail_output: "Traceback (most recent call last):\n  plt.scatter(df['log2fc'], -np.log10(df['padj']))\nKeyError: 'log2fc'",
            }),
            cell("assistant", "The plot failed. The table uses log2FoldChange; the plotting code needs that column name.", { ts: 1788998461500, duration_ms: 2100 }),
            cell("usage", "", { ts: 1788998464000, detail_output: null, usage: { round: 2, model: "deepseek-v4-pro", input_tokens: 15000, output_tokens: 900, reasoning_tokens: 0, cached_input_tokens: 12000 } }),
          ] },
        ],
        stats: { turns: 2, steps: 4, llm_ms: 3300, tool_ms: 4200, input_tokens: 27300, output_tokens: 2300, cached_input_tokens: 21225, cache_hit_pct: 77.75, tokens_per_sec: 12.5 },
      };
    });
    await page.getByTestId("trajectory-topbar").click();
    const view = page.getByTestId("trajectory-view");
    await expect(view).toBeVisible();
    await page.getByTestId("traj-inspector-close").click();
    await expect(view.getByText("Turn 2", { exact: true })).toBeVisible();
    await shot(page, info, "trajectory/01-overview");
    await view.getByTestId("traj-row-tool").first().click();
    await view.getByTestId("traj-tab-preview").click();
    await expect(view.getByTestId("traj-detail-output")).toContainText("128");
    await shot(page, info, "trajectory/02-tool-details");
    await view.getByPlaceholder("Search events").fill("log2fc");
    await view.getByTestId("traj-row-tool").click();
    await expect(view.getByTestId("traj-detail-output")).toContainText("KeyError: 'log2fc'");
    await shot(page, info, "trajectory/03-search-error");
    await view.getByPlaceholder("Search events").fill("");
    await view.getByTestId("traj-row-usage").first().click();
    await view.getByTestId("traj-tab-summary").click();
    await expect(view.getByTestId("traj-inspector")).toContainText("deepseek-v4-pro");
    await shot(page, info, "trajectory/04-usage");
  });

  test("extension installation screenshot uses the actual English Chromium manager", async ({}, info) => {
    // chrome://extensions is not provided by the stripped headless-shell binary.
    // Use Playwright's full Chromium in a fresh profile, without external requests.
    const profile = mkdtempSync(resolve(tmpdir(), "wisp-extension-guide-"));
    const macLauncher = resolve(profile, "chromium-en");
    if (process.platform === "darwin") {
      // Native macOS pages use AppleLanguages instead of --lang. A temporary
      // launcher supplies the array argument without changing user preferences.
      const executable = "'" + chromium.executablePath().replaceAll("'", "'\\''") + "'";
      writeFileSync(macLauncher, `#!/bin/sh\nexec ${executable} "$@" -AppleLanguages '(en-US)'\n`, { mode: 0o700 });
    }
    const context = await chromium.launchPersistentContext(profile, {
      ...(process.platform === "darwin" ? { executablePath: macLauncher } : { channel: "chromium" }),
      headless: true, args: ["--lang=en-US"],
      // Chromium also parses the AppleLanguages value as a startup target;
      // omit its default blank target because headless mode permits only one.
      ignoreDefaultArgs: process.platform === "darwin" ? ["--disable-extensions", "about:blank"] : ["--disable-extensions"],
      locale: "en-US",
      viewport: { width: 1200, height: 700 }, offline: true,
    });
    try {
      const page = context.pages()[0];
      await page.goto("chrome://extensions/");
      const developerMode = page.getByRole("button", { name: "Developer mode", exact: true });
      await developerMode.click();
      const loadUnpacked = page.getByRole("button", { name: "Load unpacked", exact: true });
      await expect(loadUnpacked).toBeInViewport({ ratio: 1 });
      // Wait for the developer-controls drawer to settle without opening a picker.
      await loadUnpacked.click({ trial: true });
      // Native extension-manager strings live in shadow roots.
      expect(await page.locator("body").ariaSnapshot()).not.toMatch(/[\p{Script=Han}]/u);
      const relative = "en/browser/00-extension-install.png";
      const path = process.env.WISP_TUTORIAL_SHOTS ? resolve(process.env.WISP_TUTORIAL_SHOTS, relative) : info.outputPath(relative);
      mkdirSync(dirname(path), { recursive: true });
      await page.locator("extensions-toolbar").screenshot({ path, animations: "disabled" });
    } finally {
      await context.close();
      rmSync(profile, { recursive: true, force: true });
    }
  });
}

});
}
