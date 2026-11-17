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
 *
 * The implementation is deliberately split into small single-purpose helpers:
 * parsing, normalization, classification, and printing stay separate so each
 * function has a single reason to change and shallow nesting.
 */
import { spawn } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import process from 'node:process';

const IGNORED_ADVISORY_IDS = new Set(['GHSA-FC5Q-39XC-VJPF']);
const BLOCKING_SEVERITIES = new Set(['high', 'critical']);
const FRONTEND_DIR = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ADVISORY_URL_PREFIX = 'https://github.com/advisories/';

// ---------------------------------------------------------------------------
// Generic value helpers
// ---------------------------------------------------------------------------

/** Return the first argument that is neither null nor undefined. */
function firstDefined(...values) {
  return values.find((value) => value !== undefined && value !== null);
}

function isObject(value) {
  return value !== null && typeof value === 'object';
}

function asArray(value) {
  return Array.isArray(value) ? value : [];
}

function primaryId(ids, fallback) {
  return ids[0] ?? fallback;
}

function parseJson(text, source) {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`Invalid audit JSON from ${source}: ${error.message}`);
  }
}

// ---------------------------------------------------------------------------
// Identifier & severity helpers
// ---------------------------------------------------------------------------

/** Normalize any GHSA identifier we find into a comparable uppercase form. */
function normalizeGhsaId(value) {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (/^GHSA-/i.test(trimmed)) return trimmed.toUpperCase();
  const match = trimmed.match(/GHSA-[0-9a-z-]+/i);
  return match ? match[0].toUpperCase() : null;
}

function advisoryUrl(ids) {
  const id = ids[0];
  return id ? `${ADVISORY_URL_PREFIX}${id}` : '';
}

function severityBucket(advisory, counts) {
  return advisory.severity in counts ? advisory.severity : 'unknown';
}

// ---------------------------------------------------------------------------
// Audit report extraction (v1: advisories, v2: vulnerabilities)
// ---------------------------------------------------------------------------

function collectGhsaIds(raw) {
  return [raw.github_advisory_id, raw.npm_advisory_id, raw.title]
    .map(normalizeGhsaId)
    .filter(Boolean);
}

function toV1Advisory(key, raw) {
  const ids = collectGhsaIds(raw);
  return {
    key,
    id: primaryId(ids, String(key)),
    allIds: ids,
    severity: String(firstDefined(raw.severity, 'unknown')).toLowerCase(),
    title: firstDefined(raw.title, '(untitled advisory)'),
    moduleName: firstDefined(raw.module_name, raw.module, '(unknown module)'),
    patchedVersions: firstDefined(raw.patched_versions, 'unknown'),
    url: firstDefined(raw.url, ''),
  };
}

function extractV1Advisories(advisories) {
  return Object.entries(advisories).map(([key, raw]) => toV1Advisory(key, raw));
}

function isViaReference(entry) {
  return typeof entry === 'string';
}

function unique(values) {
  return [...new Set(values)];
}

function collectViaIds(entry) {
  return unique([entry.url, entry.source, entry.id, entry.title].map(normalizeGhsaId).filter(Boolean));
}

function viaSeverity(entry) {
  return String(firstDefined(entry.severity, 'unknown')).toLowerCase();
}

function toViaCause(entry) {
  return {
    allIds: collectViaIds(entry),
    severity: viaSeverity(entry),
    title: firstDefined(entry.title, '(untitled advisory)'),
    moduleName: firstDefined(entry.name, '(unknown module)'),
  };
}

function toPackageCause(pkg, raw) {
  return {
    allIds: [],
    severity: String(firstDefined(raw.severity, 'unknown')).toLowerCase(),
    title: `Vulnerability in ${pkg}`,
    moduleName: pkg,
  };
}

/**
 * Resolve one package's `via` list into individual advisory causes. Each
 * object entry is one cause with its own ids and severity. A string entry is a
 * reference to another vulnerability entry and is followed through
 * `vulnerabilities`; the visited set makes cycles terminate.
 */
