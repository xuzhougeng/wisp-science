import { test, expect } from "@playwright/test";
import path from "node:path";
import { tauriMock } from "./mock-tauri";

// Same IDs, labels, preferences and saved-account state as the native settings
// render fixture. Rendering the shared document catches clipped labels without
// depending on credentials, model APIs or a running desktop host.
function settingsFixture({ locale, dark }: { locale: string; dark: boolean }) {
  const w = window as any;
  const invoke = w.__TAURI__.core.invoke;
  w.__mockAppearancePrefs = {
    saved: true, theme: dark ? "dark" : "light", light_palette: "paper", dark_palette: "charcoal",
    ui_font_size: 14, code_font_size: 12, ui_font_family: "", code_font_family: "",
    send_with_modifier: true, selection_popup_enabled: true, custom_css: "",
  };
  const canvas = document.createElement("canvas");
  canvas.width = 1536; canvas.height = 2288;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "rgb(31,153,140)"; ctx.fillRect(32, 40, 128, 128);
  const sprite = canvas.toDataURL();
  w.__TAURI__.core.invoke = async (command: string, args: any) => {
    const values = args instanceof Map ? Object.fromEntries(args) : args ?? {};
    if (command === "get_settings") return { ...await invoke(command, args), locale,
      resume_last_session: true, notifications_enabled: true, pet_enabled: true,
      pet_directory: "/fixture/pets/research-companion", workspace_dir: "/fixture/research/workspace" };
    if (command === "list_models") {
      const base = (await invoke(command, args))[0];
      return [
        { ...base, id: "api-1", label: "科研项目的长名称模型配置 · Research workspace primary model", provider: "openai_responses", model: "vendor/research-model-with-a-long-exact-id", active: true, supports_vision: true },
        { ...base, id: "api-2", label: "离线验收 Secondary model", provider: "anthropic", model: "research-secondary", active: false, supports_vision: false, use_for_vision: false },
        { ...base, id: "sub-1", label: "订阅模型 Subscription research model", provider: "openai_codex", model: "subscription-model-exact-id", active: false },
      ];
    }
    if (command === "list_acp_agents") return [];
    if (command === "codex_subscription_status") return { signed_in: values.provider === "codex", account_id: values.provider === "codex" ? "synthetic-account-1" : "" };
    if (command === "get_pet") return { enabled: true, directory: "/fixture/pets/research-companion", error: null,
      asset: { id: "research-companion", displayName: "研究伙伴 Research companion", description: "Synthetic settings fixture · saved pet metadata", spriteVersionNumber: 2, spritesheetDataUrl: sprite,
        frameCounts: { idle: 1, "running-right": 1, "running-left": 1, waving: 1, jumping: 1, failed: 1, waiting: 1, running: 1, review: 1 } } };
    return invoke(command, args);
  };
}

for (const locale of ["zh", "en"]) for (const dark of [false, true]) for (const width of [1100, 680]) {
  test(`H1–H2 shared settings fit ${locale} ${dark ? "dark" : "light"} ${width}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 760 });
    await page.emulateMedia({ colorScheme: dark ? "dark" : "light" });
    await page.addInitScript({ content: `(${tauriMock.toString()})();(${settingsFixture.toString()})(${JSON.stringify({ locale, dark })});` });
    await page.goto(`/?mockLocale=${locale}`);
    await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
    for (const section of ["general", "appearance", "pet", "models", "subscriptions"]) {
      if (section === "subscriptions") await page.getByTestId("models-category-subscriptions").click();
      else await page.getByTestId(`settings-nav-${section}`).click();
      const pane = page.locator(".settings-page");
      // Visibility assertions alone allow opacity:0. This specifically guards
      // the reported desktop observation of a mounted but invisible Settings.
      await expect.poll(() => pane.evaluate(el => getComputedStyle(el).opacity)).toBe("1");
      await expect.poll(() => page.locator(".settings-content").evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
      if (section === "general") await expect(page.getByTestId("send-shortcut")).toHaveValue("modifier_enter");
      if (section === "models") {
        await expect(pane).toContainText("vendor/research-model-with-a-long-exact-id");
        await expect(pane.locator(".settings-model-default")).toHaveCount(1);
      }
      if (section === "subscriptions") {
        await expect(page.getByTestId("subscription-account-codex").getByRole("status")).toHaveText(locale === "zh" ? "已保存账号" : "Account saved");
        await expect(page.getByTestId("add-codex-model")).toBeEnabled();
        await expect(page.getByTestId("add-xai-model")).toBeDisabled();
      }
      const name = `${section}-${locale}-${dark ? "dark" : "light"}-${width === 680 ? "narrow" : "wide"}.png`;
      await page.screenshot({ path: process.env.WISP_WEB_SETTINGS_SNAPSHOTS ? path.join(process.env.WISP_WEB_SETTINGS_SNAPSHOTS, name) : testInfo.outputPath(name), animations: "disabled" });
    }
  });
}
