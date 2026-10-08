// Documentation screenshots for the remote web access tutorial, not a test
// suite. Requires Node 22.13+ and the ui-tests dependencies.
//
// The browser pages are the relay's real remote.html / remote.js, connected to
// a scripted desktop that speaks the sealed-frame protocol with demonstration
// data: no relay, desktop or network is involved. The desktop Settings
// screenshot needs a freshly built UI served at WISP_TUTORIAL_URL and is
// skipped without it.
import { readFileSync, mkdirSync } from 'node:fs';
import { webcrypto } from 'node:crypto';
import { createRequire, stripTypeScriptTypes } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(resolve(root, 'ui-tests/package.json'));
const { chromium } = require('playwright');
const source = name => readFileSync(resolve(root, 'crates/wisp-sync/src', name), 'utf8');
const version = /^version = "([^"]+)"/m.exec(readFileSync(resolve(root, 'Cargo.toml'), 'utf8'))[1];
// The fixed vector from crates/wisp-sync/src/remote.rs.
const CODE = '0001-0203-0405-0607-0809-0a0b-0c0d-0e0f';
const SID = 'cf4778d13d24d0dd1313bca1709267bb';
const PAGE = `http://localhost:8787/remote#${CODE}`;

// A deterministic volcano plot, so the pictures in the article do not change
// between captures.
function volcano() {
  let seed = 7;
  const random = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const normal = () => Math.sqrt(-2 * Math.log(1 - random())) * Math.cos(2 * Math.PI * random());
  const [W, H, L, R, T, B] = [640, 400, 60, 20, 16, 52];
  const x = value => L + ((value + 5) / 10) * (W - L - R);
  const y = value => H - B - (value / 20) * (H - T - B);
  let marks = '';
  for (let i = 0; i < 1100; i++) {
    const change = normal() * 1.35;
    const significance = Math.abs(change) ** 1.5 * (0.5 + random() * 2.3) + random() * 1.1;
    if (Math.abs(change) > 4.8 || significance > 19.5) continue;
    const hit = significance > 1.3 && Math.abs(change) > 1;
    const colour = hit ? (change > 0 ? '#c2410c' : '#0f766e') : '#b8b4aa';
    marks += `<circle cx="${x(change).toFixed(1)}" cy="${y(significance).toFixed(1)}" r="2.6" fill="${colour}" fill-opacity="${hit ? 0.8 : 0.45}"/>`;
  }
  const ticks = [-4, -2, 0, 2, 4].map(v => `<text x="${x(v)}" y="${H - B + 20}" text-anchor="middle">${v}</text>`).join('')
    + [0, 5, 10, 15, 20].map(v => `<text x="${L - 10}" y="${y(v) + 4}" text-anchor="end">${v}</text>`).join('');
  const guide = 'stroke="#8a857a" stroke-dasharray="4 4" stroke-width="1"';
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" font-family="Segoe UI, Helvetica, Arial, sans-serif" font-size="13" fill="#3c3a35">`
    + `<rect width="${W}" height="${H}" fill="#ffffff"/>`
    + `<line x1="${x(-1)}" x2="${x(-1)}" y1="${T}" y2="${H - B}" ${guide}/><line x1="${x(1)}" x2="${x(1)}" y1="${T}" y2="${H - B}" ${guide}/>`
    + `<line x1="${L}" x2="${W - R}" y1="${y(1.3)}" y2="${y(1.3)}" ${guide}/>${marks}`
    + `<path d="M${L} ${T}V${H - B}H${W - R}" fill="none" stroke="#3c3a35" stroke-width="1.2"/>${ticks}`
    + `<text x="${(L + W - R) / 2}" y="${H - 12}" text-anchor="middle" font-size="14">log2 fold change (old vs young)</text>`
    + `<text transform="translate(18 ${(T + H - B) / 2}) rotate(-90)" text-anchor="middle" font-size="14">-log10 FDR</text></svg>`;
  return Buffer.from(svg).toString('base64');
}

