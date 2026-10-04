import { test, expect } from '@playwright/test';
import { tauriMock } from './mock-tauri';

// #1061: a subagent is a conversation another conversation's agent started.
test('a subagent conversation nests under its parent and can only be watched', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1');
  await page.locator('.proj-card-main').first().click();

  const group = page.getByTestId('sidebar-subagents');
  const toggle = group.getByTestId('sidebar-subagent-toggle');
  await expect(toggle).toContainText('Subagents');
  const child = group.locator('[data-session-id="subagent-child"]');
  await expect(child).toHaveAttribute('data-session-subagent', 'true');
  await expect(page.getByTestId('sidebar-branches')).toHaveCount(0);

  // The parent stays an ordinary, writable conversation.
  const parent = page.locator('.sidebar [data-session-id="subagent-parent"]');
  await expect(parent).toHaveAttribute('data-session-subagent', 'false');
  await parent.click();
  const composer = page.locator('#composer-input');
  await expect(composer).toBeEnabled();

  await child.click();
  await expect(composer).toBeDisabled();
  await expect(composer).toHaveAttribute('placeholder', /watch only/);

  await toggle.click();
  await expect(child).toHaveCount(0);
});
