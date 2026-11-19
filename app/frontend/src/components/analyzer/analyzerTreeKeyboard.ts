/**
 * Keyboard and focus helpers for the Analyzer tree (issue #103).
 *
 * The role=tree container is the single tab stop and the only DOM focus
 * holder; treeitems are addressed through aria-activedescendant instead of
 * roving tabIndex. Keeping the decision logic here makes that single focus
 * model easy to review and keeps the AnalyzerResults component simple.
 */

import type { KeyboardEvent } from 'react';
import { showAllButtonDomId, treeItemDomId } from './analyzerTreeA11y';

/** Resolve the active id to a currently visible row, if any. */
export function resolveActiveId(rowIds: readonly string[], activeId: string | null): string | null {
  if (rowIds.length === 0) return null;
  return activeId && rowIds.includes(activeId) ? activeId : rowIds[0];
}

/** Compute the next active id for an arrow key, clamped to the row list. */
export function rowIdForArrowKey(ids: readonly string[], index: number, key: string): string | null {
  if (key === 'ArrowDown') return ids[Math.min(index + 1, ids.length - 1)];
  if (key === 'ArrowUp') return ids[Math.max(index - 1, 0)];
  return null;
}

export type TreeKeyOutcome = { readonly kind: 'ignored' } | { readonly kind: 'moved'; readonly nextId: string };

/**
 * Classify a keydown on the tree: only arrow keys originating on the tree
 * container itself move the active item. Events from inner controls (the
 * Show more / Show fewer buttons) keep their native keyboard behaviour.
 */
export function classifyTreeKeyDown(
  event: KeyboardEvent<HTMLDivElement>,
  rowIds: readonly string[],
  activeId: string | null,
): TreeKeyOutcome {
  if (event.target !== event.currentTarget || rowIds.length === 0) return { kind: 'ignored' };
  const currentId = resolveActiveId(rowIds, activeId);
  if (!currentId) return { kind: 'ignored' };
  const nextId = rowIdForArrowKey(rowIds, rowIds.indexOf(currentId), event.key);
  if (!nextId) return { kind: 'ignored' };
  return { kind: 'moved', nextId };
}

/** Apply a classified keydown: move the active item without moving DOM focus. */
export function handleTreeKeyDown(
  event: KeyboardEvent<HTMLDivElement>,
  rowIds: readonly string[],
  activeId: string | null,
  setActiveId: (nextId: string) => void,
): void {
  const outcome = classifyTreeKeyDown(event, rowIds, activeId);
  if (outcome.kind !== 'moved') return;
  event.preventDefault();
  setActiveId(outcome.nextId);
}

/** aria-activedescendant value for the active row, or undefined when invalid. */
export function activeDescendantFor(rowIds: readonly string[], activeId: string | null): string | undefined {
  return activeId && rowIds.includes(activeId) ? treeItemDomId(activeId) : undefined;
}

/**
 * Take (and clear) a pending Show more / Show fewer focus target, returning
 * the DOM id of the replacement control. Called from useLayoutEffect so
 * focus is restored after the replacement row has been committed.
 */
export function consumePendingShowAllFocus(pendingRef: { current: string | null }): string | null {
  const parentId = pendingRef.current;
  if (!parentId) return null;
  pendingRef.current = null;
  return showAllButtonDomId(parentId);
}
