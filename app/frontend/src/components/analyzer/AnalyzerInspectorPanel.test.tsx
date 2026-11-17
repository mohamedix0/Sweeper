import { render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it } from 'vitest';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { AnalyzerInspectorPanel } from './AnalyzerInspectorPanel';
import type { PreviewRiskCalloutSource } from '../PreviewRiskCallouts';

type PanelProps = ComponentProps<typeof AnalyzerInspectorPanel>;
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
    itemId: 'n1',
    title: 'app.log',
    description: KB_EXPLANATION,
    risk: 'Caution',
    ...overrides,
  };
}

function source(overrides: Partial<PreviewRiskCalloutSource> = {}): PreviewRiskCalloutSource {
  return {
    itemId: 'n1',
    title: 'app.log',
    description: KB_EXPLANATION,
    risk: 'Caution',
    targetId: 't-caution',
    ...overrides,
  };
}

function node(overrides: Partial<AnalyzerNodeDto> = {}): AnalyzerNodeDto {
  return {
    id: 'n1',
    name: 'app.log',
    path: 'C:\\ProgramData\\Vendor\\Logs\\app.log',
    isDir: false,
    sizeBytes: '100',
    itemCount: '1',
    risk: 'Caution',
    description: KB_EXPLANATION,
    reason: null,
    tier: 'Tier2',
    targetId: 't-caution',
    isLinkPoint: false,
    isCloudPlaceholder: false,
    displayOnly: false,
    selectable: true,
    children: [],
    ...overrides,
  };
}

function panelProps(overrides: Partial<PanelProps> = {}): PanelProps {
  return {
    inspected: inspection(),
    inspectedNode: node(),
    selectedIds: new Set(['n1']),
    selectionSources: [source()],
    allowDeleteUnknown: true,
    requiresConfirmation: false,
    confirmText: '',
    onConfirmText: () => {},
    preview,
    cleaning: false,
    cleanProgressLabel: null,
    cleanSummary: null,
    onOpenLog: () => {},
    ...overrides,
  };
}

function renderPanel(overrides: Partial<PanelProps> = {}) {
  return render(<AnalyzerInspectorPanel {...panelProps(overrides)} />);
}

function calloutGroup() {
  return screen.getByRole('group', { name: 'Elevated risk in this selection' });
}

describe('AnalyzerInspectorPanel Preview Risk callouts', () => {
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
      inspected: inspection({ itemId: 'n2', risk: 'Unknown', title: 'Unrelated inspected item' }),
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
      selectedIds: new Set(['n1', 'n2', 'n3']),
      selectionSources: [
        source({ itemId: 'n1', title: 'Vendor Logs', risk: 'Caution' }),
        source({ itemId: 'n2', title: 'Orphaned cache', risk: 'Unknown', description: 'KB: unrecognised cache.' }),
        source({ itemId: 'n3', title: 'Added folder', risk: 'Unknown', targetId: 'custom-path-9' }),
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
      inspected: inspection({ itemId: 'n2', title: 'Inspected but unselected', risk: 'Caution' }),
      selectedIds: new Set(['n1']),
      selectionSources: [
        source({ itemId: 'n1', title: 'Selected item', risk: 'Safe' }),
        source({ itemId: 'n2', title: 'Inspected but unselected', risk: 'Caution' }),
      ],
    });

    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
    expect(screen.queryByRole('note', { name: 'Caution risk: Inspected but unselected' })).not.toBeInTheDocument();
  });

  it('refreshes the callout list when the selection changes', () => {
    const { rerender } = renderPanel({
      selectedIds: new Set(['n1']),
      selectionSources: [source({ itemId: 'n1', title: 'First', risk: 'Caution' })],
    });
    expect(within(calloutGroup()).getByRole('note', { name: 'Caution risk: First' })).toBeInTheDocument();

    rerender(<AnalyzerInspectorPanel {...panelProps({
      selectedIds: new Set(['n2', 'n3']),
      selectionSources: [
        source({ itemId: 'n2', title: 'Second', risk: 'Unknown' }),
        source({ itemId: 'n3', title: 'Third', risk: 'Caution' }),
      ],
    })} />);
    const group = calloutGroup();
    expect(within(group).getAllByRole('note')).toHaveLength(2);
    expect(within(group).queryByRole('note', { name: 'Caution risk: First' })).not.toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Unknown risk: Second' })).toBeInTheDocument();
    expect(within(group).getByRole('note', { name: 'Caution risk: Third' })).toBeInTheDocument();

    rerender(<AnalyzerInspectorPanel {...panelProps({
      selectedIds: new Set(),
      selectionSources: [],
    })} />);
    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
  });
});
