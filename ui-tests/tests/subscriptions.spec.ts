import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

async function setup(page: Page, locale = "en", saved = false) {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}`);
  await page.evaluate(({ saved }) => {
    const w = window as any;
    const base = w.__TAURI__.core.invoke;
    const profile = (provider: string, index = 0) => ({ id: `${provider}-${index}`, label: provider === "xai" ? "My Grok" : "My ChatGPT", provider: provider === "xai" ? "xai_oauth" : "openai_codex", api_url: provider === "xai" ? "https://api.x.ai/v1" : "https://chatgpt.com/backend-api", model: provider === "xai" ? "grok-4.6" : "gpt-5.5", has_api_key: false, active: false, max_tokens: 4096, context_window: 128000, reasoning_effort: "", supports_vision: true, use_for_vision: false, use_for_image_generation: false, use_for_video_generation: false });
    const state = w.__subscription = { accounts: { codex: saved, xai: saved } as any, phase: "pending", starts: 0, cancelled: [] as string[], startDelay: 0, saveError: false, saves: [] as any[], edits: [] as any[], profiles: saved ? [profile("codex"), profile("xai")] : [] as any[] };
    const plain = (v: any): any => v instanceof Map ? Object.fromEntries([...v].map(([k, v]) => [k, plain(v)])) : v;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const a = plain(args) ?? {};
      const provider = a.provider ?? "codex";
      if (cmd === "list_models") return [...await base(cmd, args), ...state.profiles];
      if (cmd === "codex_subscription_status") return { signed_in: state.accounts[provider], account_id: state.accounts[provider] ? `${provider}-account` : "" };
      if (cmd === "start_codex_login") {
        const id = `login-${++state.starts}`;
        await new Promise(r => setTimeout(r, state.startDelay));
        return { login_id: id, method: a.method, url: provider === "xai" ? "https://accounts.x.ai/device?code=TEST-1234" : "https://auth.openai.com/codex/device", verification_uri: "https://auth.openai.com/codex/device", user_code: a.method === "device" ? "TEST-1234" : "", message: "" };
      }
      if (cmd === "submit_codex_login_redirect") { state.phase = "success"; return { status: "success", message: "", account_id: "account-test" }; }
      if (cmd === "codex_login_status") return { status: state.phase, message: state.phase === "error" ? "ChatGPT device authorization failed (HTTP 403). Check Wisp network/proxy settings." : "", account_id: state.phase === "success" ? "account-test" : "" };
      if (cmd === "cancel_codex_login") { state.cancelled.push(a.loginId); return null; }
      if (cmd === "save_codex_login") {
        state.saves.push(a);
        if (state.saveError) throw new Error("Keyring temporarily unavailable");
        state.accounts[provider] = true;
        if (!a.accountOnly) state.profiles.push({ ...profile(provider, state.profiles.length), model: a.model, label: a.label || "My model" });
        return [...await base("list_models", {}), ...state.profiles];
      }
      if (cmd === "save_model" && state.profiles.some((p: any) => p.id === a.profile.id)) {
        state.edits.push(a.profile);
        state.profiles = state.profiles.map((p: any) => p.id === a.profile.id ? { ...p, ...a.profile } : p);
        return [...await base("list_models", {}), ...state.profiles];
      }
      if (cmd === "set_active_model") {
        state.profiles = state.profiles.map((p: any) => ({ ...p, active: p.id === a.id }));
        return [...await base(cmd, args), ...state.profiles];
      }
      if (cmd === "remove_model") {
        state.profiles = state.profiles.filter((p: any) => p.id !== a.id);
        return [...await base(cmd, args), ...state.profiles];
      }
      return base(cmd, args);
    };
  }, { saved });
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
}
async function subscriptions(page: Page) { await page.getByTestId("models-category-subscriptions").click(); }

for (const locale of ["en", "zh"]) {
  test(`subscription accounts share Models tabs, with only ID and alias editable (${locale})`, async ({ page }, testInfo) => {
    await setup(page, locale, true);
    await expect(page.getByTestId("settings-nav-subscriptions")).toHaveCount(0);
    await expect(page.getByRole("tab")).toHaveCount(3);
    await expect(page.getByTestId("models-category-http")).toContainText("(2)");
    await expect(page.getByTestId("add-codex-login")).toHaveCount(0);
    await expect(page.getByText("My ChatGPT", { exact: true })).toHaveCount(0);
    await expect(page.getByText("My Grok", { exact: true })).toHaveCount(0);
    await page.locator(".settings-list-row").first().click();
    await expect(page.getByTestId("settings-provider").locator("option")).toHaveCount(3);
    await page.keyboard.press("Escape");
    await subscriptions(page);
    await expect(page.getByTestId("models-category-subscriptions")).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("subscription-model")).toHaveCount(2);
    await expect(page.getByTestId("model-presets")).toHaveCount(0);
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.screenshot({ path: testInfo.outputPath(`subscriptions-${locale}.png`), animations: "disabled" });
    for (const provider of ["codex", "xai"]) {
      await page.getByTestId(`subscription-account-${provider}`).locator(".subscription-model-edit").click();
      const form = page.getByTestId("subscription-model-form");
      await expect(form.locator("input")).toHaveCount(2);
      await expect(form.locator("select")).toHaveCount(0);
      await expect(page.getByTestId("settings-provider")).toHaveCount(0);
      await expect(page.locator("#model-form-api-key")).toHaveCount(0);
      await page.getByTestId("subscription-model-label").fill(`Renamed ${provider}`);
      await page.getByTestId("save-subscription-model").click();
      await expect(page.getByTestId(`subscription-account-${provider}`)).toContainText(`Renamed ${provider}`);
    }
    const edits = await page.evaluate(() => (window as any).__subscription.edits);
    expect(edits.map((p: any) => p.provider)).toEqual(["openai_codex", "xai_oauth"]);
    expect(edits.every((p: any) => p.max_tokens === 4096 && p.context_window === 128000)).toBe(true);
    await page.getByTestId("add-codex-model").click();
    await expect(page.getByTestId("codex-login-form").locator("input")).toHaveCount(2);
    await expect(page.getByTestId("codex-login-start")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`subscription-model-${locale}.png`), animations: "disabled" });
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("subscriptions-page")).toBeVisible();
    await page.getByTestId("open-acp-agents-from-settings").click();
    await expect(page.getByTestId("acp-agents-settings")).toBeVisible();
    await expect(page.getByTestId("subscriptions-page")).toHaveCount(0);
  });
}

for (const provider of ["codex", "xai"]) {
  test(`${provider} sign-in saves an account without creating a model, then adds multiple aliases`, async ({ page }) => {
    await setup(page);
    await subscriptions(page);
    await expect(page.getByTestId(`add-${provider}-model`)).toBeDisabled();
    await page.getByTestId(`add-${provider}-login`).click();
    await expect(page.getByTestId("codex-login-model")).toHaveCount(0);
    if (provider === "codex") await page.getByTestId("codex-login-method").selectOption("device");
    else await expect(page.getByTestId("codex-login-method")).toBeHidden();
    await page.getByTestId("codex-login-start").click();
    await expect(page.getByTestId("codex-user-code")).toContainText("TEST-1234");
    await expect(page.getByTestId("codex-login-message")).not.toHaveClass(/fail/);
    await page.evaluate(() => (window as any).__subscription.phase = "success");
    await page.getByTestId("codex-login-save").click();
    await expect(page.getByTestId("subscription-model")).toHaveCount(0);
    for (const alias of ["Research", "Writing"]) {
      await page.getByTestId(`add-${provider}-model`).click();
      await expect(page.getByTestId("codex-login-form").locator("input")).toHaveCount(2);
      await expect(page.getByTestId("codex-login-model")).toHaveValue(provider === "xai" ? "grok-4.6" : "gpt-5.5");
      await page.getByTestId("codex-login-label").fill(alias);
      await page.getByTestId("codex-login-save").click();
      await expect(page.getByTestId(`subscription-account-${provider}`)).toContainText(alias);
    }
    await expect(page.getByTestId("subscription-model")).toHaveCount(2);
    expect(await page.evaluate(() => (window as any).__subscription.starts)).toBe(1);
    const saves = await page.evaluate(() => (window as any).__subscription.saves);
    expect(saves[0]).toMatchObject({ provider, accountOnly: true, loginId: "login-1", useSaved: false });
    expect(saves.slice(1)).toEqual(expect.arrayContaining([expect.objectContaining({ accountOnly: false, useSaved: true, provider })]));
  });
}

test("leaving before the challenge arrives cancels it without reopening sign-in", async ({ page }) => {
  await setup(page);
  await page.evaluate(() => (window as any).__subscription.startDelay = 700);
  await subscriptions(page);
  await page.getByTestId("add-codex-login").click();
  await page.getByTestId("codex-login-method").selectOption("device");
  await page.getByTestId("codex-login-start").click();
  await page.getByTestId("settings-nav-general").click();
  await expect.poll(() => page.evaluate(() => (window as any).__subscription.cancelled)).toEqual(["login-1"]);
  await expect(page.getByTestId("codex-login-form")).toHaveCount(0);
  await page.getByTestId("settings-nav-models").click();
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
});

test("device error can retry and Escape cancels only sign-in", async ({ page }) => {
  await setup(page);
  await subscriptions(page);
  await page.getByTestId("add-codex-login").click();
  await page.getByTestId("codex-login-method").selectOption("device");
  await page.evaluate(() => (window as any).__subscription.phase = "error");
  await page.getByTestId("codex-login-start").click();
  await expect(page.getByTestId("codex-login-message")).toHaveClass(/fail/);
  await expect(page.getByTestId("codex-login-message")).toContainText("HTTP 403");
  await page.evaluate(() => (window as any).__subscription.phase = "pending");
  await page.getByTestId("codex-login-start").click();
  await expect(page.getByTestId("codex-user-code")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__subscription.cancelled)).toContain("login-2");
  await expect(page.locator(".settings-page")).toBeVisible();
});

test("a save failure keeps authorization available for retry", async ({ page }) => {
  await setup(page);
  await subscriptions(page);
  await page.getByTestId("add-codex-login").click();
  await page.getByTestId("codex-login-method").selectOption("device");
  await page.evaluate(() => { (window as any).__subscription.phase = "success"; (window as any).__subscription.saveError = true; });
  await page.getByTestId("codex-login-start").click();
  await page.getByTestId("codex-login-save").click();
  await expect(page.getByTestId("codex-login-message")).toContainText("Keyring temporarily unavailable");
  await expect(page.getByTestId("codex-login-save")).toBeEnabled();
  await page.evaluate(() => (window as any).__subscription.saveError = false);
  await page.getByTestId("codex-login-save").click();
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
  expect(await page.evaluate(() => (window as any).__subscription.saves.map((s: any) => s.loginId))).toEqual(["login-1", "login-1"]);
});

test("browser callback remains mounted while typing; immediate Escape closes model form", async ({ page }) => {
  await setup(page, "en", true);
  await subscriptions(page);
  await page.getByTestId("add-codex-model").click();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
  await page.getByTestId("add-codex-login").click();
  await expect(page.locator(".subscription-manual")).toHaveCount(0);
  await page.getByTestId("codex-login-start").click();
  await page.locator(".subscription-manual summary").click();
  await page.getByTestId("codex-login-redirect").fill("http://localhost:1455/auth/callback?code=fake&state=test");
  await expect(page.getByTestId("codex-submit-redirect")).toBeVisible();
  await page.getByTestId("codex-submit-redirect").click();
  await expect(page.getByTestId("codex-login-save")).toBeEnabled();
  await expect(page.locator(".subscription-manual")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
});


test("subscription account cards fit narrow windows", async ({ page }) => {
  await setup(page, "zh", true);
  await subscriptions(page);
  for (const width of [760, 390]) {
    await page.setViewportSize({ width, height: 820 });
    const overflow = await page.getByTestId("subscriptions-page").evaluate(el => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    await expect(page.getByTestId("add-xai-model")).toBeVisible();
  }
});
