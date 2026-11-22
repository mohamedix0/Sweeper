import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { AuditRow } from './components/logs/logParsers';
import type { ScanItemDto, TargetScanResultDto } from './contract/bindings';
import { AuditTable } from './components/logs/AuditTable';
import { PathWell } from './components/cleanSharedInspector';
import { PreviewRiskCallouts } from './components/PreviewRiskCallouts';
import { ScanGroupRow } from './components/scan/ScanGroupRow';
import { ScanItemRow } from './components/scan/ScanItemRow';

/**
 * Issue #140 (U3): every path/identity shown immediately before permanent
 * deletion must be a Unicode bidirectional isolate. Without an isolate, a
 * strong RTL run inside a name such as `דוח-2024.txt` reorders the neutral
 * separators (backslashes, hyphens, digits) around it, so the identity the
 * user confirms can render differently from the target that is deleted.
 *
 * `<bdi>` is the semantic isolate: the UA stylesheet gives it
 * `unicode-bidi: isolate`, and it inherits the document's LTR base direction.
 * These tests render RTL-containing paths through each real component and
 * assert the isolated node exists and preserves the exact logical text order.
 */

// Strong RTL run ("report" in Hebrew) followed by neutral digits/hyphen/dot.
const RTL = '\u05D3\u05D5\u05D7';
const RTL_LEAF = `${RTL}-2024.txt`;
const RTL_DIR = `C:\\Users\\Alice\\Documents\\${RTL}-2024\\`;
const RTL_PATH = `${RTL_DIR}${RTL_LEAF}`;

function isolatedText(container: HTMLElement, selector: string): string | undefined {
  return container.querySelector(`${selector} bdi`)?.textContent ?? undefined;
}

function targetWith(overrides: Partial<TargetScanResultDto> = {}): TargetScanResultDto {
  return {
    targetId: 't-rtl',
    targetName: 'RTL group',
    tier: 'Tier1',
    risk: 'Safe',
    resolvedPaths: ['C:\\Users\\Alice\\Documents'],
    scanned: true,
    statusReason: null,
    totalBytes: '0',
    eligibleBytes: '0',
    fileCount: '0',
    eligibleCount: '0',
    skippedCount: '0',
    items: [],
    ...overrides,
  };
}

describe('BiDi isolation on deletion-confirmation paths (issue #140)', () => {
  it('isolates the Inspector path directory and leaf filename', () => {
    const { container } = render(<PathWell path={RTL_PATH} />);
    expect(isolatedText(container, '.path-well-dir')).toBe(RTL_DIR);
    expect(isolatedText(container, '.path-well-file')).toBe(RTL_LEAF);
  });

  it('isolates the Preview risk callout identity', () => {
    const { container } = render(
      <PreviewRiskCallouts
        items={[{ id: 'i-1', identity: RTL_PATH, risk: 'Caution', explanation: 'Existing explanation.' }]}
      />,
    );
    expect(isolatedText(container, '.preview-risk-callout-identity-text')).toBe(RTL_PATH);
  });

  it('isolates the Scan group root path', () => {
    const target = targetWith({ resolvedPaths: [`C:\\Users\\Alice\\Documents\\${RTL}-2024`] });
    const { container } = render(
      <ScanGroupRow
        target={target}
        allowDeleteUnknown={false}
        selectedIds={new Set()}
        collapsed={false}
        groupActive={false}
        cleaning={false}
        onFocusGroup={() => {}}
        onToggleCollapsed={() => {}}
        onToggleGroupSelect={() => {}}
      />,
    );
    expect(isolatedText(container, '.scan-group-path-text')).toBe(
      `C:\\Users\\Alice\\Documents\\${RTL}-2024`,
    );
  });

  it('isolates the Scan item name and nested relative path', () => {
    const itemPath = `C:\\Users\\Alice\\Documents\\Reports\\${RTL_LEAF}`;
    const item: ScanItemDto = {
      id: 'i-1',
      targetId: 't-rtl',
      path: itemPath,
      size: '10',
      mtimeUnixSecs: '1700000000',
      ageEligible: true,
    };
    const target = targetWith({ items: [item] });
    const { container } = render(
      <ScanItemRow
        item={item}
        target={target}
        allowDeleteUnknown={false}
        selected={false}
        inspected={false}
        cleaning={false}
        onToggle={() => {}}
      />,
    );
    expect(isolatedText(container, '.item-name')).toBe(RTL_LEAF);
    expect(isolatedText(container, '.scan-subpath-inline')).toBe(`Reports\\${RTL_LEAF}`);
  });

  it('isolates the audit log location cell', () => {
    const row: AuditRow = {
      type: 'file',
      category: 'Cache',
      location: RTL_PATH,
      size: '1 KB',
      outcome: 'Eligible',
      outcomeTone: 'eligible',
      kind: 'File',
    };
    const { container } = render(
      <AuditTable
        auditRows={[row]}
        visibleRows={[row]}
        outcomeFilter="all"
        onOutcome={() => {}}
        expanded={new Set()}
        onToggle={() => {}}
      />,
    );
    expect(isolatedText(container, 'td.audit-location')).toBe(RTL_PATH);
  });
});
