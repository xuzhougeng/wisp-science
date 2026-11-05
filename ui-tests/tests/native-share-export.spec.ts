import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Exercise the exact shared renderer shipped in WinUI. No IPC, file picker or network.
test.beforeEach(async ({ page }) => {
  await page.route('**/__native-share/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)!;
    const path = name === 'native-share.js'
      ? '../../apps/windows/Wisp.Science.Preview/Assets/Share/native-share.js'
      : name === 'share-export.css' ? '../../ui/src/styles/share-export.css' : '../../ui/src/' + name;
    await route.fulfill(name === 'index.html'
      ? { contentType: 'text/html', body: '<!doctype html><meta http-equiv="Content-Security-Policy" content="img-src data:; connect-src \'none\'"><link rel="stylesheet" href="share-export.css"><body></body>' }
      : { contentType: name.endsWith('.css') ? 'text/css' : 'text/javascript', body: readFileSync(resolve(__dirname, path), 'utf8') });
  });
  await page.goto('/__native-share/index.html');
});

function payload(format: string) {
  return { format, width: 600, locale: 'en', dark: true, variables: { '--bg-app': '#171614', '--text': '#ebe8e2' },
    title: 'wisp-science', stamp: '2026-10-30', footer: 'Shared from Wisp Science',
    labels: { user: 'You', assistant: 'Wisp Science', reasoning: 'Thinking' },
    rows: [{ role: 'user', text: 'xxx example' }, { role: 'assistant', text: 'Result' }],
    html: '<!doctype html><html><head></head><body><main class="share-page"><header class="share-head"><h1>Wisp</h1></header><div class="thread"><article class="msg user"><div class="role">你</div><div class="body">xxx example</div></article><article class="msg assistant"><div class="role">Wisp</div><div class="body md"><table><tr><th>A</th><th>B</th></tr><tr><td>12</td><td>24</td></tr></table><pre><code>print(42)</code></pre></div></article></div><footer class="share-foot"></footer></main></body></html>' };
}

test('PNG uses requested width, shared table rasterization and dark theme', async ({ page }) => {
  const result = await page.evaluate(async data => {
    const url = '/__native-share/native-share.js';
    const { renderNativeShare } = await import(url);
    const output = await renderNativeShare(data);
    const image = new Image(); image.src = 'data:image/png;base64,' + output; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0);
    return { width: image.width, height: image.height, pixel: [...ctx.getImageData(0, 0, 1, 1).data], raster: (window as any).__shareRasterInfo };
  }, payload('png'));
  expect(result.width).toBe(1200); expect(result.height).toBeGreaterThan(300);
  expect(result.pixel).toEqual([23, 22, 20, 255]); expect(result.raster.tableCount).toBe(1);
});

test('HTML freezes locale, labels, date and theme without loading external images', async ({ page }) => {
  const data = payload('html'); data.html = data.html.replace('<table>', '<img src="https://example.invalid/private.png" alt="local placeholder"><table>');
  const requests: string[] = [];
  await page.route('https://example.invalid/**', route => { requests.push(route.request().url()); return route.abort(); });
  const output = await page.evaluate(async data => {
    const url = '/__native-share/native-share.js'; return (await import(url)).renderNativeShare(data);
  }, data);
  expect(requests).toEqual([]); expect(output).toContain('lang="en"'); expect(output).toContain('2026-10-30');
  expect(output).toContain('>You</div>'); expect(output).toContain('--bg-app: #171614');
  expect(output).toContain('src="https://example.invalid/private.png"');
});

test('PNG does not paint clipped accessibility MathML or transparent subtrees', async ({ page }) => {
  const data = payload('png');
  data.html = data.html.replace('<table>', '<span class="katex-mathml">InvisibleMath</span><span style="opacity:0"><b>InvisibleText</b></span><table>');
  const painted = await page.evaluate(async data => {
    const text: string[] = [];
    const original = CanvasRenderingContext2D.prototype.fillText;
    CanvasRenderingContext2D.prototype.fillText = function (...args: Parameters<typeof original>) { text.push(args[0]); return original.apply(this, args); };
    const url = '/__native-share/native-share.js'; await (await import(url)).renderNativeShare(data); return text.join(' ');
  }, data);
  expect(painted).not.toContain('InvisibleMath'); expect(painted).not.toContain('InvisibleText');
});

test('mismatched selection fails before rendering', async ({ page }) => {
  const data = payload('png'); data.rows.pop();
  const message = await page.evaluate(async data => {
    const url = '/__native-share/native-share.js';
    try { await (await import(url)).renderNativeShare(data); return ''; } catch (error) { return String(error); }
  }, data);
  expect(message).toContain('does not match');
});
