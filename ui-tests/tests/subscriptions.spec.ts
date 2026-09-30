import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";
import { expectPrimaryButton } from "./button-style";

async function setup(page: Page, locale = "en", saved = false) {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}`);
  await page.evaluate(({ saved }) => {
    const w = window as any;
    const base = w.__TAURI__.core.invoke;
    const profile = (provider: string, index = 0) => ({ id: `${provider}-${index}`, label: provider === "xai" ? "My Grok" : "My ChatGPT", provider: provider === "xai" ? "xai_oauth" : "openai_codex", api_url: provider === "xai" ? "https://api.x.ai/v1" : "https://chatgpt.com/backend-api", model: provider === "xai" ? "grok-4.6" : "gpt-5.5", has_api_key: false, active: false, max_tokens: 4096, context_window: 128000, reasoning_effort: "", supports_vision: true, use_for_vision: false, use_for_image_generation: false, use_for_video_generation: false });
    const account = (account_id: string, active = true) => ({ account_id, email: `${account_id}@example.com`, plan_type: "plus", active });
    const now = Math.floor(Date.now() / 1000);
    const usage = (account_id: string) => ({ account_id, plan_type: "plus", limit_reached: false, primary: { used_percent: 12, window_seconds: 18000, reset_at: now + 9000 }, secondary: { used_percent: 30, window_seconds: 604800, reset_at: now + 273600 } });
    const state = w.__subscription = { accounts: { codex: saved, xai: saved } as any, phase: "pending", starts: 0, cancelled: [] as string[], startDelay: 0, saveError: false, saves: [] as any[], edits: [] as any[], profiles: saved ? [profile("codex"), profile("xai")] : [] as any[],
      pool: saved ? [account("codex-account")] : [] as any[], usage: {} as any, usageErrors: {} as any, usageCalls: [] as string[], switches: [] as string[], removals: [] as string[], local: [] as any[], importError: "" };
    const plain = (v: any): any => v instanceof Map ? Object.fromEntries([...v].map(([k, v]) => [k, plain(v)])) : v;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const a = plain(args) ?? {};
      const provider = a.provider ?? "codex";
      if (cmd === "list_models") return [...await base(cmd, args), ...state.profiles];
      if (cmd === "codex_subscription_status") return { signed_in: state.accounts[provider], account_id: state.accounts[provider] ? (provider === "codex" ? state.pool.find((p: any) => p.active)?.account_id ?? "codex-account" : `${provider}-account`) : "" };
      if (cmd === "list_codex_accounts") return state.pool;
      if (cmd === "codex_account_usage") {
        state.usageCalls.push(a.accountId);
        if (state.usageErrors[a.accountId]) throw new Error(state.usageErrors[a.accountId]);
        return state.usage[a.accountId] ?? usage(a.accountId);
      }
      if (cmd === "switch_codex_account") {
        state.switches.push(a.accountId);
        state.pool = state.pool.map((p: any) => ({ ...p, active: p.account_id === a.accountId }));
        return state.pool;
      }
      if (cmd === "remove_codex_account") {
        state.removals.push(a.accountId);
        state.pool = state.pool.filter((p: any) => p.account_id !== a.accountId);
        return state.pool;
      }
      if (cmd === "import_local_codex_accounts") {
        if (state.importError) throw new Error(state.importError);
        const fresh = state.local.filter((l: any) => !state.pool.some((p: any) => p.account_id === l.account_id));
        state.pool = [...state.pool, ...fresh].map((p: any, i: number) => ({ ...p, active: state.pool.some((q: any) => q.active) ? p.active : i === 0 }));
        state.accounts.codex = state.pool.length > 0;
        return { imported: state.local.length, accounts: state.pool };
      }
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
        if (w.__holdSubscriptionSave) await new Promise<void>(resolve => { w.__finishSubscriptionSave = resolve; });
        if (state.saveError) throw new Error("Keyring temporarily unavailable");
        state.accounts[provider] = true;
        if (provider === "codex" && state.pool.length === 0) state.pool = [account("account-test")];
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
    await expect(page.getByTestId("codex-login-save")).toHaveCount(0);
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

for (const locale of ["en", "zh"]) for (const dark of [false, true]) {
  test(`account confirmation is styled, responsive and busy-safe (${locale}, ${dark ? "dark" : "light"})`, async ({ page }, testInfo) => {
    const provider = dark ? "xai" : "codex";
    await page.emulateMedia({ colorScheme: dark ? "dark" : "light" });
    await setup(page, locale);
    await subscriptions(page);
    await page.getByTestId(`add-${provider}-login`).click();
    await expect(page.getByTestId("codex-login-save")).toHaveCount(0);
    await page.evaluate(() => {
      (window as any).__subscription.phase = "success";
      (window as any).__subscription.saveError = true;
      (window as any).__holdSubscriptionSave = true;
    });
    await page.getByTestId("codex-login-start").click();
    const save = page.getByTestId("codex-login-save");
    const footer = page.getByTestId("codex-login-form").locator(".settings-footer");
    await expect(save).toBeEnabled();
    await expect(footer.getByRole("button")).toHaveCount(3);
    await expect(footer.getByRole("button").last()).toHaveText(locale === "zh" ? "保存账号" : "Save account");

    for (const width of [1280, 760, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await expectPrimaryButton(save);
      const metrics = await footer.evaluate(el => {
        const buttons = [...el.querySelectorAll("button")];
        const last = buttons[buttons.length - 1];
        const style = getComputedStyle(last);
        const box = el.getBoundingClientRect();
        return {
          heights: buttons.map(button => button.getBoundingClientRect().height),
          rounded: parseFloat(style.borderRadius),
          primary: style.backgroundColor,
          secondary: getComputedStyle(buttons[0]).backgroundColor,
          overflow: el.scrollWidth - el.clientWidth,
          contained: buttons.every(button => {
            const rect = button.getBoundingClientRect();
            return rect.left >= box.left - 1 && rect.right <= box.right + 1;
          }),
          clipped: buttons.some(button => button.scrollWidth > button.clientWidth + 1),
        };
      });
      expect(metrics.heights.every(height => height >= 40)).toBe(true);
      expect(Math.max(...metrics.heights) - Math.min(...metrics.heights)).toBeLessThanOrEqual(1);
      expect(metrics.rounded).toBeGreaterThan(0);
      expect(metrics.primary).not.toBe(metrics.secondary);
      expect(metrics.overflow).toBeLessThanOrEqual(1);
      expect(metrics.contained).toBe(true);
      expect(metrics.clipped).toBe(false);
      await page.screenshot({ path: testInfo.outputPath(`account-confirmation-${width}.png`), animations: "disabled" });
    }

    await expectPrimaryButton(save, true);
    await save.focus();
    await page.keyboard.press("Enter");
    await expect(save).toHaveText(locale === "zh" ? "正在保存账号…" : "Saving account…");
    await expect(save).toHaveAttribute("aria-busy", "true");
    for (const button of await footer.getByRole("button").all()) await expect(button).toBeDisabled();
    await page.keyboard.press("Enter");
    expect(await page.evaluate(() => (window as any).__subscription.saves.length)).toBe(1);
    await page.screenshot({ path: testInfo.outputPath("account-confirmation-busy.png"), animations: "disabled" });
    await page.evaluate(() => (window as any).__finishSubscriptionSave());
    await expect(page.getByTestId("codex-login-message")).toContainText("Keyring temporarily unavailable");
    await expect(save).toBeEnabled();
    await expect(save).toHaveAttribute("aria-busy", "false");
    await expect(save).toHaveText(locale === "zh" ? "保存账号" : "Save account");
    await page.screenshot({ path: testInfo.outputPath("account-confirmation-error.png"), animations: "disabled" });
    await page.evaluate(() => {
      (window as any).__holdSubscriptionSave = false;
      (window as any).__subscription.saveError = false;
    });
    await save.click();
    await expect(page.getByTestId("subscriptions-page")).toBeVisible();
    expect(await page.evaluate(() => (window as any).__subscription.saves.map((s: any) => s.loginId))).toEqual(["login-1", "login-1"]);
  });
}

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
  await expectPrimaryButton(page.getByTestId("codex-submit-redirect"));
  await page.getByTestId("codex-submit-redirect").click();
  await expect(page.getByTestId("codex-login-save")).toBeEnabled();
  await expect(page.locator(".subscription-manual")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("subscriptions-page")).toBeVisible();
});

async function seedPool(page: Page) {
  await page.evaluate(() => {
    const state = (window as any).__subscription;
    const now = Math.floor(Date.now() / 1000);
    state.pool = [
      { account_id: "acct-a", email: "alice@example.com", plan_type: "plus", active: true },
      { account_id: "acct-b", email: "", plan_type: "pro", active: false },
    ];
    state.usage = {
      "acct-a": { account_id: "acct-a", plan_type: "plus", limit_reached: false, primary: { used_percent: 42.4, window_seconds: 18000, reset_at: now + 9000 }, secondary: { used_percent: 85, window_seconds: 604800, reset_at: now + 273600 } },
      "acct-b": { account_id: "acct-b", plan_type: "pro", limit_reached: true, primary: { used_percent: 100, window_seconds: 18000, reset_at: now + 9000 }, secondary: null },
    };
  });
}

for (const locale of ["en", "zh"]) {
  test(`ChatGPT accounts show quota, switch, and remove behind a confirmation (${locale})`, async ({ page }, testInfo) => {
    const zh = locale === "zh";
    await setup(page, locale, true);
    await seedPool(page);
    await subscriptions(page);
    const panel = page.getByTestId("codex-accounts");
    const rows = panel.getByTestId("codex-account");
    const a = panel.locator('[data-account-id="acct-a"]');
    const b = panel.locator('[data-account-id="acct-b"]');
    await expect(rows).toHaveCount(2);
    await expect(panel.locator("h4")).toHaveText(zh ? "账号（2）" : "Accounts (2)");
    await expect(page.getByTestId("add-codex-login")).toHaveText(zh ? "添加账号" : "Add account");
    await expect(a).toContainText("alice@example.com");
    await expect(a).toContainText("Plus");
    await expect(b).toContainText("acct-b");
    await expect(b).toContainText("Pro");
    await expect(a.getByTestId("codex-account-active")).toHaveText(zh ? "当前账号" : "Active account");
    await expect(b.getByTestId("codex-account-active")).toHaveCount(0);
    await expect(a.getByTestId("codex-account-use")).toHaveCount(0);

    const meters = a.getByTestId("codex-usage-meter");
    await expect(meters).toHaveCount(2);
    await expect(meters.nth(0).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "42");
    await expect(meters.nth(0)).toContainText(zh ? "5 小时额度" : "5-hour limit");
    await expect(meters.nth(0)).toContainText(zh ? "已用 42% · 2小时30分钟后重置" : "42% used · resets in 2h 30m");
    await expect(meters.nth(0)).not.toHaveClass(/warn/);
    await expect(meters.nth(1)).toContainText(zh ? "7 天额度" : "7-day limit");
    await expect(meters.nth(1)).toContainText(zh ? "3天4小时后重置" : "resets in 3d 4h");
    await expect(meters.nth(1)).toHaveClass(/warn/);
    const fills = await panel.locator(".codex-usage-bar > span").evaluateAll(spans => spans.map(span => getComputedStyle(span).backgroundColor));
    expect(fills.every(fill => fill !== "rgba(0, 0, 0, 0)")).toBe(true);
    expect(new Set(fills).size).toBe(3);
    await expect(b.getByTestId("codex-limit-reached")).toHaveText(zh ? "已达上限" : "Limit reached");
    await expect(b.getByTestId("codex-usage-meter")).toHaveCount(1);
    await expect(b.getByTestId("codex-usage-meter")).toHaveClass(/full/);
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.screenshot({ path: testInfo.outputPath(`codex-accounts-${locale}.png`), animations: "disabled" });

    await b.getByTestId("codex-account-use").click();
    await expect(b.getByTestId("codex-account-active")).toBeVisible();
    await expect(a.getByTestId("codex-account-active")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__subscription.switches)).toEqual(["acct-b"]);

    await a.getByTestId("codex-account-remove").click();
    const confirm = page.getByTestId("codex-account-remove-confirm");
    await expect(confirm).toContainText("alice@example.com");
    await page.keyboard.press("Escape");
    await expect(confirm).toHaveCount(0);
    await expect(page.getByTestId("subscriptions-page")).toBeVisible();
    await expect(rows).toHaveCount(2);

    await a.getByTestId("codex-account-remove").click();
    await confirm.getByRole("button", { name: zh ? "移除账号" : "Remove account" }).click();
    await expect(rows).toHaveCount(1);
    await expect(panel.locator("h4")).toHaveText(zh ? "账号（1）" : "Accounts (1)");
    expect(await page.evaluate(() => (window as any).__subscription.removals)).toEqual(["acct-a"]);
  });
}

test("a failed usage request is shown per account and refresh retries it", async ({ page }) => {
  await setup(page, "en", true);
  await seedPool(page);
  await page.evaluate(() => (window as any).__subscription.usageErrors["acct-b"] = "ChatGPT usage request failed (HTTP 401)");
  await subscriptions(page);
  const b = page.locator('[data-account-id="acct-b"]');
  await expect(b.getByTestId("codex-usage-error")).toContainText("HTTP 401");
  await expect(page.locator('[data-account-id="acct-a"]').getByTestId("codex-usage")).toBeVisible();
  await page.evaluate(() => (window as any).__subscription.usageErrors = {});
  await page.getByTestId("codex-usage-refresh").click();
  await expect(b.getByTestId("codex-usage-error")).toHaveCount(0);
  await expect(b.getByTestId("codex-limit-reached")).toBeVisible();
  const calls = await page.evaluate(() => (window as any).__subscription.usageCalls);
  expect(calls.filter((id: string) => id === "acct-b")).toHaveLength(2);
});

for (const locale of ["en", "zh"]) {
  test(`importing local ChatGPT sign-ins reports failures and adds accounts (${locale})`, async ({ page }) => {
    const zh = locale === "zh";
    await setup(page, locale);
    await subscriptions(page);
    const importButton = page.getByTestId("import-codex-local");
    await expect(importButton).toHaveText(zh ? "导入本地登录" : "Import local sign-in");
    await expect(importButton).toHaveAttribute("title", /\.codex\/auth\.json/);
    await expect(page.getByTestId("codex-account")).toHaveCount(0);
    await expect(page.getByTestId("add-codex-model")).toBeDisabled();

    await page.evaluate(() => (window as any).__subscription.importError = "No local ChatGPT sign-in found. Run `codex login` first.");
    await importButton.click();
    const message = page.getByTestId("codex-accounts-message");
    await expect(message).toHaveClass(/fail/);
    await expect(message).toContainText("codex login");
    await expect(page.getByTestId("codex-account")).toHaveCount(0);

    await page.evaluate(() => {
      const state = (window as any).__subscription;
      state.importError = "";
      state.local = [
        { account_id: "cli-account", email: "cli@example.com", plan_type: "plus", active: false },
        { account_id: "proxy-account", email: "proxy@example.com", plan_type: "team", active: false },
      ];
    });
    await importButton.click();
    await expect(message).toHaveClass(/ok/);
    await expect(message).toContainText(zh ? "已导入 2 个 ChatGPT 账号" : "Imported 2 ChatGPT account(s)");
    await expect(page.getByTestId("codex-account")).toHaveCount(2);
    await expect(page.locator('[data-account-id="cli-account"]').getByTestId("codex-account-active")).toBeVisible();
    await expect(page.locator('[data-account-id="proxy-account"]').getByTestId("codex-account-use")).toBeVisible();
    await expect(page.getByTestId("codex-usage")).toHaveCount(2);
    await expect(page.getByTestId("add-codex-model")).toBeEnabled();
    await expect(page.getByTestId("add-codex-login")).toHaveText(zh ? "添加账号" : "Add account");

    await page.getByTestId("add-codex-login").click();
    await expect(page.getByTestId("codex-saved-account")).toContainText(zh ? "当前账号 · cli-account" : "Active account · cli-account");
    await expect(page.getByTestId("codex-add-account-hint")).toBeVisible();
    await expect(page.getByTestId("codex-login-start")).toHaveText(zh ? "登录" : "Sign in");
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("subscriptions-page")).toBeVisible();
  });
}

test("subscription account cards fit narrow windows", async ({ page }) => {
  await setup(page, "zh", true);
  await seedPool(page);
  await page.evaluate(() => (window as any).__subscription.pool[0].email = "a-very-long-research-group-address@university.example.edu");
  await subscriptions(page);
  await expect(page.getByTestId("codex-usage-meter")).toHaveCount(3);
  for (const width of [760, 390]) {
    await page.setViewportSize({ width, height: 820 });
    const overflow = await page.getByTestId("subscriptions-page").evaluate(el => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    const contained = await page.getByTestId("codex-accounts").evaluate(el => {
      const box = el.getBoundingClientRect();
      return [...el.querySelectorAll("button, .codex-usage-meter")].every(node => {
        const rect = node.getBoundingClientRect();
        return rect.left >= box.left - 1 && rect.right <= box.right + 1;
      });
    });
    expect(contained).toBe(true);
    await expect(page.getByTestId("import-codex-local")).toBeVisible();
    await expect(page.getByTestId("add-xai-model")).toBeVisible();
  }
});
