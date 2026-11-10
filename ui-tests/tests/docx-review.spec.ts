import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { tauriMock } from './mock-tauri';

const fixture = readFileSync(resolve(__dirname, '../fixtures/docx-review.docx')).toString('base64');
const plain = readFileSync(resolve(__dirname, '../fixtures/office-preview.docx')).toString('base64');

async function setup(page: Page, options: { writable: boolean; conflict: boolean; modal?: boolean; empty?: boolean } = { writable: true, conflict: false }) {
  await page.addInitScript(tauriMock);
  await page.goto('/');
  await expect(page.locator('.proj-card-main').first()).toBeVisible();
  await page.evaluate(({ fixture, plain, options }) => {
    const w = window as any, original = w.__TAURI__.core.invoke;
    const fromJs = (value: any): any => value instanceof Map
      ? Object.fromEntries([...value].map(([key, item]) => [key, fromJs(item)])) : value;
    let version = 0;
    let conflict = options.conflict;
    let changes = [
      { id: 'rewrite', paragraph_id: 'p2', kind: 'rewrite', old: 'Old conclusion', new: 'Revised conclusion' },
      { id: 'deletion', paragraph_id: 'p3', kind: 'deletion', old: 'Remove this sentence', new: '' },
      { id: 'insertion', paragraph_id: 'p4', kind: 'insertion', old: '', new: 'New supporting sentence' },
    ].map(change => ({ ...change, author: 'Wisp', date: '2026-11-05T00:00:00Z' }));
    if (options.empty) changes = [];
    const snapshot = () => ({ source_sha256: `version-${version}`, changes, unsupported_count: options.empty ? 0 : 1, writable: options.writable });
    w.docxCalls = [];
    w.__TAURI__.core.invoke = async (cmd: string, input: any) => {
      const args = fromJs(input);
      if (cmd === 'list_dir') return [{ name: 'review.docx', is_dir: false, size: 2000, modified_unix_millis: 0 }];
      if (cmd === 'read_file') return { path: args.path, mime: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', text: null, base64: null };
      if (cmd === 'read_file_bytes') return Uint8Array.from(atob(changes.length ? fixture : plain), c => c.charCodeAt(0));
      if (cmd === 'read_docx_review') { w.docxCalls.push({ cmd, args }); return snapshot(); }
      if (cmd === 'resolve_docx_review') {
        w.docxCalls.push({ cmd, args });
        if (conflict) { conflict = false; version++; throw new Error('DOCX changed; refresh the preview before reviewing'); }
        if (args.sourceSha256 !== `version-${version}`) throw new Error('Stale hash');
        changes = changes.filter(change => args.selection.type === 'kind' ? change.kind !== args.selection.value : change.id !== args.selection.value);
        version++;
        return snapshot();
      }
      return original(cmd, input);
    };
  }, { fixture, plain, options });
  await page.locator('.proj-card-main').first().click();
  await page.getByRole('button', { name: 'Files', exact: true }).click();
  if (options.modal) {
    await page.locator('[data-workspace-path="review.docx"]').click();
  } else {
    await page.locator('[data-workspace-path="review.docx"]').click({ button: 'right' });
    await page.locator('.ctx-menu').getByRole('button', { name: 'Open in center', exact: true }).click();
  }
  if (options.empty) {
    await expect(page.getByTestId('docx-no-changes')).toBeVisible();
    return;
  }
  await expect(page.getByTestId('docx-change')).toHaveCount(3);
  if (options.writable) await expect(page.getByTestId('docx-accept-rewrites')).toBeEnabled();
}

const row = (page: Page, id: string) => page.locator(`[data-change-id="${id}"]`);
const writes = (page: Page) => page.evaluate(() => (window as any).docxCalls.filter((c: any) => c.cmd === 'resolve_docx_review').map((c: any) => c.args));

test('DOCX review controls follow the UI font size preference', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('wisp-ui-font-size', '21'));
  await setup(page);
  // The 12px review labels and actions scale by 21/14, including nested buttons.
  const controls = page.locator('.docx-review-toolbar button, .docx-review-change-head, .docx-review-change button');
  await expect(controls).toHaveCount(12);
  for (const control of await controls.all()) await expect(control).toHaveCSS('font-size', '18px');
});

