import { test, expect, Page } from '@playwright/test';
import { tauriMock } from './mock-tauri';

// The Agents pane lists every subagent of the current conversation (child
// conversations and explore runs) and opens each one's final answer in place.
async function openAgentsPane(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator('.sidebar [data-session-id="subagent-parent"]').click();
  await page.getByRole('button', { name: 'Toggle panel' }).click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  return page.getByTestId('agent-workflows');
}

async function lastInvokeArgs(page: Page, cmd: string) {
  return page.evaluate((name) => {
    const calls = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === name);
    const args = calls.at(-1)?.args ?? null;
    return args instanceof Map ? Object.fromEntries(args) : args;
  }, cmd);
}

test('subagents list in progress and completed, with explore runs folded', async ({ page }) => {
  const panel = await openAgentsPane(page);
  const running = panel.getByTestId('agent-activity-running');
  await expect(running.getByTestId('agent-activity-row')).toHaveCount(1);
  await expect(running).toContainText('Align the reads');
  await expect(running.locator('.agent-activity-status')).toHaveText('Running');

  const completed = panel.getByTestId('agent-activity-completed');
  await expect(completed).toContainText('Summarize QC metrics');
  await expect(completed).toContainText('12 samples pass, 1 fails.');
  const explore = completed.getByTestId('agent-explore-group');
  await expect(explore).toContainText('Quick explorations · 2');
  await expect(explore).not.toHaveAttribute('open', '');
  await expect(explore.getByTestId('agent-activity-row').first()).toBeHidden();
  await explore.locator('summary').click();
  await expect(explore.getByTestId('agent-activity-row')).toHaveCount(2);
  await expect(explore.locator('[data-activity-id="explore:e2"] .agent-activity-status')).toHaveText('Failed');
  await expect(panel.getByTestId('agent-open-workflows')).toBeVisible();
  await page.screenshot({ path: '../test-results/subagent-activity-list.png' });
});

test('a subagent opens its final answer in place and Escape returns to the list', async ({ page }) => {
  const panel = await openAgentsPane(page);
  await panel.locator('[data-activity-id="subagent-done"]').click();
  const detail = panel.getByTestId('agent-activity-detail');
  await expect(detail).toContainText('Summarize QC metrics');
  await expect(detail.getByTestId('agent-activity-answer').locator('h2')).toHaveText('QC summary');
  await expect(detail.getByTestId('agent-activity-answer').locator('li')).toHaveCount(2);
  await expect(detail.locator('.agent-activity-status')).toHaveText('Completed');
  await expect(detail).toContainText('7 tool calls');
  await expect(detail).toContainText('4m 5s');
  await expect(detail.getByTestId('agent-activity-stop')).toHaveCount(0);
  await page.screenshot({ path: '../test-results/subagent-activity-detail.png' });

  // Escape closes only the detail layer; the pane itself stays open.
  await page.keyboard.press('Escape');
  await expect(detail).toHaveCount(0);
  await expect(panel.getByTestId('agent-activity-running')).toBeVisible();

  // The explore detail shows the trace path and has no conversation to open.
  await panel.getByTestId('agent-explore-group').locator('summary').click();
  await panel.locator('[data-activity-id="explore:e1"]').click();
  await expect(detail).toContainText('/proj/.wisp/subagents/explore-1.txt');
  await expect(detail.getByTestId('agent-activity-open')).toHaveCount(0);
  await detail.getByTestId('agent-activity-back').click();
  await expect(detail).toHaveCount(0);
});

test('a running subagent can be stopped and a child conversation opened from the pane', async ({ page }) => {
  const panel = await openAgentsPane(page);
  await panel.locator('[data-activity-id="subagent-child"]').click();
  const detail = panel.getByTestId('agent-activity-detail');
  await expect(detail).toContainText('Working…');
  await detail.getByTestId('agent-activity-stop').click();
  await expect.poll(() => lastInvokeArgs(page, 'stop_agent')).toMatchObject({ sessionId: 'subagent-child' });

  await detail.getByTestId('agent-activity-open').click();
  const composer = page.locator('#composer-input');
  await expect(composer).toBeDisabled();
  await expect(composer).toHaveAttribute('placeholder', /watch only/);
  // Switching conversations leaves the detail page.
  await expect(detail).toHaveCount(0);
});
