import { test, expect, type Page } from '@playwright/test';
import { tauriMock } from './mock-tauri';

const frame = 'subagent-parent';
const task = (id: string, status = 'queued', revision = 2) => ({
  id, title: `Read paper ${id}`, instruction: 'Extract methods and evidence.',
  status, revision, completed_units: status === 'completed' ? 1 : 0,
  total_units: 1, failed_units: status === 'failed' ? 1 : 0, log: [], result: '',
});
const batch = (source = 'map_items') => ({
  id: 'swarm-1', source, title: 'Compare the methods in these papers', model: 'scripted-worker',
  started_at_ms: 100, revision: 2, phase: 'starting', total: 3,
  tasks: [task('a'), task('b'), task('c')], summary: '',
});

async function open(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator(`.sidebar [data-session-id="${frame}"]`).click();
}

async function emit(page: Page, progress: Record<string, unknown>, frameId = frame) {
  await page.evaluate(payload => (window as any).__tauriEmit('agent', payload), {
    kind: 'ToolPresentation', frame_id: frameId, presentation_id: progress.id,
    presentation_kind: 'agent_swarm', payload: progress,
  });
}

test('map swarm shows real phases and keeps the selected assistant live', async ({ page }) => {
  await open(page);
  const progress = batch();
  await emit(page, progress);
  const dock = page.getByTestId('swarm-dock');
  await expect(dock).toContainText('Starting tasks');
  await dock.getByRole('button').click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.getByTestId('swarm-worker')).toHaveCount(3);
  await expect(panel).toHaveClass(/swarm-starting/);
  await page.screenshot({ path: test.info().outputPath('swarm-start.png') });

  await emit(page, { ...progress, revision: 3, phase: 'running', tasks: [task('a', 'running', 3), task('b', 'running', 3)] });
  await expect(dock).toContainText('2 working');
  const worker = panel.locator('[data-task-id="a"]');
  await expect(worker).toHaveAttribute('aria-pressed', 'false');
  await worker.click();
  await expect(panel.getByTestId('swarm-task-detail')).toContainText('Extract methods');
  await emit(page, { ...progress, revision: 4, phase: 'running', tasks: [{ ...task('a', 'running', 4), log: ['read: papers/a.txt', 'Extracting study design'] }] });
  await expect(panel.getByTestId('swarm-task-detail')).toContainText('Extracting study design');
  await expect(worker).toHaveAttribute('aria-pressed', 'true');

  await emit(page, { ...progress, revision: 5, phase: 'running', tasks: [{ ...task('a', 'completed', 5), result: '<script>never execute</script>\nRandomized study.' }] });
  await expect(panel.getByTestId('swarm-task-result')).toContainText('Randomized study.');
  await expect(panel.getByTestId('swarm-task-result').locator('script')).toHaveCount(0);
  // Immediately after selecting: one Escape dismisses the detail only.
  await page.keyboard.press('Escape');
  await expect(panel.getByTestId('swarm-task-detail')).toHaveCount(0);
  await expect(panel).toBeVisible();
  await expect(worker).toHaveAttribute('aria-pressed', 'false');

  await emit(page, { ...progress, revision: 6, phase: 'summarizing', tasks: [task('b', 'completed', 6), task('c', 'completed', 6)] });
  await expect(panel).toHaveClass(/swarm-gathering/);
  await expect(dock).toContainText('Bringing results together');
  await page.screenshot({ path: test.info().outputPath('swarm-summary.png') });
  await emit(page, { ...progress, revision: 7, phase: 'completed', tasks: [], summary: 'Three methods compared; evidence is ready.' });
  await expect(panel.getByTestId('swarm-summary')).toHaveText('Three methods compared; evidence is ready.');
  await expect(panel).not.toHaveClass(/swarm-moving/);
  await expect(dock).toContainText('3/3 completed');
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
  await expect(dock).toBeVisible();
});

