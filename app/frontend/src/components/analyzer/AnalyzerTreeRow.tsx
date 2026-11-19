import type { MouseEvent, KeyboardEvent } from 'react';
import { ChevronDownIcon, ChevronRightIcon, CloudIcon, DriveIcon, FileIcon, FolderIcon, HardDriveIcon, LinkIcon, ShieldIcon } from '../Icons';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath } from '../../utils/format';
import { analyzerStatusPill, analyzerStatusText, isDriveRoot, isNodeSelectable, nodeTypeTag, shareBarFillPx, formatSharePercent, type FlatAnalyzerRow } from './analyzerHelpers';
import { isFewerRowId, isMoreRowId, showAllButtonDomId, showAllRowParentId, treeItemDomId } from './analyzerTreeA11y';

type RowProps = {
  row: FlatAnalyzerRow;
  rootTotalBytes: bigint;
  expanded: Set<string>;
  selectedIds: Set<string>;
  inspectedId: string | null;
  allowDeleteUnknown: boolean;
  cleaning: boolean;
  onToggleExpand: (node: AnalyzerNodeDto) => void;
  onToggleSelect: (node: AnalyzerNodeDto) => void;
  onInspect: (args: { id: string }) => void;
  onShowMore: (args: { parentId: string }) => void;
  onDrill: (args: { path: string }) => void;
};

function semanticNodeIcon(node: AnalyzerNodeDto) {
  if (node.path.toLowerCase().endsWith('.vhdx')) return <HardDriveIcon className="row-icon" aria-hidden="true" />;
  if (node.isLinkPoint) return <LinkIcon className="row-icon" aria-hidden="true" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="row-icon" aria-hidden="true" />;
  if (node.risk === 'Protected') return <ShieldIcon className="row-icon" aria-hidden="true" />;
  if (isDriveRoot({ node })) return <DriveIcon className="row-icon" aria-hidden="true" />;
  if (node.isDir) return <FolderIcon className="row-icon" aria-hidden="true" />;
  return <FileIcon className="row-icon" aria-hidden="true" />;
}

function indentClassOf(input: { depth: number }): string {
  if (input.depth >= 1) return 'tree-indent has-guide';
  return 'tree-indent';
}

function MaybeGuideLine(input: { depth: number }) {
  if (input.depth >= 1) return <span className="tree-guide-line" />;
  return null;
}

function IndentGuide(input: { depth: number }) {
  return (
    <span className={indentClassOf({ depth: input.depth })} aria-hidden="true">
      <MaybeGuideLine depth={input.depth} />
      {Array.from({ length: input.depth }).map((_, i) => <span key={i} className="indent-step" />)}
    </span>
  );
}

function MoreRow(props: { node: AnalyzerNodeDto; depth: number; onShowMore: (args: { parentId: string }) => void }) {
  const parentId = showAllRowParentId(props.node.id);
  return (
    <div id={treeItemDomId(props.node.id)} className="analyzer-row analyzer-more-row" role="treeitem" aria-level={props.depth + 1} aria-selected={false}>
      <div className="analyzer-col-main tree-cell-name">
        <IndentGuide depth={props.depth} />
        <button type="button" id={showAllButtonDomId(parentId)} className="btn small subtle-btn analyzer-show-more-btn" onClick={() => props.onShowMore({ parentId })}>{props.node.name}</button>
      </div>
      <div className="analyzer-col-share tree-cell-share" /><div className="analyzer-col-count muted num" /><div className="analyzer-col-sizecell num" /><div className="analyzer-col-guard" />
    </div>
  );
}

function SelectableInput(props: { path: string; checked: boolean; cleaning: boolean; onToggle: () => void }) {
  return (
    <label className="checkbox analyzer-checkbox" title="Select for Review & Clean">
      <input type="checkbox" aria-label={`Select analyzer node ${props.path}`} checked={props.checked} disabled={props.cleaning} onChange={props.onToggle} />
    </label>
  );
}