// Everything the scripted desktop answers with. Names and numbers are examples.
function demo(zh) {
  const t = (cn, en) => (zh ? cn : en);
  const reply = [
    t('## 差异分析结果', '## Differential expression'),
    '',
    t('把 `batch` 作为协变量重新拟合后，共 **312** 个基因 FDR < 0.05（上调 187，下调 125）。前 5 个基因：',
      'With `batch` as a covariate, **312** genes pass FDR < 0.05 (187 up, 125 down). The top five:'),
    '',
    t('| 基因 | log2FC | FDR |', '| Gene | log2FC | FDR |'),
    '| --- | --- | --- |',
    '| CDKN1A | 2.84 | 1.2e-18 |',
    '| IL6 | 2.31 | 4.5e-15 |',
    '| MMP3 | 2.07 | 8.9e-13 |',
    '| LMNB1 | -1.92 | 3.3e-12 |',
    '| HMGB1 | -1.45 | 7.1e-10 |',
    '',
    t('![火山图](figures/volcano_batch.png)', '![Volcano plot](figures/volcano_batch.png)'),
    '',
    t('完整结果在 [de_results_batch.csv](results/de_results_batch.csv)。下一步可以：',
      'The full table is in [de_results_batch.csv](results/de_results_batch.csv). Next steps:'),
    '',
    t('1. 对上调基因做通路富集', '1. Run pathway enrichment on the up-regulated genes'),
    t('2. 把结论写入 `reports/summary.md`', '2. Write the conclusions to `reports/summary.md`'),
  ].join('\n');
  const plain = { tool_name: null, input: null, ok: null, status: null };
  const session = (id, title, minutes, status) => ({ id, title, ts: 1791400000, activity_at: 1791420000 - minutes * 60, status });
  const file = (name, size) => ({ name, is_dir: false, size });
  const folder = name => ({ name, is_dir: true, size: 0 });
  const data = {
    projects: [
      { id: 'p-1', name: t('RNA-seq 衰老队列', 'RNA-seq aging cohort'), session_count: 3, running_count: 0, needs_you_count: 1 },
      { id: 'p-2', name: t('文献综述', 'Literature review'), session_count: 5, running_count: 1, needs_you_count: 0 },
      { id: 'p-3', name: t('单细胞图谱', 'Single-cell atlas'), session_count: 2, running_count: 0, needs_you_count: 0 },
    ],
    sessions: [
      session('s-1', t('差异分析（batch 协变量）', 'Differential expression (batch covariate)'), 2, 'needs_you'),
      session('s-2', t('质控与归一化', 'QC and normalization'), 190, 'complete'),
      session('s-3', t('样本信息整理', 'Sample sheet cleanup'), 1500, 'complete'),
    ],
    items: [
      { ...plain, role: 'user', text: t('把 batch 作为协变量重跑差异分析，画火山图，列出前 5 个基因。', 'Rerun differential expression with batch as a covariate, draw a volcano plot and list the top five genes.') },
      { ...plain, role: 'tool', tool_name: 'python', input: 'scripts/deseq2_batch.py', text: '312 genes with FDR < 0.05', ok: true, call_id: 'c1' },
      { ...plain, role: 'assistant', text: reply },
    ],
    approvals: [{
      approval_id: 'a1', frame_id: 's-1', tool: 'shell', message: t('运行通路富集脚本', 'Run the pathway enrichment script'),
      preview: 'Rscript scripts/enrichment.R \\\n  --input results/de_results_batch.csv \\\n  --direction up --out results/enrichment_up.csv',
    }],
    directories: {
      '.': [folder('data'), folder('figures'), folder('reports'), folder('results'), folder('scripts'), file('README.md', 1840)],
      figures: [file('pca_batch.png', 96412), file('volcano_batch.png', 182340)],
    },
  };
  const picture = { path: 'thumbnail', mime: 'image/svg+xml', text: null, base64: volcano(), truncated: false, total_bytes: 182340 };
  return (command, project, args) => {
    switch (command) {
      case 'list_projects': return data.projects;
      case 'remote_sessions': return project === 'p-1' ? data.sessions : [];
      case 'native_conversation_snapshot':
        return { items: data.items, approvals: data.approvals, running: false, stopping: false, read_only: false, error: null };
      case 'native_conversation_image': return picture;
      case 'native_conversation_panel_file_directory': return { path: args.path, entries: data.directories[args.path] ?? [] };
      default: return null;
    }
  };
}

// Stands in for relay and desktop on the page's own WebSocket.
async function serveRemote(page, answer) {
  const secret = Uint8Array.from(CODE.replace(/-/g, '').match(/../g), hex => parseInt(hex, 16));
  const raw = await webcrypto.subtle.digest('SHA-256', Buffer.concat([Buffer.from('wisp-remote/key/v1'), secret]));
  const key = await webcrypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']);
  const seal = async value => {
    const iv = webcrypto.getRandomValues(new Uint8Array(12));
    const body = await webcrypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: Buffer.from('wisp-remote/v1/h2c') }, key, Buffer.from(JSON.stringify(value)));
    return Buffer.concat([iv, Buffer.from(body)]).toString('base64');
  };
  await page.route('**/remote', route => route.fulfill({ contentType: 'text/html; charset=utf-8', body: source('remote.html') }));
  await page.route('**/remote.js', route => route.fulfill({ contentType: 'text/javascript; charset=utf-8', body: source('remote.js') }));
  await page.routeWebSocket(new RegExp(`/v1/remote/client/${SID}$`), async socket => {
    const nonce = webcrypto.randomUUID();
    let chain = Promise.resolve();
    socket.onMessage(frame => {
      chain = chain.then(async () => {
        const bytes = Buffer.from(String(frame), 'base64');
        const plain = await webcrypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.subarray(0, 12), additionalData: Buffer.from('wisp-remote/v1/c2h') }, key, bytes.subarray(12));
        const request = JSON.parse(Buffer.from(plain).toString());
        const result = answer(request.command, request.project_id, request.args);
        socket.send(await seal({ type: 'response', response: { schema: 'wisp.native-settings.v1', id: request.id, result, error: null } }));
      });
    });
    socket.send(await seal({ type: 'hello', nonce, name: 'lab-workstation', version }));
  });
}

