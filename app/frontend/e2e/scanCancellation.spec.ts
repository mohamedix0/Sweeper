import { expect, test, type Page } from '@playwright/test';

// The mock backend runs in the browser, so a native window close is not
// producible here. The Rust `CloseRequested` hook itself is covered by Gate F;
// this spec mirrors that hook through the mock's `cancelActiveScans` control
// and asserts the webview reflects the cancellation visibly.

type MockWithScanControls = {
  setScanStepDelayMs: (ms: number) => void;
  cancelActiveScans: () => void;
};

async function mockControls(page: Page) {
  return page.evaluateHandle(() => {
    const w = window as unknown as { __sweeperMock: MockWithScanControls };
    return w.__sweeperMock;
  });
}

async function slowScan(page: Page, stepDelayMs: number) {
  const mock = await mockControls(page);
  await mock.evaluate((controls, delay) => controls.setScanStepDelayMs(delay), stepDelayMs);
  await mock.dispose();
}

test('Stop cancels a running Scan visibly', async ({ page }) => {
  await page.goto('/');
  await slowScan(page, 150);
  await page.getByRole('button', { name: 'Start Scan' }).click();

  const stop = page.getByRole('button', { name: 'Stop Scan' });
  await expect(stop).toBeVisible();
  await stop.click();

  await expect(page.getByRole('alert')).toContainText('Scan cancelled.');
  await expect(stop).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Start Scan' })).toBeVisible();
});

test('a close-request cancellation is reflected in the open webview', async ({ page }) => {
  await page.goto('/');
  await slowScan(page, 150);
  await page.getByRole('button', { name: 'Start Scan' }).click();
  await expect(page.getByRole('button', { name: 'Stop Scan' })).toBeVisible();

  await page.evaluate(() => {
    const w = window as unknown as { __sweeperMock: MockWithScanControls };
    w.__sweeperMock.cancelActiveScans();
    window.dispatchEvent(new CustomEvent('sweeper:close-requested'));
  });

  await expect(page.getByRole('alert')).toContainText('Scan cancelled.');
  await expect(page.getByRole('button', { name: 'Stop Scan' })).toHaveCount(0);
});
