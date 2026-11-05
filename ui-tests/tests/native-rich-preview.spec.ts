import { test, expect, type Page } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

const structureFixtures = {
  pdb: 'HETATM    1  C1  LIG A   1       0.000   0.000   0.000  1.00 20.00           C\nHETATM    2  C2  LIG A   1       1.500   0.000   0.000  1.00 20.00           C\nHETATM    3  O1  LIG A   1       2.500   1.000   0.000  1.00 20.00           O\nCONECT    1    2\nCONECT    2    3\nEND\n',
  mol2: '@<TRIPOS>MOLECULE\nethanol\n3 2 0 0 0\nSMALL\nNO_CHARGES\n@<TRIPOS>ATOM\n1 C1 0 0 0 C.3 1 LIG 0\n2 C2 1.5 0 0 C.3 1 LIG 0\n3 O1 2.5 1 0 O.3 1 LIG 0\n@<TRIPOS>BOND\n1 1 2 1\n2 2 3 1\n',
  cif: 'data_ethanol\nloop_\n_atom_site.group_PDB\n_atom_site.id\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\nHETATM 1 C C1 LIG A 1 0 0 0\nHETATM 2 C C2 LIG A 1 1.5 0 0\nHETATM 3 O O1 LIG A 1 2.5 1 0\n#\n',
};

test('native library SVG retains text pixels without executing the image document', async ({ page }) => {
  await nativePreview(page);
  const external: string[] = [];
  await page.context().route('https://example.invalid/**', route => { external.push(route.request().url()); return route.abort(); });
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="100"><rect width="300" height="100" fill="white"/><text x="12" y="65" font-size="28" fill="black">Axis labels</text><script>window.pwned=true</script><image href="https://example.invalid/image.png" width="1" height="1"/></svg>';
  await page.evaluate(base64 => (window as any).previewReceive({ data: { kind: 'image', format: 'image/svg+xml', base64, label: 'Labeled plot' } }), Buffer.from(svg).toString('base64'));
  // Open, then Escape immediately: the renderer forwards to the owning native
  // overlay even when focus has never entered the image.
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.at(-1).type)).toBe('escape');
  const image = page.locator('#content img');
  await expect(image).toHaveAttribute('alt', 'Labeled plot');
  const darkPixels = await image.evaluate((image: HTMLImageElement) => {
    const canvas = document.createElement('canvas'); canvas.width = 300; canvas.height = 100;
    const context = canvas.getContext('2d')!; context.drawImage(image, 0, 0);
    const pixels = context.getImageData(10, 30, 240, 45).data;
    let count = 0;
    for (let i = 0; i < pixels.length; i += 4) if (pixels[i] < 80 && pixels[i + 1] < 80 && pixels[i + 2] < 80 && pixels[i + 3] > 200) count++;
    return count;
  });
  expect(darkPixels).toBeGreaterThan(100);
  expect(await page.evaluate(() => (window as any).pwned)).toBeUndefined();
  expect(external).toEqual([]);
  expect(await page.evaluate(() => (window as any).previewMessages.find((m: any) => m.type === 'image-size'))).toEqual({ type: 'image-size', width: 300, height: 100 });
  await page.setViewportSize({ width: 180, height: 60 });
  const bounds = await image.boundingBox();
  expect(bounds!.width).toBeLessThanOrEqual(180); expect(bounds!.height).toBeLessThanOrEqual(60);
});

test('native image failures recover and cancelled decoding cannot replace a newer preview', async ({ page }) => {
  await nativePreview(page);
  const send = (data: object) => page.evaluate(data => (window as any).previewReceive({ data }), data);
  await send({ kind: 'image', format: 'text/html', base64: 'PGh0bWw+', imageError: 'Unable to read this library item.' });
  await expect(page.locator('#status')).toHaveText('Unable to read this library item.');
  await send({ kind: 'image', format: 'image/svg+xml', base64: 'bad' });
  await expect(page.locator('#content img')).toHaveCount(0);
  const base64 = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text y="8">A</text></svg>').toString('base64');
  await send({ kind: 'image', format: 'image/svg+xml', base64 });
  await expect(page.locator('#content img')).toBeVisible(); await expect(page.locator('#status')).toBeEmpty();
  await page.evaluate(async base64 => {
    const receive = (window as any).previewReceive;
    await Promise.all([receive({ data: { kind: 'image', format: 'image/svg+xml', base64 } }), receive({ data: { kind: 'math', text: 'x' } })]);
  }, base64);
  await expect(page.locator('.katex')).toBeVisible(); await expect(page.locator('#content img')).toHaveCount(0);
});