function resolveCauses(pkg, vulnerabilities, visited) {
  if (visited.has(pkg)) return [];
  visited.add(pkg);

  const raw = vulnerabilities[pkg];
  if (!isObject(raw)) return [];

  const causes = [];
  for (const entry of asArray(raw.via)) {
    if (isViaReference(entry)) {
      causes.push(...resolveCauses(entry, vulnerabilities, visited));
    } else if (isObject(entry)) {
      causes.push(toViaCause(entry));
    }
  }

  return causes.length > 0 ? causes : [toPackageCause(pkg, raw)];
}

function describeFix(fix, pkg) {
  if (fix === true) return 'a fix is available';
  if (fix && typeof fix === 'object') {
    return `a fix is available (${firstDefined(fix.name, pkg)})`;
  }
  return 'none reported';
}

function toAdvisoryFromCause(pkg, raw, cause) {
  return {
    key: pkg,
    id: primaryId(cause.allIds, pkg),
    allIds: cause.allIds,
    severity: cause.severity,
    title: cause.title,
    moduleName: cause.moduleName,
    patchedVersions: describeFix(raw.fixAvailable, pkg),
    url: advisoryUrl(cause.allIds),
  };
}

function extractV2Advisories(vulnerabilities) {
  const advisories = [];
  for (const [pkg, raw] of Object.entries(vulnerabilities)) {
    const causes = resolveCauses(pkg, vulnerabilities, new Set());
    for (const cause of causes) advisories.push(toAdvisoryFromCause(pkg, raw, cause));
  }
  return advisories;
}

/**
 * Normalize both npm/pnpm audit shapes into a flat list of advisories.
 *   v1: { advisories: { "<id>": { github_advisory_id, severity, ... } } }
 *   v2: { vulnerabilities: { "<pkg>": { severity, via: [...] } } }
 * We consume whichever shape is present so a single issue is never counted twice.
 */
function extractAdvisories(report) {
  if (!isObject(report)) return [];
  if (isObject(report.advisories)) return extractV1Advisories(report.advisories);
  if (isObject(report.vulnerabilities)) return extractV2Advisories(report.vulnerabilities);
  return [];
}

// ---------------------------------------------------------------------------
// Report validation (fail closed on registry errors / unsupported shapes)
// ---------------------------------------------------------------------------

function hasSupportedShape(report) {
  return isObject(report.advisories) || isObject(report.vulnerabilities);
}

function describeReportError(report) {
  if (!isObject(report.error)) return 'registry error object';
  return firstDefined(report.error.message, report.error.summary, 'registry error object');
}

/**
 * Fail closed for error objects and unknown shapes. A non-zero audit exit is
 * expected whenever advisories exist, so validity is judged from the report
 * itself: valid (even empty) reports pass through regardless of exit status.
 */
function validateReport(report) {
  if (!isObject(report)) {
    throw new Error('Unsupported audit report: expected a JSON object');
  }
  if (report.error !== undefined) {
    throw new Error(`Audit reported an error: ${describeReportError(report)}`);
  }
  if (!hasSupportedShape(report)) {
    throw new Error('Unsupported audit report shape: expected advisories or vulnerabilities');
  }
  return report;
}

// ---------------------------------------------------------------------------
// Report loading (stdin / piped input / live pnpm audit)
// ---------------------------------------------------------------------------

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

async function readForcedStdin() {
  const text = await readStdin();
  if (!text.trim()) {
    throw new Error('--stdin was set but no JSON was provided on stdin');
  }
  return parseJson(text, 'stdin');
}

async function readPipedStdin() {
  const text = await readStdin(300);
  return text.trim() ? parseJson(text, 'stdin') : null;
}

async function loadFromPipe() {
  if (process.stdin.isTTY) return null;
  return readPipedStdin();
}

