import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

function pdfFixture() {
  const streams = ['0 0 1 rg 20 20 80 60 re f', '1 0 0 rg 20 20 80 60 re f'];
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 5 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 6 0 R >>',
    ...streams.map(stream => `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`)];
  let pdf = '%PDF-1.4\n';
  const offsets = [0];
  objects.forEach((object, i) => { offsets.push(pdf.length); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(pdf).toString('base64');
}

test('native formula and PDF viewer renders locally with failure and navigation states', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('https://wisp-preview.local/**', async route => {
    const name = new URL(route.request().url()).pathname.slice(1);
    const root = /^(katex|KaTeX|pdf)/.test(name) ? '../ui/vendor-src' : '../apps/windows/Wisp.Science.Preview/Assets/RichPreview';
    await route.fulfill({ body: await readFile(path.resolve(root, name)), contentType: name.endsWith('.html') ? 'text/html' : name.endsWith('.css') ? 'text/css' : name.endsWith('.woff2') ? 'font/woff2' : 'text/javascript' });
  });
  await page.addInitScript(() => {
    const w = window as any;
    w.previewMessages = []; w.chrome ??= {};
    w.chrome.webview = { postMessage: (message: unknown) => w.previewMessages.push(message),
      addEventListener: (_: string, callback: unknown) => { w.previewReceive = callback; } };
  });
  await page.goto('https://wisp-preview.local/index.html');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.some((m: any) => m.type === 'ready'))).toBe(true);
  const send = (data: object) => page.evaluate(data => (window as any).previewReceive({ data }), { background: '#fff', foreground: '#111', fontSize: 16, ...data });
  await send({ kind: 'math', text: '\\frac{a}{b}+\\alpha^2', display: true });
  await expect(page.locator('.katex')).toBeVisible();
  await expect(page.locator('math')).toHaveCount(1);
  await send({ kind: 'math', text: '\\unknowncommand{a}', display: false });
  await expect(page.locator('#status')).toContainText('公式预览失败');
  await expect(page.locator('#content')).toHaveText('\\unknowncommand{a}');
  await send({ kind: 'math', text: '\\href{https://example.com}{click}', display: false });
  await expect(page.locator('#content a')).toHaveCount(0);
  await send({ kind: 'pdf', base64: pdfFixture() });
  const canvas = page.locator('canvas');
  await expect(canvas).toBeVisible();
  await expect(page.locator('#page')).toHaveText('1 / 2');
  const pixel = () => canvas.evaluate((element: HTMLCanvasElement) => Array.from(element.getContext('2d')!.getImageData(30, 140, 1, 1).data));
  expect(await pixel()).toEqual([0, 0, 255, 255]);
  await page.getByRole('button', { name: '下一页' }).click();
  await expect(page.locator('#page')).toHaveText('2 / 2');
  await expect.poll(pixel).toEqual([255, 0, 0, 255]);
  await page.getByRole('button', { name: '放大', exact: true }).click();
  await expect(canvas).toHaveAttribute('width', '250');
  await send({ kind: 'pdf', base64: Buffer.from('invalid').toString('base64') });
  await expect(page.locator('#status')).toContainText('PDF预览失败');
  expect(errors).toEqual([]);
});