test('swarm typography follows the UI font size on desktop and narrow screens', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('wisp-ui-font-size', '21'));
  await open(page);
  await emit(page, batch());
  const dock = page.getByTestId('swarm-dock');
  await expect(dock.locator('.swarm-dock-copy strong')).toHaveCSS('font-size', '19.5px');
  await expect(dock.locator('.swarm-stages li').first()).toHaveCSS('font-size', '18px');
  await dock.getByRole('button').click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.locator('h2')).toHaveCSS('font-size', '27px');
  await expect(panel.locator('.swarm-worker-copy strong').first()).toHaveCSS('font-size', '18px');
  await expect(panel.locator('.swarm-worker-copy small').first()).toHaveCSS('font-size', '16.5px');
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCSS('font-size', '18px');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(panel.locator('.swarm-roster-footer small')).toHaveCSS('font-size', '15px');
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCSS('font-size', '16.5px');
  await expect(panel.locator('.swarm-stages li').first()).toHaveCSS('font-size', '16.5px');
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
});

for (const source of ['reader', 'explore']) {
  test(`${source} uses the same view, reduced motion and immediate Escape`, async ({ page }) => {
    await open(page);
    await emit(page, { ...batch(source), phase: 'running' });
    const dock = page.getByTestId('swarm-dock');
    await dock.getByRole('button').click();
    const panel = page.getByTestId('swarm-window');
    await expect(panel.locator('.swarm-hub')).toHaveCSS('animation-name', 'swarm-breathe');
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(dock).toBeVisible();
    await dock.getByRole('button').click();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(panel.locator('.swarm-hub')).toHaveCSS('animation-name', 'none');
    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
    const rect = await panel.boundingBox();
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(390);
    await expect.poll(() => panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`${source}-dark-mobile.png`) });
  });
}

test('failed and paused tasks stay honest; snapshots do not regress or leak between sessions', async ({ page }) => {
  await open(page);
  const progress = batch();
  await emit(page, { ...progress, revision: 5, phase: 'running', tasks: [task('a', 'completed', 5)] });
  // A snapshot requested before the live update may arrive afterwards.
  await emit(page, progress);
  const dock = page.getByTestId('swarm-dock');
  await expect(dock).toContainText('1/3 completed');
  await dock.getByRole('button').click();
  await expect(page.getByTestId('swarm-worker')).toHaveCount(3);
  await page.getByRole('button', { name: 'Stop this turn' }).click();
  await expect.poll(() => page.evaluate(() => {
    const call = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === 'stop_agent').at(-1);
    return call?.args instanceof Map ? call.args.get('sessionId') : call?.args?.sessionId;
  })).toBe(frame);
  await emit(page, { ...progress, revision: 6, phase: 'paused', tasks: [task('b', 'failed', 6), task('c', 'paused', 6)], summary: 'Stopped. Resume to continue.' });
  await expect(dock).toContainText('1/3 completed');
  await expect(dock).toContainText('1 failed');
  await expect(page.getByTestId('swarm-window')).not.toHaveClass(/swarm-moving/);
  await page.keyboard.press('Escape');
  await page.locator('.sidebar [data-session-id="subagent-child"]').click();
  await expect(dock).toHaveCount(0);
  await page.locator(`.sidebar [data-session-id="${frame}"]`).click();
  await expect(dock).toContainText('Paused');
  await emit(page, { ...progress, id: 'swarm-2', started_at_ms: 200 });
  await emit(page, { ...progress, revision: 10, phase: 'completed' });
  await expect(dock).toContainText('Starting tasks');
});

test('the swarm Escape layer leaves an underlying Agents detail open', async ({ page }) => {
  await page.addInitScript(tauriMock);
  await page.goto('/?mockSubagent=1&mockSubagentActivity=1');
  await page.locator('.proj-card-main').first().click();
  await page.locator(`.sidebar [data-session-id="${frame}"]`).click();
  await page.getByRole('button', { name: 'Toggle panel' }).click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  await page.locator('[data-activity-id="subagent-done"]').click();
  await expect(page.getByTestId('agent-activity-detail')).toBeVisible();
  await emit(page, batch());
  await page.getByTestId('swarm-dock').getByRole('button').click();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('swarm-window')).toHaveCount(0);
  await expect(page.getByTestId('agent-activity-detail')).toBeVisible();
});
