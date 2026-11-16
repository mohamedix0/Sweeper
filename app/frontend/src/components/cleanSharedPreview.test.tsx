import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PreviewBreakdownBlock } from './cleanSharedPreview';
import type { PreviewResult } from './cleanSharedCore';

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
