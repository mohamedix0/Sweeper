import { render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it } from 'vitest';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { AnalyzerInspectorPanel } from './AnalyzerInspectorPanel';

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

  it('passes a Custom Path callout when the inspected node belongs to a Custom Path target', () => {
    renderPanel({
      inspected: inspection({ risk: 'Unknown', title: 'My added folder' }),
      inspectedNode: node({ targetId: 'custom-path-1', risk: 'Unknown' }),
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

    rerender(<AnalyzerInspectorPanel {...panelProps({ inspected: inspection({ risk: 'Unknown', title: 'Now selected' }) })} />);
    expect(within(calloutGroup()).getByRole('note', { name: 'Unknown risk: Now selected' })).toBeInTheDocument();

    rerender(<AnalyzerInspectorPanel {...panelProps({ inspected: inspection({ risk: 'Safe' }) })} />);
    expect(screen.queryByRole('group', { name: 'Elevated risk in this selection' })).not.toBeInTheDocument();
  });
});
