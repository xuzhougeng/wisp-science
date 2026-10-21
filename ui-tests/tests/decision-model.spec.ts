import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function openDecisionTab(page: Page, locale = "en", seed?: { settings?: any; keys?: string[] }) {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}`);
  if (seed) {
    await page.evaluate((seed) => {
      const state = (window as any).__decisionModel;
      if (seed.settings) state.settings = { ...state.settings, ...seed.settings };
      if (seed.keys) state.keys = seed.keys;
    }, seed);
  }
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
  await page.getByTestId("models-category-decision").click();
  await expect(page.getByTestId("decision-model-settings")).toBeVisible();
}

const decisionState = (page: Page) => page.evaluate(() => JSON.parse(JSON.stringify((window as any).__decisionModel)));

test("decision model tab configures a StartLux endpoint with its own key", async ({ page }, testInfo) => {
  await openDecisionTab(page);
  await expect(page.getByRole("tab")).toHaveCount(4);
  await expect(page.getByTestId("models-category-decision")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("models-category-http")).toHaveAttribute("aria-selected", "false");
  // The add-model toolbar belongs to the API model list, not this tab.
  await expect(page.getByTestId("add-provider")).toBeHidden();
  await expect(page.getByTestId("decision-status")).toContainText("Off");

  await page.getByTestId("decision-preset").selectOption("startlux_bamboo");
  await expect(page.getByTestId("decision-base-url")).toHaveValue("https://api.bamboonode.dev");
  await expect(page.getByTestId("decision-model-id")).toHaveValue("startlux-decision-27b-autothink");
  await expect(page.getByTestId("decision-timeout")).toHaveValue("60");
  await expect(page.getByTestId("decision-preset-link")).toContainText("Get a Bamboo API key");
  await page.locator(".decision-enable-row label.toggle").click();
  await expect(page.getByTestId("decision-enabled")).toBeChecked();

  // A hosted preset without a key is refused before any round trip.
  await page.getByTestId("decision-save").click();
  await expect(page.getByTestId("decision-message")).toHaveText("This service needs an API key.");
  expect((await decisionState(page)).saves).toHaveLength(0);

  await page.getByTestId("decision-think-mode").selectOption("never");
  await page.getByTestId("decision-api-key").fill("bk-test");
  await page.getByTestId("decision-test").click();
  await expect(page.getByTestId("decision-message")).toHaveText("Connected: startlux-decision-27b-autothink answered in 42 ms.");
  const tested = (await decisionState(page)).tests[0];
  expect(tested.apiKey).toBe("bk-test");
  expect(tested.settings).toMatchObject({ preset: "startlux_bamboo", think_mode: "never", enabled: true });
  expect((await decisionState(page)).saves).toHaveLength(0);

  await page.getByTestId("decision-save").click();
  await expect(page.getByTestId("decision-message")).toHaveText("Saved");
  const saved = (await decisionState(page)).saves[0];
  expect(saved.apiKey).toBe("bk-test");
  expect(saved.settings).toMatchObject({ enabled: true, preset: "startlux_bamboo", base_url: "https://api.bamboonode.dev", think_mode: "never", timeout_secs: 60 });
  await expect(page.getByTestId("decision-status")).toHaveText("In use: StartLux (hosted on Bamboo) · startlux-decision-27b-autothink");
  await expect(page.getByTestId("decision-api-key")).toHaveValue("");
  await expect(page.locator("label[for=decision-api-key]")).toContainText("Configured");
  await page.setViewportSize({ width: 1180, height: 900 });
  await page.screenshot({ path: testInfo.outputPath("decision-model.png"), animations: "disabled" });

  // Each preset keeps its own key: TypeSafe has none stored.
  await page.getByTestId("decision-preset").selectOption("typesafe");
  await expect(page.getByTestId("decision-base-url")).toHaveValue("https://api.typesafe.ai");
  await expect(page.locator("label[for=decision-api-key]")).toContainText("Not configured");
  await expect(page.getByTestId("decision-clear-key")).toHaveCount(0);
  // A self-hosted server may run without keys.
  await page.getByTestId("decision-preset").selectOption("startlux_local");
  await expect(page.locator("label[for=decision-api-key]")).toContainText("Optional");
  await page.getByTestId("decision-save").click();
  await expect(page.getByTestId("decision-status")).toContainText("StartLux (self-hosted) · http://127.0.0.1:8090");
  expect((await decisionState(page)).saves[1]).toMatchObject({ apiKey: "", settings: { preset: "startlux_local", model: "" } });
});

test("an existing TypeSafe key shows as in use and can be cleared", async ({ page }) => {
  await openDecisionTab(page, "zh", { settings: { enabled: true }, keys: ["typesafe"] });
  await expect(page.getByTestId("decision-status")).toHaveText("使用中：TypeSafe Jev · jev-latest");
  // The TypeSafe console link moved here from the Credentials page.
  await page.getByTestId("decision-preset-link").click();
  await expect.poll(() => page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === "open_external_url");
    const args = calls.at(-1)?.args;
    return args instanceof Map ? args.get("url") : args?.url;
  })).toBe("https://console.typesafe.ai");
  await expect(page.getByTestId("decision-enabled")).toBeChecked();
  await expect(page.locator("label[for=decision-api-key]")).toContainText("已配置");
  await page.getByTestId("decision-clear-key").click();
  await expect(page.getByTestId("decision-message")).toHaveText("已清除 API key");
  expect((await decisionState(page)).cleared).toEqual(["typesafe"]);
  await expect(page.getByTestId("decision-status")).toHaveText("已启用 TypeSafe Jev，但还没有保存 API key，暂不可用。");
  await expect(page.getByTestId("decision-clear-key")).toHaveCount(0);

  // A failed probe reports the server's reason without saving anything.
  await page.evaluate(() => { (window as any).__decisionModel.testError = "api: 401 invalid key"; });
  await page.getByTestId("decision-api-key").fill("wrong-key");
  await page.getByTestId("decision-test").click();
  // The shared API-error hint may follow the server's reason.
  await expect(page.getByTestId("decision-message")).toContainText("测试失败：api: 401 invalid key");
  expect((await decisionState(page)).saves).toHaveLength(0);
});
