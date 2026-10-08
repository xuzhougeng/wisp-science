import { test, expect } from '@playwright/test';
import { tauriMock } from './mock-tauri';

// #1061: a subagent is a conversation another conversation's agent started.
test('a subagent conversation nests under its parent and can only be watched', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1');
  await page.locator('.proj-card-main').first().click();

  const group = page.getByTestId('sidebar-subagents');
  const toggle = page.getByTestId('sidebar-nest-toggle');
  const child = group.locator('[data-session-id="subagent-child"]');
  await expect(child).toHaveAttribute('data-session-subagent', 'true');
  // The row names its own kind; there is no "Subagents" heading above it.
  await expect(child.locator('.session-branch-icon')).toHaveAttribute('title', 'Subagents');
  await expect(child).toHaveAttribute('aria-description', 'Subagents');
  await expect(group).not.toContainText('Subagents');
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
  await expect(child).toBeHidden();
});
