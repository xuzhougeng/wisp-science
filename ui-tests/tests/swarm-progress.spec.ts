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
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.getByTestId('swarm-worker')).toHaveCount(3);
  await expect(panel).toHaveClass(/swarm-starting/);
  await expect(panel.locator('.swarm-metric[data-status="total"] dd')).toHaveText('3');
  await expect(panel.locator('.swarm-stages .current')).toHaveAttribute('aria-current', 'step');
  await page.screenshot({ path: test.info().outputPath('swarm-start.png') });

  await emit(page, { ...progress, revision: 3, phase: 'running', tasks: [task('a', 'running', 3), task('b', 'running', 3)] });
  await expect(dock).toContainText('2 working');
  await expect(panel.locator('.swarm-metric[data-status="running"] dd')).toHaveText('2');
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
  await expect(panel.locator('.swarm-metric[data-status="completed"] dd')).toHaveText('3');
  await expect(panel.locator('.swarm-metric[data-status="failed"] dd')).toHaveText('0');
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
  await expect(dock.locator('.swarm-phase-badge')).toHaveCSS('font-size', '16.5px');
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.locator('h2')).toHaveCSS('font-size', '27px');
  await expect(panel.getByTestId('swarm-request')).toHaveCSS('font-size', '19.5px');
  await expect(panel.locator('.swarm-stages li').first()).toHaveCSS('font-size', '18px');
  await expect(panel.locator('.swarm-worker-copy strong').first()).toHaveCSS('font-size', '18px');
  await expect(panel.locator('.swarm-worker-copy small').first()).toHaveCSS('font-size', '16.5px');
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCSS('font-size', '18px');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(panel.locator('.swarm-roster-footer small')).toHaveCSS('font-size', '15px');
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCSS('font-size', '16.5px');
  await expect(panel.locator('.swarm-stages li').first()).toHaveCSS('font-size', '16.5px');
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
});

test('the compact swarm dock follows the composer column on wide and narrow panes', async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await open(page);
  await emit(page, { ...batch('reader'), phase: 'running', tasks: [task('a', 'running'), task('b'), task('c')] });
  const dock = page.locator('.composer > [data-testid="swarm-dock"]');
  await expect(dock).toBeVisible();
  await expect(dock.locator('.swarm-stages')).toHaveCount(0);
  for (const width of [1920, 1024]) {
    await page.setViewportSize({ width, height: 1080 });
    await expect.poll(async () => {
      const card = await dock.boundingBox();
      const composer = await page.locator('.composer-inner').boundingBox();
      return !!card && !!composer && Math.abs(card.x - composer.x) <= 1 && Math.abs(card.width - composer.width) <= 1;
    }).toBe(true);
    expect((await dock.boundingBox())!.height).toBeLessThan(90);
  }
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('swarm-window')).toHaveCount(0);
  await expect(dock).toBeVisible();
});

