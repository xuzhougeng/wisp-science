import { test, expect, type Page } from '@playwright/test';
import { tauriMock } from './mock-tauri';

// Ctrl/Cmd+A while reading a document selects that document's text, not the
// sidebar, tab strip, path chip and toolbars around it.
const files = {
  'METHODS.md': '# Methods overview\n\nFirst paragraph of the methods.\n\n## Statistics\n\nLast paragraph of the methods.\n',
  'run.sh': 'echo one\necho two\necho three',
  'fit.R': 'fit <- lm(y ~ x)\nsummary(fit)\n',
};

async function setup(page: Page) {
  await page.addInitScript(tauriMock);
  await page.goto('/');
  await expect(page.locator('.proj-card-main').first()).toBeVisible();
  await page.evaluate(files => {
    const w = window as any, original = w.__TAURI__.core.invoke;
    w.__TAURI__.core.invoke = async (cmd: string, args: any) => {
      const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
      if (cmd === 'list_dir') return Object.entries(files).map(([name, text]) => ({ name, is_dir: false, size: text.length, modified_unix_millis: 0 }));
      if (cmd === 'read_file') {
        const path = arg('path'), name = path.split(/[\\/]/).at(-1);
        if (name in files) return { path, mime: 'text/plain', text: files[name as keyof typeof files], base64: null, truncated: false };
      }
      return original(cmd, args);
    };
  }, files);
  await page.locator('.proj-card-main').first().click();
  await page.getByRole('button', { name: 'Files', exact: true }).click();
}

async function openCenter(page: Page, path: string) {
  await page.locator(`[data-workspace-path="${path}"]`).click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('button', { name: 'Open in center', exact: true }).click();
  await expect(page.locator('.center-file-preview')).toHaveAttribute('data-file-path', path);
}

// The current selection's text, and whether both of its ends sit inside the
// open document's selectable body.
function selection(page: Page) {
  return page.evaluate(() => {
    const s = window.getSelection()!;
    const body = document.querySelector('.center-file-preview [data-select-all]');
    const inside = !!body && !!s.anchorNode && body.contains(s.anchorNode) && body.contains(s.focusNode);
    return { text: s.toString(), inside };
  });
}

test('Ctrl/Cmd+A in an open Markdown document selects only the document', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'METHODS.md');
  const preview = page.locator('.center-file-preview');
  await expect(preview.locator('h1')).toHaveText('Methods overview');

  // Nothing inside the document was clicked yet: the open document still owns the shortcut.
  await page.keyboard.press('ControlOrMeta+A');
  const selected = await selection(page);
  expect(selected.inside).toBe(true);
  expect(selected.text).toContain('Methods overview');
  expect(selected.text).toContain('Last paragraph of the methods.');
  // Not the path chip, the tab strip or the Files pane around it.
  expect(selected.text).not.toContain('METHODS.md');
  expect(selected.text).not.toContain('run.sh');
});

test('Ctrl/Cmd+A in a code preview selects the source without line numbers', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'run.sh');
  const body = page.locator('.center-file-preview .rp-code-body');
  await expect(body).toContainText('echo three');
  await body.click();
  await page.keyboard.press('ControlOrMeta+A');
  const selected = await selection(page);
  expect(selected.inside).toBe(true);
  expect(selected.text.trimEnd()).toBe(files['run.sh']);
});

test('Ctrl/Cmd+A in a script editor selects the whole source in the editor', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'fit.R');
  const editor = page.getByRole('textbox', { name: 'Source editor' });
  await expect(editor).toHaveValue(files['fit.R']);
  await expect(editor).not.toBeFocused();
  await page.keyboard.press('ControlOrMeta+A');
  await expect(editor).toBeFocused();
  expect(await editor.evaluate((el: HTMLTextAreaElement) => [el.selectionStart, el.selectionEnd]))
    .toEqual([0, files['fit.R'].length]);
});

test('beside the conversation, Ctrl/Cmd+A follows where the user last pointed', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'METHODS.md');
  const preview = page.locator('.center-file-preview');
  await expect(preview.locator('h1')).toHaveText('Methods overview');
  await preview.locator('[data-center-split]').click();
  await expect(page.locator('.chat-stage')).toBeVisible();

  // Pointing at the conversation leaves the document alone.
  await page.locator('.chat-stage').click({ position: { x: 8, y: 8 } });
  await page.keyboard.press('ControlOrMeta+A');
  expect((await selection(page)).inside).toBe(false);

  // Pointing back at the document scopes the shortcut to it again.
  await preview.getByText('First paragraph of the methods.').click();
  await page.keyboard.press('ControlOrMeta+A');
  const selected = await selection(page);
  expect(selected.inside).toBe(true);
  expect(selected.text).toContain('Last paragraph of the methods.');
});
