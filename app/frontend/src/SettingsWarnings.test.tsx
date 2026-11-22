import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { SettingsView } from './components/SettingsView';
import { mockController } from './api/mock';
import type { Settings } from './api/client';

function baseSettings(): Settings {
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

function renderSettings(settings: Settings) {
  return render(
    <SettingsView
      client={mockController}
      settings={settings}
      elevationStatus={{ isElevated: false }}
      elevationError={null}
      onSaved={() => {}}
    />,
  );
}

describe('SettingsView sanitization warning notice (#146)', () => {
  it('renders a notice when loaded settings carry sanitization warnings', () => {
    const warning = 'min_age_hours out of bounds; reset to default (24h)';
    // Cast keeps this regression test compilable against the pre-fix contract;
    // the production client normalizes the same field to an array.
    const settings = { ...baseSettings(), warnings: [warning] } as Settings;

    renderSettings(settings);

    expect(screen.getByRole('status', { name: /settings warnings/i })).toHaveTextContent(warning);
  });

  it('does not render a notice when there are no warnings', () => {
    renderSettings(baseSettings());

    expect(screen.queryByRole('status', { name: /settings warnings/i })).not.toBeInTheDocument();
  });
});