function DisabledInput(props: { path: string; checked: boolean; statusText: string }) {
  return (
    <span className="analyzer-selection-fixed-slot analyzer-disabled-slot" title={props.statusText}>
      <span className="scan-disabled-checkbox-outline analyzer-disabled-checkbox-outline" aria-hidden="true" />
      <input className="sr-only" type="checkbox" aria-label={`Select analyzer node ${props.path}`} checked={props.checked} disabled readOnly tabIndex={-1} />
    </span>
  );
}

function SelectionSlot(props: { path: string; selectable: boolean; checked: boolean; cleaning: boolean; statusText: string; onToggle: () => void }) {
  if (props.selectable) return <SelectableInput path={props.path} checked={props.checked} cleaning={props.cleaning} onToggle={props.onToggle} />;
  return <DisabledInput path={props.path} checked={props.checked} statusText={props.statusText} />;
}

function ShareCell(props: { shareFillWidthPx: number; shareLabel: string }) {
  if (props.shareFillWidthPx <= 0) {
    return <div className="analyzer-share-cell"><span className="analyzer-share-empty muted" aria-hidden="true">—</span><span className="analyzer-share-num num">{props.shareLabel}</span></div>;
  }
  return (
    <div className="analyzer-share-cell">
      <svg className="analyzer-share-svg" viewBox="0 0 48 6" preserveAspectRatio="none" role="presentation" aria-hidden="true">
        <rect className="analyzer-share-bg" x={0} y={0} width={48} height={6} rx={3} />
        <rect className="analyzer-share-fill" x={0} y={0} width={props.shareFillWidthPx} height={6} rx={3} />
      </svg>
      <span className="analyzer-share-num num">{props.shareLabel}</span>
    </div>
  );
}

function rowClassName(args: { inspected: boolean; depth: number }): string {
  let cls = 'analyzer-row';
  if (args.inspected) cls += ' inspected';
  if (args.depth >= 1) cls += ' depth-child analyzer-child-row';
  return cls;
}

function selectionSlotClass(selectable: boolean): string {
  if (selectable) return 'tree-slot tree-slot-selection';
  return 'tree-slot tree-slot-selection locked';
}

function expandLabel(args: { expanded: boolean; name: string }): string {
  if (args.expanded) return `Collapse ${args.name}`;
  return `Expand ${args.name}`;
}

function drillTitle(args: { canDrill: boolean }): string | undefined {
  if (!args.canDrill) return undefined;
  return 'Double-click to inspect folder as root';
}

function shareOf(input: { nodeSize: bigint; rootTotalBytes: bigint }) {
  if (input.rootTotalBytes <= 0n) return 0;
  return (Number(input.nodeSize) / Number(input.rootTotalBytes)) * 100;
}

function canDrillRow(input: { node: AnalyzerNodeDto; hasChildren: boolean; depth: number; cleaning: boolean }): boolean {
  if (!input.node.isDir) return false;
  if (!input.hasChildren) return false;
  if (input.depth <= 0) return false;
  if (input.cleaning) return false;
  return true;
}

type RowVm = { selectable: boolean; checked: boolean; hasChildren: boolean; isExpanded: boolean; statusText: string; pill: ReturnType<typeof analyzerStatusPill>; typeTag: string | null; shareLabel: string; shareFillWidthPx: number; pathDisplay: string; pathDisplayShort: string; inspectedMatch: boolean; canDrill: boolean };

