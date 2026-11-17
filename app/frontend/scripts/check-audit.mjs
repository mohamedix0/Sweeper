#!/usr/bin/env node
/**
 * Frontend dependency-audit gate with one tightly scoped, documented exception.
 *
 * Runs `pnpm audit --json` in app/frontend (or consumes a JSON report from
 * stdin) and fails when ANY high or critical advisory is present, EXCEPT the
 * single known advisory listed in IGNORED_ADVISORY_IDS. Lower-severity
 * advisories are reported but never fail the gate.
 *
 * TEMPORARY EXCEPTION - GHSA-fc5q-39xc-vjpf (CVE-2026-112366)
 *   picomatch <= 4.0.7 ReDoS. The GitHub Advisory Database reports NO patched
 *   version (latest published picomatch is 4.0.7; advisory updated 2026-11-17).
 *   It is reachable only through devDependencies
 *   (@typescript-eslint -> tinyglobby -> picomatch) and never ships in the
 *   runtime bundle.
 *   REMOVE this id from IGNORED_ADVISORY_IDS as soon as picomatch publishes a
 *   patched version. Tracked in the GitHub issue referenced by ci.yml.
 */
import { spawn } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import process from 'node:process';

const IGNORED_ADVISORY_IDS = new Set(['GHSA-FC5Q-39XC-VJPF']);
const BLOCKING_SEVERITIES = new Set(['high', 'critical']);
const SEVERITY_ORDER = ['info', 'low', 'moderate', 'high', 'critical'];
const FRONTEND_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** Normalize any GHSA identifier we find into a comparable uppercase form. */
function normalizeGhsaId(value) {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (/^GHSA-/i.test(trimmed)) return trimmed.toUpperCase();
  const match = trimmed.match(/GHSA-[0-9a-z-]+/i);
  return match ? match[0].toUpperCase() : null;
}

function severityRank(severity) {
  const index = SEVERITY_ORDER.indexOf(String(severity || '').toLowerCase());
  return index === -1 ? 0 : index;
}

function parseJson(text, source) {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`Invalid audit JSON from ${source}: ${error.message}`);
  }
}

/**
 * Normalize both npm/pnpm audit shapes into a flat list of advisories.
 *   v1: { advisories: { "<id>": { github_advisory_id, severity, ... } } }
 *   v2: { vulnerabilities: { "<pkg>": { severity, via: [...] } } }
 * We consume whichever shape is present so a single issue is never counted twice.
 */
function extractAdvisories(report) {
  const advisories = [];
  if (!report || typeof report !== 'object') return advisories;

  if (report.advisories && typeof report.advisories === 'object') {
    for (const [key, raw] of Object.entries(report.advisories)) {
      const ids = [raw.github_advisory_id, raw.npm_advisory_id, raw.title]
        .map(normalizeGhsaId)
        .filter(Boolean);
      advisories.push({
        key,
        id: ids[0] ?? String(key),
        allIds: ids,
        severity: String(raw.severity ?? 'unknown').toLowerCase(),
        title: raw.title ?? '(untitled advisory)',
        moduleName: raw.module_name ?? raw.module ?? '(unknown module)',
        patchedVersions: raw.patched_versions ?? 'unknown',
        url: raw.url ?? '',
      });
    }
    return advisories;
  }

  if (report.vulnerabilities && typeof report.vulnerabilities === 'object') {
    for (const [pkg, raw] of Object.entries(report.vulnerabilities)) {
      const via = Array.isArray(raw.via) ? raw.via : [];
      const ids = [];
      let severity = String(raw.severity ?? '').toLowerCase();
      for (const entry of via) {
        if (typeof entry === 'string') {
          const id = normalizeGhsaId(entry);
          if (id) ids.push(id);
          continue;
        }
        const id = normalizeGhsaId(entry.url ?? entry.source ?? entry.id ?? '');
        if (id) ids.push(id);
        if (severityRank(entry.severity) > severityRank(severity)) {
          severity = String(entry.severity).toLowerCase();
        }
      }
      const fix = raw.fixAvailable;
      const patchedVersions =
        fix === true
          ? 'a fix is available'
          : typeof fix === 'object' && fix !== null
            ? `a fix is available (${fix.name ?? pkg})`
            : 'none reported';
      advisories.push({
        key: pkg,
        id: ids[0] ?? pkg,
        allIds: ids,
        severity: severity || 'unknown',
        title: `Vulnerability in ${pkg}`,
        moduleName: pkg,
        patchedVersions,
        url: ids[0] ? `https://github.com/advisories/${ids[0]}` : '',
      });
    }
  }

  return advisories;
}

