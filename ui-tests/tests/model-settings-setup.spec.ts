import { expect, test, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

// Settings › Models as a first-time user meets it: the fresh placeholder, the
// "Add API access" form, a keyless local gateway such as Magpie on
// 127.0.0.1:3425, and the facts that gateway's GET /v1/models reports.

test.beforeEach(async ({ page }) => {
  await page.addInitScript(tauriMock);
});

async function openModels(page: Page, query = "", locale = "en") {
  const params = new URLSearchParams(query);
  if (locale !== "en") params.set("mockLocale", locale);
  const search = params.toString();
  await page.goto(search ? `/?${search}` : "/");
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.getByTestId("settings-nav-models").click();
}

function rows(page: Page) {
  return page.getByTestId("model-profile-row");
}

function invokes(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    return ((window as any).__skillInvokeLog ?? [])
      .filter((call: any) => call.cmd === name)
      .map((call: any) => plain(call.args));
  }, cmd);
}

async function openAdvanced(page: Page) {
  await page.getByTestId("model-advanced-section").locator(":scope > summary").click();
}

test("adding API access keeps the working default", async ({ page }) => {
  await openModels(page);
  await expect(rows(page).first().locator(".settings-model-default")).toBeVisible();
  await page.getByTestId("add-provider").click();
  // The DeepSeek suggestion already has a stored key in the mock list.
  await page.getByTestId("save-provider").click();
  await expect(page.getByTestId("provider-add-form")).toBeHidden();
  await expect(rows(page)).toHaveCount(4);
  // Only the original profile is still the default.
  await expect(page.locator(".model-settings-pane .settings-model-default")).toHaveCount(1);
  await expect(rows(page).first().locator(".settings-model-default")).toBeVisible();
  const active = await page.evaluate(async () =>
    ((await (window as any).__TAURI__.core.invoke("list_models")) as any[]).find((m) => m.active)?.id);
  expect(active).toBe("default");
  // The first chat model is saved first, so on a fresh install it is the one
  // that becomes the default.
  expect((await invokes(page, "save_model")).map((call: any) => call.profile.model))
    .toEqual(["deepseek-v4-flash", "deepseek-v4-pro"]);
});

test("a fresh install shows an unconfigured placeholder that the first model replaces", async ({ page }) => {
  await openModels(page, "mockFreshInstall=1");
  const placeholder = rows(page).first();
  await expect(placeholder.getByTestId("model-not-configured")).toHaveText("Not configured");
  await expect(placeholder.getByTestId("model-not-configured-hint")).toContainText("Base URL, model ID and API key");
  await expect(placeholder.locator(".settings-list-untitled")).toHaveText("Unnamed model");
  await expect(placeholder.locator(".settings-model-default")).toHaveCount(0);
  // Editing it starts from an empty name with a hint, not "Default".
  await placeholder.click();
  const name = page.getByRole("textbox", { name: "Display name", exact: true });
  await expect(name).toHaveValue("");
  await expect(name).toHaveAttribute("placeholder", "Optional; blank shows the model ID");
  await page.locator(".settings-footer").getByRole("button", { name: "Cancel", exact: true }).click();

  await page.getByTestId("add-provider").click();
  await page.getByTestId("provider-api-key").fill("sk-test");
  await page.getByTestId("save-provider").click();
  await expect(page.getByTestId("provider-add-form")).toBeHidden();
  const flash = rows(page).filter({ hasText: "deepseek-v4-flash" });
  await expect(flash.locator(".settings-model-default")).toBeVisible();
  await expect(page.locator(".model-settings-pane .settings-model-default")).toHaveCount(1);
});

test("an untouched placeholder name falls back to the model id everywhere", async ({ page }) => {
  await openModels(page, "mockLegacyPlaceholder=1");
  const row = rows(page).first();
  await expect(row.locator(".settings-list-title")).toContainText("deepseek/deepseek-flash");
  await expect(row.locator(".settings-list-title")).not.toContainText("Default");
  await page.locator(".settings-nav").getByRole("button", { name: "Back to app" }).click();
  await page.locator(".proj-card-main").first().click();
  const trigger = page.getByTestId("composer-model-trigger");
  await expect(trigger).toHaveText("deepseek/deepseek-flash");
  await trigger.click();
  await expect(page.locator(".model-menu .model-menu-label").first()).toHaveText("deepseek/deepseek-flash");
});

test("list rows name the API address so equal model ids stay distinguishable", async ({ page }) => {
  await openModels(page, "mockLegacyPlaceholder=1");
  await expect(rows(page).first().getByTestId("model-profile-sub")).toHaveText("127.0.0.1:3425");
  await expect(rows(page).filter({ hasText: "opus-4.8" }).getByTestId("model-profile-sub"))
    .toHaveText("api.anthropic.com");
});

