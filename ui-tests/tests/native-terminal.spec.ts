import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

test('native terminal renders VT and isolates streams while bridging input and resize', async ({ page }) => {
  page.on('pageerror', error => console.error(error.message));
  page.on('console', message => { if (message.type() === 'error') console.error(message.text()); });
  const origin = 'https://wisp-terminal.local';
  await page.route(`${origin}/**`, async route => {
    const name = new URL(route.request().url()).pathname.slice(1);
    const root = name.startsWith('xterm') ? '../ui/vendor-src' : '../apps/windows/Wisp.Science.Preview/Assets/Terminal';
    const body = await readFile(path.resolve(root, name));
    await route.fulfill({ body, contentType: name.endsWith('.html') ? 'text/html' : name.endsWith('.css') ? 'text/css' : 'text/javascript' });
  });
  await page.addInitScript(() => {
    const w = window as any;
    w.terminalMessages = [];
    w.chrome ??= {};
    w.chrome.webview = {
      postMessage: (message: unknown) => w.terminalMessages.push(message),
      addEventListener: (_: string, callback: (message: unknown) => void) => { w.terminalReceive = callback; },
    };
  });
  await page.goto(`${origin}/index.html`);
  await expect.poll(() => page.evaluate(() => (window as any).terminalMessages.some((m: any) => m.type === 'ready'))).toBe(true);
  const send = (data: unknown) => page.evaluate(data => (window as any).terminalReceive({ data }), data);
  const state = { type: 'state', version: 1, enabled: true, fontSize: 14, fontFamily: 'monospace', background: '#101010', foreground: '#ffffff' };
  await send(state);
  const output = async (text: string, version = 1, reset = false) => send({ type: 'output', version, reset, base64: Buffer.from(text).toString('base64') });
  await output('\x1b[31mRED\x1b[0m\r\nprogress 10%\r\x1b[2Kdone\r\n根尖\r\n', 1, true);
  const screen = page.locator('.xterm-accessibility-tree');
  await expect(screen).toContainText('RED');
  await expect(screen).toContainText('done');
  await expect(screen).toContainText('根尖');
  await expect(screen).not.toContainText('progress');
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('echo test');
  await page.keyboard.press('Control+c');
  await expect.poll(() => page.evaluate(() => (window as any).terminalMessages.filter((m: any) => m.type === 'input').map((m: any) => m.data).join(''))).toContain('echo test\x03');
  await send({ ...state, enabled: false });
  const count = await page.evaluate(() => (window as any).terminalMessages.filter((m: any) => m.type === 'input').length);
  await page.keyboard.type('blocked');
  expect(await page.evaluate(() => (window as any).terminalMessages.filter((m: any) => m.type === 'input').length)).toBe(count);
  await page.setViewportSize({ width: 700, height: 320 });
  await expect.poll(() => page.evaluate(() => (window as any).terminalMessages.filter((m: any) => m.type === 'resize').length)).toBeGreaterThan(1);
  await send({ ...state, version: 2 });
  await output('STALE', 1);
  await output('CURRENT', 2, true);
  await expect(screen).toContainText('CURRENT');
  await expect(screen).not.toContainText('STALE');
  await expect(screen).not.toContainText('RED');
});
