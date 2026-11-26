import { render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it } from 'vitest';
import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { ScanResultsSection } from './ScanResultsSection';

type Props = ComponentProps<typeof ScanResultsSection>;

function item(id: string, targetId: string): ScanItemDto {
  return {
    id,
    targetId,
    path: `C:\\Users\\Alice\\AppData\\Local\\Temp\\${id}.tmp`,
    size: '1024',
    mtimeUnixSecs: '1700000000',
    ageEligible: true,
  };
}

function target(index: number): TargetScanResultDto {
  const targetId = `t-item-${String(index)}`;
  return {
    targetId,
    targetName: `Item ${String(index)}`,
    tier: 'Tier1',
    risk: 'Safe',
    resolvedPaths: ['C:\\Users\\Alice\\AppData\\Local\\Temp'],
    scanned: true,
    statusReason: null,
    totalBytes: '1024',
    eligibleBytes: '1024',
    fileCount: '1',
    eligibleCount: '1',
    skippedCount: '0',
    items: [item(`${targetId}-item-1`, targetId)],
  };
}

function propsWithTargets(targets: TargetScanResultDto[]): Props {
  return {
    targets,
    visibleTargets: targets.map((t) => ({ target: t, items: t.items })),
    allow: false,
    minAgeHours: null,
    selectedIds: new Set(),
    selectionSources: [],
    collapsedTargets: new Set(),
    showAllGroups: new Set(),
    focus: null,
    focusedGroup: null,
    inspected: null,
    inspectedCtx: null,
    cleaning: false,
    scanning: false,
    eligibleSafeIdsLength: targets.length,
    allEligibleSelected: false,
    masterIndeterminate: false,
    requiresConfirmation: false,
    confirmText: '',
    preview: null,
    cleanProgressLabel: null,
    cleanSummary: null,
    onToggleSelectAll: () => {},
    onFocusGroup: () => {},
    onToggleCollapsed: () => {},
    onToggleGroupSelect: () => {},
    onToggleItem: () => {},
    onToggleShowAll: () => {},
    onConfirmText: () => {},
    onOpenLog: () => {},
  };
}

function manyTargets(count: number): TargetScanResultDto[] {
  return Array.from({ length: count }, (_unused, i) => target(i + 1));
}

describe('ScanResultsSection contextual empty-state', () => {
  it('renders the completion hint below the last row when the result set is sparse', () => {
    render(<ScanResultsSection {...propsWithTargets(manyTargets(2))} />);

    const hint = screen.getByRole('status', { name: 'Scan results complete' });
    expect(hint).toBeInTheDocument();
    expect(hint).toHaveTextContent("That's everything found — review your selection");

    // The hint lives inside the results pane, after the last target row.
    const list = document.querySelector('.scan-results-shell .target-list');
    expect(list).not.toBeNull();
    expect(list?.lastElementChild).toBe(hint);
    expect(list?.querySelectorAll('article')).toHaveLength(2);
  });

  it('does not show the completion hint when the result set is dense', () => {
    render(<ScanResultsSection {...propsWithTargets(manyTargets(12))} />);

    expect(screen.queryByRole('status', { name: 'Scan results complete' })).not.toBeInTheDocument();
  });

  it('does not show the completion hint before a Scan has produced results', () => {
    render(<ScanResultsSection {...propsWithTargets([])} />);

    expect(screen.queryByRole('status', { name: 'Scan results complete' })).not.toBeInTheDocument();
  });
});
