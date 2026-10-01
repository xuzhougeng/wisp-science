import {test,expect,type Page} from '@playwright/test';
import {tauriMock} from './mock-tauri';
import { openSidebarEntry } from './sidebar-nav';

async function open(page:Page,locale='en') {
  await page.addInitScript(tauriMock);
  await page.goto(`/?mockLocale=${locale}&mockBranches=1`);
  await page.locator('.proj-card-main').first().click();
  await page.locator('.sidebar [data-session-id]').first().click();
  await expect(page.getByTestId('archive-topbar')).toBeEnabled();
}

test('archive review edits survive typing, confirmation locks the notebook, journey can continue research',async({page})=>{
  await open(page);
  await page.getByTestId('archive-topbar').click();
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByTestId('archive-title')).toHaveValue('Root cell annotation');
  await expect(dialog.getByTestId('archive-confirm')).toBeDisabled();
  await dialog.getByTestId('archive-title').fill('Reviewed research');
  await dialog.getByTestId('archive-title').pressSequentially(' milestone');
  await expect(dialog.getByTestId('archive-title')).toHaveValue('Reviewed research milestone');
  await dialog.getByTestId('archive-consent').check();
  await dialog.getByTestId('archive-report').fill('The selected result and known limitations.');
  await expect(dialog.getByTestId('archive-confirm')).toBeDisabled();
  await dialog.getByTestId('archive-consent').check();
  await dialog.getByTestId('archive-confirm').click();
  await expect(dialog.getByRole('heading',{name:'Archived milestone',exact:true})).toBeVisible();
  await expect(dialog.getByTestId('archive-title')).not.toBeEditable();
  await dialog.getByRole('button',{name:'Open saved material',exact:true}).click();
  await expect(page.locator('.artifact-modal')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('.artifact-modal')).toHaveCount(0);
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('archive-readonly')).toBeVisible();
  await expect(page.locator('#composer-input')).toBeDisabled();
  await openSidebarEntry(page,'Research journey');
  await page.getByTestId('research-journey').getByRole('button',{name:/Reviewed research milestone/}).first().click();
  await page.getByTestId('journey-open-archive').click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('research-journey')).toBeVisible();
  await page.getByTestId('journey-open-archive').click();
  await dialog.getByRole('button',{name:'Continue research',exact:true}).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId('research-journey')).toHaveCount(0);
  await expect(page.locator('#composer-input')).toBeEnabled();
});

test('prepare output-limit error is localized and leaves the notebook editable',async({page})=>{
  await open(page,'zh');
  await page.evaluate(()=>{(window as any).__archivePrepareError="response ended with status 'incomplete' (max_output_tokens)";});
  await page.getByTestId('archive-topbar').click();
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByRole('alert')).toContainText('输出额度');
  await expect(dialog.getByRole('alert')).not.toContainText('incomplete');
  await expect(dialog.getByTestId('archive-confirm')).toBeDisabled();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('#composer-input')).toBeEnabled();
});

test('slash archive opens review; changed-file failure keeps the draft editable',async({page})=>{
  await open(page);
  await page.locator('#composer-input').fill('/archive');
  await page.locator('#composer-input').press('Enter');
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByTestId('archive-title')).toBeVisible();
  await page.evaluate(()=>{(window as any).__archiveFailure=true;});
  await dialog.getByTestId('archive-consent').check();
  await dialog.getByTestId('archive-confirm').click();
  await expect(dialog.getByRole('alert')).toContainText('File changed');
  await expect(dialog.getByTestId('archive-title')).toBeEditable();
  await page.keyboard.press('Escape');
  await expect(page.locator('#composer-input')).toBeEnabled();
});

test('Chinese review fits narrow layouts and protects shared materials',async({page},testInfo)=>{
  await open(page,'zh');
  await page.getByTestId('archive-topbar').click();
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByTestId('archive-title')).toBeVisible();
  const protectedFile=dialog.getByTestId('archive-file').filter({hasText:'results/final.csv'});
  await expect(protectedFile.locator('option[value=delete]')).toBeDisabled();
  const scratch=dialog.getByTestId('archive-file').filter({hasText:'scratch/trial.rds'});
  await scratch.locator('select').selectOption('reference');
  await expect(dialog.getByTestId('archive-delete-total')).toContainText('0 bytes');
  for(const width of [1488,760,390]) {
    await page.setViewportSize({width,height:900});
    const bounds=(await dialog.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x+bounds.width).toBeLessThanOrEqual(width);
    expect(await dialog.evaluate(e=>e.scrollWidth<=e.clientWidth+1)).toBe(true);
    await expect(dialog.getByTestId('archive-confirm')).toBeInViewport();
    await page.screenshot({path:testInfo.outputPath(`archive-${width}.png`)});
  }
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
});

test('archival from another window locks the active notebook without locking a different conversation',async({page})=>{
  await open(page);
  await page.evaluate(()=>{(window as any).__tauriEmitToOtherWindow('research-archived',{frame_id:'conversation-main',project_id:'default'});});
  await expect(page.locator('#composer-input')).toBeDisabled();
  await expect(page.getByTestId('archive-readonly')).toBeVisible();
  await page.locator('.sidebar [data-session-id="conversation-branch"]').click();
  await expect(page.locator('#composer-input')).toBeEnabled();
  await page.locator('.sidebar [data-session-id="conversation-main"]').click();
  await expect(page.locator('#composer-input')).toBeDisabled();
});

test('archive review minimizes while preparing and restores the finished draft',async({page})=>{
  await open(page);
  await page.evaluate(()=>{(window as any).__archivePrepareDelay=2500;});
  await page.getByTestId('archive-topbar').click();
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByRole('status')).toContainText('Preparing and saving research materials');
  await page.getByTestId('archive-minimize').click();
  await expect(dialog).toHaveCount(0);
  const pill=page.getByTestId('archive-pill');
  await expect(pill).toBeVisible();
  await expect(page.locator('#composer-input')).toBeEnabled();
  await page.locator('.sidebar [data-session-id="conversation-branch"]').click();
  await expect(page.locator('#composer-input')).toBeEnabled();
  await expect(pill).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(pill).toBeVisible();
  await expect(pill).toContainText('Archive ready for review');
  await page.getByTestId('archive-pill-restore').click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId('archive-title')).toHaveValue('Root cell annotation');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(pill).toHaveCount(0);
});

test('minimized archive finishes confirmation in the background and locks the archived notebook',async({page})=>{
  await open(page);
  await page.evaluate(()=>{(window as any).__archiveConfirmDelay=2500;});
  await page.getByTestId('archive-topbar').click();
  const dialog=page.getByTestId('archive-review');
  await expect(dialog.getByTestId('archive-title')).toHaveValue('Root cell annotation');
  await dialog.getByTestId('archive-consent').check();
  await dialog.getByTestId('archive-confirm').click();
  await page.getByTestId('archive-minimize').click();
  const pill=page.getByTestId('archive-pill');
  await expect(dialog).toHaveCount(0);
  await expect(pill).toBeVisible();
  await page.locator('.sidebar [data-session-id="conversation-branch"]').click();
  await expect(page.locator('#composer-input')).toBeEnabled();
  await expect(pill).toContainText('Archived milestone');
  await page.locator('.sidebar [data-session-id="conversation-main"]').click();
  await expect(page.locator('#composer-input')).toBeDisabled();
  await expect(page.getByTestId('archive-readonly')).toBeVisible();
  await page.getByTestId('archive-pill-restore').click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('heading',{name:'Archived milestone',exact:true})).toBeVisible();
});
