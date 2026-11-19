import { expect, test, type Page } from '@playwright/test';

type SweeperMock = {
  setElevationStatus: (status: { isElevated: boolean }) => void;
  setRelaunchResult: (result: { launched: boolean } | { error: { code: string; details: { reason?: string; count?: string } } }) => void;
  reset: () => void;
  startScan?: (tiers: unknown, onProgress?: (event: any) => void) => Promise<{ sessionId: string }>;
  startAnalyzer?: (root: string | null, onProgress?: (event: any) => void) => Promise<any>;
};

declare global {
  interface Window {
    __sweeperMock?: SweeperMock;
  }
}

async function installSlowScanMock(page: Page, holdMs = 1200) {
  await page.waitForFunction(() => typeof window.__sweeperMock?.startScan === 'function');
  await page.evaluate((delayMs) => {
    const mock = window.__sweeperMock as SweeperMock & { startScan: NonNullable<SweeperMock['startScan']> };
    const original = mock.startScan.bind(mock);
    mock.startScan = async (tiers, onProgress) => {
      const result = await original(tiers, onProgress);
      // Hold completion so UI stays in "scanning" state with latest progress visible.
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      return result;
    };
  }, holdMs);
}

async function installSlowAnalyzerMock(page: Page, longPath: string, updates = 6, stepDelayMs = 120) {
  await page.waitForFunction(() => typeof window.__sweeperMock?.startAnalyzer === 'function');
  await page.evaluate(
    ({ deepPath, count, delayMs }) => {
      const mock = window.__sweeperMock as SweeperMock & { startAnalyzer: NonNullable<SweeperMock['startAnalyzer']> };
      const original = mock.startAnalyzer.bind(mock);
      const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
      mock.startAnalyzer = async (root, onProgress) => {
        onProgress?.({ sessionId: 'mock-analyzer-pending', scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: deepPath });
        await sleep(delayMs);
        for (let i = 1; i <= count; i += 1) {
          onProgress?.({
            sessionId: 'mock-analyzer-pending',
            scannedDirs: String(i),
            scannedFiles: String(i * 10),
            scannedBytes: String(i * 4096),
            currentPath: `${deepPath}\\segment-${i}`,
          });
          await sleep(delayMs);
        }
        return original(root, onProgress);
      };
    },
    { deepPath: longPath, count: updates, delayMs: stepDelayMs },
  );
}