function buildVm(input: { props: RowProps; node: AnalyzerNodeDto; depth: number }): RowVm {
  const selectable = isNodeSelectable({ node: input.node, allowDeleteUnknown: input.props.allowDeleteUnknown });
  const checked = input.props.selectedIds.has(input.node.id);
  const hasChildren = input.node.isDir && (input.node.children?.length ?? 0) > 0;
  const isExpanded = input.props.expanded.has(input.node.id);
  const status = analyzerStatusText({ node: input.node, selectable, allowDeleteUnknown: input.props.allowDeleteUnknown });
  const pill = analyzerStatusPill({ node: input.node, selectable });
  const typeTag = nodeTypeTag({ node: input.node });
  const nodeSize = BigInt(input.node.sizeBytes || '0');
  const shareLabel = formatSharePercent({ share: shareOf({ nodeSize, rootTotalBytes: input.props.rootTotalBytes }) });
  const shareFillWidthPx = shareBarFillPx({ nodeBytes: nodeSize, rootTotalBytes: input.props.rootTotalBytes });
  const pathDisplay = normalizeDisplayPath({ p: input.node.path });
  const pathDisplayShort = middleTruncatePath({ path: pathDisplay, maxLen: 64 });
  const inspectedMatch = input.props.inspectedId === input.node.id;
  const canDrill = canDrillRow({ node: input.node, hasChildren, depth: input.depth, cleaning: input.props.cleaning });
  return { selectable, checked, hasChildren, isExpanded, statusText: status.text, pill, typeTag, shareLabel, shareFillWidthPx, pathDisplay, pathDisplayShort, inspectedMatch, canDrill };
}

function expandIcon(expanded: boolean) {
  if (expanded) return <ChevronDownIcon />;
  return <ChevronRightIcon />;
}

function ExpandToggle(input: { hasChildren: boolean; expanded: boolean; name: string; onClick: (e: MouseEvent) => void }) {
  if (!input.hasChildren) return <span className="tree-toggle-spacer" aria-hidden="true" />;
  return <button type="button" className="tree-toggle" onClick={input.onClick} aria-label={expandLabel({ expanded: input.expanded, name: input.name })}>{expandIcon(input.expanded)}</button>;
}

function MaybeTypeTag(input: { typeTag: string | null }) {
  if (!input.typeTag) return null;
  return <span className="inline-type-tag muted">· {input.typeTag}</span>;
}

function MaybeDrillButton(input: { node: AnalyzerNodeDto; cleaning: boolean; onDrillClick: (e: MouseEvent) => void }) {
  if (!input.node.isDir) return null;
  return <button type="button" className="analyzer-drill-btn" onClick={input.onDrillClick} aria-label={`Drill into ${input.node.name}`} title="Drill into folder" disabled={input.cleaning}>Drill</button>;
}

function NameLine(input: { node: AnalyzerNodeDto; typeTag: string | null; cleaning: boolean; onDrillClick: (e: MouseEvent) => void }) {
  return (
    <div className="analyzer-name-line">
      <span className="analyzer-name" title={input.node.name}>{input.node.name}</span>
      <MaybeTypeTag typeTag={input.typeTag} />
      <MaybeDrillButton node={input.node} cleaning={input.cleaning} onDrillClick={input.onDrillClick} />
    </div>
  );
}

function treeIndentClass(input: { depth: number }): string {
  if (input.depth >= 1) return `tree-indent depth-${input.depth} has-guide`;
  return `tree-indent depth-${input.depth}`;
}

function TreeIndent(input: { depth: number }) {
  return (
    <span className={treeIndentClass({ depth: input.depth })} aria-hidden="true">
      <MaybeGuideLine depth={input.depth} />
      {Array.from({ length: input.depth }).map((_, i) => <span key={i} className={`indent-step indent-step-d${input.depth}-${i}`} />)}
    </span>
  );
}

function RowMain(props: { node: AnalyzerNodeDto; depth: number; vm: RowVm; cleaning: boolean; onExpand: (e: MouseEvent) => void; onDrill: (e: MouseEvent) => void; onToggleSelect: () => void }) {
  return (
    <div className="analyzer-col-main tree-cell-name">
      <TreeIndent depth={props.depth} />
      <span className="tree-slot tree-slot-chevron"><ExpandToggle hasChildren={props.vm.hasChildren} expanded={props.vm.isExpanded} name={props.node.name} onClick={props.onExpand} /></span>
      <span className={selectionSlotClass(props.vm.selectable)} onClick={(e) => e.stopPropagation()}>
        <SelectionSlot path={props.node.path} selectable={props.vm.selectable} checked={props.vm.checked} cleaning={props.cleaning} statusText={props.vm.statusText} onToggle={props.onToggleSelect} />
      </span>
      <span className="tree-slot tree-slot-icon analyzer-type-slot" aria-hidden="true">{semanticNodeIcon(props.node)}</span>
      <div className="analyzer-node-main">
        <NameLine node={props.node} typeTag={props.vm.typeTag} cleaning={props.cleaning} onDrillClick={props.onDrill} />
        <div className="analyzer-path analyzer-node-path" title={props.vm.pathDisplay}>{props.vm.pathDisplayShort}</div>
        <span className="sr-only">{props.vm.statusText}</span>
      </div>
    </div>
  );
}