test('long Reader requests remain body text and partial failures keep their own counts', async ({ page }) => {
  await open(page);
  const request = '重新整理 BAM 输入路径，生成 samples.csv 和分析计划。' +
    '/data2/project/long-read-isoform-quantification/workflow-output/sample-34/pass.untagged.bam '.repeat(4);
  const progress = {
    ...batch('reader'), phase: 'partial', title: request, total: 28,
    tasks: Array.from({ length: 28 }, (_, i) => ({
      ...task(String(i), i === 0 ? 'completed' : 'failed'),
      title: `样本 ${i + 1}：整理长读长测序输入路径与分析结果`,
      completed_units: 3, total_units: 3, failed_units: i === 0 ? 0 : 3,
    })),
  };
  await emit(page, progress);
  const dock = page.getByTestId('swarm-dock');
  await expect(dock).toContainText('1/28 completed');
  await expect(dock).toContainText('27 failed');
  await expect(dock.locator('.swarm-phase-badge')).toHaveText('Finished with gaps');
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel).toHaveAttribute('data-phase', 'partial');
  await expect(panel.getByRole('heading', { name: 'Task overview', exact: true })).toBeVisible();
  await expect(panel.getByTestId('swarm-request')).toHaveText(request);
  await expect(panel.getByTestId('swarm-request')).toHaveCSS('font-weight', '400');
  for (const [status, count] of [['total', '28'], ['running', '0'], ['completed', '1'], ['failed', '27']]) {
    await expect(panel.locator(`.swarm-metric[data-status="${status}"] dd`)).toHaveText(count);
  }
  const meter = panel.locator('.swarm-overview > .swarm-meter');
  const completed = meter.locator('[data-status="completed"]');
  const failed = meter.locator('[data-status="failed"]');
  expect((await failed.boundingBox())!.width / (await completed.boundingBox())!.width).toBeCloseTo(27, 0);
  expect(await failed.evaluate(el => getComputedStyle(el).backgroundColor))
    .not.toBe(await completed.evaluate(el => getComputedStyle(el).backgroundColor));
  await expect(panel.locator('.swarm-hub')).toHaveCSS('animation-name', 'none');
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath('reader-partial-overview.png') });

  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await expect.poll(() => panel.evaluate(el => [el, ...el.querySelectorAll('.swarm-detail, .swarm-request, .swarm-metrics')]
    .every(node => node.scrollWidth <= node.clientWidth + 1))).toBe(true);
  await page.screenshot({ path: test.info().outputPath('reader-partial-mobile.png') });
  await panel.locator('[data-task-id="1"]').click();
  await page.keyboard.press('Escape');
  await expect(panel.getByTestId('swarm-task-detail')).toHaveCount(0);
  await expect(panel).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
});

test('finished Reader progress can be dismissed and its results stay in Agents', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: 'Toggle panel' }).click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  const agents = page.getByTestId('agent-workflows');
  await expect(agents).toContainText('No subagents in this conversation yet');
  const progress = {
    ...batch('reader'), phase: 'partial', revision: 4,
    tasks: [
      { ...task('a', 'completed', 4), result: 'Grounded evidence from the saved conversation.' },
      task('b', 'failed', 4), task('c', 'failed', 4),
    ],
    summary: 'One relevant session; two sessions could not be read.',
  };
  await emit(page, progress);
  const card = agents.getByTestId('swarm-panel');
  await expect(card).toContainText('Session retrieval');
  await expect(card).toContainText('1/3 completed');
  await expect(agents).not.toContainText('No subagents in this conversation yet');
  await expect(card.getByRole('button', { name: 'Stop this turn' })).toHaveCount(0);
  await page.getByTestId('swarm-dismiss').click();
  await expect(page.getByTestId('swarm-dock')).toHaveCount(0);
  await expect(page.getByTestId('swarm-window')).toHaveCount(0);
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: 'View parallel tasks and results' }).click();
  const overlay = page.getByTestId('swarm-window');
  await expect(overlay.getByTestId('swarm-summary')).toHaveText(progress.summary);
  await overlay.locator('[data-task-id="a"]').click();
  await expect(overlay.getByTestId('swarm-task-result')).toHaveText('Grounded evidence from the saved conversation.');
  await page.keyboard.press('Escape');
  await expect(overlay.getByTestId('swarm-task-detail')).toHaveCount(0);
  await expect(overlay).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(overlay).toHaveCount(0);
  await expect(agents).toBeVisible();
  await expect(page.getByTestId('swarm-dock')).toHaveCount(0);

  // Neither a snapshot refresh nor switching conversations revives this dock.
  // Loading a conversation returns the right pane to Artifacts, so Agents is
  // reopened after each switch; the child conversation has no batch of its own.
  await emit(page, { ...progress, revision: 5 });
  await page.locator('.sidebar [data-session-id="subagent-child"]').click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  await expect(agents).toContainText('No subagents in this conversation yet');
  await expect(card).toHaveCount(0);
  await page.locator(`.sidebar [data-session-id="${frame}"]`).click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  await expect(card).toContainText('Session retrieval');
  await expect(page.getByTestId('swarm-dock')).toHaveCount(0);
  await emit(page, { ...batch('reader'), id: 'swarm-new', started_at_ms: 200 });
  await expect(page.getByTestId('swarm-dock')).toBeVisible();
  await expect(page.getByTestId('swarm-dismiss')).toHaveCount(0);
});

