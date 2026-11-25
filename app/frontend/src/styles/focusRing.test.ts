import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Vitest runs with the frontend package as its cwd (see checkAudit.test.ts).
const tokensCss = readFileSync(resolve(process.cwd(), 'src', 'styles', 'tokens.css'), 'utf8');
const appCss = readFileSync(resolve(process.cwd(), 'src', 'App.css'), 'utf8');

function lightThemeBlock(): string {
  const start = tokensCss.indexOf('[data-theme="light"]');
  expect(start).toBeGreaterThanOrEqual(0);
  // The theme blocks in tokens.css contain custom-property declarations only.
  const end = tokensCss.indexOf('}', start);
  return tokensCss.slice(start, end);
}

function parseFocusRing(block: string): { r: number; g: number; b: number; alpha: number } {
  const match = block.match(
    /--focus-ring:\s*0 0 0 2px rgba\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(0?\.\d+|\d+)\s*\)/,
  );
  expect(match).not.toBeNull();
  return {
    r: Number(match![1]),
    g: Number(match![2]),
    b: Number(match![3]),
    alpha: Number(match![4]),
  };
}

function srgbToLinear(value: number): number {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function relativeLuminance(r: number, g: number, b: number): number {
  return 0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);
}

function contrastAgainstWhite({ r, g, b, alpha }: ReturnType<typeof parseFocusRing>): number {
  // Composite the translucent ring over the light-theme white surface (#FFFFFF).
  const over = (channel: number) => alpha * channel + (1 - alpha) * 255;
  const luminance = relativeLuminance(over(r), over(g), over(b));
  return 1.05 / (luminance + 0.05);
}

describe('focus indicator tokens (issue #139)', () => {
  it('light-theme focus ring clears WCAG 2.2 Focus Appearance 3:1 against white', () => {
    const contrast = contrastAgainstWhite(parseFocusRing(lightThemeBlock()));
    expect(contrast).toBeGreaterThanOrEqual(3);
  });

  it('App.css restores a system-color outline under forced-colors (Windows High Contrast)', () => {
    const forcedColorsBlock = appCss.match(/@media \(forced-colors: active\)\s*\{([\s\S]*?)\}/);
    expect(forcedColorsBlock).not.toBeNull();
    expect(forcedColorsBlock![1]).toMatch(/outline:\s*2px solid Highlight/);
  });
});
