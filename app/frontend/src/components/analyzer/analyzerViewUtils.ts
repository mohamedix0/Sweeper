import { formatBytes } from '../../utils/format';
import { TYPED_CONFIRMATION_PHRASE } from '../cleanShared';
import { isMoreRowId, isNodeSelectable } from './analyzerHelpers';

type RootLike = { children?: unknown[] } | null;
type NodeLike = { id: string; isDir: boolean; children?: NodeLike[] };

function addExpandable(args: { ids: string[]; node: NodeLike; isRoot: boolean }) {
  const hasKids = (args.node.children?.length ?? 0) > 0;
  if (args.isRoot) return;
  if (!args.node.isDir) return;
  if (hasKids) args.ids.push(args.node.id);
}

function walkExpandable(args: { ids: string[]; node: NodeLike; isRoot: boolean }) {
  addExpandable(args);
  for (const c of args.node.children ?? []) walkExpandable({ ids: args.ids, node: c as NodeLike, isRoot: false });
}

export function collectExpandableIds(args: { filteredRoot: RootLike }): string[] {
  if (!args.filteredRoot) return [];
  const ids: string[] = [];
  for (const c of args.filteredRoot.children ?? []) walkExpandable({ ids, node: c as NodeLike, isRoot: false });
  return ids;
}

export function visibleSelectableIdsOf(args: { rows: { node: { id: string } }[]; allowDeleteUnknown: boolean }) {
  return args.rows
    .filter((r) => {
      if (isMoreRowId(r.node.id)) return false;
      return isNodeSelectable({ node: r.node as never, allowDeleteUnknown: args.allowDeleteUnknown });
    })
    .map((r) => r.node.id);
}

export function cleanProgressLabelOf(progress: { completedItems: string; totalItems: string; bytesFreedSoFar: string; status: string } | null) {
  if (!progress) return null;
  return `Progress: ${progress.completedItems} / ${progress.totalItems} items — ${formatBytes({ bytes: progress.bytesFreedSoFar })} freed so far (${progress.status})`;
}

export function canPreviewOf(args: { sessionId: string | null; selectedSize: number; analyzing: boolean; previewLoading: boolean; cleaning: boolean; requiresConfirmation: boolean; confirmText: string }): boolean {
  if (!args.sessionId) return false;
  if (args.selectedSize <= 0) return false;
  if (args.analyzing) return false;
  if (args.previewLoading) return false;
  if (args.cleaning) return false;
  if (!args.requiresConfirmation) return true;
  return args.confirmText === TYPED_CONFIRMATION_PHRASE;
}
