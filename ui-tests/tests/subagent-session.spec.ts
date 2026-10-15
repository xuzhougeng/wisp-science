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

async function lastDeleteArgs(page: import('@playwright/test').Page) {
  return page.evaluate(() => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((call: any) => call.cmd === 'delete_session');
    const args = calls.at(-1)?.args;
    return args instanceof Map ? Object.fromEntries(args) : (args ?? null);
  });
}

for (const includeChildren of [true, false]) {
  test(`deleting a parent ${includeChildren ? 'also deletes' : 'can keep'} its subagents`, async ({ page }) => {
    await page.addInitScript(tauriMock);
    await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
    await page.locator('.proj-card-main').first().click();
    await page.locator('.sidebar [data-session-id="subagent-parent"]').click({ button: 'right' });
    await page.locator('.ctx-menu').getByRole('button', { name: 'Delete', exact: true }).click();

    const modal = page.locator('.confirm-modal');
    const choice = modal.getByTestId('session-children-choice');
    await expect(choice).toContainText('Subagents 2');
    const box = choice.getByRole('checkbox');
    await expect(box).toBeChecked();
    if (!includeChildren) {
      await box.uncheck();
      await expect(choice).toContainText('standalone conversations');
    }
    await modal.getByRole('button', { name: 'Delete', exact: true }).click();

    await expect.poll(() => lastDeleteArgs(page)).toMatchObject({ id: 'subagent-parent', includeChildren });
    await expect(page.locator('[data-session-id="subagent-parent"]')).toHaveCount(0);
    const child = page.locator('.sidebar [data-session-id="subagent-child"]');
    if (includeChildren) {
      await expect(child).toHaveCount(0);
      await expect(page.locator('[data-session-id="subagent-done"]')).toHaveCount(0);
    } else {
      // A kept subagent is an ordinary, writable conversation now.
      await expect(child).toHaveAttribute('data-session-subagent', 'false');
      await child.click();
      await expect(page.locator('#composer-input')).toBeEnabled();
    }
  });
}

// Every delete_session call, nested Map arguments included as plain objects.
async function deleteCalls(page: import('@playwright/test').Page) {
  return page.evaluate(() => {
    const plain = (value: any): any => value instanceof Map
      ? Object.fromEntries([...value].map(([key, item]) => [key, plain(item)]))
      : value;
    return ((window as any).__skillInvokeLog ?? [])
      .filter((call: any) => call.cmd === 'delete_session')
      .map((call: any) => plain(call.args));
  });
}

const familyFiles = {
  id: 'subagent-parent', includeChildren: true, includeArtifacts: true,
  artifactFingerprint: 'subagent-parent:family',
  childArtifactFingerprints: { 'subagent-child': 'subagent-child:family', 'subagent-done': 'subagent-done:family' },
};

test('deleting a parent with its subagents previews and deletes their files too', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator('.sidebar [data-session-id="subagent-parent"]').click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('button', { name: 'Delete', exact: true }).click();

  const modal = page.locator('.confirm-modal');
  const cascade = modal.getByTestId('session-children-choice').getByRole('checkbox');
  await expect(cascade).toBeChecked();
  await modal.getByRole('checkbox', { name: 'Also delete associated artifacts and files' }).check();
  await expect(modal).toContainText('3 artifacts · 3 files to process');
  for (const id of ['subagent-parent', 'subagent-child', 'subagent-done']) {
    await expect(modal).toContainText(`results/${id}.svg`);
  }

  // Keeping the subagents returns to the parent's own preview, and back.
  await cascade.uncheck();
  await expect(modal).toContainText('1 artifacts · 2 files to process');
  await expect(modal).not.toContainText('results/subagent-child.svg');
  await cascade.check();
  await expect(modal).toContainText('3 artifacts · 3 files to process');
  await modal.getByRole('button', { name: 'Delete', exact: true }).click();

  await expect.poll(() => deleteCalls(page)).toEqual([expect.objectContaining(familyFiles)]);
  await expect(page.locator('[data-session-id="subagent-parent"]')).toHaveCount(0);
  await expect(page.locator('[data-session-id="subagent-child"]')).toHaveCount(0);
  await expect(page.locator('[data-session-id="subagent-done"]')).toHaveCount(0);
});

test('a selected subagent is deleted with its selected parent, from the parent family preview', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
  await page.locator('.proj-card-main').first().click();
  const sidebar = page.locator('.sidebar');
  await sidebar.getByRole('button', { name: 'Select', exact: true }).click();
  await sidebar.locator('[data-session-id="subagent-parent"]').click();
  await sidebar.locator('[data-session-id="subagent-child"]').click();
  await sidebar.getByTestId('bulk-delete-sessions').click();

  const modal = page.locator('.confirm-modal');
  await expect(modal.getByTestId('session-children-choice')).toContainText('Subagents 1');
  await modal.getByRole('checkbox', { name: 'Also delete associated artifacts and files' }).check();
  await expect(modal).toContainText('3 artifacts · 3 files to process');
  await modal.getByRole('button', { name: 'Delete', exact: true }).click();

  // One call: the selected subagent goes with its parent, whatever the order.
  await expect(page.locator('[data-session-id="subagent-parent"]')).toHaveCount(0);
  expect(await deleteCalls(page)).toEqual([expect.objectContaining(familyFiles)]);
});

for (const withFiles of [true, false]) {
  test(`selecting a parent with every subagent deletes them as one family${withFiles ? ', files included' : ''}`, async ({ page }) => {
    await page.addInitScript(tauriMock);
    await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
    await page.locator('.proj-card-main').first().click();
    const sidebar = page.locator('.sidebar');
    await sidebar.getByRole('button', { name: 'Select', exact: true }).click();
    await sidebar.getByRole('checkbox', { name: 'Select all' }).click();
    await sidebar.getByTestId('bulk-delete-sessions').click();

    // Nothing is left to keep, so there is no choice, yet the subagents'
    // files are previewed with family semantics like a cascade.
    const modal = page.locator('.confirm-modal');
    const confirm = modal.getByRole('button', { name: 'Delete', exact: true });
    await expect(confirm).toBeEnabled();
    await expect(modal.getByTestId('session-children-choice')).toHaveCount(0);
    if (withFiles) {
      await modal.getByRole('checkbox', { name: 'Also delete associated artifacts and files' }).check();
      await expect(modal).toContainText('3 artifacts · 3 files to process');
      for (const id of ['subagent-parent', 'subagent-child', 'subagent-done']) {
        await expect(modal).toContainText(`results/${id}.svg`);
      }
    }
    await confirm.click();

    // One call for the parent, whatever order the selection is in.
    for (const id of ['subagent-parent', 'subagent-child', 'subagent-done']) {
      await expect(page.locator(`[data-session-id="${id}"]`)).toHaveCount(0);
    }
    expect(await deleteCalls(page)).toEqual([expect.objectContaining(withFiles
      ? familyFiles
      : { id: 'subagent-parent', includeChildren: true, includeArtifacts: false })]);
  });
}
