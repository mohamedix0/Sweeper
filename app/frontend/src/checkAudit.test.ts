import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Vitest runs with the frontend package as its cwd. `import.meta.url` is a
// Vite dev-server URL here, so resolve the script from the project root.
const SCRIPT_PATH = resolve(process.cwd(), 'scripts', 'check-audit.mjs');
const EXCEPTED_ID = 'GHSA-FC5Q-39XC-VJPF';

interface GateResult {
  status: number | null;
  output: string;
}

/** Run the real audit gate in a child process against a synthetic report. */
function runGate(report: unknown): GateResult {
  const result = spawnSync(process.execPath, [SCRIPT_PATH, '--stdin'], {
    input: JSON.stringify(report),
    encoding: 'utf8',
  });
  return {
    status: result.status,
    output: `${result.stdout ?? ''}${result.stderr ?? ''}`,
  };
}

function exceptedPicomatchCause() {
  return {
    source: 123,
    name: 'picomatch',
    title: 'picomatch ReDoS',
    url: `https://github.com/advisories/${EXCEPTED_ID}`,
    severity: 'high',
  };
}

function highVuln(via: Array<unknown>) {
  return { severity: 'high', via, fixAvailable: false };
}

function expectBlocked(report: unknown, reason?: string): void {
  const result = runGate(report);
  expect(result.status).toBeGreaterThan(0);
  if (reason) expect(result.output).toContain(reason);
}

function expectPassed(report: unknown): void {
  expect(runGate(report).status).toBe(0);
}

describe('check-audit.mjs gate (synthetic reports)', () => {
  it('fails closed when an excepted cause is combined with a cycle', () => {
    expectBlocked(
      {
        vulnerabilities: {
          picomatch: highVuln([exceptedPicomatchCause()]),
          tinyglobby: highVuln(['tinyglobby']),
          parentpkg: highVuln(['picomatch', 'tinyglobby']),
        },
      },
      'Unresolved audit reference: tinyglobby (cycle)',
    );
  });

  it('fails closed when an excepted cause is combined with a missing reference', () => {
    expectBlocked(
      {
        vulnerabilities: {
          picomatch: highVuln([exceptedPicomatchCause()]),
          parentpkg: highVuln(['picomatch', 'does-not-exist']),
        },
      },
      'Unresolved audit reference: does-not-exist (missing)',
    );
  });

  it('passes a plain inherited picomatch exception with no cycle', () => {
    expectPassed({
      vulnerabilities: {
        picomatch: highVuln([exceptedPicomatchCause()]),
        tinyglobby: highVuln(['picomatch']),
      },
    });
  });

  it('passes a valid empty advisories object', () => {
    expectPassed({ advisories: {} });
  });

  it('rejects an array-shaped advisories container', () => {
    const result = runGate({ advisories: [] });
    expect(result.status).toBeGreaterThan(0);
    expect(result.output).toContain('Unsupported audit report shape');
  });

  it('blocks an unrelated high advisory alongside the exception', () => {
    expectBlocked({
      vulnerabilities: {
        picomatch: highVuln([
          exceptedPicomatchCause(),
          {
            source: 456,
            name: 'picomatch',
            title: 'Unrelated high advisory',
            url: 'https://github.com/advisories/GHSA-aaaa-bbbb-cccc',
            severity: 'high',
          },
        ]),
      },
    });
  });

  it('passes an unrelated low advisory alongside the exception', () => {
    expectPassed({
      vulnerabilities: {
        picomatch: highVuln([
          exceptedPicomatchCause(),
          {
            source: 456,
            name: 'other',
            title: 'Unrelated low advisory',
            url: 'https://github.com/advisories/GHSA-dddd-eeee-ffff',
            severity: 'low',
          },
        ]),
      },
    });
  });
});
