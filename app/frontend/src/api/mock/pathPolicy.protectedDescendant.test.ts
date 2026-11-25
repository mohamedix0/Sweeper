import { describe, expect, it } from 'vitest';
import { evaluateCustomPath } from './pathPolicy';

// Mirrors the synthetic `missing` / `linkpoint` convention: the mock has no
// filesystem, so this fictional segment stands in for the backend's on-disk
// `contains_subtree_protected_descendant` walk finding a `.git` (or similar)
// folder beneath the selected subtree. Kept as a literal so this test proves
// the behaviour independently of the implementation's exported constant.
const PROTECTED_DESCENDANT_SEGMENT = 'protecteddescendant';

describe('mock Custom Path mirrors backend subtree-protected-descendant policy', () => {
  it('denies a subtree containing a protected descendant (e.g. a .git folder)', () => {
    // Backend: policy.rs::contains_subtree_protected_descendant -> deny.
    // Mock before the fix: this path was allowed because `protecteddescendant`
    // is an ordinary segment and no descendant walk is performed.
    const result = evaluateCustomPath(`C:\\Temp\\Jobs\\${PROTECTED_DESCENDANT_SEGMENT}`);
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('subtree-protected descendant');
  });

  it('detects the synthetic marker case-insensitively (ASCII-folding parity)', () => {
    const result = evaluateCustomPath(`C:\\Temp\\Jobs\\${PROTECTED_DESCENDANT_SEGMENT.toUpperCase()}`);
    expect(result.allowed).toBe(false);
    expect(result.reason).toContain('subtree-protected descendant');
  });

  it('allows a benign subtree with no protected descendant', () => {
    expect(evaluateCustomPath('C:\\Temp\\Jobs\\scratch')).toEqual({
      path: 'C:\\Temp\\Jobs\\scratch',
      allowed: true,
      reason: 'ok',
    });
  });
});
