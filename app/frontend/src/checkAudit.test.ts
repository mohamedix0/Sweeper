import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Vitest runs with the frontend package as its cwd. `import.meta.url` is a
// Vite dev-server URL here, so resolve the script from the project root.
const SCRIPT_PATH = resolve(process.cwd(), 'scripts', 'check-audit.mjs');

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

function highCause(id: string, name = 'picomatch') {
  return {
    source: 123,
    name,
    title: `${name} ReDoS`,
    url: `https://github.com/advisories/${id}`,
    severity: 'high',
  };
}

function highVuln(via: Array<unknown>) {
  return { severity: 'high', via, fixAvailable: false };
}

function lowVuln(via: Array<unknown>) {
  return { severity: 'low', via, fixAvailable: false };
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
  // Table-driven cases: every high/critical advisory blocks the gate, while
  // lower-severity findings are reported without failing it.
  it.each([
    {
      name: 'blocks a single high advisory',
      report: { vulnerabilities: { picomatch: highVuln([highCause('GHSA-aaaa-bbbb-cccc')]) } },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'blocks a high advisory alongside an unrelated high advisory',
      report: {
        vulnerabilities: {
          picomatch: highVuln([
            highCause('GHSA-aaaa-bbbb-cccc'),
            highCause('GHSA-bbbb-cccc-dddd', 'tinyglobby'),
          ]),
        },
      },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'blocks a high advisory inherited through a parent package',
      report: {
        vulnerabilities: {
          picomatch: highVuln([highCause('GHSA-aaaa-bbbb-cccc')]),
          tinyglobby: highVuln(['picomatch']),
        },
      },
      assert: (report: unknown) => expectBlocked(report),
    },
    {
      name: 'fails closed when a high advisory is combined with a cycle',
      report: {
        vulnerabilities: {
          picomatch: highVuln([highCause('GHSA-aaaa-bbbb-cccc')]),
          tinyglobby: highVuln(['tinyglobby']),
          parentpkg: highVuln(['picomatch', 'tinyglobby']),
        },
      },
      assert: (report: unknown) =>
        expectBlocked(report, 'Unresolved audit reference: tinyglobby (cycle)'),
    },
    {
      name: 'fails closed when a high advisory is combined with a missing reference',
      report: {
        vulnerabilities: {
          picomatch: highVuln([highCause('GHSA-aaaa-bbbb-cccc')]),
          parentpkg: highVuln(['picomatch', 'does-not-exist']),
        },
      },
      assert: (report: unknown) =>
        expectBlocked(report, 'Unresolved audit reference: does-not-exist (missing)'),
    },
    {
      name: 'blocks a high advisory in the v1 advisories shape',
      report: {
        advisories: {
          'GHSA-aaaa-bbbb-cccc': {
            github_advisory_id: 'GHSA-aaaa-bbbb-cccc',
            npm_advisory_id: 'GHSA-aaaa-bbbb-cccc',
            title: 'ReDoS',
            severity: 'high',
            module_name: 'other',
          },
        },
      },
      assert: (report: unknown) => expectBlocked(report),
    },
  ])('$name', ({ report, assert: assertAudit }) => {
    assertAudit(report);
  });

  // Regression guard: re-adding the withdrawn advisory to IGNORED_ADVISORY_IDS
  // must fail this test. Exit code 1 is required; 2 would mean a processing
  // error rather than a correctly classified blocking advisory.
  it('blocks the withdrawn picomatch advisory ID', () => {
    const result = runGate({
      vulnerabilities: { picomatch: highVuln([highCause('GHSA-FC5Q-39XC-VJPF')]) },
    });
    expect(result.status).toBe(1);
    expect(result.output).toContain('BLOCKING high/critical advisories:');
    expect(result.output).toContain('GHSA-FC5Q-39XC-VJPF');
    expect(result.output).toContain('FAIL: high/critical advisory gate failed.');
    expect(result.output).not.toContain('IGNORED');
  });

  it('passes an unrelated low advisory', () => {
    expectPassed({
      vulnerabilities: {
        other: lowVuln([
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

  it('passes a low advisory in the v1 advisories shape', () => {
    expectPassed({
      advisories: {
        'GHSA-dddd-eeee-ffff': {
          github_advisory_id: 'GHSA-dddd-eeee-ffff',
          npm_advisory_id: 'GHSA-dddd-eeee-ffff',
          title: 'Low finding',
          severity: 'low',
          module_name: 'other',
        },
      },
    });
  });

  it('passes a valid empty advisories object', () => {
    expectPassed({ advisories: {} });
  });

  it('passes a valid empty vulnerabilities object', () => {
    expectPassed({ vulnerabilities: {} });
  });

  it('rejects an array-shaped advisories container', () => {
    const result = runGate({ advisories: [] });
    expect(result.status).toBeGreaterThan(0);
    expect(result.output).toContain('Unsupported audit report shape');
  });

  it('rejects a registry error report', () => {
    const result = runGate({ error: { message: 'registry unavailable' } });
    expect(result.status).toBeGreaterThan(0);
    expect(result.output).toContain('Audit reported an error');
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
