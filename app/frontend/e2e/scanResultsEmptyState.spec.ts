import { expect, test } from '@playwright/test';

/**
 * Red-capable E2E repro for spec #58 R2 / ticket #60.
 *
 * At a tall viewport in the light theme, a Scan that returns only a handful of
 * synthetic targets used to leave the lower results pane reading as a broken
 * empty white block (blankRatio ~72% at 1440x1200). This test drives only the
 * browser mock backend with fictional fixtures (Alice / item-N); it performs no
 * deletion and commits no screenshots. Playwright uses the existing system
 * Edge channel from playwright.config.ts.
 */
const HINT_NAME = 'Scan results complete';
const HINT_TEXT = 'That\'s everything found';

test.describe('Scan results empty-state (mock sandbox, light theme, tall viewport)', () => {
  test.use({ viewport: { width: 1440, height: 1200 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      try {
        window.localStorage.setItem('sweeper-theme', 'light');
      } catch {
        // Storage may be unavailable on an opaque origin; the app falls back.
      }
    });
  });

  test('a sparse Scan shows a persistent completion hint inside the full-height results pane', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');

    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText('Ready to scan')).toHaveCount(0);

    const list = page.locator('.scan-results-shell .target-list');
    await expect(list).toBeVisible();
    const rowCount = await list.getByRole('article').count();
    expect(rowCount).toBeGreaterThan(0);
    expect(rowCount).toBeLessThan(8); // sparse result set

    const hint = page.getByRole('status', { name: HINT_NAME });
    await expect(hint).toBeVisible();
    await expect(hint).toContainText(HINT_TEXT);
    await expect(list.locator(`[role="status"][aria-label="${HINT_NAME}"]`)).toHaveCount(1);

    // The hint is anchored inside the tall results pane and stays there
    // (persistent), so the lower pane no longer reads as a dead white block.
    const box = await hint.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height).toBeLessThanOrEqual(1200);
    await page.waitForTimeout(200);
    // Full intersection, not just CSS visibility: the hint must not be clipped
    // below the results pane's scrollport.
    await expect(hint).toBeInViewport({ ratio: 1 });
  });
});
