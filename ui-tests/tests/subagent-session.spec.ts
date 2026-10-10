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

for (const parentIsBranch of [false, true]) {
  test(`an existing subagent follows its parent into a folder even with stale folder metadata (branch=${parentIsBranch})`, async ({ page }) => {
    await page.addInitScript(tauriMock);
    await page.goto(`/?mockSubagent=1&mockSubagentFolder=1&mockSubagentParentBranch=${parentIsBranch ? '1' : '0'}`);
    await page.locator('.proj-card-main').first().click();
    const folder = page.locator('.side-folder[data-folder-name="Analysis"]').locator('..');
    await expect(folder.locator('[data-session-id="subagent-parent"]')).toBeVisible();
    await expect(folder.getByTestId('sidebar-subagents').locator('[data-session-id="subagent-child"]')).toHaveCount(1);
    await expect(page.locator('.side-ungrouped [data-session-id="subagent-child"]')).toHaveCount(0);
    await folder.locator('[data-session-id="subagent-child"]').click();
    await expect(page.getByTestId('subagent-full-permission')).toBeVisible();
    if (!parentIsBranch) await page.screenshot({ path: '../test-results/subagent-group-and-permissions.png', fullPage: true });
  });
}

test('a watched subagent can enable and revoke its own Full Permission', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator('[data-session-id="subagent-child"]').click();
  await expect(page.locator('#composer-input')).toBeDisabled();
  const permission = page.getByTestId('subagent-full-permission');
  await expect(permission).toHaveAttribute('aria-pressed', 'false');
  await permission.click();
  await expect(page.getByRole('heading', { name: 'Enable Full Permission?' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(permission).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('button', { name: 'Agent options', exact: true }).click();
  const menu = page.getByRole('menu', { name: 'Agent options' });
  const toggle = menu.getByTestId('full-permission-toggle');
  const row = menu.locator('label.agent-menu-row', { hasText: 'Full Permission' });
  await row.click();
  await expect(page.getByRole('heading', { name: 'Enable Full Permission?' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toBeVisible();
  await expect(toggle).not.toBeChecked();
  await row.click();
  await page.getByRole('button', { name: 'Enable Full Permission', exact: true }).click();
  await expect(toggle).toBeChecked();
  await expect.poll(() => page.evaluate(() => {
    const args = ((window as any).__skillInvokeLog ?? [])
      .filter((entry: any) => entry.cmd === 'set_session_full_permission').at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({
      sessionId: 'subagent-child', enabled: true,
    });
  await expect(permission).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await page.locator('[data-session-id="subagent-parent"]').click();
  await page.getByRole('button', { name: 'Agent options', exact: true }).click();
  await expect(menu.getByTestId('full-permission-toggle')).not.toBeChecked();
  await page.keyboard.press('Escape');
  await page.locator('[data-session-id="subagent-child"]').click();
  await expect(permission).toHaveAttribute('aria-pressed', 'true');
  await permission.click();
  await expect(permission).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#composer-input')).toBeDisabled();
});

test('Full Permission confirmation keeps the child target when navigation changes', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator('[data-session-id="subagent-child"]').click();
  await page.getByTestId('subagent-full-permission').click();
  await expect(page.getByRole('heading', { name: 'Enable Full Permission?' })).toBeVisible();
  await page.evaluate(() => (window as any).__tauriEmit('open-session', {
    projectId: 'default', sessionId: 'subagent-parent',
  }));
  await expect(page.locator('#composer-input')).toBeEnabled();
  await page.getByRole('button', { name: 'Enable Full Permission', exact: true }).click();
  await expect.poll(() => page.evaluate(() => {
    const args = ((window as any).__skillInvokeLog ?? [])
      .filter((entry: any) => entry.cmd === 'set_session_full_permission').at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : args;
  })).toMatchObject({ sessionId: 'subagent-child', enabled: true });
  await page.getByRole('button', { name: 'Agent options', exact: true }).click();
  await expect(page.getByTestId('full-permission-toggle')).not.toBeChecked();
});
