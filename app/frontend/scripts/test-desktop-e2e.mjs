import { chromium } from '@playwright/test';
import process from 'node:process';
import { setTimeout as delay } from 'node:timers/promises';
import {
  assert,
  assertSweeperWindowVisible,
  cleanupTemp,
  closeBrowser,
  findSweeperPage,
  getFreePort,
  killProcessTree,
  probeEval,
  readScanEval,
  resolveDesktopExePath,
  runScanFlow,
  safeEval,
  scanReadyEval,
  spawnDesktop,
  stopProbeEval,
  verifyBasicNavigation,
  verifyStatePreserved,
  waitForCdp,
  waitForExit,
} from './test-desktop-e2e.helpers.mjs';

function attachCollectors(page) {
  const collectors = { consoleErrors: [], pageErrors: [] };
  page.on('console', (msg) => {
    if (msg.type() === 'error') collectors.consoleErrors.push(msg.text());
  });
  page.on('pageerror', (err) => {
    collectors.pageErrors.push(String(err));
  });
  return collectors;
}

function collectProblems(collectors) {
  const problems = [];
  for (const msg of collectors.consoleErrors) problems.push(`console.error: ${msg}`);
  for (const msg of collectors.pageErrors) problems.push(`pageerror: ${msg}`);
  return problems;
}

async function runPageFlow(page) {
  await verifyBasicNavigation(page);
  await page.evaluate(probeEval);
  const sawProgress = await runScanFlow(page);
  await page.waitForFunction(scanReadyEval, undefined, { timeout: 300_000 });
  await verifyStatePreserved(page, await page.evaluate(readScanEval));
  await delay(250);
  return sawProgress;
}

async function finishChecks(ctx) {
  const probe = await safeEval(ctx.page, stopProbeEval);
  assert(probe && typeof probe.maxDelta === 'number', 'UI responsiveness probe did not run');
  assert(probe.maxDelta < 2000, `UI responsiveness probe detected main-thread stall (max RAF delta ${probe.maxDelta.toFixed(1)}ms >= 2000ms, samples=${probe.samples}, sawProgress=${ctx.sawProgress})`);
  await delay(1500);
  await assertSweeperWindowVisible(ctx.child.pid);
  const problems = collectProblems(ctx.collectors);
  const cspProblems = problems.filter((msg) => /csp|content security policy|isolation|runtime/i.test(msg));
  assert(problems.length === 0, `Unexpected WebView2 console/page errors:\n${problems.join('\n')}`);
  assert(cspProblems.length === 0, `Unexpected CSP/isolation/runtime errors:\n${cspProblems.join('\n')}`);
}

async function runDesktopE2E(opts) {
  await waitForCdp(opts.port, 30_000);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${opts.port}`);
  try {
    const page = await findSweeperPage(browser, 30_000);
    const collectors = attachCollectors(page);
    const sawProgress = await runPageFlow(page);
    await finishChecks({ page, child: opts.child, collectors, sawProgress });
    process.stdout.write(`desktop-e2e: ok (exe=${opts.exePath}, pid=${opts.child.pid}, cdpPort=${opts.port}, consoleErrors=0, pageErrors=0, cspIsolationRuntimeErrors=0, sawProgress=${sawProgress})\n`);
  } finally {
    await closeBrowser(browser);
  }
}

async function main() {
  const exePath = resolveDesktopExePath();
  const port = await getFreePort();
  const { child, tempRoot } = spawnDesktop(exePath, port);
  try {
    await runDesktopE2E({ exePath, port, child });
  } finally {
    killProcessTree(child.pid);
    await waitForExit(child, 10_000);
    cleanupTemp(tempRoot);
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error));
  process.exit(1);
});