test('native molecules use the packaged WASM worker and cancel without stale drawings', async ({ page }) => {
  await nativePreview(page);
  const send = (data: object) => page.evaluate(data => (window as any).previewReceive({ data }), { background: '#fff', foreground: '#111', fontSize: 14, ...data });
  await send({ kind: 'molecule', text: 'CCO' });
  await expect(page.locator('svg.rp-molecule path')).not.toHaveCount(0);
  await expect(page.locator('.rp-error')).toHaveCount(0);
  await send({ kind: 'molecule', text: 'invalid molecule <script>window.pwned=true</script>' });
  await expect(page.locator('.rp-error')).toContainText('Invalid molecule');
  expect(await page.evaluate(() => (window as any).pwned)).toBeUndefined();
  await page.evaluate(async () => {
    const receive = (window as any).previewReceive;
    const pending = receive({ data: { kind: 'molecule', text: 'CCCC' } });
    await new Promise(resolve => setTimeout(resolve, 0));
    await receive({ data: { kind: 'math', text: 'x', display: true } });
    await pending;
  });
  await expect(page.locator('.katex')).toBeVisible();
  await expect(page.locator('.rp-molecule')).toHaveCount(0);
});

test('native structures parse PDB, MOL2 and mmCIF in a disposable WebGL document', async ({ page }) => {
  await nativePreview(page);
  for (const [format, text] of Object.entries(structureFixtures)) {
    await page.evaluate(data => (window as any).previewReceive({ data }), { kind: 'structure', text, format });
    const frame = page.frameLocator('iframe.rp-3dmol');
    await expect(frame.locator('body')).toHaveAttribute('data-ready', 'true');
    await expect(frame.locator('#status')).toContainText('3 atoms');
    await expect(frame.locator('canvas')).toBeVisible();
    // The context really draws atoms, rather than returning only a blank canvas.
    const colors = await frame.locator('canvas').evaluate((canvas: HTMLCanvasElement) => {
      const copy = document.createElement('canvas'); copy.width = canvas.width; copy.height = canvas.height;
      const context = copy.getContext('2d')!; context.drawImage(canvas, 0, 0);
      const pixels = context.getImageData(0, 0, copy.width, copy.height).data;
      const colors = new Set(); for (let i = 0; i < pixels.length; i += 4) colors.add(`${pixels[i]},${pixels[i + 1]},${pixels[i + 2]}`);
      return colors.size;
    });
    expect(colors).toBeGreaterThan(5);
  }
  await page.frameLocator('iframe.rp-3dmol').locator('canvas').click();
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.at(-1).type)).toBe('escape');
  const detached = page.frames().find(frame => frame.url().endsWith('structure-preview.html'))!;
  await page.evaluate(() => (window as any).previewReceive({ data: { kind: 'math', text: 'x' } }));
  expect(detached.isDetached()).toBe(true);
  await expect(page.locator('iframe')).toHaveCount(0);
});

test('native alignments normalize supported formats and reject mismatched or oversized data', async ({ page }) => {
  await nativePreview(page);
  const inputs = [
    ['fasta', '>one\nAC-G\n>two\nACTG\n'],
    ['clustal', 'CLUSTAL W\n\none AC-\ntwo ACT\n    ** \n\none G\ntwo G\n'],
    ['stockholm', '# STOCKHOLM 1.0\none AC.G\ntwo ACTG\n#=GC RF xxxx\n//\n'],
  ];
  for (const [format, text] of inputs) {
    await page.evaluate(data => (window as any).previewReceive({ data }), { kind: 'msa', text, format });
    await expect(page.locator('.rp-msa-bar')).toHaveText('2 sequences · 4 positions');
    const frame = page.frameLocator('iframe.rp-alignment');
    await expect(frame.locator('body')).toHaveAttribute('data-ready', 'true');
    const normalized = await frame.locator('nightingale-msa').evaluate((tag: any) => tag.sequenceViewer.sequences.raw);
    expect(JSON.stringify(normalized)).toContain('AC-G');
    await expect(frame.locator('msa-labels')).toContainText('one');
    await expect(frame.locator('msa-sequence-viewer canvas:visible')).toHaveCount(1);
    const painted = () => frame.locator('msa-sequence-viewer canvas:visible').evaluate((canvas: HTMLCanvasElement) => {
      const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
      let colored = 0; for (let i = 0; i < pixels.length; i += 4) if (pixels[i + 3] && pixels[i] !== pixels[i + 1]) colored++;
      return colored;
    });
    await expect.poll(painted).toBeGreaterThan(10);
  }
  await page.evaluate(text => (window as any).previewReceive({ data: { kind: 'msa', text } }), '>one\n'+'A'.repeat(120)+'\n>two\n'+'C'.repeat(120));
  const longFrame = page.frameLocator('iframe.rp-alignment');
  await expect(longFrame.locator('#range')).toHaveText('1–50 / 120');
  await longFrame.getByRole('button', { name: 'Next', exact: true }).click();
  await expect(longFrame.locator('#range')).toHaveText('51–100 / 120');
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.at(-1).type)).toBe('escape');
  for (const text of ['>one\nAC\n>two\nACG', '>one\n'+ 'A'.repeat(20001), '>one\nAC\n>one\nAC']) {
    await page.evaluate(text => (window as any).previewReceive({ data: { kind: 'msa', text } }), text);
    await expect(page.locator('.rp-error')).toBeVisible();
    await expect(page.locator('iframe.rp-alignment')).toHaveCount(0);
  }
  await page.evaluate(() => (window as any).previewReceive({ data: { kind: 'fasta', text: '>literal <script>\nACGT' } }));
  await expect(page.locator('.rp-fasta-hdr')).toHaveText('>literal <script>');
  await expect(page.locator('#content script')).toHaveCount(0);
});