test("the edit form keeps the essentials visible and collapses the rest", async ({ page }) => {
  await openModels(page);
  await rows(page).first().click();
  for (const label of ["Base URL", "API key (stored in OS keyring)", "Model ID", "Display name"]) {
    await expect(page.getByLabel(label, { exact: true })).toBeVisible();
  }
  await expect(page.getByTestId("settings-provider")).toBeVisible();
  await expect(page.getByLabel("Base URL", { exact: true }))
    .toHaveAttribute("placeholder", "e.g. https://api.deepseek.com or http://127.0.0.1:3425/v1");
  await expect(page.getByTestId("model-api-url-hint")).toContainText("with or without /v1");
  const advanced = page.getByTestId("model-advanced-section");
  await expect(advanced).not.toHaveAttribute("open", "");
  for (const field of ["Max output tokens", "Context window (tokens)", "Endpoint suffix (optional)"]) {
    await expect(page.getByLabel(field, { exact: true })).toBeHidden();
  }
  await expect(page.getByTestId("model-proxy-mode")).toBeHidden();
  await openAdvanced(page);
  await expect(page.getByLabel("Max output tokens", { exact: true })).toBeVisible();
  await expect(page.getByTestId("model-endpoint-suffix")).toBeVisible();
  await expect(page.getByTestId("model-proxy-mode")).toBeVisible();
  await expect(page.getByTestId("use-for-expert")).toBeVisible();
});

test("Save marks every missing field at once and focuses the first", async ({ page }) => {
  await openModels(page, "mockFreshInstall=1");
  await rows(page).first().click();
  await page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true }).click();
  const status = page.locator(".settings-status.fail");
  await expect(status).toHaveText("Fill in or fix the highlighted fields: Base URL, API key, Model ID.");
  const url = page.locator("#model-form-api-url");
  await expect(url).toBeFocused();
  for (const id of ["#model-form-api-url", "#model-form-api-key", "#model-form-model"]) {
    await expect(page.locator(id)).toHaveAttribute("aria-invalid", "true");
    await expect(page.locator(id)).toHaveClass(/invalid/);
  }
  expect(await invokes(page, "save_model")).toEqual([]);
  // Typing clears that field's mark only.
  await url.fill("https://api.deepseek.com");
  await expect(url).not.toHaveAttribute("aria-invalid", "true");
  await expect(page.locator("#model-form-model")).toHaveAttribute("aria-invalid", "true");
  // Test connection reports the same way.
  await page.getByRole("button", { name: "Test connection", exact: true }).click();
  await expect(status).toHaveText("Fill in or fix the highlighted fields: API key, Model ID.");
  await expect(page.locator("#model-form-api-key")).toBeFocused();
  expect(await invokes(page, "validate_settings")).toEqual([]);
});

test("an invalid endpoint suffix opens Advanced and is marked", async ({ page }) => {
  await openModels(page);
  await rows(page).first().click();
  await openAdvanced(page);
  await page.getByTestId("model-endpoint-suffix").fill("坏网关");
  await page.locator(".model-advanced-section > summary").click();
  await page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator(".settings-status.fail")).toContainText("must be a URL path such as /anthropic");
  await expect(page.getByTestId("model-advanced-section")).toHaveAttribute("open", "");
  await expect(page.getByTestId("model-endpoint-suffix")).toBeFocused();
  await expect(page.getByTestId("model-endpoint-suffix")).toHaveAttribute("aria-invalid", "true");
  expect(await invokes(page, "save_model")).toEqual([]);
});

test("a local gateway saves without an API key", async ({ page }) => {
  await openModels(page, "mockFreshInstall=1");
  await rows(page).first().click();
  await page.getByLabel("Base URL", { exact: true }).fill("http://127.0.0.1:3425/v1");
  await expect(page.getByTestId("model-api-key-local-hint")).toContainText("may stay empty");
  await page.getByRole("textbox", { name: "Model ID", exact: true }).fill("deepseek/deepseek-flash");
  await page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(async () => (await invokes(page, "save_model")).at(-1) ?? null).toMatchObject({
    key: null,
    profile: { id: "default", api_url: "http://127.0.0.1:3425/v1", model: "deepseek/deepseek-flash" },
  });
  await page.getByLabel("Base URL", { exact: true }).waitFor({ state: "detached" });
});

test("Fetch models suggests ids and prefills the gateway's facts for an uncatalogued model", async ({ page }) => {
  await openModels(page, "mockLegacyPlaceholder=1");
  await rows(page).first().click();
  const model = page.getByRole("textbox", { name: "Model ID", exact: true });
  await expect(model).toHaveValue("deepseek/deepseek-flash");
  await page.getByTestId("model-fetch-models").click();
  await expect(page.getByTestId("model-fetch-status")).toContainText("2 models found");
  await expect(page.getByTestId("model-fetch-status")).toContainText("deepseek/deepseek-flash");
  await expect.poll(async () => (await invokes(page, "list_endpoint_models")).at(-1) ?? null).toMatchObject({
    provider: "openai",
    apiUrl: "http://127.0.0.1:3425/v1",
    profileId: "default",
  });
  await expect(page.locator("#model-id-suggestions option")).toHaveCount(2);
  await expect(page.locator("#model-form-model")).toHaveAttribute("list", "model-id-suggestions");
  await openAdvanced(page);
  await expect(page.getByLabel("Context window (tokens)", { exact: true })).toHaveValue("1000000");
  await expect(page.getByLabel("Max output tokens", { exact: true })).toHaveValue("384000");
  const effort = page.getByRole("combobox", { name: "Reasoning effort", exact: true });
  await expect(effort.locator("option:not([disabled])")).toHaveText(["Default (provider)", "None", "Low", "High", "Max"]);
  await expect(page.locator("#model-reasoning-hint")).toContainText("this endpoint's model list");
  await effort.selectOption("max");
  await page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(async () => (await invokes(page, "save_model")).at(-1) ?? null).toMatchObject({
    profile: {
      context_window: 1000000,
      max_tokens: 384000,
      reasoning_effort: "max",
      endpoint_efforts: ["none", "low", "high", "max"],
    },
  });
});

