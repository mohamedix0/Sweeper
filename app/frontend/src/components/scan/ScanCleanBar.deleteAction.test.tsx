import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ScanCleanBar, type ScanCleanBarProps } from './ScanCleanBar';

const LARGE_BYTES = '27520002949'; // formats to 25.62 GB

function barProps(input: Partial<ScanCleanBarProps> = {}): ScanCleanBarProps {
  return {
    selectedIdsCount: 30461,
    selectedCategoryCount: 3,
    selectedBytes: LARGE_BYTES,
    cleaning: false,
    previewLoading: false,
    hasPreview: true,
    canPreview: false,
    canClean: true,
    reviewPending: false,
    onClear: vi.fn(),
    onPreview: vi.fn(),
    onClean: vi.fn(),
    ...input,
  };
}

describe('ScanCleanBar post-Preview delete action', () => {
  it('shows a prominent enabled delete CTA with the exact selected count and reclaimable size', () => {
    render(<ScanCleanBar {...barProps()} />);

    const cta = screen.getByRole('button', { name: 'Clean Permanently' });
    expect(cta).toBeEnabled();
    expect(cta).toHaveTextContent('Delete 30,461 files permanently · 25.62 GB');
    expect(cta).toHaveClass('danger', 'primary');
    expect(screen.queryByText(/Preparing safety review/)).not.toBeInTheDocument();
  });

  it('invokes the single clean handler when the delete CTA is clicked', async () => {
    const onClean = vi.fn();
    const user = userEvent.setup();
    render(<ScanCleanBar {...barProps({ onClean })} />);

    await user.click(screen.getByRole('button', { name: 'Clean Permanently' }));

    expect(onClean).toHaveBeenCalledTimes(1);
  });

  it('shows an explicit pending status instead of a silently disabled CTA while safety review resolves', () => {
    render(<ScanCleanBar {...barProps({ reviewPending: true, canClean: false })} />);

    const status = screen.getByRole('status');
    expect(status).toHaveTextContent('Preparing safety review… 30,461 items');
    expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeDisabled();
  });

  it('keeps the deselect control visually distinct from the destructive action', () => {
    render(<ScanCleanBar {...barProps()} />);

    const deselect = screen.getByRole('button', { name: 'Unselect all' });
    const cta = screen.getByRole('button', { name: 'Clean Permanently' });
    expect(deselect).toHaveClass('subtle-btn');
    expect(deselect).not.toHaveClass('danger');
    expect(cta).toHaveClass('danger');
  });
});
