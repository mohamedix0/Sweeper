import { chromium } from '@playwright/test';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function getFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      if (!address || typeof address === 'string') {
        server.close(() => reject(new Error('Failed to allocate a TCP port')));
        return;
      }
      const { port } = address;
      server.close((err) => (err ? reject(err) : resolve(port)));
    });
  });
}

async function waitForCdp(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (response.ok) {
        return;
      }
      lastError = new Error(`CDP endpoint returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await delay(250);
  }
  throw new Error(`Timed out waiting for WebView2 CDP endpoint on port ${port}: ${String(lastError)}`);
}

function isRelevantSweeperPage(page) {
  const url = page.url();
  if (!url || url === 'about:blank') {
    return false;
  }
  if (url.includes('tauri-isolation') || url.includes('/isolation/')) {
    return false;
  }
  return true;
}

async function findSweeperPage(browser, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    for (const context of browser.contexts()) {
      for (const page of context.pages()) {
        if (!isRelevantSweeperPage(page)) {
          continue;
        }
        try {
          const hasSweeperUi = await page.evaluate(() => {
            const bodyText = document.body?.innerText ?? '';
            return (
              document.title.includes('Sweeper') ||
              bodyText.includes('Scan') ||
              bodyText.includes('Analyzer') ||
              bodyText.includes('Settings')
            );
          });
          if (hasSweeperUi) {
            return page;
          }
        } catch {
          // Page may still be navigating; retry until timeout.
        }
      }
    }
    await delay(250);
  }
  throw new Error('Could not locate the main Sweeper WebView2 page over CDP');
}

function killProcessTree(pid) {
  if (!pid) {
    return;
  }
  spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' });
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    delay(timeoutMs),
  ]);
}

function assertSweeperWindowVisible(pid) {
  const command = [
    '$ErrorActionPreference = "Stop";',
    `$p = Get-Process -Id ${pid} -ErrorAction Stop;`,
    'if ($p.MainWindowTitle -ne "Sweeper") { throw "Unexpected window title: $($p.MainWindowTitle)" }',
    'if ($p.MainWindowHandle -eq 0) { throw "Sweeper window handle is not visible" }',
    'Write-Output $p.MainWindowTitle',
  ].join(' ');
  const result = spawnSync('powershell', ['-NoProfile', '-Command', command], {
    encoding: 'utf8',
  });
  if (result.status !== 0) {
    const details = (result.stderr || result.stdout || '').trim();
    throw new Error(`Sweeper window visibility check failed: ${details}`);
  }
  const title = (result.stdout || '').trim();
  assert(title === 'Sweeper', `Expected Sweeper window title, got "${title}"`);
}

async function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const exePath = process.env.SWEEPER_DESKTOP_EXE
    ? path.resolve(process.env.SWEEPER_DESKTOP_EXE)
    : path.resolve(scriptDir, '../../../target/debug/sweeper-desktop.exe');

  if (!fs.existsSync(exePath)) {
    const repoRoot = path.resolve(scriptDir, '../../..');
    const targetDir = path.join(repoRoot, 'target');
    throw new Error(
      `sweeper-desktop executable not found at ${exePath}. Build it first with CARGO_TARGET_DIR set to the repository-root target directory so Cargo writes to <repo>/target/debug/ instead of app/target/debug/ (PowerShell: $env:CARGO_TARGET_DIR='${targetDir}'; cargo build --manifest-path app/Cargo.toml --package sweeper-desktop --locked).`,
    );
  }

  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'sweeper-desktop-e2e-'));
  const appData = path.join(tempRoot, 'AppData', 'Roaming');
  const localAppData = path.join(tempRoot, 'AppData', 'Local');
  const webViewUserData = path.join(tempRoot, 'WebView2');
  fs.mkdirSync(appData, { recursive: true });
  fs.mkdirSync(localAppData, { recursive: true });
  fs.mkdirSync(webViewUserData, { recursive: true });

  const port = await getFreePort();
  const child = spawn(exePath, [], {
    cwd: path.dirname(exePath),
    env: {
      ...process.env,
      SWEEPER_E2E_READ_ONLY: '1',
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      WEBVIEW2_USER_DATA_FOLDER: webViewUserData,
      APPDATA: appData,
      LOCALAPPDATA: localAppData,
    },
    stdio: 'ignore',
    windowsHide: false,
  });

  let browser;
  const consoleErrors = [];
  const pageErrors = [];

  try {
    await waitForCdp(port, 30_000);
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    const page = await findSweeperPage(browser, 30_000);

    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        consoleErrors.push(msg.text());
      }
    });
    page.on('pageerror', (err) => {
      pageErrors.push(String(err));
    });

    await page.waitForLoadState('domcontentloaded', { timeout: 30_000 });
    await page.getByRole('button', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
    await page.getByRole('button', { name: 'Analyzer' }).waitFor({ timeout: 30_000 });
    await page.getByRole('button', { name: 'Settings' }).waitFor({ timeout: 30_000 });

    // Real Tauri IPC + navigation: settings and elevation status load without IPC errors.
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByRole('heading', { name: 'Settings' }).waitFor({ timeout: 30_000 });
    await page.getByText(/Standard \(Non-Elevated\)|Administrator \(Elevated\)/).waitFor({ timeout: 30_000 });

    await page.getByRole('button', { name: 'Scan & Clean' }).click();
    await page.getByRole('heading', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
    await page.getByRole('button', { name: 'Start Scan' }).waitFor({ timeout: 30_000 });

    // UI responsiveness probe (Bug 1 guard): measure renderer responsiveness while Start Scan runs/completes.
    await page.evaluate(() => {
      const state = { maxDelta: 0, last: performance.now(), running: true, samples: 0 };
      (window).__sweeperUiProbe = state;
      const tick = () => {
        if (!state.running) return;
        const now = performance.now();
        const delta = now - state.last;
        if (delta > state.maxDelta) state.maxDelta = delta;
        state.last = now;
        state.samples += 1;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });

    let sawProgress = false;
    try {
      await page.getByRole('button', { name: 'Start Scan' }).click();

      // Fast scans may complete before progress UI is observed; slower scans should show progress ticks/counts.
      try {
        await page.locator('.scan-progress-notice, [aria-label="Scan target progress"]').first().waitFor({ timeout: 5_000 });
        sawProgress = true;
      } catch {
        sawProgress = false;
      }

      await page.waitForFunction(
        () => {
          const inspector = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').includes('Inspector'));
          const hasResults = document.querySelectorAll('article.target-card').length > 0;
          const noTargets = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').trim() === 'No targets');
          const readyVisible = Array.from(document.querySelectorAll('h3, p, span, div')).some((el) => (el.textContent || '').trim() === 'Ready to scan');
          const progressVisible = !!document.querySelector('.scan-progress-notice');
          const errorNotice = Array.from(document.querySelectorAll('[role="alert"]')).some((el) => /scan failed|session expired|invalid/i.test(el.textContent || ''));
          const resultsReady = hasResults || noTargets;
          return !progressVisible && !errorNotice && inspector && resultsReady && !readyVisible;
        },
        undefined,
        // Debug-mode cold-cache scans on real disks can take several minutes.
        { timeout: 300_000 },
      );

      const initialScanState = await page.evaluate(() => {
        const hasResults = document.querySelectorAll('article.target-card').length > 0;
        const noTargets = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').trim() === 'No targets');
        const inspector = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').includes('Inspector'));
        const readyVisible = Array.from(document.querySelectorAll('h3, p, span, div')).some((el) => (el.textContent || '').trim() === 'Ready to scan');
        return { hasResults, noTargets, inspector, readyVisible };
      });

      // Bug 4: navigating to Settings and back to Scan & Clean preserves scan results.
      await page.getByRole('button', { name: 'Settings' }).click();
      await page.getByRole('heading', { name: 'Settings' }).waitFor({ timeout: 30_000 });
      await page.getByRole('button', { name: 'Scan & Clean' }).click();
      await page.getByRole('heading', { name: 'Scan & Clean' }).waitFor({ timeout: 30_000 });
      const preserved = await page.evaluate(() => {
        const hasResults = document.querySelectorAll('article.target-card').length > 0;
        const noTargets = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').trim() === 'No targets');
        const inspector = Array.from(document.querySelectorAll('h3')).some((el) => (el.textContent || '').includes('Inspector'));
        const readyVisible = Array.from(document.querySelectorAll('h3, p, span, div')).some((el) => (el.textContent || '').trim() === 'Ready to scan');
        return { hasResults, noTargets, inspector, readyVisible };
      });
      assert(
        preserved.hasResults === initialScanState.hasResults &&
          preserved.noTargets === initialScanState.noTargets &&
          preserved.inspector &&
          !preserved.readyVisible,
        `Scan state was not preserved after navigating Settings -> Scan & Clean (initial=${JSON.stringify(initialScanState)}, preserved=${JSON.stringify(preserved)})`,
      );
    } finally {
      // Allow a brief tail sample so we catch stalls at completion boundary, then stop probe.
      await delay(250);
      const probe = await page
        .evaluate(() => {
          const state = /** @type {{ maxDelta: number; samples: number; running: boolean } | undefined} */ (window.__sweeperUiProbe);
          if (!state) return null;
          state.running = false;
          return { maxDelta: state.maxDelta, samples: state.samples };
        })
        .catch(() => null);
      assert(probe && typeof probe.maxDelta === 'number', 'UI responsiveness probe did not run');
      assert(
        probe.maxDelta < 2000,
        `UI responsiveness probe detected main-thread stall (max RAF delta ${probe.maxDelta.toFixed(1)}ms >= 2000ms, samples=${probe.samples}, sawProgress=${sawProgress})`,
      );
    }

    // Allow late async console/page errors from startup/scan IPC to surface.
    await delay(1500);

    assertSweeperWindowVisible(child.pid);

    const runtimeProblems = [
      ...consoleErrors.map((msg) => `console.error: ${msg}`),
      ...pageErrors.map((msg) => `pageerror: ${msg}`),
    ];
    const cspIsolationRuntimeProblems = runtimeProblems.filter((msg) => /csp|content security policy|isolation|runtime/i.test(msg));
    assert(
      runtimeProblems.length === 0,
      `Unexpected WebView2 console/page errors:\n${runtimeProblems.join('\n')}`,
    );
    assert(
      cspIsolationRuntimeProblems.length === 0,
      `Unexpected CSP/isolation/runtime errors:\n${cspIsolationRuntimeProblems.join('\n')}`,
    );

    process.stdout.write(
      `desktop-e2e: ok (exe=${exePath}, pid=${child.pid}, cdpPort=${port}, consoleErrors=0, pageErrors=0, cspIsolationRuntimeErrors=0, sawProgress=${sawProgress})\n`,
    );
  } finally {
    try {
      if (browser) {
        await browser.close();
      }
    } catch {
      // Best-effort; process teardown follows.
    }
    killProcessTree(child.pid);
    await waitForExit(child, 10_000);
    try {
      fs.rmSync(tempRoot, { recursive: true, force: true });
    } catch {
      // Best-effort cleanup: WebView2 may briefly hold temp profile files on Windows.
    }
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error));
  process.exit(1);
});