function GuardCell(input: { pill: ReturnType<typeof analyzerStatusPill> }) {
  if (input.pill) return <span className={input.pill.className}>{input.pill.label}</span>;
  return <span className="status-muted-inline" />;
}

function isActivateKey(key: string): boolean {
  if (key === 'Enter') return true;
  return key === ' ';
}

function canToggleSelection(input: { vm: RowVm; cleaning: boolean }): boolean {
  if (!input.vm.selectable) return false;
  return !input.cleaning;
}

function rowHandlers(input: { props: RowProps; node: AnalyzerNodeDto; vm: RowVm }) {
  const activate = () => {
    input.props.onInspect({ id: input.node.id });
    if (canToggleSelection({ vm: input.vm, cleaning: input.props.cleaning })) input.props.onToggleSelect(input.node);
  };
  const onRowDoubleClick = (e: MouseEvent) => {
    if (!input.vm.canDrill) return;
    e.stopPropagation();
    input.props.onDrill({ path: input.node.path });
  };
  const onRowKeyDown = (e: KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if (!isActivateKey(e.key)) return;
    e.preventDefault();
    activate();
  };
  const onExpand = (e: MouseEvent) => { e.stopPropagation(); input.props.onToggleExpand(input.node); };
  const onDrill = (e: MouseEvent) => {
    e.stopPropagation();
    if (input.props.cleaning) return;
    input.props.onDrill({ path: input.node.path });
  };
  return { onRowClick: activate, onRowDoubleClick, onRowKeyDown, onExpand, onDrill };
}

function expandedAttr(input: { hasChildren: boolean; expanded: boolean }): boolean | undefined {
  if (!input.hasChildren) return undefined;
  return input.expanded;
}

export function AnalyzerTreeRow(props: RowProps) {
  const { node, depth } = props.row;
  if (isMoreRowId(node.id) || isFewerRowId(node.id)) return <MoreRow node={node} depth={depth} onShowMore={props.onShowMore} />;
  const vm = buildVm({ props, node, depth });
  const h = rowHandlers({ props, node, vm });
  return (
    <div id={treeItemDomId(node.id)} role="treeitem" aria-expanded={expandedAttr({ hasChildren: vm.hasChildren, expanded: vm.isExpanded })} aria-selected={vm.checked} aria-level={depth + 1} className={rowClassName({ inspected: vm.inspectedMatch, depth })} data-depth={depth} title={drillTitle({ canDrill: vm.canDrill })} onClick={h.onRowClick} onDoubleClick={h.onRowDoubleClick} onKeyDown={h.onRowKeyDown}>
      <RowMain node={node} depth={depth} vm={vm} cleaning={props.cleaning} onExpand={h.onExpand} onDrill={h.onDrill} onToggleSelect={() => props.onToggleSelect(node)} />
      <div className="analyzer-col-share tree-cell-share"><ShareCell shareFillWidthPx={vm.shareFillWidthPx} shareLabel={vm.shareLabel} /></div>
      <div className="analyzer-col-count muted num">{node.itemCount}</div>
      <div className="analyzer-col-sizecell num">{formatBytes({ bytes: node.sizeBytes })}</div>
      <div className="analyzer-col-guard"><GuardCell pill={vm.pill} /></div>
    </div>
  );
}
