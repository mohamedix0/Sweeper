import { render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it } from 'vitest';
import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { ScanInspectorPanel } from './ScanInspectorPanel';
import type { PreviewRiskCalloutSource } from '../PreviewRiskCallouts';

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

function source(overrides: Partial<PreviewRiskCalloutSource> = {}): PreviewRiskCalloutSource {
  return {
    itemId: 'item-1',
    title: 'app.log',
    description: KB_EXPLANATION,
    risk: 'Caution',
    targetId: 't-caution',
    ...overrides,
  };
}

function item(overrides: Partial<ScanItemDto> = {}): ScanItemDto {
  return {
    id: 'item-1',
    targetId: 't-caution',
    path: 'C:\\ProgramData\\Vendor\\Logs\\app.log',
    size: '100',
    mtimeUnixSecs: '1700000000',
    ageEligible: true,
    ...overrides,
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
    selectionSources: [source()],
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
  it('passes a Caution callout with the selected item identity, Risk and Knowledge Base explanation', () => {
    renderPanel({ selectionSources: [source({ risk: 'Caution' })] });

    const group = calloutGroup();
    const note = within(group).getByRole('note', { name: 'Caution risk: app.log' });
    expect(within(note).getByText('Caution')).toBeInTheDocument();
    expect(within(note).getByText(KB_EXPLANATION)).toBeInTheDocument();
  });

  it('passes an Unknown callout for a selected Unknown item', () => {
    renderPanel({ selectionSources: [source({ risk: 'Unknown', title: 'Unrecognised folder' })] });

    const note = within(calloutGroup()).getByRole('note', { name: 'Unknown risk: Unrecognised folder' });
    expect(within(note).getByText('Unknown')).toBeInTheDocument();
  });

  it('passes a Custom Path callout when a selected item belongs to a Custom Path target', () => {
    renderPanel({
      inspected: inspection({ itemId: 'item-2', risk: 'Unknown', title: 'Unrelated inspected item' }),
      selectionSources: [source({ risk: 'Unknown', title: 'My added folder', targetId: 'custom-path-1' })],
    });

    const note = within(calloutGroup()).getByRole('note', { name: 'Custom Path risk: My added folder' });
    expect(within(note).getByText('Custom Path')).toBeInTheDocument();
  });

  it('passes no callout for a Safe-only selection', () => {
    renderPanel({ selectionSources: [source({ risk: 'Safe', title: 'User Temp' })] });

    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    expect(screen.getByText('Preview Breakdown')).toBeInTheDocument();
  });

  it('passes one callout per selected elevated-Risk item', () => {
    renderPanel({
      selectedIds: new Set(['item-1', 'item-2', 'item-3']),
      selectionSources: [
        source({ itemId: 'item-1', title: 'Vendor Logs', risk: 'Caution' }),
        source({ itemId: 'item-2', title: 'Orphaned cache', risk: 'Unknown', description: 'KB: unrecognised cache.' }),
        source({ itemId: 'item-3', title: 'Added folder', risk: 'Unknown', targetId: 'custom-path-9' }),
      ],
    });

    const group = calloutGroup();
    expect(within(group).getAllByRole('note')).toHaveLength(3);
    expect(within(group).getByRole('note', { name: 'Caution risk: Vendor Logs' })).toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Unknown risk: Orphaned cache' })).toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Custom Path risk: Added folder' })).toBeInTheDocument();
  });

  it('passes no callout for an inspected item that is not selected', () => {
    renderPanel({
      inspected: inspection({ itemId: 'item-2', title: 'Inspected but unselected', risk: 'Caution' }),
      selectedIds: new Set(['item-1']),
      selectionSources: [
        source({ itemId: 'item-1', title: 'Selected item', risk: 'Safe' }),
        source({ itemId: 'item-2', title: 'Inspected but unselected', risk: 'Caution' }),
      ],
    });

    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
    expect(screen.queryByRole('note', { name: 'Caution risk: Inspected but unselected' })).not.toBeInTheDocument();
  });

  it('refreshes the callout list when the selection changes', () => {
    const { rerender } = renderPanel({
      selectedIds: new Set(['item-1']),
      selectionSources: [source({ itemId: 'item-1', title: 'First', risk: 'Caution' })],
    });
    expect(within(calloutGroup()).getByRole('note', { name: 'Caution risk: First' })).toBeInTheDocument();

    rerender(<ScanInspectorPanel {...panelProps({
      selectedIds: new Set(['item-2', 'item-3']),
      selectionSources: [
        source({ itemId: 'item-2', title: 'Second', risk: 'Unknown' }),
        source({ itemId: 'item-3', title: 'Third', risk: 'Caution' }),
      ],
    })} />);
    const group = calloutGroup();
    expect(within(group).getAllByRole('note')).toHaveLength(2);
    expect(within(group).queryByRole('note', { name: 'Caution risk: First' })).not.toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Unknown risk: Second' })).toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Caution risk: Third' })).toBeInTheDocument();

    rerender(<ScanInspectorPanel {...panelProps({
      selectedIds: new Set(),
      selectionSources: [],
    })} />);
    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
  });
});
