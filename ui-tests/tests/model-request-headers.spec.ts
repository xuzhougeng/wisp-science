import { test, expect } from "@playwright/test";
import { tauriMock } from "./mock-tauri";

for (const locale of ["en", "zh"]) {
  for (const mode of ["add", "edit"]) {
    test(`request header explanations and live examples work in ${locale} (${mode})`, async ({ page }, testInfo) => {
      await page.addInitScript(tauriMock);
      await page.goto(`/?mockLocale=${locale}`);
      await page.getByRole("button", { name: locale === "zh" ? "设置" : "Settings", exact: true }).click();
      await page.getByTestId("settings-nav-models").click();
      if (mode === "add") {
        await page.getByTestId("model-presets").getByRole("button", { name: "OpenCode", exact: true }).click();
      } else {
        await page.locator(".settings-list-row").first().click();
      }
      // Request headers sit inside the form's collapsed Advanced section.
      await page.getByTestId("model-advanced-section").locator(":scope > summary").click();
      const advanced = page.getByTestId("model-advanced-options");
      await expect(advanced.locator("summary")).toHaveText(
        locale === "zh" ? "请求附加信息（高级）" : "Request headers (advanced)",
      );
      await advanced.locator("summary").click();
      await expect(advanced.locator(".model-request-intro")).toContainText(
        locale === "zh" ? "作为 HTTP 请求头" : "add HTTP headers",
      );
      const userAgent = page.getByTestId("model-user-agent");
      const agentPreview = page.getByTestId("model-user-agent-preview");
      const sessionName = page.getByTestId("model-session-header-name");
      const sessionPreview = page.getByTestId("model-session-header-preview");
      const disabled = locale === "zh" ? "已关闭，不会发送此请求头。" : "Disabled — this header will not be sent.";
      await expect(userAgent).toHaveAccessibleName(/User-Agent/);
      await expect(userAgent).toHaveAccessibleDescription(/research-client\/1.0/);
      await expect(sessionName).toHaveAccessibleDescription(/x-opencode-session/);
      await expect(agentPreview).toHaveText("User-Agent: wisp-science");
      await userAgent.fill("research-client/1.0");
      await expect(agentPreview).toHaveText("User-Agent: research-client/1.0");
      await page.getByTestId("model-send-user-agent").uncheck();
      await expect(agentPreview).toHaveText(disabled);
      await expect(userAgent).toBeDisabled();
      await page.getByTestId("model-send-user-agent").check();
      await expect(userAgent).toHaveValue("research-client/1.0");
      await expect(agentPreview).toHaveText("User-Agent: research-client/1.0");
      await userAgent.fill("");
      await expect(agentPreview).toHaveText("User-Agent: wisp-science");
      if (mode === "edit") await expect(sessionPreview).toHaveText(disabled);
      await page.getByTestId("model-send-session-id").check();
      const generated = locale === "zh" ? "<Wisp 自动生成的会话 ID>" : "<automatically generated conversation ID>";
      await expect(sessionPreview).toHaveText(`x-opencode-session: ${generated}`);
      await sessionName.fill("x-research-session");
      await expect(sessionPreview).toHaveText(`x-research-session: ${generated}`);
      await page.getByTestId("model-send-session-id").uncheck();
      await expect(sessionPreview).toHaveText(disabled);
      await expect(sessionName).toBeDisabled();
      await page.getByTestId("model-send-session-id").check();
      await expect(sessionName).toHaveValue("x-research-session");
      await sessionName.fill("");
      await expect(sessionPreview).toHaveText(`x-opencode-session: ${generated}`);

      for (const width of [1440, 640]) {
        await page.setViewportSize({ width, height: 1200 });
        // Long custom values must wrap within their own preview instead of widening the form.
        await userAgent.fill(`research-client/${"a".repeat(180)}`);
        const groups = advanced.locator(".model-request-header");
        await expect(groups).toHaveCount(2);
        for (const group of await groups.all()) {
          const bounds = (await group.boundingBox())!;
          const preview = (await group.locator("code").boundingBox())!;
          expect(preview.x).toBeGreaterThan(bounds.x);
          expect(preview.x + preview.width).toBeLessThanOrEqual(bounds.x + bounds.width);
          expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
        }
        await userAgent.fill("research-client/1.0");
        await advanced.screenshot({ path: testInfo.outputPath(`request-headers-${width}.png`) });
      }
    });
  }
}