test('Agents shows live parallel batches and stops the current turn', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: 'Toggle panel' }).click();
  await page.locator('.rightpane').getByRole('button', { name: 'Agents', exact: true }).click();
  const card = page.getByTestId('agent-workflows').getByTestId('swarm-panel');
  for (const [index, source] of ['reader', 'map_items', 'explore'].entries()) {
    const progress = {
      ...batch(source), id: `parallel-${source}`, started_at_ms: 200 + index,
      phase: 'running', tasks: [task('a', 'running'), task('b'), task('c')],
    };
    await emit(page, progress);
    await expect(card).toContainText('1 working');
    await expect(page.getByTestId('swarm-dismiss')).toHaveCount(0);
    await card.getByRole('button', { name: 'View parallel tasks and results' }).click();
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('swarm-window')).toHaveCount(0);
    await expect(card).toBeVisible();
    await card.getByRole('button', { name: 'Stop this turn' }).click();
    await expect.poll(() => page.evaluate(() =>
      ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === 'stop_agent').length,
    )).toBe(index + 1);
    await expect.poll(() => page.evaluate(() => {
      const call = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === 'stop_agent').at(-1);
      return call?.args instanceof Map ? call.args.get('sessionId') : call?.args?.sessionId;
    })).toBe(frame);
    await emit(page, { ...progress, phase: 'cancelled', revision: 3, tasks: [task('a', 'cancelled', 3), task('b', 'cancelled', 3), task('c', 'cancelled', 3)] });
    await expect(card.getByRole('button', { name: 'Stop this turn' })).toHaveCount(0);
    await page.getByTestId('swarm-dismiss').click();
    await expect(card).toContainText('Stopped');
  }
});