const browser = await chromium.launch({ headless: true });
try {
  for (const locale of ['zh', 'en']) {
    const zh = locale === 'zh';
    const output = name => {
      const path = resolve(root, 'docs/assets/tutorials', zh ? '' : 'en', 'remote-web', `${name}.png`);
      mkdirSync(dirname(path), { recursive: true });
      console.log(path);
      return path;
    };
    const open = async (viewport, deviceScaleFactor = 1) => {
      const context = await browser.newContext({ viewport, deviceScaleFactor, locale: zh ? 'zh-CN' : 'en-US', timezoneId: 'Asia/Shanghai' });
      const page = await context.newPage();
      page.on('pageerror', error => console.error(error.message));
      page.on('console', message => { if (message.type() === 'error') console.error(message.text()); });
      return page;
    };
    const settle = async page => {
      await page.evaluate(async () => { await document.fonts.ready; });
      await page.mouse.move(0, 0);
      await page.waitForTimeout(300);
    };

    // 01: the desktop side, where the relay is entered and the link appears.
    if (process.env.WISP_TUTORIAL_URL) {
      const page = await open({ width: 1440, height: 1000 });
      const mock = stripTypeScriptTypes(readFileSync(resolve(root, 'ui-tests/tests/mock-tauri.ts'), 'utf8')).replace(/^export /gm, '');
      await page.addInitScript({ content: `${mock}\ntauriMock();` });
      await page.goto(`${process.env.WISP_TUTORIAL_URL}/?mockLocale=${locale}`);
      await page.locator('.proj-card-main').first().click();
      await page.getByRole('button', { name: zh ? '设置' : 'Settings', exact: true }).click();
      await page.getByRole('button', { name: zh ? '远程接入' : 'Remote Access', exact: true }).click();
      await page.getByTestId('remote-channel-row').click();
      await page.getByTestId('remote-relay-url').fill('https://relay.example.com');
      await page.getByTestId('remote-relay-token').fill('demonstration-token');
      await page.getByTestId('remote-enabled-detail').check();
      await page.getByTestId('remote-code').waitFor();
      await settle(page);
      await page.screenshot({ path: output('01-settings'), animations: 'disabled' });
      await page.context().close();
    } else {
      console.log('01-settings skipped: set WISP_TUTORIAL_URL to a served UI build');
    }

    // 02 and 03: the page in a desktop browser.
    const wide = await open({ width: 1440, height: 1000 });
    await serveRemote(wide, demo(zh));
    await wide.goto(`${PAGE}/p/p-1/s/s-1`);
    await wide.locator('#main .img img').waitFor();
    await wide.locator('#main').evaluate(node => (node.scrollTop = 0));
    await settle(wide);
    await wide.screenshot({ path: output('02-conversation'), animations: 'disabled' });
    await wide.goto(`${PAGE}/p/p-1/s/s-1/f/${encodeURIComponent('figures/volcano_batch.png')}`);
    await wide.locator('#panel-body .preview img').waitFor();
    await wide.locator('#main').evaluate(node => (node.scrollTop = 0));
    await settle(wide);
    await wide.screenshot({ path: output('03-files'), animations: 'disabled' });
    await wide.context().close();

    // 04: three phone screens side by side.
    const phone = await open({ width: 390, height: 780 }, 2);
    await serveRemote(phone, demo(zh));
    const screens = [];
    const capture = async () => {
      await settle(phone);
      screens.push((await phone.screenshot({ animations: 'disabled' })).toString('base64'));
    };
    await phone.goto(`${PAGE}/p/p-1`);
    await phone.locator('#sessions a.row').first().waitFor();
    await capture();
    await phone.locator('#sessions a.row').first().click();
    await phone.locator('.approval').waitFor();
    await phone.locator('#main').evaluate(node => (node.scrollTop = node.scrollHeight));
    await capture();
    await phone.locator('#files').click();
    await phone.locator('#panel-body a.row').first().waitFor();
    await capture();
    await phone.context().close();
    const board = await open({ width: 1440, height: 1000 });
    await board.setContent(
      '<body style="margin:0;height:100vh;display:flex;gap:56px;align-items:center;justify-content:center;background:#ece9e2">'
      + screens.map(image => `<img src="data:image/png;base64,${image}" style="height:800px;border:8px solid #1f1e1b;border-radius:30px;box-shadow:0 14px 40px rgba(31,30,27,.2)">`).join('')
      + '</body>');
    await board.screenshot({ path: output('04-phone') });
    await board.context().close();
  }
} finally {
  await browser.close();
}
