import net from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';

export {
  assertSweeperWindowVisible,
  cleanupTemp,
  killProcessTree,
  resolveDesktopExePath,
  spawnDesktop,
  waitForExit,
} from './test-desktop-e2e.win.mjs';

export function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function freePortServer() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server));
  });
}

function closePortServer(server) {
  return new Promise((resolve, reject) => {
    server.close((err) => {
      if (err) reject(err);
      else resolve();
    });
  });
}

export async function getFreePort() {
  const server = await freePortServer();
  const address = server.address();
  await closePortServer(server);
  if (!address || typeof address === 'string') throw new Error('Failed to allocate a TCP port');
  return address.port;
}

export function uiEval() {
  if (document.title.includes('Sweeper')) return true;
  const text = document.body?.innerText || '';
  if (text.includes('Scan')) return true;
  if (text.includes('Analyzer')) return true;
  return text.includes('Settings');
}

export function probeEval() {
  const state = { maxDelta: 0, last: performance.now(), running: true, samples: 0 };
  window.__sweeperUiProbe = state;
  function tick() {
    if (!state.running) return;
    const now = performance.now();
    const delta = now - state.last;
    if (delta > state.maxDelta) state.maxDelta = delta;
    state.last = now;
    state.samples += 1;
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);
}

export function scanReadyEval() {
  const txt = (el) => String(el.textContent || '');
  if (document.querySelector('.scan-progress-notice')) return false;
  const alerts = Array.from(document.querySelectorAll('[role="alert"]')).map(txt);
  if (alerts.some((t) => /scan failed|session expired|invalid/i.test(t))) return false;
  const headings = Array.from(document.querySelectorAll('h3')).map(txt);
  if (!headings.some((t) => t.includes('Inspector'))) return false;
  const hasResults = document.querySelectorAll('article.target-card').length > 0;
  const noTargets = headings.some((t) => t.trim() === 'No targets');
  if (!hasResults && !noTargets) return false;
  const ready = Array.from(document.querySelectorAll('h3, p, span, div')).map(txt);
  return !ready.some((t) => t.trim() === 'Ready to scan');
}

export function readScanEval() {
  const txt = (el) => String(el.textContent || '');
  const headings = Array.from(document.querySelectorAll('h3')).map(txt);
  const ready = Array.from(document.querySelectorAll('h3, p, span, div')).map(txt);
  return {
    hasResults: document.querySelectorAll('article.target-card').length > 0,
    noTargets: headings.some((t) => t.trim() === 'No targets'),
    inspector: headings.some((t) => t.includes('Inspector')),
    readyVisible: ready.some((t) => t.trim() === 'Ready to scan'),
  };
}

export function stopProbeEval() {
  const state = window.__sweeperUiProbe;
  if (!state) return null;
  state.running = false;
  return { maxDelta: state.maxDelta, samples: state.samples };
}

export async function safeEval(page, fn) {
  try { return await page.evaluate(fn); } catch { return false; }
}

async function cdpOk(port) {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/json/version`);
    return response.ok;
  } catch { return false; }
}

export async function waitForCdp(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && !(await cdpOk(port))) await delay(250);
  if (await cdpOk(port)) return;
  throw new Error(`Timed out waiting for WebView2 CDP endpoint on port ${port}`);
}

function relevantUrl(url) {
  if (!url) return false;
  if (url === 'about:blank') return false;
  if (url.includes('tauri-isolation')) return false;
  return !url.includes('/isolation/');
}

function browserPages(browser) {
  return browser.contexts().flatMap((context) => context.pages()).filter((page) => relevantUrl(page.url()));
}

async function pageIsSweeper(page) {
  return Boolean(await safeEval(page, uiEval));
}

async function findSweeperPageOnce(browser) {
  const pages = browserPages(browser);
  let i = 0;
  while (i < pages.length && !(await pageIsSweeper(pages[i]))) i += 1;
  return pages[i] ?? null;
}

export async function findSweeperPage(browser, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let page = await findSweeperPageOnce(browser);
  while (Date.now() < deadline && !page) {
    await delay(250);
    page = await findSweeperPageOnce(browser);
  }
  if (page) return page;
  throw new Error('Could not locate the main Sweeper WebView2 page over CDP');
}

export async function verifyBasicNavigation(page) {
  await page.waitForLoadState('domcontentloaded', { timeout: 30_000 });
  await page.getByRole('button', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Analyzer' }).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('heading', { name: 'Settings' }).waitFor({ timeout: 30_000 });
  await page.getByText(/Standard \(Non-Elevated\)|Administrator \(Elevated\)/).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Scan & Clean' }).click();
  await page.getByRole('heading', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Start Scan' }).waitFor({ timeout: 30_000 });
}

export async function detectScanProgress(page) {
  try {
    await page.locator('.scan-progress-notice, [aria-label="Scan target progress"]').first().waitFor({ timeout: 5_000 });
    return true;
  } catch { return false; }
}

export async function runScanFlow(page) {
  await page.getByRole('button', { name: 'Start Scan' }).click();
  return detectScanProgress(page);
}

export async function verifyStatePreserved(page, initialScanState) {
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.getByRole('heading', { name: 'Settings' }).waitFor({ timeout: 30_000 });
  await page.getByRole('button', { name: 'Scan & Clean' }).click();
  await page.getByRole('heading', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
  const preserved = await page.evaluate(readScanEval);
  assert(preserved.hasResults === initialScanState.hasResults && preserved.noTargets === initialScanState.noTargets && preserved.inspector && !preserved.readyVisible, `Scan state was not preserved after navigating Settings -> Scan & Clean (initial=${JSON.stringify(initialScanState)}, preserved=${JSON.stringify(preserved)})`);
}

export async function closeBrowser(browser) {
  try { await browser.close(); } catch { /* best effort */ }
}