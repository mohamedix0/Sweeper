import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PreviewBreakdownBlock } from './cleanSharedPreview';
import type { PreviewResult } from './cleanSharedCore';
import type { PreviewRiskCalloutItem } from './PreviewRiskCallouts';

function preview(overrides: Partial<PreviewResult> = {}): PreviewResult {
  return {
    previewToken: 'tok',
    breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '2', selectedBytes: '200' }],
    totalSelectedCount: '2',
    totalSelectedBytes: '200',
    issuedAtMs: 0,
    ...overrides,
  };
}

function calloutItem(overrides: Partial<PreviewRiskCalloutItem> = {}): PreviewRiskCalloutItem {
  return {
    id: 'i-1',
    identity: 'Regenerable cache',
    risk: 'Caution',
    explanation: 'Existing Knowledge Base explanation for this item.',
    ...overrides,
  };
}

function renderPreview(items: PreviewRiskCalloutItem[], previewOverrides: Partial<PreviewResult> = {}) {
  return (
    <PreviewBreakdownBlock
      preview={preview(previewOverrides)}
      cleaning={false}
      cleanProgressLabel={null}
      calloutItems={items}
    />
  );
}

describe('PreviewBreakdownBlock skipped preview items', () => {
  it('shows skippedCount and the skippedReasons list when present', () => {
    render(
      <PreviewBreakdownBlock
        preview={preview({ skippedCount: '1', skippedReasons: ['selected item no longer exists'] })}
        cleaning={false}
        cleanProgressLabel={null}
      />,
    );

    const skipped = screen.getByRole('status', { name: 'Preview skipped items' });
    expect(within(skipped).getByText('Skipped:')).toBeInTheDocument();
    expect(within(skipped).getByText('1')).toBeInTheDocument();
    expect(within(skipped).getByText('selected item no longer exists')).toBeInTheDocument();
  });

  it('renders nothing for skipped fields when absent', () => {
    render(
      <PreviewBreakdownBlock preview={preview()} cleaning={false} cleanProgressLabel={null} />,
    );

    expect(screen.queryByRole('status', { name: 'Preview skipped items' })).not.toBeInTheDocument();
    expect(screen.queryByText('Skipped:')).not.toBeInTheDocument();
  });
});

describe('PreviewBreakdownBlock per-item Risk callouts', () => {
  it('renders no callout and leaves the breakdown layout unchanged for a Safe-only selection', () => {
    render(renderPreview([calloutItem({ risk: 'Safe', identity: 'User Temp' })]));

    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    expect(screen.queryByText('Review elevated risk before confirming')).not.toBeInTheDocument();
    expect(screen.getByRole('table', { name: 'Preview breakdown by Target' })).toBeInTheDocument();
    expect(screen.getByText('Total selected:')).toBeInTheDocument();
  });

  it('renders one callout with identity, Risk badge, explanation and one consequence for a Caution item', () => {
    render(renderPreview([calloutItem()]));

    const notes = screen.getAllByRole('note');
    expect(notes).toHaveLength(1);
    const note = notes[0];
    expect(note).toHaveAccessibleName('Caution risk: Regenerable cache');
    expect(within(note).getByText('Caution')).toBeInTheDocument();
    expect(within(note).getByText('Warning')).toBeInTheDocument();
    expect(within(note).getByText('Regenerable cache')).toBeInTheDocument();
    expect(within(note).getByText('Existing Knowledge Base explanation for this item.')).toBeInTheDocument();
    expect(within(note).getByText(/Removing this item is permanent/)).toBeInTheDocument();
  });

  it('renders a callout for an Unknown item', () => {
    render(renderPreview([calloutItem({ id: 'i-u', risk: 'Unknown', identity: 'Unrecognised folder' })]));

    const note = screen.getByRole('note', { name: 'Unknown risk: Unrecognised folder' });
    expect(within(note).getByText('Unknown')).toBeInTheDocument();
    expect(within(note).getByText(/does not recognise this item/)).toBeInTheDocument();
  });

  it('renders a callout for a Custom Path item', () => {
    render(renderPreview([calloutItem({ id: 'i-cp', risk: 'Custom Path', identity: 'My added folder' })]));

    const note = screen.getByRole('note', { name: 'Custom Path risk: My added folder' });
    expect(within(note).getByText('Custom Path')).toBeInTheDocument();
    expect(within(note).getByText(/only its contents will be removed/)).toBeInTheDocument();
  });

  it('renders callouts only for elevated-Risk items in a mixed selection and never for Protected items', () => {
    render(
      renderPreview([
        calloutItem({ id: 'i-s', risk: 'Safe', identity: 'Safe temp' }),
        calloutItem({ id: 'i-c', risk: 'Caution', identity: 'Caution cache' }),
        calloutItem({ id: 'i-p', risk: 'Protected', identity: 'Protected system folder' }),
      ]),
    );

    expect(screen.getAllByRole('note')).toHaveLength(1);
    expect(screen.getByRole('note', { name: 'Caution risk: Caution cache' })).toBeInTheDocument();
    expect(screen.queryByText('Protected system folder')).not.toBeInTheDocument();
    expect(screen.queryByText('Safe temp')).not.toBeInTheDocument();
  });

  it('keeps the skipped-item notice visible without duplicating it as a callout', () => {
    render(
      renderPreview(
        [calloutItem({ id: 'i-c', risk: 'Caution', identity: 'Caution cache' })],
        { skippedCount: '1', skippedReasons: ['selected item no longer exists'] },
      ),
    );

    const skipped = screen.getByRole('status', { name: 'Preview skipped items' });
    expect(within(skipped).getByText('selected item no longer exists')).toBeInTheDocument();
    expect(screen.getAllByRole('note')).toHaveLength(1);
    expect(within(screen.getByRole('note')).queryByText('selected item no longer exists')).not.toBeInTheDocument();
  });

  it('updates the callout list immediately when the current selection changes', () => {
    const { rerender } = render(renderPreview([calloutItem({ risk: 'Safe' })]));
    expect(screen.queryByRole('note')).not.toBeInTheDocument();

    rerender(renderPreview([calloutItem({ id: 'i-u', risk: 'Unknown', identity: 'Now selected' })]));
    expect(screen.getAllByRole('note')).toHaveLength(1);
    expect(screen.getByRole('note', { name: 'Unknown risk: Now selected' })).toBeInTheDocument();

    rerender(renderPreview([]));
    expect(screen.queryByRole('note')).not.toBeInTheDocument();
  });

  it('does not steal focus from the typed confirmation control and keeps callouts keyboard reachable', () => {
    render(
      <>
        <input aria-label="Typed confirmation" autoFocus />
        {renderPreview([calloutItem()])}
      </>,
    );

    expect(screen.getByLabelText('Typed confirmation')).toHaveFocus();

    const note = screen.getByRole('note');
    note.focus();
    expect(note).toHaveFocus();
    expect(screen.getByLabelText('Typed confirmation')).not.toHaveFocus();
  });

  it('renders the callout group above the breakdown table in document order', () => {
    render(renderPreview([calloutItem()]));

    const group = screen.getByRole('group', { name: 'Elevated risk in this selection' });
    const table = screen.getByRole('table', { name: 'Preview breakdown by Target' });
    expect(group.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
