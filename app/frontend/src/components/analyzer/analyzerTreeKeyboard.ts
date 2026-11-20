/**
 * Keyboard and focus helpers for the Analyzer tree (issue #103).
 *
 * The role=tree container is the single sequential tab stop and the only
 * container that holds DOM focus during navigation; treeitems are addressed
 * through aria-activedescendant instead of roving tabIndex. Descendant row
 * controls carry tabIndex={-1}: they are reached through an explicit
 * interaction mode entered with ArrowRight from the active row and exited with
 * Escape, which keeps every action keyboard-reachable without extra tab stops.
 * Keeping the decision logic here makes that model easy to review and keeps
 * the AnalyzerResults component simple (CodeScene complexity gate).
 */

import type { KeyboardEvent } from 'react';
import { ANALYZER_TREE_DOM_ID, TREE_CONTROL_ATTR, showAllButtonDomId, treeItemDomId } from './analyzerTreeA11y';

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

export type TreeKeyContext = {
  rowIds: readonly string[];
  activeId: string | null;
  setActiveId: (id: string) => void;
  activateRow: (id: string) => void;
  collapseRow: (id: string) => void;
};

/** Return DOM focus to the single tree tab stop. */
export function focusTree(): void {
  document.getElementById(ANALYZER_TREE_DOM_ID)?.focus();
}

function controlsForRow(rowElement: Element | null): HTMLElement[] {
  if (!rowElement) return [];
  return Array.from(rowElement.querySelectorAll<HTMLElement>(`[${TREE_CONTROL_ATTR}]`));
}

function controlsForActiveRow(rowIds: readonly string[], activeId: string | null): HTMLElement[] {
  const id = resolveActiveId(rowIds, activeId);
  return id ? controlsForRow(document.getElementById(treeItemDomId(id))) : [];
}

/**
 * Apply a keydown on the tree. Keys originating on the tree container navigate
 * rows / activate the active row / enter interaction mode; keys originating on
 * a row control are handled in interaction mode and otherwise keep their
 * native behaviour (notably Enter/Space activation of the focused control).
 */
export function handleTreeKeyDown(event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext): void {
  if (event.target === event.currentTarget) {
    handleTreeModeKeyDown(event, context);
    return;
  }
  handleControlModeKeyDown(event, context);
}

function handleTreeModeKeyDown(event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext): void {
  const currentId = resolveActiveId(context.rowIds, context.activeId);
  if (!currentId) return;

  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    const nextId = rowIdForArrowKey(context.rowIds, context.rowIds.indexOf(currentId), event.key);
    if (!nextId) return;
    event.preventDefault();
    context.setActiveId(nextId);
    return;
  }

  // Enter / Space activate the active row even though DOM focus stays on the
  // tree, so the per-row onRowKeyDown handler never sees these keys.
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    context.activateRow(currentId);
    return;
  }

  // ArrowRight enters the interaction mode on the active row's first control.
  if (event.key === 'ArrowRight') {
    const controls = controlsForActiveRow(context.rowIds, context.activeId);
    if (controls.length === 0) return;
    event.preventDefault();
    controls[0].focus();
    return;
  }

  // ArrowLeft follows the APG tree pattern: collapse an expanded active row.
  if (event.key === 'ArrowLeft') {
    const rowElement = document.getElementById(treeItemDomId(currentId));
    if (rowElement?.getAttribute('aria-expanded') !== 'true') return;
    event.preventDefault();
    context.collapseRow(currentId);
  }
}

function handleControlModeKeyDown(event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext): void {
  const target = event.target as HTMLElement | null;
  if (!target || typeof target.hasAttribute !== 'function' || !target.hasAttribute(TREE_CONTROL_ATTR)) return;
  // Ignore bubbled synthetic keydowns where the control is not actually focused.
  if (document.activeElement !== target) return;

  if (event.key === 'Escape') {
    event.preventDefault();
    focusTree();
    return;
  }

  if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
    const controls = controlsForRow(target.closest('[role="treeitem"]'));
    const index = controls.indexOf(target);
    if (index < 0) return;
    const next = event.key === 'ArrowRight'
      ? Math.min(index + 1, controls.length - 1)
      : Math.max(index - 1, 0);
    event.preventDefault();
    controls[next]?.focus();
    return;
  }

  // Up / Down while interacting exit to tree navigation on the adjacent row.
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    const currentId = resolveActiveId(context.rowIds, context.activeId);
    if (!currentId) return;
    const nextId = rowIdForArrowKey(context.rowIds, context.rowIds.indexOf(currentId), event.key);
    if (!nextId) return;
    event.preventDefault();
    context.setActiveId(nextId);
    focusTree();
    return;
  }

  // Buttons natively activate on both Enter and Space, and checkboxes on
  // Space. Enter on a focused checkbox is the one gap to fill explicitly.
  if (event.key === 'Enter' && target instanceof HTMLInputElement && target.type === 'checkbox' && !target.disabled) {
    event.preventDefault();
    target.click();
  }
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
