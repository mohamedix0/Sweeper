import { render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it } from 'vitest';
import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { ScanInspectorPanel } from './ScanInspectorPanel';

type PanelProps = ComponentProps<typeof ScanInspectorPanel>;
type Inspection = NonNullable<PanelProps['inspected']>;

const KB_EXPLANATION = 'Knowledge Base: vendor application log, regenerated on demand.';

const preview: PanelProps['preview'] = {
  previewToken: 'tok',
  breakdown: [{ targetId: 't-caution', targetName: 'Vendor Logs', selectedCount: '1', selectedBytes: '100' }],
  totalSelectedCount: '1',
  totalSelectedBytes: '100',
  issuedAtMs: 0,
};

function inspection(overrides: Partial<Inspection> = {}): Inspection {
  return {
    itemId: 'item-1',
    title: 'app.log',
    description: KB_EXPLANATION,
    risk: 'Caution',
    ...overrides,
  };
}

function item(): ScanItemDto {
  return {
    id: 'item-1',
    targetId: 't-caution',
    path: 'C:\\ProgramData\\Vendor\\Logs\\app.log',
    size: '100',
    mtimeUnixSecs: '1700000000',
    ageEligible: true,
  };
}

function target(overrides: Partial<TargetScanResultDto> = {}): TargetScanResultDto {
  return {
    targetId: 't-caution',
    targetName: 'Vendor Logs',
    tier: 'Tier2',
    risk: 'Caution',
    resolvedPaths: ['C:\\ProgramData\\Vendor\\Logs'],
    scanned: true,
    statusReason: null,
    totalBytes: '100',
    eligibleBytes: '100',
    fileCount: '1',
    eligibleCount: '1',
    skippedCount: '0',
    items: [item()],
    ...overrides,
  };
}

function panelProps(overrides: Partial<PanelProps> = {}): PanelProps {
  return {
    focus: { kind: 'item', itemId: 'item-1' },
    focusedGroup: null,
    inspected: inspection(),
    inspectedCtx: { target: target(), item: item() },
    allowDeleteUnknown: true,
    minAgeHours: null,
    selectedIds: new Set(['item-1']),
    requiresConfirmation: false,
    confirmText: '',
    onConfirmText: () => {},
    cleaning: false,
    preview,
    cleanProgressLabel: null,
    cleanSummary: null,
    onOpenLog: () => {},
    ...overrides,
  };
}

function renderPanel(overrides: Partial<PanelProps> = {}) {
  return render(<ScanInspectorPanel {...panelProps(overrides)} />);
}

function calloutGroup() {
  return screen.getByRole('group', { name: 'Elevated risk in this selection' });
}

describe('ScanInspectorPanel Preview Risk callouts', () => {
  it('passes a Caution callout with the selection identity, Risk and Knowledge Base explanation', () => {
    renderPanel({ inspected: inspection({ risk: 'Caution' }) });

    const group = calloutGroup();
    const note = within(group).getByRole('note', { name: 'Caution risk: app.log' });
    expect(within(note).getByText('Caution')).toBeInTheDocument();
    expect(within(note).getByText(KB_EXPLANATION)).toBeInTheDocument();
  });

  it('passes an Unknown callout for an Unknown selection', () => {
    renderPanel({ inspected: inspection({ risk: 'Unknown', title: 'Unrecognised folder' }) });

    const note = within(calloutGroup()).getByRole('note', { name: 'Unknown risk: Unrecognised folder' });
    expect(within(note).getByText('Unknown')).toBeInTheDocument();
  });

  it('passes a Custom Path callout when the inspected item belongs to a Custom Path target', () => {
    renderPanel({
      inspected: inspection({ risk: 'Unknown', title: 'My added folder' }),
      inspectedCtx: { target: target({ targetId: 'custom-path-1', targetName: 'Custom Path (Unknown)', risk: 'Unknown' }), item: item() },
    });

    const note = within(calloutGroup()).getByRole('note', { name: 'Custom Path risk: My added folder' });
    expect(within(note).getByText('Custom Path')).toBeInTheDocument();
  });

  it('passes no callout for a Safe-only selection', () => {
    renderPanel({ inspected: inspection({ risk: 'Safe' }) });

    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    expect(screen.getByText('Preview Breakdown')).toBeInTheDocument();
  });

  it('refreshes the callout list when the inspected selection changes', () => {
    const { rerender } = renderPanel({ inspected: inspection({ risk: 'Safe' }) });
    expect(screen.queryByRole('note')).not.toBeInTheDocument();

    rerender(<ScanInspectorPanel {...panelProps({ inspected: inspection({ risk: 'Unknown', title: 'Now selected' }) })} />);
    expect(within(calloutGroup()).getByRole('note', { name: 'Unknown risk: Now selected' })).toBeInTheDocument();

    rerender(<ScanInspectorPanel {...panelProps({ inspected: inspection({ risk: 'Safe' }) })} />);
    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
  });
});
