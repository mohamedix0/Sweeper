import { ChevronDownIcon, ChevronRightIcon, CloudIcon, DriveIcon, FileIcon, FolderIcon, HardDriveIcon, LinkIcon, ShieldIcon } from '../Icons';
import type { AnalyzerNodeDto } from '../../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath } from '../../utils/format';
import { analyzerStatusPill, analyzerStatusText, isDriveRoot, isMoreRowId, isNodeSelectable, moreRowParentId, nodeTypeTag, shareBarFillPx, formatSharePercent, type FlatAnalyzerRow } from './analyzerHelpers';

function semanticNodeIcon(node: AnalyzerNodeDto) {
  if (node.path.toLowerCase().endsWith('.vhdx')) return <HardDriveIcon className="row-icon" aria-hidden="true" />;
  if (node.isLinkPoint) return <LinkIcon className="row-icon" aria-hidden="true" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="row-icon" aria-hidden="true" />;
  if (node.risk === 'Protected') return <ShieldIcon className="row-icon" aria-hidden="true" />;
  if (isDriveRoot(node)) return <DriveIcon className="row-icon" aria-hidden="true" />;
  return node.isDir ? <FolderIcon className="row-icon" aria-hidden="true" /> : <FileIcon className="row-icon" aria-hidden="true" />;
}