test('native Office renders DOCX math and images, workbook sheets and formulas, and narrow slides', async ({ page }) => {
  await page.setViewportSize({ width: 380, height: 460 });
  const errors: string[] = [], external: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.context().route('https://example.invalid/**', route => { external.push(route.request().url()); return route.abort(); });
  await nativePreview(page);
  const send = async (kind: string, fixture = `office-preview.${kind}`) => {
    const base64 = (await readFile(path.resolve('fixtures', fixture))).toString('base64');
    await page.evaluate(data => (window as any).previewReceive({ data }), { kind, base64, background: '#fff', foreground: '#111', fontSize: 14 });
  };
  await send('docx');
  await expect(page.locator('.rp-docx')).toContainText('Native Office preview');
  await expect(page.locator('.rp-docx table')).toContainText('Sample');
  await expect(page.locator('.rp-docx math')).toBeVisible();
  const documentBounds = await page.locator('.rp-docx').evaluate(el => ({
    viewport: el.getBoundingClientRect().left,
    page: el.querySelector('section.docx')!.getBoundingClientRect().left,
  }));
  expect(documentBounds.page).toBeGreaterThanOrEqual(documentBounds.viewport);
  await expect.poll(() => page.locator('.rp-docx img').evaluateAll(images => images.every(i => (i as HTMLImageElement).naturalWidth > 0)) ).toBe(true);
  await expect(page.locator('.rp-docx img')).toHaveCount(1);
  await expect(page.locator('#quote-actions')).toBeHidden();
  await send('xlsx', 'office-multisheet.xlsx');
  await expect(page.locator('.rp-xlsx')).toContainText('FOXA2');
  await expect(page.locator('.rp-xlsx-cell.merged')).not.toHaveCount(0);
  await expect(page.locator('.rp-xlsx-grid')).toHaveCSS('overflow', 'auto');
  await expect(page.locator('.rp-xlsx-cell').first()).toHaveCSS('position', 'absolute');
  const gridHeight = await page.locator('.rp-xlsx-grid').evaluate(el => el.clientHeight);
  expect(gridHeight).toBeGreaterThan(180); expect(gridHeight).toBeLessThan(460);
  const tabs = page.locator('.rp-xlsx-tabs button');
  expect(await tabs.count()).toBeGreaterThan(1);
  await tabs.nth(1).click();
  await expect(tabs.nth(1)).toHaveClass('active');
  await expect(page.locator('.rp-xlsx-grid')).toContainText('Second sheet content');
  await expect(page.locator('.rp-xlsx-warning')).toContainText('受限范围');
  await send('xlsx', 'office-external-reference.xlsx');
  await page.locator('.rp-xlsx-cell', { hasText: '84' }).click();
  await expect(page.locator('.rp-xlsx-formula-value')).toHaveText("='[1]Sheet1'!A1");
  await send('pptx');
  await expect(page.locator('.rp-pptx [data-slide-index="0"]')).toBeVisible();
  const width = await page.locator('.rp-pptx').evaluate(e => e.clientWidth);
  expect(width).toBeGreaterThan(300); expect(width).toBeLessThan(381);
  await expect.poll(() => page.locator('.rp-pptx [data-slide-index="0"]').innerText()).not.toBe('');
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.at(-1).type)).toBe('escape');
  expect(external).toEqual([]); expect(errors).toEqual([]);
});

