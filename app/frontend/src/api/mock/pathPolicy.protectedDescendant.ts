/**
 * Synthetic convention for the backend's on-disk protected-descendant rule.
 *
 * The mock backend has no filesystem, so it cannot walk a selected directory
 * the way `sweeper-core::policy::contains_subtree_protected_descendant` does.
 * Mirroring the existing `missing` / `linkpoint` segments, a path segment with
 * this fictional name stands in for "this subtree contains a subtree-protected
 * descendant (e.g. a `.git` folder)" and must be denied by Policy.
 *
 * Kept in its own module so it can be wired into `pathPolicy.ts` with a single
 * import + guard, avoiding edits to that file's header or lexical helpers.
 */
export const PROTECTED_DESCENDANT_SEGMENT = 'protecteddescendant';

/**
 * True when any already ASCII-folded segment is the synthetic marker.
 *
 * Mirrors the backend rule that selecting a subtree which contains a
 * subtree-protected descendant is denied. Comparison is against ASCII-folded
 * text, matching Rust `eq_ignore_ascii_case` / `to_ascii_uppercase` semantics.
 */
export function hasSyntheticProtectedDescendant(foldedSegments: readonly string[]): boolean {
  return foldedSegments.some((segment) => segment === PROTECTED_DESCENDANT_SEGMENT);
}