export function AnalyzerTreeRow(props: {
  row: FlatAnalyzerRow;
  rootTotalBytes: bigint;
  expanded: Set<string>;
  selectedIds: Set<string>;
  inspectedId: string | null;
  allowDeleteUnknown: boolean;
  cleaning: boolean;
  onToggleExpand: (node: AnalyzerNodeDto) => void;
  onToggleSelect: (node: AnalyzerNodeDto) => void;
  onInspect: (id: string) => void;
  onShowMore: (parentId: string) => void;
  onDrill: (path: string) => void;
}) {
  const { node, depth } = props.row;
  if (isMoreRowId(node.id)) {
    const parentId = moreRowParentId(node.id);
    return (
      <div className="analyzer-row analyzer-more-row" role="treeitem" aria-level={depth + 1} aria-selected={false} tabIndex={0}>
        <div className="analyzer-col-main tree-cell-name">
          <span className={`tree-indent${depth >= 1 ? ' has-guide' : ''}`} aria-hidden="true">
            {depth >= 1 && <span className="tree-guide-line" />}
            {Array.from({ length: depth }).map((_, i) => <span key={i} className="indent-step" />)}
          </span>
          <button type="button" className="btn small subtle-btn analyzer-show-more-btn" onClick={() => props.onShowMore(parentId)}>{node.name}</button>
        </div>
        <div className="analyzer-col-share tree-cell-share" /><div className="analyzer-col-count muted num" /><div className="analyzer-col-sizecell num" /><div className="analyzer-col-guard" />
      </div>
    );
  }
  const selectable = isNodeSelectable(node, props.allowDeleteUnknown);
  const checked = props.selectedIds.has(node.id);
  const hasChildren = node.isDir && (node.children?.length ?? 0) > 0;
  const isExpanded = props.expanded.has(node.id);
  const status = analyzerStatusText(node, selectable, props.allowDeleteUnknown);
  const pill = analyzerStatusPill(node, selectable, props.allowDeleteUnknown);
  const typeTag = nodeTypeTag(node);
  const nodeSize = BigInt(node.sizeBytes || '0');
  const share = props.rootTotalBytes > 0n ? (Number(nodeSize) / Number(props.rootTotalBytes)) * 100 : 0;
  const shareLabel = formatSharePercent(share);
  const shareFillWidthPx = shareBarFillPx(nodeSize, props.rootTotalBytes);
  const pathDisplay = normalizeDisplayPath(node.path);
  const pathDisplayShort = middleTruncatePath(pathDisplay, 64);
  const inspectedMatch = props.inspectedId === node.id;
  const selectionSlot = selectable ? (
    <label className="checkbox analyzer-checkbox" title="Select for Review & Clean">
      <input type="checkbox" aria-label={`Select analyzer node ${node.path}`} checked={checked} disabled={props.cleaning} onChange={() => props.onToggleSelect(node)} />
    </label>
  ) : (
    <span className="analyzer-selection-fixed-slot analyzer-disabled-slot" title={status.text}>
      <span className="scan-disabled-checkbox-outline analyzer-disabled-checkbox-outline" aria-hidden="true" />
      <input className="sr-only" type="checkbox" aria-label={`Select analyzer node ${node.path}`} checked={checked} disabled readOnly tabIndex={-1} />
    </span>
  );
  return (
    <div
      role="treeitem"
      aria-expanded={hasChildren ? isExpanded : undefined}
      aria-selected={checked}
      aria-level={depth + 1}
      tabIndex={0}
      className={`analyzer-row ${inspectedMatch ? 'inspected' : ''} ${depth >= 1 ? 'depth-child analyzer-child-row' : ''}`}
      data-depth={depth}
      title={hasChildren && depth > 0 ? 'Double-click to inspect folder as root' : undefined}
      onClick={() => { props.onInspect(node.id); if (selectable && !props.cleaning) props.onToggleSelect(node); }}
      onDoubleClick={(e) => {
        if (!node.isDir || !hasChildren || depth <= 0 || props.cleaning) return;
        e.stopPropagation();
        props.onDrill(node.path);
      }}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key !== 'Enter' && e.key !== ' ') return;
        e.preventDefault();
        props.onInspect(node.id);
        if (selectable && !props.cleaning) props.onToggleSelect(node);
      }}
    >
      <div className="analyzer-col-main tree-cell-name">
        <span className={`tree-indent depth-${depth}${depth >= 1 ? ' has-guide' : ''}`} aria-hidden="true">
          {depth >= 1 && <span className="tree-guide-line" />}
          {Array.from({ length: depth }).map((_, i) => <span key={i} className={`indent-step indent-step-d${depth}-${i}`} />)}
        </span>
        <span className="tree-slot tree-slot-chevron">
          {hasChildren ? (
            <button type="button" className="tree-toggle" onClick={(e) => { e.stopPropagation(); props.onToggleExpand(node); }} aria-label={isExpanded ? `Collapse ${node.name}` : `Expand ${node.name}`}>
              {isExpanded ? <ChevronDownIcon /> : <ChevronRightIcon />}
            </button>
          ) : <span className="tree-toggle-spacer" aria-hidden="true" />}
        </span>
        <span className={`tree-slot tree-slot-selection ${selectable ? '' : 'locked'}`} onClick={(e) => e.stopPropagation()}>{selectionSlot}</span>
        <span className="tree-slot tree-slot-icon analyzer-type-slot" aria-hidden="true">{semanticNodeIcon(node)}</span>
        <div className="analyzer-node-main">
          <div className="analyzer-name-line">
            <span className="analyzer-name" title={node.name}>{node.name}</span>
            {typeTag && <span className="inline-type-tag muted">· {typeTag}</span>}
            {node.isDir && (
              <button type="button" className="analyzer-drill-btn" onClick={(e) => { e.stopPropagation(); if (props.cleaning) return; props.onDrill(node.path); }} aria-label={`Drill into ${node.name}`} title="Drill into folder" disabled={props.cleaning}>Drill</button>
            )}
          </div>
          <div className="analyzer-path analyzer-node-path" title={pathDisplay}>{pathDisplayShort}</div>
          <span className="sr-only">{status.text}</span>
        </div>
      </div>
      <div className="analyzer-col-share tree-cell-share">
        <div className="analyzer-share-cell">
          {shareFillWidthPx <= 0 ? <span className="analyzer-share-empty muted" aria-hidden="true">—</span> : (
            <svg className="analyzer-share-svg" viewBox="0 0 48 6" preserveAspectRatio="none" role="presentation" aria-hidden="true">
              <rect className="analyzer-share-bg" x={0} y={0} width={48} height={6} rx={3} />
              <rect className="analyzer-share-fill" x={0} y={0} width={shareFillWidthPx} height={6} rx={3} />
            </svg>
          )}
          <span className="analyzer-share-num num">{shareLabel}</span>
        </div>
      </div>
      <div className="analyzer-col-count muted num">{node.itemCount}</div>
      <div className="analyzer-col-sizecell num">{formatBytes(node.sizeBytes)}</div>
      <div className="analyzer-col-guard">{pill ? <span className={pill.className}>{pill.label}</span> : <span className="status-muted-inline" />}</div>
    </div>
  );
}