test('native Office labels follow the shared English and Chinese catalog', async ({ page }) => {
  await nativePreview(page);
  const catalog = JSON.parse(await readFile(path.resolve('../apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json'), 'utf8'));
  const workbookBytes = await readFile(path.resolve('fixtures/office-multisheet.xlsx'));
  const workbook = workbookBytes.toString('base64');
  // Plain text is a valid single-cell spreadsheet. Truncate an actual ZIP
  // header so each Office parser receives an invalid archive instead.
  const brokenArchive = workbookBytes.subarray(0, 8).toString('base64');
  for (const locale of ['en', 'zh']) {
    const strings = catalog[locale];
    for (const kind of ['docx', 'xlsx', 'pptx']) {
      const officeLabels = { loading: strings.loading, error: strings[`preview.${kind}_error`],
        formulaLabel: strings['preview.xlsx_formula'], truncated: strings['preview.xlsx_truncated'] };
      await page.evaluate(data => (window as any).previewReceive({ data }), { kind, locale, officeLabels, base64: brokenArchive });
      await expect(page.locator('.rp-error')).toHaveText(officeLabels.error);
      // Decode failures must also use the document-specific text, not a PDF error.
      await page.evaluate(data => (window as any).previewReceive({ data }), { kind, locale, officeLabels, base64: '!!!' });
      await expect(page.locator('#status')).toHaveText(officeLabels.error);
      if (kind !== 'xlsx') continue;
      await page.evaluate(data => (window as any).previewReceive({ data }), { kind, locale, officeLabels, base64: workbook });
      await expect(page.locator('.rp-xlsx-formula > span').first()).toHaveText(officeLabels.formulaLabel);
      await page.locator('.rp-xlsx-tabs button').nth(1).click();
      await expect(page.locator('.rp-xlsx-warning')).toHaveText(officeLabels.truncated);
    }
  }
});

test('native Office failures and late renders cannot replace the next preview', async ({ page }) => {
  await nativePreview(page);
  const send = (data: object) => page.evaluate(data => (window as any).previewReceive({ data }), { background: '#fff', foreground: '#111', fontSize: 14, ...data });
  for (const kind of ['docx', 'xlsx', 'pptx']) {
    await send({ kind, base64: Buffer.from('PK\x03\x04broken archive').toString('base64') });
    await expect(page.locator('.rp-error')).toContainText('无法预览');
  }
  const base64 = (await readFile(path.resolve('fixtures/office-preview.docx'))).toString('base64');
  await page.evaluate(base64 => {
    const receive = (window as any).previewReceive;
    void receive({ data: { kind: 'docx', base64 } });
    void receive({ data: { kind: 'math', text: 'x^2', display: true } });
  }, base64);
  await expect(page.locator('.katex')).toBeVisible();
  await expect(page.locator('.rp-docx')).toHaveCount(0);
  // A real XLSX worker is cancelled by the next payload before it returns.
  const workbook = (await readFile(path.resolve('fixtures/office-preview.xlsx'))).toString('base64');
  await page.evaluate(async base64 => {
    const receive = (window as any).previewReceive;
    const pending = receive({ data: { kind: 'xlsx', base64 } });
    await new Promise(resolve => setTimeout(resolve, 0));
    await receive({ data: { kind: 'math', text: 'y^2', display: true } });
    await pending;
  }, workbook);
  await expect(page.locator('.katex')).toContainText('y');
  await expect(page.locator('.rp-xlsx')).toHaveCount(0);
});