test('DOCX renders native revision markup and accepts deletions and rewrites as separate batches', async ({ page }) => {
  await setup(page);
  await expect(page.locator('.rp-docx del').first()).toContainText('Old conclusion');
  await expect(page.locator('.rp-docx ins').first()).toContainText('Revised conclusion');
  await page.getByTestId('docx-review').screenshot({ path: test.info().outputPath('docx-review.png') });
  await expect(page.getByTestId('docx-review')).toContainText('Complex revisions are kept unchanged');
  await page.getByTestId('docx-accept-deletions').click();
  await expect(row(page, 'deletion')).toHaveCount(0);
  await expect(row(page, 'rewrite')).toBeVisible();
  await page.getByTestId('docx-accept-rewrites').click();
  await expect(row(page, 'rewrite')).toHaveCount(0);
  await expect(row(page, 'insertion')).toBeVisible();
  await row(page, 'insertion').getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(page.getByTestId('docx-no-changes')).toBeVisible();
  // With nothing left to review the bar collapses and the splitter goes away.
  await expect(page.getByTestId('docx-accept-deletions')).toBeHidden();
  await expect(page.getByTestId('docx-review-resizer')).toHaveCount(0);
  await expect(page.locator('.rp-docx')).toContainText('Word local fixture');
  await expect(page.locator('.rp-docx ins, .rp-docx del')).toHaveCount(0);
  expect(await writes(page)).toEqual([
    { path: 'review.docx', sourceSha256: 'version-0', selection: { type: 'kind', value: 'deletion' }, action: 'accept' },
    { path: 'review.docx', sourceSha256: 'version-1', selection: { type: 'kind', value: 'rewrite' }, action: 'accept' },
    { path: 'review.docx', sourceSha256: 'version-2', selection: { type: 'change', value: 'insertion' }, action: 'accept' },
  ]);
});

test('DOCX rejects a whole rewrite and accepts an individual deletion without resolving other changes', async ({ page }) => {
  await setup(page);
  await row(page, 'rewrite').getByRole('button', { name: 'Reject', exact: true }).click();
  await expect(row(page, 'rewrite')).toHaveCount(0);
  await row(page, 'deletion').getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(page.getByTestId('docx-change')).toHaveCount(1);
  await expect(row(page, 'insertion')).toBeVisible();
  expect(await writes(page)).toMatchObject([
    { sourceSha256: 'version-0', selection: { type: 'change', value: 'rewrite' }, action: 'reject' },
    { sourceSha256: 'version-1', selection: { type: 'change', value: 'deletion' }, action: 'accept' },
  ]);
});