for (const source of ['reader', 'explore']) {
  test(`${source} uses the same view, reduced motion and immediate Escape`, async ({ page }) => {
    await open(page);
    await emit(page, { ...batch(source), phase: 'running' });
    const dock = page.getByTestId('swarm-dock');
    await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
    const panel = page.getByTestId('swarm-window');
    await expect(panel.locator('.swarm-hub')).toHaveCSS('animation-name', 'swarm-breathe');
    await page.keyboard.press('Escape');
    await expect(panel).toHaveCount(0);
    await expect(dock).toBeVisible();
    await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
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
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
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
  await page.getByTestId('swarm-dock').getByRole('button', { name: 'View assistants', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('swarm-window')).toHaveCount(0);
  await expect(page.getByTestId('agent-activity-detail')).toBeVisible();
});

const lastCall = (page: Page, cmd: string) => page.evaluate(cmd => {
  const call = ((window as any).__skillInvokeLog ?? []).filter((c: any) => c.cmd === cmd).at(-1);
  const args = call?.args instanceof Map ? Object.fromEntries(call.args) : call?.args;
  return args ? { sessionId: args.sessionId, taskIds: args.taskIds ?? null } : null;
}, cmd);

test('finished Reader batches can retry their failed assistants, all at once or one at a time', async ({ page }) => {
  await open(page);
  const progress = {
    ...batch('reader'), phase: 'partial', revision: 4,
    tasks: [
      { ...task('a', 'completed', 4), total_units: 3, completed_units: 3 },
      { ...task('b', 'failed', 4), total_units: 10, completed_units: 10, failed_units: 1, failed_unit_indexes: [7],
        log: ['Reading part 8 / 10', 'Part 8 failed: Reader returned incomplete JSON.'] },
      { ...task('c', 'failed', 4), total_units: 2, completed_units: 2, failed_units: 2 },
    ],
    summary: 'One session read; two could not be read.',
  };
  await emit(page, progress);
  const dock = page.getByTestId('swarm-dock');
  await expect(dock.getByTestId('swarm-retry')).toHaveAttribute('aria-label', 'Retry failed tasks (2)');
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.getByTestId('swarm-retry-note')).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toHaveCount(0);
  // Chunk counts render as a per-assistant bar with the failed share kept distinct.
  const bar = panel.locator('[data-task-id="b"] .swarm-worker-bar');
  await expect(bar).toBeVisible();
  expect((await bar.locator('.swarm-worker-bar-failed').boundingBox())!.width).toBeGreaterThan(0);
  await expect(panel.locator('.swarm-orbit').nth(1)).toHaveAttribute('data-status', 'failed');
  await expect(panel.locator('.swarm-orbit').nth(3)).toHaveAttribute('data-status', 'idle');

  // One task from its own page.
  await panel.locator('[data-task-id="b"]').click();
  await panel.getByTestId('swarm-retry-task').click();
  await expect.poll(() => lastCall(page, 'retry_swarm_tasks')).toEqual({ sessionId: frame, taskIds: ['b'] });
  // The backend resumes the same batch: failed task back to work, controls swap.
  await emit(page, { ...progress, revision: 5, phase: 'running', summary: '',
    tasks: [{ ...task('b', 'queued', 5), total_units: 10, completed_units: 9, log: ['Retry: re-reading failed part(s) 8'] }] });
  await expect(panel.getByTestId('swarm-task-detail')).toContainText('Retry: re-reading failed part(s) 8');
  await expect(panel.getByTestId('swarm-retry-task')).toHaveCount(0);
  await expect(panel.getByTestId('swarm-retry')).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Stop this turn' })).toBeVisible();
  await expect(panel).toHaveClass(/swarm-moving/);
  // A re-queued assistant is waiting, not working, until the backend starts it.
  await expect(dock).toContainText('1/3 completed · 0 working · 1 failed');
  await emit(page, { ...progress, revision: 6, phase: 'running',
    tasks: [{ ...task('b', 'running', 6), total_units: 10, completed_units: 9, log: ['Retry: re-reading failed part(s) 8', 'Reading part 8 / 10'] }] });
  await expect(dock).toContainText('1 working');
  await expect(panel.locator('[data-task-id="b"] .swarm-avatar')).toHaveCSS('animation-name', 'swarm-ring');
  await emit(page, { ...progress, revision: 7, phase: 'partial', summary: '## Reader retry\nRecovered evidence.',
    tasks: [{ ...task('b', 'completed', 7), total_units: 10, completed_units: 10, result: '\n[8] recovered quote\n' }] });
  await expect(panel.getByTestId('swarm-task-result')).toContainText('recovered quote');
  await page.keyboard.press('Escape');
  await expect(panel.getByTestId('swarm-summary')).toContainText('Recovered evidence.');
  await expect(panel.getByTestId('swarm-retry')).toContainText('Retry failed tasks (1)');

  // The remaining failure, from the roster; a backend refusal is shown in place.
  await page.evaluate(() => { (window as any).__failSwarmRetry = true; });
  await panel.getByTestId('swarm-retry').click();
  await expect.poll(() => lastCall(page, 'retry_swarm_tasks')).toEqual({ sessionId: frame, taskIds: null });
  await expect(panel.getByRole('alert')).toContainText('Wait for the current turn to finish');
  await expect(panel.getByTestId('swarm-retry')).toBeEnabled();
  await page.keyboard.press('Escape');
  await expect(panel).toHaveCount(0);
});

test('batches that are not Reader retrieval, or still running, offer no retry', async ({ page }) => {
  await open(page);
  await emit(page, { ...batch('map_items'), phase: 'partial', revision: 3, tasks: [task('a', 'completed', 3), task('b', 'failed', 3), task('c', 'failed', 3)] });
  const dock = page.getByTestId('swarm-dock');
  await expect(dock).toContainText('2 failed');
  await expect(dock.getByTestId('swarm-retry')).toHaveCount(0);
  await dock.getByRole('button', { name: 'View assistants', exact: true }).click();
  const panel = page.getByTestId('swarm-window');
  await expect(panel.getByTestId('swarm-retry')).toHaveCount(0);
  await expect(panel.getByTestId('swarm-retry-note')).toHaveCount(0);
  await panel.locator('[data-task-id="b"]').click();
  await expect(panel.getByTestId('swarm-retry-task')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await emit(page, { ...batch('reader'), id: 'swarm-live', started_at_ms: 300, phase: 'running', revision: 3, tasks: [task('a', 'failed', 3), task('b', 'running', 3), task('c')] });
  await expect(dock).toContainText('1 failed');
  await expect(dock.getByTestId('swarm-retry')).toHaveCount(0);
  await expect(page.getByTestId('swarm-dismiss')).toHaveCount(0);
});
