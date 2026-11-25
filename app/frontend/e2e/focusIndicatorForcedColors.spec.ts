import { expect, test, type Page } from '@playwright/test';

/**
 * Real-browser coverage for issue #139 (mock sandbox only, no real C: paths and
 * no deletion). Playwright uses the existing system Edge channel from
 * playwright.config.ts; no browser is downloaded.
 *
 * Forced-colors (Windows High Contrast) suppresses box-shadow. The base focus
 * rule removes the UA outline, so without an explicit forced-colors override a
 * keyboard-focused control has no visible indicator at all.
 */
type FocusedOutline = {
  focusVisible: boolean;
  outlineStyle: string;
  outlineWidth: string;
};

async function readFocusedOutline(page: Page): Promise<FocusedOutline | null> {
  return page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (!el) return null;
    const style = getComputedStyle(el);
    return {
      focusVisible: el.matches(':focus-visible'),
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
    };
  });
}

test.describe('Focus indicator under forced-colors (issue #139, mock sandbox, system Edge)', () => {
  test('keyboard focus keeps a visible 2px outline when box-shadow is suppressed', async ({ page }) => {
    await page.emulateMedia({ forcedColors: 'active' });
    await page.goto('/');

    // Wait for the mock shell to render, then move focus with the keyboard so
    // :focus-visible applies to the focused control.
    await expect(page.getByRole('button', { name: 'Analyzer' })).toBeVisible({ timeout: 30_000 });
    await page.keyboard.press('Tab');
    await expect.poll(async () => (await readFocusedOutline(page))?.focusVisible).toBe(true);

    const outline = await readFocusedOutline(page);
    expect(outline).not.toBeNull();
    expect(outline!.outlineStyle).toBe('solid');
    expect(parseFloat(outline!.outlineWidth)).toBeGreaterThanOrEqual(2);
  });
});
