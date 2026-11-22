import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SettingsCustomPathsCard } from './SettingsCustomPathsCard';
import type { Settings } from '../../api/client';

function baseDraft(): Settings {
  return {
    enableTier1: true,
    enableTier2: true,
    enableTier3: false,
    enableElevationTargets: false,
    minAgeHours: 24,
    allowDeleteUnknown: false,
    customPaths: [],
    staleProjectDays: 30,
  };
}

function renderCard() {
  render(
    <SettingsCustomPathsCard
      draft={baseDraft()}
      setDraft={vi.fn()}
      saving={false}
      validatingPath={false}
      customPathInput=""
      setCustomPathInput={vi.fn()}
      onApplyPreset={vi.fn()}
      onAdd={vi.fn()}
      onRemove={vi.fn()}
    />,
  );
}

describe('SettingsCustomPathsCard safety-setting label (#149)', () => {
  it('uses the canonical "Allow deleting unknown folders" label and notes non-custom Unknown folders', () => {
    renderCard();

    // Accessible name matches the canonical glossary term.
    const checkbox = screen.getByLabelText('Allow deleting unknown folders');
    expect(checkbox).toBeInTheDocument();
    expect(checkbox).toHaveAttribute('type', 'checkbox');

    // Visible copy matches, including the note that the same field also gates
    // non-custom Unknown folders.
    expect(screen.getByText(/Allow deleting unknown folders/i)).toBeInTheDocument();
    expect(screen.getByText(/also applies to non-custom Unknown folders/i)).toBeInTheDocument();

    // Regression guard: the drifted copy must be gone.
    expect(screen.queryByText(/Allow cleanup of custom paths/i)).not.toBeInTheDocument();
  });
});
