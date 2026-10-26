import { test, expect, type Page } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

const ADDRESS_REQUIRED = "Enter a proxy address, or choose Inherit Network settings or Direct.";

function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const plain = (value: any): any => {
      if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
      if (Array.isArray(value)) return value.map(plain);
      if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
      return value;
    };
    const calls = ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === name);
    return plain(calls.at(-1)?.args ?? null);
  }, cmd);
}

async function openSettings(page: Page, section: string, locale = "en") {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}`);
  await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
  await page.getByTestId(`settings-nav-${section}`).click();
}

test("a model API endpoint keeps its own route and new models on it start from it", async ({ page }) => {
  await openSettings(page, "models");
  const save = page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true });
  await page.locator(".settings-list-row").first().click();
  const mode = page.getByTestId("model-proxy-mode");
  await expect(mode).toHaveValue("inherit");
  await expect(page.getByTestId("model-proxy")).toContainText("Inherit uses Model API in Settings → Network.");

  // A custom route without an address is rejected before anything is saved.
  await mode.selectOption("custom");
  await expect(page.getByTestId("model-proxy-address")).toBeVisible();
  await save.click();
  await expect(page.getByText(ADDRESS_REQUIRED)).toBeVisible();
  expect(await lastInvokeArgs(page, "save_model")).toBeNull();

  await mode.selectOption("direct");
  await expect(page.getByTestId("model-proxy-address")).toBeHidden();
  await save.click();
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    profile: { id: "default", proxy_url: "none" },
  });

  // The route reopens with its endpoint; another endpoint still inherits.
  await page.locator(".settings-list-row").first().click();
  await expect(page.getByTestId("model-proxy-mode")).toHaveValue("direct");
  await page.locator(".settings-footer").getByRole("button", { name: "Cancel", exact: true }).click();
  await page.locator(".settings-list-row").nth(1).click();
  await expect(page.getByTestId("model-proxy-mode")).toHaveValue("inherit");
  await page.getByTestId("model-proxy-mode").selectOption("custom");
  await page.getByTestId("model-proxy-address").fill("socks5://127.0.0.1:1080");
  await save.click();
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({
    profile: { id: "opus", proxy_url: "socks5://127.0.0.1:1080" },
  });

  // Adding models on the routed DeepSeek endpoint shows its route, and an
  // explicit choice there applies to the whole endpoint.
  await page.getByTestId("add-provider").click();
  await expect(page.getByTestId("model-proxy-mode")).toHaveValue("direct");
  await page.getByTestId("model-proxy-mode").selectOption("inherit");
  await page.getByTestId("save-provider").click();
  await expect(page.getByTestId("provider-add-form")).toBeHidden();
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({ profile: { proxy_url: "" } });
  await page.locator(".settings-list-row").first().click();
  await expect(page.getByTestId("model-proxy-mode")).toHaveValue("inherit");
});

test("saving a model without touching its route leaves the saved route alone", async ({ page }) => {
  await openSettings(page, "models");
  await page.locator(".settings-list-row").first().click();
  await page.locator(".settings-footer").getByRole("button", { name: "Save", exact: true }).click();
  await expect.poll(() => lastInvokeArgs(page, "save_model")).toMatchObject({ profile: { id: "default" } });
  // Omitted, not null: the backend keeps the address's route.
  expect(Object.keys((await lastInvokeArgs(page, "save_model")).profile)).not.toContain("proxy_url");
});

test("each subscription saves its own route", async ({ page }) => {
  await openSettings(page, "models");
  await page.getByTestId("models-category-subscriptions").click();
  const xai = page.getByTestId("subscription-account-xai");
  const mode = xai.getByTestId("subscription-proxy-xai-mode");
  const save = xai.getByTestId("save-subscription-proxy-xai");
  await expect(mode).toHaveValue("inherit");
  await expect(save).toBeDisabled();

  await mode.selectOption("custom");
  await save.click();
  await expect(xai.getByText(ADDRESS_REQUIRED)).toBeVisible();
  expect(await lastInvokeArgs(page, "set_subscription_proxy")).toBeNull();

  await xai.getByTestId("subscription-proxy-xai-address").fill("socks5://127.0.0.1:1080");
  await save.click();
  await expect.poll(() => lastInvokeArgs(page, "set_subscription_proxy")).toEqual({
    provider: "xai",
    proxyUrl: "socks5://127.0.0.1:1080",
  });
  await expect(xai.getByRole("status").filter({ hasText: "Saved" })).toBeVisible();
  await expect(save).toBeDisabled();
  await expect(page.getByTestId("subscription-proxy-codex-mode")).toHaveValue("inherit");

  // The saved route is read back when the tab opens again.
  await page.getByTestId("models-category-decision").click();
  await page.getByTestId("models-category-subscriptions").click();
  await expect(page.getByTestId("subscription-proxy-xai-mode")).toHaveValue("custom");
  await expect(page.getByTestId("subscription-proxy-xai-address")).toHaveValue("socks5://127.0.0.1:1080");
});

test("a custom MCP connection saves its own route", async ({ page }) => {
  await openSettings(page, "connections");
  await page.getByRole("button", { name: "Add connection" }).click();
  await page.getByLabel("Name").fill("local-mcp");
  await page.getByRole("textbox", { name: "Command" }).fill("npx");
  const mode = page.getByTestId("conn-proxy-mode");
  await expect(mode).toHaveValue("inherit");
  await expect(page.getByTestId("conn-proxy")).toContainText("Inherit uses MCP services in Settings → Network.");
  const save = page.getByRole("button", { name: "Save", exact: true });

  await mode.selectOption("custom");
  await save.click();
  await expect(page.getByTestId("conn-test-status")).toHaveText(ADDRESS_REQUIRED);
  expect(await lastInvokeArgs(page, "add_mcp_connection")).toBeNull();

  await mode.selectOption("direct");
  await save.click();
  await expect.poll(() => lastInvokeArgs(page, "add_mcp_connection")).toMatchObject({
    conn: { name: "local-mcp", proxy_url: "none" },
  });
});

test("route choices are labelled in Chinese", async ({ page }) => {
  await openSettings(page, "models", "zh");
  await page.locator(".settings-list-row").first().click();
  const route = page.getByTestId("model-proxy");
  await expect(route.getByText("网络代理", { exact: true })).toBeVisible();
  await expect(page.getByTestId("model-proxy-mode").locator("option")).toHaveText(["继承网络设置", "直连", "自定义代理"]);
  await expect(route).toContainText("「继承」沿用 设置 → 网络 中「模型 API」的配置。");
});
