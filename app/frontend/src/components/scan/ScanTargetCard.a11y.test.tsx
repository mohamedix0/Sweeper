import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { ScanTargetCard } from './ScanTargetCard';
import { VISIBLE_CHILD_CAP } from './scanHelpers';

function makeItems(count: number): ScanItemDto[] {
  return Array.from({ length: count }, (_, i) => ({
    id: `item-${i}`,
    targetId: 't-safe',
    path: `C:\\Temp\\file-${i}.tmp`,
    size: '100',
    mtimeUnixSecs: '1700000000',
    ageEligible: true,
  }));
}

function makeTarget(items: ScanItemDto[]): TargetScanResultDto {
  return {
    targetId: 't-safe',
    targetName: 'User Temp',
    tier: 'Tier1',
    risk: 'Safe',
    resolvedPaths: ['C:\\Temp'],
    scanned: true,
    statusReason: null,
    totalBytes: '10000',
    eligibleBytes: '10000',
    fileCount: String(items.length),
    eligibleCount: String(items.length),
    skippedCount: '0',
    items,
  };
}

function FocusHarness() {
  const [showAll, setShowAll] = useState(false);
  const items = makeItems(VISIBLE_CHILD_CAP + 5);
  return (
    <ScanTargetCard
      target={makeTarget(items)}
      visibleItems={items}
      allowDeleteUnknown={false}
      minAgeHours={24}
      selectedIds={new Set<string>()}
      collapsed={false}
      showAll={showAll}
      focusedItemId={null}
      groupActive={false}
      cleaning={false}
      onFocusGroup={() => {}}
      onToggleCollapsed={() => {}}
      onToggleGroupSelect={() => {}}
      onToggleItem={() => {}}
      onToggleShowAll={() => setShowAll((value) => !value)}
    />
  );
}

describe('ScanTargetCard show-more / show-fewer focus', () => {
  it('restores focus to the replacement Show fewer / Show more control after toggling', async () => {
    const user = userEvent.setup();
    render(<FocusHarness />);

    const showMore = screen.getByRole('button', { name: /Show more/ });
    showMore.focus();
    expect(document.activeElement).toBe(showMore);

    await user.click(showMore);
    const showFewer = screen.getByRole('button', { name: 'Show fewer files' });
    expect(screen.queryByRole('button', { name: /Show more/ })).not.toBeInTheDocument();
    expect(document.activeElement).toBe(showFewer);

    await user.click(showFewer);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /Show more/ }));
  });
});
