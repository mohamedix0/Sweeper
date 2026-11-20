/**
 * Row activation shared by pointer clicks and keyboard activation from the
 * role=tree container (issue #103). Keeping this here means the eligibility /
 * cleaning guards are evaluated in exactly one place, whether activation is
 * triggered by a click, by Enter/Space on the tree, or by the interaction
 * mode's native control activation.
 */

import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { FlatAnalyzerRow } from './analyzerHelpers';
import { isNodeSelectable } from './analyzerHelpers';
import { isFewerRowId, isMoreRowId, showAllRowParentId } from './analyzerTreeA11y';

export type ActivationArgs = {
  rows: readonly FlatAnalyzerRow[];
  allowDeleteUnknown: boolean;
  cleaning: boolean;
  onInspect: (args: { id: string }) => void;
  onToggleSelect: (node: AnalyzerNodeDto) => void;
  onToggleExpand: (node: AnalyzerNodeDto) => void;
  onShowMore: (args: { parentId: string }) => void;
};

/** Activate the given row: explain it and toggle selection when eligible. */
export function activateAnalyzerRow(args: {
  row: FlatAnalyzerRow | undefined;
  allowDeleteUnknown: boolean;
  cleaning: boolean;
  onInspect: (args: { id: string }) => void;
  onToggleSelect: (node: AnalyzerNodeDto) => void;
  onShowMore: (args: { parentId: string }) => void;
}): void {
  const row = args.row;
  if (!row) return;
  const { node } = row;

  // Show more / Show fewer pseudo-rows activate their existing button path.
  if (isMoreRowId(node.id) || isFewerRowId(node.id)) {
    args.onShowMore({ parentId: showAllRowParentId(node.id) });
    return;
  }

  args.onInspect({ id: node.id });
  if (isNodeSelectable({ node, allowDeleteUnknown: args.allowDeleteUnknown }) && !args.cleaning) {
    args.onToggleSelect(node);
  }
}

/** Build the callbacks the tree key handler uses to act on the active row. */
export function activationCallbacks(args: ActivationArgs) {
  const rowById = (id: string) => args.rows.find((row) => row.node.id === id);
  return {
    activateRow: (id: string) => activateAnalyzerRow({
      row: rowById(id),
      allowDeleteUnknown: args.allowDeleteUnknown,
      cleaning: args.cleaning,
      onInspect: args.onInspect,
      onToggleSelect: args.onToggleSelect,
      onShowMore: args.onShowMore,
    }),
    collapseRow: (id: string) => {
      const row = rowById(id);
      if (row) args.onToggleExpand(row.node);
    },
  };
}