/** Read piped stdin, with an optional timeout to avoid hanging on an idle pipe. */
function readStdin(timeoutMs = 0) {
  return new Promise((resolveRead) => {
    let data = '';
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolveRead(value);
    };
    if (timeoutMs > 0) {
      const timer = setTimeout(() => {
        try {
          process.stdin.pause();
        } catch {
          /* stdin already closed */
        }
        finish(data);
      }, timeoutMs);
      if (typeof timer.unref === 'function') timer.unref();
    }
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (chunk) => {
      data += chunk;
    });
    process.stdin.on('end', () => finish(data));
    process.stdin.on('error', () => finish(data));
  });
}

/** Run `pnpm audit --json` from app/frontend; a non-zero audit exit is expected. */
function runAudit() {
  return new Promise((resolveAudit, rejectAudit) => {
    const options = { cwd: FRONTEND_DIR, stdio: ['ignore', 'pipe', 'pipe'] };
    // On Windows pnpm is a .cmd shim that needs a shell. Passing the whole
    // command as one string avoids Node's DEP0190 (args + shell are unsafe to
    // combine); the command itself is static, so there is no injection surface.
    const child =
      process.platform === 'win32'
        ? spawn('pnpm audit --json', { ...options, shell: true })
        : spawn('pnpm', ['audit', '--json'], options);
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', rejectAudit);
    child.on('close', () => {
      const text = stdout.trim() || stderr.trim();
      if (!text) {
        rejectAudit(new Error('pnpm audit --json produced no output'));
        return;
      }
      resolveAudit(parseJson(text, 'pnpm audit --json'));
    });
  });
}

async function loadReport() {
  const forceStdin = process.argv.includes('--stdin');
  if (forceStdin) {
    const text = await readStdin();
    if (!text.trim()) {
      throw new Error('--stdin was set but no JSON was provided on stdin');
    }
    return parseJson(text, 'stdin');
  }
  if (!process.stdin.isTTY) {
    const text = await readStdin(300);
    if (text.trim()) return parseJson(text, 'stdin');
  }
  return runAudit();
}

async function checkAudit() {
  const report = await loadReport();
  const advisories = extractAdvisories(report);

  const counts = { info: 0, low: 0, moderate: 0, high: 0, critical: 0, unknown: 0 };
  for (const advisory of advisories) {
    const severity = advisory.severity in counts ? advisory.severity : 'unknown';
    counts[severity] += 1;
  }

  const ignored = [];
  const blocking = [];
  const informational = [];
  for (const advisory of advisories) {
    if (advisory.allIds.some((id) => IGNORED_ADVISORY_IDS.has(id.toUpperCase()))) {
      ignored.push(advisory);
    } else if (BLOCKING_SEVERITIES.has(advisory.severity)) {
      blocking.push(advisory);
    } else {
      informational.push(advisory);
    }
  }

  console.log('=== Frontend dependency audit (high/critical gate) ===');
  console.log(`Advisories reported: ${advisories.length}`);
  console.log(
    `  info=${counts.info} low=${counts.low} moderate=${counts.moderate} ` +
      `high=${counts.high} critical=${counts.critical}`,
  );

  if (advisories.length === 0) {
    console.log('No advisories reported by the audit.');
  }

  for (const advisory of informational) {
    console.log('');
    console.log(`[${advisory.severity.toUpperCase()}] ${advisory.id} - ${advisory.moduleName}`);
    console.log(`    ${advisory.title}`);
    if (advisory.url) console.log(`    ${advisory.url}`);
  }

  for (const advisory of ignored) {
    console.log('');
    console.log(
      `[IGNORED - TEMPORARY EXCEPTION] ${advisory.id} - ${advisory.moduleName} ` +
        `(severity: ${advisory.severity})`,
    );
    console.log(`    Title: ${advisory.title}`);
    console.log(`    Patched versions: ${advisory.patchedVersions || 'NONE'}`);
    console.log('    Why ignored: NO patched version is available (advisory updated 2026-11-17).');
    console.log('                 It is reachable only via devDependencies');
    console.log('                 (@typescript-eslint -> tinyglobby -> picomatch) and never');
    console.log('                 ships in the runtime bundle.');
    console.log('    ACTION REQUIRED: remove this exception once a patched version is released.');
    if (advisory.url) console.log(`    ${advisory.url}`);
  }

  if (blocking.length > 0) {
    console.log('');
    console.log('BLOCKING high/critical advisories (not on the exception list):');
    for (const advisory of blocking) {
      console.log(
        `  - ${advisory.id} - ${advisory.moduleName} (severity: ${advisory.severity}): ` +
          advisory.title,
      );
      if (advisory.url) console.log(`      ${advisory.url}`);
    }
    console.log('');
    console.log('FAIL: high/critical advisory gate failed.');
    process.exitCode = 1;
  } else {
    const ignoredLabel = [...IGNORED_ADVISORY_IDS].join(', ');
    console.log('');
    console.log(
      `PASS: no high/critical advisories other than the documented exception (${ignoredLabel}).`,
    );
  }
}

try {
  await checkAudit();
} catch (error) {
  console.error(`check-audit: ERROR: ${error.message}`);
  process.exitCode = 2;
}
