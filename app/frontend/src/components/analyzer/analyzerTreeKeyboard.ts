/**
 * Keyboard and focus helpers for the Analyzer tree (issue #103).
 *
 * The role=tree container is the single sequential tab stop and the only
 * container that holds DOM focus during navigation; treeitems are addressed
 * through aria-activedescendant instead of roving tabIndex. Descendant row
 * controls carry tabIndex={-1}: they are reached through an explicit
 * interaction mode entered with ArrowRight from the active row and exited with
 * Escape, which keeps every action keyboard-reachable without extra tab stops.
 *
 * Key handling is a data-driven dispatch: each mode has a small key -> action
 * table, and every action is a named, single-purpose function that reports
 * whether it handled the key. The two entry points therefore only resolve the
 * context, look up the action and apply preventDefault once, which keeps the
 * decision logic reviewable (CodeScene complexity gate).
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

/** True when the bubbled event came from the control that actually has focus. */
function isFocusedTreeControl(value: EventTarget | null): value is HTMLElement {
  if (!(value instanceof HTMLElement)) return false;
  if (!value.hasAttribute(TREE_CONTROL_ATTR)) return false;
  return document.activeElement === value;
}

/**
 * Owning row id of a focused control, read from the DOM rather than from the
 * tree's activeId. This also covers the replacement Show more / Show fewer
 * button: its owning row is the pseudo-row that was just re-rendered.
 */
function rowIdForControl(target: HTMLElement, rowIds: readonly string[]): string | null {
  const rowElement = target.closest('[role="treeitem"]');
  if (!rowElement) return null;
  const prefix = treeItemDomId('');
  if (!rowElement.id.startsWith(prefix)) return null;
  const rowId = rowElement.id.slice(prefix.length);
  return rowIds.includes(rowId) ? rowId : null;
}

/** Actions report whether they handled a key, so prevention is applied once. */
type TreeModeAction = (event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext, currentId: string) => boolean;
type ControlModeAction = (event: KeyboardEvent<HTMLDivElement>, target: HTMLElement, context: TreeKeyContext) => boolean;

/** ArrowUp/Down move the active row; navigation is clamped by rowIdForArrowKey. */
function moveActiveRow(event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext, currentId: string): boolean {
  const nextId = rowIdForArrowKey(context.rowIds, context.rowIds.indexOf(currentId), event.key);
  if (!nextId) return false;
  context.setActiveId(nextId);
  return true;
}

/** Enter / Space activate the active row while DOM focus stays on the tree. */
function activateActiveRow(_event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext, currentId: string): boolean {
  context.activateRow(currentId);
  return true;
}

/** ArrowRight enters interaction mode on the active row's first control. */
function enterInteractionMode(_event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext, currentId: string): boolean {
  const controls = controlsForActiveRow(context.rowIds, currentId);
  if (controls.length === 0) return false;
  controls[0].focus();
  return true;
}

/** ArrowLeft follows the APG tree pattern: collapse an expanded active row. */
function collapseActiveRow(_event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext, currentId: string): boolean {
  const rowElement = document.getElementById(treeItemDomId(currentId));
  if (rowElement?.getAttribute('aria-expanded') !== 'true') return false;
  context.collapseRow(currentId);
  return true;
}

/** Escape returns focus to the tree with the control's row as the active row. */
function returnFocusToTree(
  _event: KeyboardEvent<HTMLDivElement>,
  target: HTMLElement,
  context: TreeKeyContext,
): boolean {
  const originId = rowIdForControl(target, context.rowIds);
  if (originId) context.setActiveId(originId);
  focusTree();
  return true;
}

/** Direction of a horizontal control-cycle arrow. */
function controlStep(key: string): number {
  return key === 'ArrowRight' ? 1 : -1;
}

/** ArrowLeft/ArrowRight cycle the focused row's controls. */
function cycleRowControl(event: KeyboardEvent<HTMLDivElement>, target: HTMLElement, _context: TreeKeyContext): boolean {
  const controls = controlsForRow(target.closest('[role="treeitem"]'));
  const index = controls.indexOf(target);
  if (index < 0) return false;
  const next = Math.min(Math.max(index + controlStep(event.key), 0), controls.length - 1);
  controls[next]?.focus();
  return true;
}

/** Up / Down while interacting navigate from the focused control's own row. */
function navigateFromControl(event: KeyboardEvent<HTMLDivElement>, target: HTMLElement, context: TreeKeyContext): boolean {
  const originId = rowIdForControl(target, context.rowIds);
  if (!originId) return false;
  const nextId = rowIdForArrowKey(context.rowIds, context.rowIds.indexOf(originId), event.key);
  if (!nextId) return false;
  context.setActiveId(nextId);
  focusTree();
  return true;
}

/** Buttons activate natively; Enter on a focused, enabled checkbox is the gap. */
function clickEnabledCheckbox(_event: KeyboardEvent<HTMLDivElement>, target: HTMLElement, _context: TreeKeyContext): boolean {
  if (!(target instanceof HTMLInputElement)) return false;
  if (target.type !== 'checkbox') return false;
  if (target.disabled) return false;
  target.click();
  return true;
}

// Map lookup (not a plain object) so inherited names such as "toString" can
// never be mistaken for an action for an otherwise unhandled key.
const TREE_MODE_ACTIONS: ReadonlyMap<string, TreeModeAction> = new Map<string, TreeModeAction>([
  ['ArrowDown', moveActiveRow],
  ['ArrowUp', moveActiveRow],
  ['Enter', activateActiveRow],
  [' ', activateActiveRow],
  ['ArrowRight', enterInteractionMode],
  ['ArrowLeft', collapseActiveRow],
]);

const CONTROL_MODE_ACTIONS: ReadonlyMap<string, ControlModeAction> = new Map<string, ControlModeAction>([
  ['Escape', returnFocusToTree],
  ['ArrowRight', cycleRowControl],
  ['ArrowLeft', cycleRowControl],
  ['ArrowDown', navigateFromControl],
  ['ArrowUp', navigateFromControl],
  ['Enter', clickEnabledCheckbox],
]);

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
  const action = TREE_MODE_ACTIONS.get(event.key);
  if (!action) return;
  if (action(event, context, currentId)) event.preventDefault();
}

function handleControlModeKeyDown(event: KeyboardEvent<HTMLDivElement>, context: TreeKeyContext): void {
  const target = event.target;
  if (!isFocusedTreeControl(target)) return;
  const action = CONTROL_MODE_ACTIONS.get(event.key);
  if (!action) return;
  if (action(event, target, context)) event.preventDefault();
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
