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

/**
 * Run the real audit gate in a child process against a synthetic report.
 * By default the report is fed via `--stdin`; `piped: true` omits that flag so
 * the script's piped-stdin path is exercised (with a non-TTY pipe it parses
 * whatever JSON was supplied instead of running a live audit).
 */
function runGate(report: unknown, options: { piped?: boolean } = {}): GateResult {
  const args = options.piped ? [SCRIPT_PATH] : [SCRIPT_PATH, '--stdin'];
  const result = spawnSync(process.execPath, args, {
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
  // Table-driven exception-regression cases: each row keeps its descriptive
  // title, exact synthetic report, and the original assertion verbatim.
  it.each([
    {
      name: 'blocks an unrelated high advisory alongside the exception',
      report: {
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
      },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'fails closed when an excepted cause is combined with a cycle',
      report: {
        vulnerabilities: {
          picomatch: highVuln([exceptedPicomatchCause()]),
          tinyglobby: highVuln(['tinyglobby']),
          parentpkg: highVuln(['picomatch', 'tinyglobby']),
        },
      },
      assert: (report: unknown) =>
        expectBlocked(report, 'Unresolved audit reference: tinyglobby (cycle)'),
    },
    {
      name: 'fails closed when an excepted cause is combined with a missing reference',
      report: {
        vulnerabilities: {
          picomatch: highVuln([exceptedPicomatchCause()]),
          parentpkg: highVuln(['picomatch', 'does-not-exist']),
        },
      },
      assert: (report: unknown) =>
        expectBlocked(report, 'Unresolved audit reference: does-not-exist (missing)'),
    },
    {
      name: 'passes an unrelated low advisory alongside the exception',
      report: {
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
      },
      assert: (report: unknown) => expectPassed(report),
    },
    {
      name: 'blocks a high advisory whose title names the exception but whose url does not (v2)',
      report: {
        vulnerabilities: {
          other: highVuln([
            {
              source: 789,
              name: 'other',
              title: `ReDoS via ${EXCEPTED_ID}`,
              url: 'https://github.com/advisories/GHSA-aaaa-bbbb-cccc',
              severity: 'high',
            },
          ]),
        },
      },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'blocks a high advisory whose title names the exception but whose id does not (v1)',
      report: {
        advisories: {
          'GHSA-aaaa-bbbb-cccc': {
            github_advisory_id: 'GHSA-aaaa-bbbb-cccc',
            npm_advisory_id: 'GHSA-aaaa-bbbb-cccc',
            title: `ReDoS via ${EXCEPTED_ID}`,
            severity: 'high',
            module_name: 'other',
          },
        },
      },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'passes the excepted advisory identified by its own url',
      report: {
        vulnerabilities: {
          picomatch: highVuln([exceptedPicomatchCause()]),
        },
      },
      assert: (report: unknown) => expectPassed(report),
    },
  ])('$name', ({ report, assert: assertAudit }) => {
    assertAudit(report);
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

  // Piped falsy JSON must be forwarded to validateReport and rejected, not
  // mistaken for "no input" and silently replaced by a live audit.
  it.each([
    { label: 'false', value: false },
    { label: 'zero', value: 0 },
    { label: 'null', value: null },
  ])('rejects falsy piped stdin ($label) instead of running a live audit', ({ value }) => {
    const result = runGate(value, { piped: true });
    expect(result.status).toBe(2);
    expect(result.output).toContain('Unsupported audit report');
  });
});
