import { expect, test, type Page } from '@playwright/test';

type Verdict = { path: string; allowed: boolean; reason: string };
type ContractError = { code: string; details: { reason: string } };

async function openApp(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => '__sweeperMock' in window);
}

async function verdictFor(page: Page, path: string): Promise<Verdict> {
  return page.evaluate(async (input) => {
    const mock = (window as unknown as { __sweeperMock: { validateCustomPath: (p: string) => Promise<Verdict> } }).__sweeperMock;
    return mock.validateCustomPath(input);
  }, path);
}

async function errorFor(page: Page, path: string): Promise<ContractError | null> {
  return page.evaluate(async (input) => {
    const mock = (window as unknown as { __sweeperMock: { validateCustomPath: (p: string) => Promise<unknown> } }).__sweeperMock;
    try {
      await mock.validateCustomPath(input);
      return null;
    } catch (error) {
      return error as ContractError;
    }
  }, path);
}

test('accepts a valid deep Custom Path', async ({ page }) => {
  await openApp(page);
  expect(await verdictFor(page, 'C:\\Temp\\CustomCleanup\\Cache')).toEqual({
    path: 'C:\\Temp\\CustomCleanup\\Cache',
    allowed: true,
    reason: 'ok',
  });
});

test('rejects Protected Container Roots and their descendants', async ({ page }) => {
  await openApp(page);
  // Bare roots are blocked by the depth guard (evaluated first, as in the backend).
  for (const path of ['C:\\Users', 'C:\\ProgramData']) {
    const verdict = await verdictFor(page, path);
    expect(verdict.allowed, path).toBe(false);
    expect(verdict.reason, path).toContain('depth >= 3');
  }
  // Deep descendants reach the Protected Container Root rule.
  for (const path of [
    'C:\\Users\\Alice\\CustomCleanup',
    'C:\\ProgramData\\Vendor\\Logs',
    'C:\\Windows\\Vendor\\Logs',
  ]) {
    const verdict = await verdictFor(page, path);
    expect(verdict.allowed, path).toBe(false);
    expect(verdict.reason, path).toContain('Protected Path');
  }
});

test('rejects Subtree-Protected Paths', async ({ page }) => {
  await openApp(page);
  const verdict = await verdictFor(page, 'C:\\Temp\\Cache\\.git\\objects');
  expect(verdict.allowed).toBe(false);
  expect(verdict.reason).toContain('Subtree-Protected Path');
});

test('rejects paths shallower than three segments', async ({ page }) => {
  await openApp(page);
  const verdict = await verdictFor(page, 'C:\\Temp\\Cache');
  expect(verdict.allowed).toBe(false);
  expect(verdict.reason).toContain('depth >= 3');
});

test('rejects Link Points and never follows them', async ({ page }) => {
  await openApp(page);
  for (const path of ['C:\\Temp\\Cache\\linkpoint', 'C:\\Temp\\linkpoint\\Cache\\Logs']) {
    const verdict = await verdictFor(page, path);
    expect(verdict.allowed, path).toBe(false);
    expect(verdict.reason, path).toContain('Link Point');
  }
});

test('empty input throws a typed InvalidInput error', async ({ page }) => {
  await openApp(page);
  expect(await errorFor(page, '   ')).toMatchObject({
    code: 'invalidInput',
    details: { reason: 'path is required' },
  });
});

test('Settings surfaces the mirrored rejection reason', async ({ page }) => {
  await openApp(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
  await page.getByLabel('Add Custom Path').fill('C:\\Temp\\Cache\\linkpoint');
  await page.getByRole('button', { name: 'Add Folder' }).click();
  await expect(page.getByRole('alert')).toContainText(/Custom Path rejected.*Link Point/i);
});

test('mirrors backend syntax checks and ASCII-only case folding', async ({ page }) => {
  await openApp(page);

  // ASCII-only folding: `ß` is not folded to `SS`, so OneDriveß is a genuine
  // sync root; U+212A KELVIN SIGN is not folded to `k`, so `.Kube` is not `.kube`.
  const sharpS = await verdictFor(page, 'C:\\Temp\\OneDriveß\\Cache');
  expect(sharpS.allowed).toBe(false);
  expect(sharpS.reason).toContain('Subtree-Protected Path');
  const kelvin = await verdictFor(page, 'C:\\Temp\\.\u212Aube\\Cache');
  expect(kelvin.allowed).toBe(true);
  expect(kelvin.reason).toBe('ok');

  // Malformed syntax is rejected before protection evaluation.
  for (const path of [
    'C:\\Temp\\\\Cache\\Logs',
    'C:\\Temp\\Cache\\Logs\\',
    'C:\\Temp\\.git.\\objects',
    'C:\\Temp\\Cache:stream\\Logs',
  ]) {
    expect((await verdictFor(page, path)).allowed, path).toBe(false);
  }

  // Well-formed mixed-separator paths stay accepted.
  expect(await verdictFor(page, 'C:/Temp\\Cache/Logs')).toEqual({
    path: 'C:\\Temp\\Cache\\Logs',
    allowed: true,
    reason: 'ok',
  });
});