test('DOCX conflicts require refresh before another write uses the new snapshot', async ({ page }) => {
  await setup(page, { writable: true, conflict: true });
  await page.getByTestId('docx-accept-rewrites').click();
  await expect(page.getByRole('alert')).toContainText('DOCX changed');
  await expect(page.getByTestId('docx-accept-deletions')).toBeDisabled();
  await expect(row(page, 'rewrite').getByRole('button', { name: 'Reject', exact: true })).toBeDisabled();
  expect(await writes(page)).toHaveLength(1);
  await page.getByRole('button', { name: 'Refresh revisions' }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByTestId('docx-accept-rewrites').click();
  await expect(row(page, 'rewrite')).toHaveCount(0);
  expect((await writes(page))[1].sourceSha256).toBe('version-1');
});

test('DOCX in a read-only workspace shows revisions with all mutation actions disabled', async ({ page }) => {
  await setup(page, { writable: false, conflict: false });
  await expect(page.getByTestId('docx-review')).toContainText('This preview is read-only');
  await expect(page.locator('.rp-docx del').first()).toContainText('Old conclusion');
  await expect(page.getByTestId('docx-accept-deletions')).toBeDisabled();
  await expect(page.getByTestId('docx-accept-rewrites')).toBeDisabled();
  for (const action of ['Accept', 'Reject']) {
    for (const button of await page.getByTestId('docx-change').getByRole('button', { name: action, exact: true }).all()) await expect(button).toBeDisabled();
  }
  expect(await writes(page)).toEqual([]);
});

test('DOCX review and document remain scrollable inside the ordinary preview dialog', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 720 });
  await setup(page, { writable: true, conflict: false, modal: true });
  await expect(page.locator('.artifact-modal .rp-docx')).toContainText('Revision review fixture');
  const bounds = await page.getByTestId('docx-review').evaluate(element => {
    const outer = element.getBoundingClientRect();
    const review = element.querySelector('.docx-review-panel')!.getBoundingClientRect();
    const document = element.querySelector('.rp-heavy')!.getBoundingClientRect();
    return { height: outer.height, review: review.height, document: document.height, bottom: document.bottom - outer.bottom };
  });
  expect(bounds.document).toBeGreaterThan(120);
  expect(bounds.review).toBeLessThan(bounds.height * 0.6);
  expect(bounds.bottom).toBeLessThanOrEqual(1);
  await page.getByTestId('docx-accept-rewrites').click();
  await expect(row(page, 'rewrite')).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.locator('.artifact-modal')).toHaveCount(0);
});

const panelHeight = (page: Page) => page.locator('.docx-review-panel').evaluate(element => element.getBoundingClientRect().height);

test('DOCX without revisions shows a compact bar instead of the batch review controls', async ({ page }) => {
  await setup(page, { writable: true, conflict: false, empty: true });
  await expect(page.locator('.rp-docx')).toContainText('Word local fixture');
  await expect(page.getByRole('button', { name: 'Refresh revisions' })).toBeVisible();
  await expect(page.getByTestId('docx-accept-deletions')).toBeHidden();
  await expect(page.getByTestId('docx-accept-rewrites')).toBeHidden();
  await expect(page.getByText('Each choice saves this document')).toBeHidden();
  await expect(page.getByTestId('docx-review-resizer')).toHaveCount(0);
  expect(await panelHeight(page)).toBeLessThan(80);
});

test('DOCX revision list and document are resized by dragging or arrow keys on the splitter', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await setup(page);
  const resizer = page.getByTestId('docx-review-resizer');
  await expect(resizer).toHaveAttribute('role', 'separator');
  const drag = async (dy: number) => {
    const box = (await resizer.boundingBox())!;
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + dy, { steps: 4 });
    await page.mouse.up();
  };
  const before = await panelHeight(page);
  await drag(-60);
  expect(Math.abs(await panelHeight(page) - (before - 60))).toBeLessThan(3);
  await drag(90);
  expect(Math.abs(await panelHeight(page) - (before + 30))).toBeLessThan(3);
  await resizer.focus();
  await page.keyboard.press('ArrowDown');
  expect(Math.abs(await panelHeight(page) - (before + 54))).toBeLessThan(3);
  // Neither side can be dragged away: the list keeps a minimum and the document stays in view.
  await drag(-2000);
  expect(await panelHeight(page)).toBeGreaterThanOrEqual(72);
  await drag(4000);
  const bounds = await page.getByTestId('docx-review').evaluate(element => {
    const outer = element.getBoundingClientRect();
    const document = element.querySelector('.rp-heavy')!.getBoundingClientRect();
    return { document: document.height, bottom: document.bottom - outer.bottom };
  });
  expect(bounds.document).toBeGreaterThanOrEqual(119);
  expect(bounds.bottom).toBeLessThanOrEqual(1);
  await expect(row(page, 'rewrite').getByRole('button', { name: 'Accept', exact: true })).toBeEnabled();
});