async function loadReport() {
  if (process.argv.includes('--stdin')) return readForcedStdin();
  const piped = await loadFromPipe();
  if (piped) return piped;
  return runAudit();
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

function countBySeverity(advisories) {
  const counts = { info: 0, low: 0, moderate: 0, high: 0, critical: 0, unknown: 0 };
  for (const advisory of advisories) counts[severityBucket(advisory, counts)] += 1;
  return counts;
}

/**
 * The exception applies to individual advisory causes, never to a whole
 * package. Each v2 `via` cause is normalized into its own advisory, so a
 * package carrying both an excepted advisory and an unrelated one still
 * produces a separate blocking advisory for the unrelated cause.
 */
function isIgnoredAdvisory(advisory) {
  return advisory.allIds.some((id) => IGNORED_ADVISORY_IDS.has(id.toUpperCase()));
}

function isBlockingAdvisory(advisory) {
  return BLOCKING_SEVERITIES.has(advisory.severity);
}

function classifyAdvisory(advisory) {
  if (isIgnoredAdvisory(advisory)) return 'ignored';
  if (isBlockingAdvisory(advisory)) return 'blocking';
  return 'informational';
}

function partitionAdvisories(advisories) {
  const groups = { ignored: [], blocking: [], informational: [] };
  for (const advisory of advisories) groups[classifyAdvisory(advisory)].push(advisory);
  return groups;
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

function printUrl(url, indent) {
  if (url) console.log(`${indent}${url}`);
}

function printHeader(advisories, counts) {
  console.log('=== Frontend dependency audit (high/critical gate) ===');
  console.log(`Advisories reported: ${advisories.length}`);
  console.log(
    `  info=${counts.info} low=${counts.low} moderate=${counts.moderate} ` +
      `high=${counts.high} critical=${counts.critical}`,
  );
  if (advisories.length === 0) console.log('No advisories reported by the audit.');
}

function printInformationalOne(advisory) {
  console.log('');
  console.log(`[${advisory.severity.toUpperCase()}] ${advisory.id} - ${advisory.moduleName}`);
  console.log(`    ${advisory.title}`);
  printUrl(advisory.url, '    ');
}

function printInformational(advisories) {
  for (const advisory of advisories) printInformationalOne(advisory);
}

function printIgnoredOne(advisory) {
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
  printUrl(advisory.url, '    ');
}

function printIgnored(advisories) {
  for (const advisory of advisories) printIgnoredOne(advisory);
}

function printBlockingOne(advisory) {
  console.log(
    `  - ${advisory.id} - ${advisory.moduleName} (severity: ${advisory.severity}): ` +
      advisory.title,
  );
  printUrl(advisory.url, '      ');
}

function printBlocking(blocking) {
  console.log('');
  console.log('BLOCKING high/critical advisories (not on the exception list):');
  for (const advisory of blocking) printBlockingOne(advisory);
  console.log('');
  console.log('FAIL: high/critical advisory gate failed.');
}

function printPass() {
  const ignoredLabel = [...IGNORED_ADVISORY_IDS].join(', ');
  console.log('');
  console.log(
    `PASS: no high/critical advisories other than the documented exception (${ignoredLabel}).`,
  );
}

function reportResult(blocking) {
  if (blocking.length > 0) {
    printBlocking(blocking);
    process.exitCode = 1;
    return;
  }
  printPass();
}

async function checkAudit() {
  const report = validateReport(await loadReport());
  const advisories = extractAdvisories(report);
  const counts = countBySeverity(advisories);
  const groups = partitionAdvisories(advisories);
  printHeader(advisories, counts);
  printInformational(groups.informational);
  printIgnored(groups.ignored);
  reportResult(groups.blocking);
}

try {
  await checkAudit();
} catch (error) {
  console.error(`check-audit: ERROR: ${error.message}`);
  process.exitCode = 2;
}