function pdfFixture(text = false) {
  const streams = ['0 0 1 rg 20 20 80 60 re f', '1 0 0 rg 20 20 80 60 re f']
    .map((stream, index) => text ? `${stream} 0 0 0 rg BT /F1 12 Tf 20 160 Td (${index ? 'Beta' : 'Alpha'} selection) Tj ET` : stream);
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>',
    ...streams.map(stream => `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  let pdf = '%PDF-1.4\n';
  const offsets = [0];
  objects.forEach((object, i) => { offsets.push(pdf.length); pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('');
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(pdf).toString('base64');
}

async function nativePreview(page: Page) {
  await page.context().route('https://wisp-preview.local/**', async route => {
    let name = new URL(route.request().url()).pathname.slice(1);
    const vendor = name.startsWith('vendor-runtime/');
    if (vendor) name = name.slice('vendor-runtime/'.length);
    const root = vendor || /^(katex|KaTeX|pdf)/.test(name) ? '../ui/vendor-src' : '../apps/windows/Wisp.Science.Preview/Assets/RichPreview';
    await route.fulfill({ body: await readFile(path.resolve(root, name)), contentType: name.endsWith('.html') ? 'text/html' : name.endsWith('.css') ? 'text/css' : name.endsWith('.wasm') ? 'application/wasm' : name.endsWith('.woff2') ? 'font/woff2' : 'text/javascript' });
  });
  await page.addInitScript(() => {
    const w = window as any;
    w.previewMessages = []; w.chrome ??= {};
    w.chrome.webview = { postMessage: (message: unknown) => w.previewMessages.push(message),
      addEventListener: (_: string, callback: unknown) => { w.previewReceive = callback; } };
  });
  await page.goto('https://wisp-preview.local/index.html');
  await expect.poll(() => page.evaluate(() => (window as any).previewMessages.some((m: any) => m.type === 'ready'))).toBe(true);
}

test('native PDF renders JPEG2000 figures and embedded ICC colors using only packaged decoders', async ({ page }) => {
  const requests: string[] = [], errors: string[] = [];
  page.context().on('request', request => requests.push(request.url()));
  page.on('pageerror', error => errors.push(error.message));
  await nativePreview(page);
  const base64 = (await readFile(path.resolve('fixtures/pdf-images-jpx-icc.pdf'))).toString('base64');
  await page.evaluate(base64 => (window as any).previewReceive({ data: { kind: 'pdf', base64, locale: 'en', fontSize: 14 } }), base64);
  const canvas = page.locator('.pdf-page > canvas');
  await expect(canvas).toBeVisible();
  const pixel = (x: number, y: number) => canvas.evaluate((canvas: HTMLCanvasElement, point) =>
    Array.from(canvas.getContext('2d')!.getImageData(Math.floor(canvas.width * point.x), Math.floor(canvas.height * point.y), 1, 1).data), { x, y });
  for (const [x, y, rgb] of [
    [.2, .4, [255, 0, 0]], [.5, .4, [0, 255, 0]], [.8, .4, [0, 0, 255]],
    [.2, .7, [0, 255, 255]], [.5, .7, [255, 0, 255]], [.8, .7, [255, 255, 0]],
  ] as [number, number, number[]][]) {
    expect(await pixel(x, y)).toEqual([...rgb, 255]);
  }
  expect(requests).toContain('https://wisp-preview.local/vendor-runtime/openjpeg.wasm');
  await page.getByRole('button', { name: 'Next page', exact: true }).click();
  await expect(page.locator('.pdf-page')).toHaveAttribute('data-page', '2');
  const converted = await pixel(.25, .5), reference = await pixel(.75, .5);
  // A missing ICC worker silently falls back to DeviceRGB (128), so checking
  // merely for a nonblank page would miss this color-management regression.
  for (let i = 0; i < 3; i++) {
    expect(converted[i]).toBeGreaterThanOrEqual(185);
    expect(converted[i]).toBeLessThanOrEqual(191);
    expect(Math.abs(converted[i] - reference[i])).toBeLessThanOrEqual(3);
  }
  expect(converted[3]).toBe(255);
  expect(requests).toContain('https://wisp-preview.local/vendor-runtime/qcms_bg.wasm');
  expect(requests.filter(url => !url.startsWith('https://wisp-preview.local/'))).toEqual([]);
  expect(errors).toEqual([]);
});

test('native formula and PDF viewer renders locally with failure and navigation states', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await nativePreview(page);
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
  const canvas = page.locator('.pdf-page > canvas');
  await expect(canvas).toBeVisible();
  await expect(page.locator('#page')).toHaveText('第 1 页，共 2 页');
  const rasterWidth = await canvas.getAttribute('width');
  const pixel = () => canvas.evaluate((element: HTMLCanvasElement) => Array.from(element.getContext('2d')!.getImageData(element.width * .15, element.height * .7, 1, 1).data));
  expect(await pixel()).toEqual([0, 0, 255, 255]);
  await page.getByRole('button', { name: '下一页' }).click();
  await expect(page.locator('#page')).toHaveText('第 2 页，共 2 页');
  await expect.poll(pixel).toEqual([255, 0, 0, 255]);
  await page.getByRole('button', { name: '放大', exact: true }).click();
  await expect(page.locator('#zoom')).toHaveText('125%');
  await expect(canvas).toHaveAttribute('width', rasterWidth!);
  await send({ kind: 'pdf', base64: pdfFixture(true), canQuote: true });
  const add = page.getByRole('button', { name: '添加到对话', exact: true });
  const jump = page.getByRole('button', { name: '添加到对话并返回对话', exact: true });
  await expect(add).toBeHidden();
  const text = page.locator('.textLayer span').filter({ hasText: 'Alpha selection' });
  await expect(text).toBeVisible();
  // Exercise mouse text selection on the real PDF.js layer, not the canvas.
  await text.dblclick({ position: { x: 8, y: 5 } });
  await expect(add).toBeEnabled();
  const selections = () => page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'selection').length);
  const reported = await selections();
  await page.evaluate(() => document.dispatchEvent(new Event('selectionchange')));
  expect(await selections()).toBe(reported);
  await add.click();
  const quotes = () => page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'quote'));
  expect(await quotes()).toEqual([{ type: 'quote', text: 'Alpha', page: 1, jump: false }]);
  await expect(add).toBeDisabled();
  await send({ type: 'quote-result', accepted: false });
  await expect(page.locator('#status')).toContainText('未能加入聊天');
  await expect(add).toBeEnabled();
  expect((await page.locator('#quote-actions').boundingBox())!.y).toBeGreaterThanOrEqual(
    await page.locator('#content').evaluate(el => el.getBoundingClientRect().top));
  expect(await quotes()).toHaveLength(1);
  await page.keyboard.press('Escape');
  await expect(add).toBeHidden();
  await expect(canvas).toBeVisible();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(0);
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(1);
  await text.dblclick({ position: { x: 8, y: 5 } });
  await send({ type: 'clear-selection' });
  await expect(add).toBeHidden();
  await page.getByRole('button', { name: '下一页' }).click();
  const secondText = page.locator('.textLayer span').filter({ hasText: 'Beta selection' });
  await expect(secondText).toBeVisible();
  await secondText.dblclick({ position: { x: 8, y: 5 } });
  await expect(jump).toBeEnabled();
  await jump.click();
  expect((await quotes()).at(-1)).toEqual({ type: 'quote', text: 'Beta', page: 2, jump: true });
  await send({ type: 'quote-result', accepted: true });
  await expect(page.locator('#status')).toContainText('已加入聊天草稿');
  await expect(jump).toBeHidden();
  await page.getByRole('button', { name: '放大', exact: true }).click();
  await expect(page.locator('#zoom')).toHaveText('125%');
  await expect(canvas).toHaveAttribute('width', rasterWidth!);
  await expect(secondText).toBeVisible();
  await secondText.dblclick({ position: { x: 8, y: 5 } });
  await expect(add).toBeEnabled();
  await page.getByRole('button', { name: '上一页' }).click();
  await expect(text).toBeVisible();
  await expect(add).toBeHidden();
  await send({ kind: 'pdf', base64: pdfFixture(), canQuote: true });
  await expect(page.locator('#status')).toContainText('没有可选择的文字');
  await expect(add).toBeHidden();
  await send({ kind: 'pdf', base64: pdfFixture(true), canQuote: false });
  await expect(add).toBeHidden();
  await send({ kind: 'pdf', base64: Buffer.from('invalid').toString('base64') });
  await expect(page.locator('#status')).toContainText('PDF预览失败');
  expect(errors).toEqual([]);
});

test('native PDF fits width, preserves page and zoom on resize, and supports wheel, pan and reset', async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 400 });
  await nativePreview(page);
  await page.evaluate(base64 => (window as any).previewReceive({ data: { kind: 'pdf', base64, locale: 'en', canQuote: true, fontSize: 14 } }), pdfFixture(true));
  const previous = page.getByRole('button', { name: 'Previous page', exact: true });
  const next = page.getByRole('button', { name: 'Next page', exact: true });
  const smaller = page.getByRole('button', { name: 'Zoom out', exact: true });
  const larger = page.getByRole('button', { name: 'Zoom in', exact: true });
  const reset = page.getByRole('button', { name: 'Reset zoom', exact: true });
  const viewport = page.locator('#content'), paper = page.locator('.pdf-page');
  await expect(paper).toBeVisible(); await expect(previous).toBeDisabled(); await expect(next).toBeEnabled();
  await expect(page.locator('#page')).toHaveText('Page 1 of 2');
  // Native dialog forwards page keys before focus enters the embedded document.
  await page.evaluate(() => (window as any).previewReceive({ data: { type: 'pdf-page', direction: 1 } }));
  await expect(page.locator('#page')).toHaveText('Page 2 of 2');
  // The label updates at render start; native page keys are enabled after
  // the canvas and text layer have finished rendering.
  await expect(previous).toBeEnabled();
  await page.evaluate(() => (window as any).previewReceive({ data: { type: 'pdf-page', direction: -1 } }));
  await expect(page.locator('#page')).toHaveText('Page 1 of 2');
  await expect(next).toBeEnabled();
  const geometry = () => viewport.evaluate(el => ({ available: el.clientWidth, width: el.querySelector('.pdf-page')!.getBoundingClientRect().width, height: el.clientHeight, fullHeight: el.scrollHeight }));
  const initial = await geometry();
  expect(initial.width).toBeCloseTo(initial.available, 0); expect(initial.fullHeight).toBeGreaterThan(initial.height);
  // Whitespace pans even at 100%; text glyphs retain selection behavior below.
  const bounds = (await viewport.boundingBox())!;
  await page.mouse.move(bounds.x + 30, bounds.y + 250); await page.mouse.down();
  await page.mouse.move(bounds.x + 30, bounds.y + 100, { steps: 8 }); await page.mouse.up();
  await expect.poll(() => viewport.evaluate(el => el.scrollTop)).toBeGreaterThan(100);
  await page.mouse.wheel(0, -120); await expect(reset).toHaveText('125%');
  expect((await geometry()).width).toBeCloseTo(initial.available * 1.25, 0);
  await reset.click(); await expect(reset).toHaveText('100%');
  for (let i = 0; i < 3; i++) await smaller.click();
  await expect(reset).toHaveText('25%'); await expect(smaller).toBeDisabled();
  await reset.click();
  for (let i = 0; i < 12; i++) await larger.click();
  await expect(reset).toHaveText('400%'); await expect(larger).toBeDisabled();
  await next.click(); await expect(page.locator('#page')).toHaveText('Page 2 of 2'); await expect(next).toBeDisabled();
  await page.setViewportSize({ width: 380, height: 500 });
  await expect.poll(async () => Math.round((await geometry()).width / (await geometry()).available)).toBe(4);
  await expect(reset).toHaveText('400%'); await expect(page.locator('#page')).toHaveText('Page 2 of 2');
  await reset.click(); await page.keyboard.press('PageUp');
  await expect(page.locator('#page')).toHaveText('Page 1 of 2'); await expect(previous).toBeDisabled();
  const alpha = page.locator('.textLayer span', { hasText: 'Alpha selection' });
  await alpha.dblclick({ position: { x: 8, y: 5 } });
  await expect(page.getByRole('button', { name: 'Add to chat', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape'); await expect(page.getByRole('button', { name: 'Add to chat', exact: true })).toBeHidden();
  await expect(paper).toBeVisible();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(0);
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(1);
});

test('document quick-action preference updates mounted PDFs and retains explicit context actions', async ({ page }) => {
  await nativePreview(page);
  const send = (data: object) => page.evaluate(data => (window as any).previewReceive({ data }), data);
  await send({ kind: 'pdf', base64: pdfFixture(true), locale: 'en', canQuote: true, selectionPopupEnabled: false });
  const paper = page.locator('.pdf-page'), actions = page.locator('#quote-actions');
  const alpha = page.locator('.textLayer span', { hasText: 'Alpha selection' });
  await alpha.dblclick({ position: { x: 8, y: 5 } });
  await expect(actions).toBeHidden();
  await alpha.click({ button: 'right', position: { x: 8, y: 5 } });
  await expect(actions).toBeVisible();
  // One Escape immediately after opening closes only this menu.
  await page.keyboard.press('Escape'); await expect(actions).toBeHidden(); await expect(paper).toBeVisible();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(0);
  await alpha.dblclick({ position: { x: 8, y: 5 } }); await expect(actions).toBeHidden();
  await page.keyboard.press('Shift+F10'); await expect(actions).toBeVisible();
  await page.getByRole('button', { name: 'Add to chat', exact: true }).click();
  await send({ type: 'quote-result', accepted: false });
  await expect(actions).toBeVisible(); await expect(page.getByRole('button', { name: 'Add to chat', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape'); await expect(actions).toBeHidden();
  await alpha.dblclick({ position: { x: 8, y: 5 } });
  const mounted = await paper.elementHandle();
  await send({ type: 'selection-settings', enabled: true }); await expect(actions).toBeVisible();
  await send({ type: 'selection-settings', enabled: false }); await expect(actions).toBeHidden();
  expect(await mounted!.evaluate(node => node.isConnected)).toBe(true);
  expect(await page.evaluate(() => window.getSelection()?.toString())).toContain('Alpha');
  // A disabled, unopened popup is not an Escape layer.
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(1);
});

test('spreadsheet context quoting works with automatic actions disabled', async ({ page }) => {
  await nativePreview(page);
  const base64 = (await readFile(path.resolve('fixtures/office-preview.xlsx'))).toString('base64');
  await page.evaluate(base64 => (window as any).previewReceive({ data: { kind: 'xlsx', base64, locale: 'en', canQuote: true, selectionPopupEnabled: false } }), base64);
  const cell = page.locator('.rp-xlsx-cell', { hasText: /^42$/ });
  const actions = page.locator('#quote-actions');
  await cell.click(); await expect(actions).toBeHidden();
  await cell.click({ button: 'right' }); await expect(actions).toBeVisible();
  await page.getByRole('button', { name: 'Add to chat', exact: true }).click();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'quote').at(-1)))
    .toEqual({ type: 'quote', text: '42', location: { kind: 'xlsx', sheet: 'Results', cells: 'B2' }, jump: false });
  await page.evaluate(() => (window as any).previewReceive({ data: { type: 'quote-result', accepted: true } }));
  await expect(actions).toBeHidden(); await expect(page.locator('.rp-xlsx-cell.selected')).toHaveCount(0);
  await page.keyboard.press('Shift+F10'); await expect(actions).toBeHidden();
});

test('document selection actions float at visible text, stay inside a narrow viewport and dismiss before the preview', async ({ page }) => {
  await page.setViewportSize({ width: 380, height: 320 });
  await nativePreview(page);
  await page.evaluate(base64 => (window as any).previewReceive({ data: {
    kind: 'pdf', base64, locale: 'en', canQuote: true, fontSize: 20,
  } }), pdfFixture(true));
  const actions = page.getByRole('toolbar', { name: 'Selection actions' });
  const paper = page.locator('.pdf-page'), text = page.locator('.textLayer span', { hasText: 'Alpha selection' });
  const before = (await paper.boundingBox())!;
  await expect(actions).toBeHidden();
  await text.dblclick({ position: { x: 8, y: 5 } });
  await expect(actions).toBeVisible();
  expect((await paper.boundingBox())!.y).toBe(before.y);
  const box = (await actions.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(12); expect(box.x + box.width).toBeLessThanOrEqual(368);
  expect(box.y).toBeGreaterThanOrEqual(12); expect(box.y + box.height).toBeLessThanOrEqual(308);
  // Scrolling the selected fragment out of view hides the popup without
  // discarding the selection. Scrolling back restores its anchored position.
  await page.locator('#content').evaluate(el => { el.scrollTop = 250; });
  await expect(actions).toBeHidden();
  await page.locator('#content').evaluate(el => { el.scrollTop = 0; });
  await expect(actions).toBeVisible();
  await page.keyboard.press('Escape'); await expect(actions).toBeHidden(); await expect(paper).toBeVisible();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(0);
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => (window as any).previewMessages.filter((m: any) => m.type === 'escape'))).toHaveLength(1);
});

test('spreadsheet cell selection opens the same floating actions without a permanent toolbar row', async ({ page }) => {
  await page.setViewportSize({ width: 380, height: 360 });
  await nativePreview(page);
  const base64 = (await readFile(path.resolve('fixtures/office-preview.xlsx'))).toString('base64');
  await page.evaluate(base64 => (window as any).previewReceive({ data: { kind: 'xlsx', base64, locale: 'en', canQuote: true, fontSize: 14 } }), base64);
  const actions = page.getByRole('toolbar', { name: 'Selection actions' });
  await expect(page.locator('nav')).toBeHidden(); await expect(actions).toBeHidden();
  const cell = page.locator('.rp-xlsx-cell').filter({ hasText: /\S/ }).first();
  await cell.click(); await expect(actions).toBeVisible();
  const expected = await cell.evaluate(el => ({ text: el.textContent!.trim(), sheet: (el as HTMLElement).dataset.sheet, cells: (el as HTMLElement).dataset.cells }));
  await actions.getByRole('button', { name: 'Add to chat', exact: true }).click();
  const quote = await page.evaluate(() => (window as any).previewMessages.find((message: any) => message.type === 'quote'));
  expect(quote).toEqual({ type: 'quote', text: expected.text, location: { kind: 'xlsx', sheet: expected.sheet, cells: expected.cells }, jump: false });
  await page.evaluate(() => (window as any).previewReceive({ data: { type: 'quote-result', accepted: true } }));
  await expect(actions).toBeHidden(); await expect(page.locator('.rp-xlsx-cell.selected')).toHaveCount(0);
  await cell.click(); await expect(actions).toBeVisible();
  await page.keyboard.press('Escape'); await expect(actions).toBeHidden(); await expect(page.locator('.rp-xlsx')).toBeVisible();
  expect(await page.evaluate(() => (window as any).previewMessages.filter((message: any) => message.type === 'escape'))).toHaveLength(0);
});
