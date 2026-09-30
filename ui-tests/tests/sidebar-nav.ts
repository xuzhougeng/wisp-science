import type { Page } from "@playwright/test";

// Short windows fold the tail of the sidebar nav into "More" (height tiers in
// ui/src/styles/sidebar.css). Open an entry wherever this viewport puts it.
export async function openSidebarEntry(page: Page, name: string | RegExp) {
  const nav = page.locator(".sidebar .nav");
  await nav.waitFor();
  const inline = nav.getByRole("button", { name, exact: true });
  if (await inline.isVisible()) {
    await inline.click();
    return;
  }
  await nav.locator(".nav-more").click();
  await page.getByTestId("sidebar-more-menu").getByRole("button", { name, exact: true }).click();
}