test.describe('Sweeper Phase 5 (mock sandbox)', () => {
  test('shows non-elevated locked state and Run as administrator feedback', async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __SWEEPER_MOCK_INIT__?: unknown }).__SWEEPER_MOCK_INIT__ = {
        relaunchResult: { error: { code: 'internal', details: { reason: 'Administrator launch was cancelled.' } } },
      };
    });
    await page.goto('/');
    await expect(page.getByLabel('Standard (Non-Elevated)')).toBeVisible();
    await expect(page.getByRole('button', { name: /Standard Mode/i }).first()).toBeVisible();
    await page.getByRole('button', { name: /Standard Mode/i }).first().click();
    await expect(page.getByRole('alert')).toContainText(/Administrator launch was cancelled/i);
  });

  test('unlocks elevation targets settings flow when elevated', async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __SWEEPER_MOCK_INIT__?: unknown }).__SWEEPER_MOCK_INIT__ = {
        elevationStatus: { isElevated: true },
      };
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await expect(page.getByLabel('Administrator (Elevated)').first()).toBeVisible();

    await page.getByLabel('Enable deep cleanup categories').check();
    const elevationToggle = page.getByLabel(
      'Allow system-wide Windows caches (Admin)',
    );
    await expect(elevationToggle).toBeEnabled();
    await elevationToggle.check();
    await page.getByRole('button', { name: 'Save Settings' }).first().click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();
  });
  test('Advanced Settings: Tier 3 toggle, min-age warning, Custom Path add/reject/remove', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();

    const minAge = page.getByLabel('Minimum file age');
    await minAge.fill('0');
    await expect(page.getByText(/below 24 hours/i)).toBeVisible();

    const tier3 = page.getByLabel('Enable deep cleanup categories');
    await expect(tier3).not.toBeChecked();
    const staleDays = page.getByLabel('Inactive project cache age');
    await expect(staleDays).toBeDisabled();
    await tier3.check();
    await expect(staleDays).toBeEnabled();
    await staleDays.fill('45');
    await expect(page.getByText(/Highlight inactive Rust build caches/i).first()).toBeVisible();
    await expect(page.getByText(/Empty Windows Recycle Bin/i).first()).toBeVisible();

    await page.getByLabel('Add Custom Path').fill('C:\\Windows\\Temp\\bad');
    await page.getByRole('button', { name: 'Add Folder' }).click();
    await expect(page.getByRole('alert')).toContainText(/Custom Path rejected/i);

    await page.getByLabel('Add Custom Path').fill('C:\\Temp\\CustomCleanup\\Cache');
    await page.getByRole('button', { name: 'Add Folder' }).click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();
    await expect(page.getByText('C:\\Temp\\CustomCleanup\\Cache')).toBeVisible();

    await page.getByRole('button', { name: 'Remove' }).click();
    await expect(page.getByText('C:\\Temp\\CustomCleanup\\Cache')).toHaveCount(0);
  });

  test('Analyzer Unknown gating and typed confirmation end-to-end', async ({ page }) => {
    await page.goto('/');
    // Enable allow_delete_unknown
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByLabel('Allow deleting unknown folders (advanced)').check();
    await page.getByRole('button', { name: 'Save Settings' }).first().click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();

    await page.getByRole('button', { name: 'Analyzer' }).click();
    await page.getByRole('button', { name: 'Analyze', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Analyzer Tree' })).toBeVisible();

    const unknown = page.getByRole('checkbox', { name: /MysteryApp\\MysteryCache/ }).first();
    await expect(unknown).toBeEnabled();
    await unknown.check();

    await expect(page.getByLabel('Typed confirmation')).toBeVisible();
    await page.getByLabel('Typed confirmation').fill('delete');
    await expect(page.getByRole('button', { name: 'Review & Clean' })).toBeDisabled();
    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Review & Clean' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
    await expect(page.getByText(/Permanent deletion \(bypasses Recycle Bin\)/)).toBeVisible();

    await page.getByRole('button', { name: /Delete .* permanently/ }).click();
    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
  });

  test('Scan Custom Path/Unknown item requires typed confirmation when allowed', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByLabel('Allow deleting unknown folders (advanced)').check();
    await page.getByRole('button', { name: 'Save Settings' }).first().click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();

    await page.getByRole('button', { name: 'Scan & Clean' }).click();
    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible();
    const customGroup = page.getByRole('article').filter({ hasText: 'Custom Path (Unknown)' }).first();
    await expect(customGroup).toBeVisible();
    const expandToggle = customGroup.getByRole('button', { name: 'Expand Custom Path (Unknown)' });
    await expandToggle.click();
    await expect(customGroup.getByRole('button', { name: 'Collapse Custom Path (Unknown)' })).toBeVisible();
    const customItemCheckbox = customGroup.getByLabel('Select C:\\Users\\Alice\\CustomCleanup\\old.bin');
    await expect(customItemCheckbox).toBeEnabled({ timeout: 15000 });
    await customItemCheckbox.check();
    await expect(page.getByLabel('Typed confirmation')).toBeVisible();
    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Review & Clean' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
  });

  test('Dark/Light theme toggle persists in localStorage', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Start Scan' })).toBeVisible();
    await expect.poll(async () => page.locator('html').getAttribute('data-theme')).toBe('dark');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    const appearance = page.getByRole('group', { name: 'Appearance' });
    await appearance.getByRole('button', { name: 'Light' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await expect.poll(() => page.evaluate(() => localStorage.getItem('sweeper-theme'))).toBe('light');
    await page.reload();
    await expect(page.getByRole('button', { name: 'Start Scan' })).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('group', { name: 'Appearance' }).getByRole('button', { name: 'Dark' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect.poll(() => page.evaluate(() => localStorage.getItem('sweeper-theme'))).toBe('dark');
  });

  test('Info tooltip opens on interaction and closes on Escape', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Start Scan' })).toBeVisible();
    const tip = page.getByRole('tooltip').filter({ hasText: /Windows Temp, crash dumps & app caches older than 24h/i });
    const infoBtn = page.getByRole('button', { name: 'About System & App Temp (Tier 1)' });
    await infoBtn.click();
    if (!(await tip.isVisible().catch(() => false))) {
      await infoBtn.focus();
      await page.keyboard.press('Enter');
    }
    await expect(tip).toBeVisible();
    await expect(tip).toContainText(/Windows Temp, crash dumps & app caches older than 24h/i);
    await page.keyboard.press('Escape');
    await expect(tip).toBeHidden();
  });

  test('Bug 2: scan progress transitions with live status/counts/bytes and completes', async ({ page }) => {
    await page.goto('/');
    await installSlowScanMock(page, 1500);
    await page.getByRole('button', { name: 'Start Scan' }).click();

    const progressNotice = page.locator('.scan-progress-notice');
    await expect(progressNotice).toBeVisible({ timeout: 10_000 });
    await expect(progressNotice).toContainText(/Scan|Scanning/i);
    await expect.poll(async () => (await progressNotice.textContent()) ?? '', { timeout: 10_000 }).toMatch(/\d+\s*\/\s*\d+\s*Targets/);
    await expect.poll(async () => (await progressNotice.textContent()) ?? '', { timeout: 10_000 }).toMatch(/items/i);
    await expect.poll(async () => (await progressNotice.textContent()) ?? '', { timeout: 10_000 }).toMatch(/B|KB|MB|GB|bytes/i);

    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('Ready to scan')).toHaveCount(0);
    await expect(page.locator('.scan-progress-notice')).toHaveCount(0);
  });

  test('Bug 4: scan results persist when navigating Settings -> Scan & Preview', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('Ready to scan')).toHaveCount(0);
    const categoryCount = await page.locator('article.target-card').count();
    expect(categoryCount).toBeGreaterThan(0);

    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await page.getByRole('button', { name: /Scan & (Clean|Preview)/ }).click();

    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible();
    await expect(page.getByText('Ready to scan')).toHaveCount(0);
    await expect(page.locator('article.target-card')).toHaveCount(categoryCount);
  });

  test('Bug 4 + 3a: analyzer results persist across nav and progress/layout do not overflow horizontally', async ({ page }) => {
    await page.goto('/');
    const longDeepPath =
      'C:\\Users\\Alice\\AppData\\Local\\Packages\\VeryLongVendorName\\Deep\\Nested\\Component\\Cache\\' +
      'Extremely-Long-Directory-Name-For-Overflow-Testing-0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ';
    await installSlowAnalyzerMock(page, longDeepPath, 8, 120);

    await page.getByRole('button', { name: 'Analyzer' }).click();
    await page.getByRole('button', { name: 'Analyze', exact: true }).click();

    const progressNotice = page.locator('.analyzer-progress-notice');
    await expect(progressNotice).toBeVisible();
    await expect(progressNotice).toContainText(/dirs\s+\d+/i);
    await expect(progressNotice).toContainText(/files\s+\d+/i);
    await expect(progressNotice).toContainText(/Current:/i);

    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            const notice = document.querySelector('.analyzer-progress-notice') as HTMLElement | null;
            const layout = document.querySelector('.analyzer-layout') as HTMLElement | null;
            const root = document.documentElement;
            return {
              noticeOk: !notice || notice.scrollWidth <= notice.clientWidth + 1,
              layoutOk: !layout || layout.scrollWidth <= layout.clientWidth + 1,
              pageOk: root.scrollWidth <= root.clientWidth + 1,
              noticeSeen: !!notice,
            };
          }),
        { timeout: 10_000 },
      )
      .toEqual({ noticeOk: true, layoutOk: true, pageOk: true, noticeSeen: true });

    await expect(page.getByRole('tree', { name: 'Analyzer results tree' })).toBeVisible({ timeout: 30_000 });
    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            const layout = document.querySelector('.analyzer-layout') as HTMLElement | null;
            const root = document.documentElement;
            return {
              layoutOk: !layout || layout.scrollWidth <= layout.clientWidth + 1,
              pageOk: root.scrollWidth <= root.clientWidth + 1,
            };
          }),
      )
      .toEqual({ layoutOk: true, pageOk: true });

    const rowCount = await page.locator('.analyzer-row[role="treeitem"]').count();
    expect(rowCount).toBeGreaterThan(0);

    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await page.getByRole('button', { name: 'Analyzer' }).click();

    await expect(page.getByRole('tree', { name: 'Analyzer results tree' })).toBeVisible();
    await expect(page.locator('.analyzer-row[role="treeitem"]')).toHaveCount(rowCount);
    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            const layout = document.querySelector('.analyzer-layout') as HTMLElement | null;
            const root = document.documentElement;
            return {
              layoutOk: !layout || layout.scrollWidth <= layout.clientWidth + 1,
              pageOk: root.scrollWidth <= root.clientWidth + 1,
            };
          }),
      )
      .toEqual({ layoutOk: true, pageOk: true });
  });
});