test("fetched facts never overwrite a limit the user typed", async ({ page }) => {
  await openModels(page, "mockLegacyPlaceholder=1");
  await rows(page).first().click();
  await openAdvanced(page);
  await page.getByLabel("Max output tokens", { exact: true }).fill("4096");
  await page.getByTestId("model-fetch-models").click();
  await expect(page.getByTestId("model-fetch-status")).toContainText("2 models found");
  await expect(page.getByLabel("Max output tokens", { exact: true })).toHaveValue("4096");
  await expect(page.getByLabel("Context window (tokens)", { exact: true })).toHaveValue("1000000");
});

test("a failed model-list fetch says why without blocking the form", async ({ page }) => {
  await openModels(page);
  await rows(page).first().click();
  await page.getByLabel("Base URL", { exact: true }).fill("http://unreachable.test/v1");
  await page.getByTestId("model-fetch-models").click();
  await expect(page.getByTestId("model-fetch-status")).toHaveClass(/fail/);
  await expect(page.getByTestId("model-fetch-status")).toContainText("Could not fetch the model list");
  await expect(page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true })).toBeEnabled();
});

test("Add API access fetches suggestions for every model row", async ({ page }) => {
  await openModels(page);
  await page.getByTestId("add-provider").click();
  await page.getByTestId("provider-api-url").fill("http://127.0.0.1:3425/v1");
  // The fetch action sits in the "Models" header row, right of its title.
  const fetch = page.getByTestId("model-fetch-models");
  await expect(page.locator(".provider-models-head > .model-fetch-btn")).toHaveCount(1);
  const title = await page.locator(".provider-models-title").boundingBox();
  const button = await fetch.boundingBox();
  expect(button!.x).toBeGreaterThan(title!.x + title!.width - 1);
  await expect(fetch).toHaveAttribute("aria-busy", "false");
  await fetch.click();
  await expect(page.getByTestId("model-fetch-status")).toContainText("2 models found");
  await expect(fetch).toHaveAttribute("aria-busy", "false");
  await expect(fetch).not.toHaveClass(/busy/);
  const first = page.getByTestId("provider-model-row").first().getByTestId("provider-model-id");
  await expect(first).toHaveAttribute("list", "model-id-suggestions");
  await first.fill("deepseek/deepseek-pro");
  await page.getByTestId("save-provider").click();
  await expect(page.getByTestId("provider-add-form")).toBeHidden();
  // The gateway's facts for that exact id become the new profile's limits.
  await expect.poll(async () => (await invokes(page, "save_model")).at(0) ?? null).toMatchObject({
    key: null,
    profile: {
      model: "deepseek/deepseek-pro",
      context_window: 1000000,
      max_tokens: 384000,
      endpoint_efforts: ["none", "high"],
    },
  });
});

test("Test connection names the protocol in plain words", async ({ page }) => {
  await openModels(page);
  await rows(page).first().click();
  await openAdvanced(page);
  await page.getByLabel("Supports image input").uncheck();
  await page.getByRole("button", { name: "Test connection", exact: true }).click();
  await expect(page.locator(".settings-status")).toHaveText("Connection OK · OpenAI Chat Completions · deepseek-v4-pro");
});

test("setup feedback is translated", async ({ page }) => {
  await openModels(page, "mockFreshInstall=1", "zh");
  const placeholder = rows(page).first();
  await expect(placeholder.getByTestId("model-not-configured")).toHaveText("未配置");
  await expect(placeholder.locator(".settings-list-untitled")).toHaveText("未命名模型");
  await placeholder.click();
  await expect(page.getByTestId("model-advanced-section").locator(":scope > summary .model-advanced-title"))
    .toHaveText("高级选项");
  await page.locator(".settings-footer").getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.locator(".settings-status.fail"))
    .toHaveText("请填写或修正标出的字段：Base URL（基础地址）、API 密钥、模型 ID。");
  await page.locator("#model-form-api-url").fill("http://127.0.0.1:3425/v1");
  await expect(page.getByTestId("model-api-key-local-hint")).toContainText("可以留空");
  await expect(page.getByTestId("model-fetch-models")).toHaveText("获取模型列表");
});
