import { test, expect, type Page } from '@playwright/test';
import { tauriMock } from './mock-tauri';

const files = {
  'ethanol.smi': 'CCO',
  'ethanol.cif': 'data_ethanol\nloop_\n_atom_site.group_PDB\n_atom_site.id\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\nHETATM 1 C C1 LIG A 1 0 0 0\nHETATM 2 C C2 LIG A 1 1.5 0 0\nHETATM 3 O O1 LIG A 1 2.5 1 0\n#\n',
  'aligned.sto': '# STOCKHOLM 1.0\nfirst AC.G\nsecond ACTG\n//\n',
  'truncated.afa': '>first\nAC-G\n>second\nACTG\n',
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
        if (name in files) return { path, mime: 'text/plain', text: files[name as keyof typeof files], base64: null, truncated: name === 'truncated.afa' };
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

test('WebView file journeys render molecules, actual mmCIF atoms and aligned residues, and release closed viewers', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'ethanol.smi');
  await expect(page.locator('.center-file-preview svg.rp-molecule path')).not.toHaveCount(0);
  await openCenter(page, 'ethanol.cif');
  const structure = page.frameLocator('.center-file-preview iframe.rp-3dmol');
  await expect(structure.locator('#status')).toContainText('3 atoms');
  await expect(structure.locator('body')).toHaveAttribute('data-ready', 'true');
  const detached = page.frames().find(frame => frame.url().endsWith('/structure-preview.html'))!;
  // Keep the actual mount node to verify parent-side disposal, not just iframe removal.
  await page.evaluate(() => { (window as any).previousScientific = document.querySelector('.center-file-preview .rp-heavy'); });
  await openCenter(page, 'aligned.sto');
  await expect.poll(() => detached.isDetached()).toBe(true);
  await expect.poll(() => page.evaluate(() => typeof (window as any).previousScientific.__wispPreviewCleanup)).toBe('undefined');
  const alignment = page.frameLocator('.center-file-preview iframe.rp-alignment');
  await expect(alignment.locator('body')).toHaveAttribute('data-ready', 'true');
  await expect(alignment.locator('msa-labels')).toContainText('first');
  await expect.poll(() => alignment.locator('msa-sequence-viewer canvas:visible').evaluate((canvas: HTMLCanvasElement) => {
    const pixels = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
    let colored = 0; for (let i = 0; i < pixels.length; i += 4) if (pixels[i + 3] && pixels[i] !== pixels[i + 1]) colored++;
    return colored;
  })).toBeGreaterThan(10);
  const alignedFrame = page.frames().find(frame => frame.url().endsWith('/alignment-preview.html'))!;
  await page.locator('.center-tab-wrap:has(.center-tab.active) .center-tab-close').click();
  await expect.poll(() => alignedFrame.isDetached()).toBe(true);
});

test('WebView rejects truncated scientific data before presenting an apparently complete alignment', async ({ page }) => {
  await setup(page);
  await openCenter(page, 'truncated.afa');
  await expect(page.locator('.center-file-preview .rp-error')).toContainText('Incomplete file');
  await expect(page.locator('.center-file-preview iframe')).toHaveCount(0);
});
