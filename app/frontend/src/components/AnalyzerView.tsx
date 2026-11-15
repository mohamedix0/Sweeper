import { useMemo, useRef, useState } from 'react';
import {
  formatContractError,
  type AnalyzerProgressEvent,
  type CleanProgressEvent,
  type ExecuteCleanResponse,
  type Settings,
  type StartAnalyzerResult,
  type SweeperClient,
} from '../api/client';
import type { AnalyzerNodeDto, ContractError } from '../contract/bindings';
import { formatBytes, middleTruncatePath, normalizeDisplayPath, pluralize } from '../utils/format';
import {
  CleanSummaryBlock,
  copyText,
  InspectorMetaGrid,
  InspectorTitleRow,
  PathActions,
  PathWell,
  PreviewBreakdownBlock,
  TYPED_CONFIRMATION_PHRASE,
  type ExplainState,
  type PreviewResult,
} from './cleanShared';
import {
  BrowseIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CloudIcon,
  DriveIcon,
  FileIcon,
  FolderIcon,
  HardDriveIcon,
  LinkIcon,
  RefreshIcon,
  ShieldIcon,
} from './Icons';

type FlatAnalyzerRow = { node: AnalyzerNodeDto; depth: number; siblingMax: bigint };
type AnalyzerStatusFilter = 'all' | 'safe' | 'caution' | 'unknown' | 'locked';
const ANALYZER_CHILD_CAP = 25;

function isMoreRowId(id: string): boolean {
  return id.startsWith('__more__:');
}
function moreRowParentId(id: string): string {
  return id.slice('__more__:'.length);
}

function buildFlat(root: AnalyzerNodeDto, expanded: Set<string>, showAllNodes: Set<string>): FlatAnalyzerRow[] {
  const rows: FlatAnalyzerRow[] = [];
  const renderChildren = (parent: AnalyzerNodeDto, depth: number) => {
    const allKids = parent.children ?? [];
    if (allKids.length === 0) return;
    const showAll = showAllNodes.has(parent.id);
    let kids = allKids;
    let hidden = 0;
    if (!showAll && allKids.length > ANALYZER_CHILD_CAP) {
      const bySize = [...allKids].sort((a, b) => {
        const sa = BigInt(a.sizeBytes || '0'), sb = BigInt(b.sizeBytes || '0');
        return sa === sb ? 0 : (sa > sb ? -1 : 1);
      }).slice(0, ANALYZER_CHILD_CAP);
      const keep = new Set(bySize.map((k) => k.id));
      kids = allKids.filter((k) => keep.has(k.id));
      hidden = allKids.length - kids.length;
    }
    let max = 0n;
    for (const c of kids) { const s = BigInt(c.sizeBytes || '0'); if (s > max) max = s; }
    if (max === 0n) max = 1n;
    for (const c of kids) {
      rows.push({ node: c, depth, siblingMax: max });
      if (c.isDir && expanded.has(c.id) && c.children && c.children.length > 0) {
        renderChildren(c, depth + 1);
      }
    }
    if (hidden > 0) {
      rows.push({
        node: {
          id: `__more__:${parent.id}`,
          name: `Showing top ${ANALYZER_CHILD_CAP} of ${allKids.length} files · Show more`,
          path: '',
          isDir: false,
          sizeBytes: '0',
          itemCount: '0',
          risk: 'Safe',
          description: '',
          reason: null,
          tier: null,
          targetId: null,
          isLinkPoint: false,
          isCloudPlaceholder: false,
          displayOnly: true,
          selectable: false,
          children: [],
        },
        depth,
        siblingMax: 1n,
      });
    }
  };
  renderChildren(root, 0);
  return rows;
}

function filterTree(node: AnalyzerNodeDto, query: string, sort: 'size' | 'items' | 'name', statusFilter: AnalyzerStatusFilter, _allowDeleteUnknown: boolean): AnalyzerNodeDto | null {
  const q = query.trim().toLowerCase();
  const statusKey = analyzerStatusKey(node);
  const statusOk = statusFilter === 'all' || statusFilter === statusKey;
  const textMatch = !q || node.name.toLowerCase().includes(q) || node.path.toLowerCase().includes(q);
  let children = (node.children ?? []).map((c) => filterTree(c, query, sort, statusFilter, _allowDeleteUnknown)).filter(Boolean) as AnalyzerNodeDto[];
  children = [...children].sort((a, b) => {
    if (sort === 'name') return a.name.localeCompare(b.name);
    if (sort === 'items') return Number(BigInt(b.itemCount || '0') - BigInt(a.itemCount || '0'));
    const sa = BigInt(a.sizeBytes || '0'), sb = BigInt(b.sizeBytes || '0');
    return sa === sb ? 0 : (sa > sb ? -1 : 1);
  });
  const selfMatch = textMatch && statusOk;
  if (!selfMatch && children.length === 0) return null;
  return { ...node, children };
}

function analyzerSelectionNeedsConfirmation(nodes: AnalyzerNodeDto[]): boolean {
  return nodes.some((n) => n.risk === 'Unknown' || !n.targetId);
}

function isNodeSelectable(node: AnalyzerNodeDto, allowDeleteUnknown: boolean): boolean {
  if (node.displayOnly || node.isLinkPoint || node.isCloudPlaceholder) return false;
  if (node.risk === 'Protected') return false;
  if (node.path.toLowerCase().endsWith('.vhdx')) return false;
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return false;
  return !!node.selectable;
}

function isCleanableEligible(node: AnalyzerNodeDto, selectable: boolean): boolean {
  if (!selectable) return false;
  const tier = node.tier;
  return tier === 'Tier1' || tier === 'Tier2' || tier === 'Tier3';
}

function analyzerStatusPill(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): { label: string; className: string } | null {
  if (isDriveRoot(node)) return { label: 'Drive Root', className: 'status-pill status-pill-readonly' };
  if (node.risk === 'Protected') return { label: 'Protected', className: 'status-pill status-pill-protected' };
  if (node.path.toLowerCase().endsWith('.vhdx') || node.isLinkPoint || node.isCloudPlaceholder || node.displayOnly) return { label: 'Read-Only', className: 'status-pill status-pill-readonly' };
  if (node.risk === 'Unknown') return { label: 'Unknown', className: 'status-pill status-pill-unknown' };
  if (node.risk === 'Caution') return { label: 'Caution', className: 'status-pill status-pill-caution' };
  if (isCleanableEligible(node, selectable)) return { label: 'Safe', className: 'status-pill status-pill-safe' };
  if (selectable) return { label: 'Safe', className: 'status-pill status-pill-safe' };
  return null;
}

function analyzerStatusKey(node: AnalyzerNodeDto): AnalyzerStatusFilter {
  if (isDriveRoot(node)) return 'locked';
  if (node.risk === 'Protected') return 'locked';
  if (node.path.toLowerCase().endsWith('.vhdx') || node.isLinkPoint || node.isCloudPlaceholder || node.displayOnly) return 'locked';
  if (node.risk === 'Unknown') return 'unknown';
  if (node.risk === 'Caution') return 'caution';
  if (node.risk === 'Safe') return 'safe';
  return 'locked';
}

function normalizedAnalyzerPath(path: string): string {
  return normalizeDisplayPath(path).replace(/[\\/]+$/, '').toLowerCase();
}

function isAncestorAnalyzerPath(ancestorPath: string, descendantPath: string): boolean {
  const a = normalizedAnalyzerPath(ancestorPath);
  const d = normalizedAnalyzerPath(descendantPath);
  if (!a || !d || a === d) return false;
  const prefix = a.endsWith('\\') ? a : `${a}\\`;
  return d.startsWith(prefix);
}

function dedupeAnalyzerNodesByCoverage(nodes: AnalyzerNodeDto[]): AnalyzerNodeDto[] {
  return nodes.filter((node, _, all) => !all.some((other) => other.id !== node.id && isAncestorAnalyzerPath(other.path, node.path)));
}

function analyzerStatusText(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): { text: string; locked: boolean; unknownGated: boolean } {
  if (selectable) {
    return { text: (node.risk === 'Unknown' || !node.targetId) ? 'Selectable (typed confirmation required)' : 'Meets age limit (> 24h old)', locked: false, unknownGated: false };
  }
  if (node.risk === 'Protected') return { text: 'Protected. Not selectable.', locked: true, unknownGated: false };
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return { text: 'Unknown gated by Settings (Allow deleting unknown folders).', locked: true, unknownGated: true };
  if (node.isLinkPoint) return { text: 'Link Point. Not selectable.', locked: true, unknownGated: false };
  if (node.isCloudPlaceholder) return { text: 'Cloud placeholder (Skipped). Not selectable.', locked: true, unknownGated: false };
  if (node.path.toLowerCase().endsWith('.vhdx')) return { text: 'Read-only virtual disk. Not selectable.', locked: true, unknownGated: false };
  if (node.displayOnly) return { text: isDriveRoot(node) ? 'Drive roots cannot be cleaned as a whole. Expand to select specific folders.' : 'Display-only. Not selectable.', locked: true, unknownGated: false };
  if (node.reason) return { text: `${node.reason}. Not selectable.`, locked: true, unknownGated: false };
  return { text: 'Not selectable.', locked: true, unknownGated: false };
}

function analyzerGuidance(node: AnalyzerNodeDto, selectable: boolean, allowDeleteUnknown: boolean): string {
  if (node.path.toLowerCase().endsWith('.vhdx')) return 'Virtual disk (WSL/Hyper-V). Read-only in Sweeper — compaction is manual and safe to do when the VM/WSL is shut down.';
  if (node.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (node.isCloudPlaceholder) return 'Cloud placeholder file. Read-only in Analyzer; content may be remote and is not removable here.';
  if (node.isLinkPoint) return 'Link point (symlink/junction). Read-only in Analyzer to avoid traversing or removing linked targets.';
  if (node.risk === 'Unknown' && !allowDeleteUnknown) return 'Unknown folder is locked by Settings. Enable "Allow deleting unknown folders" to allow cleanup after typed confirmation.';
  if (node.risk === 'Unknown') return 'Unknown folder. Review carefully; typed DELETE confirmation is required before cleaning.';
  if (selectable && isCleanableEligible(node, selectable)) return 'Eligible cache/build artifact under current settings. Selected items are permanently deleted after review and confirmation.';
  if (selectable) return 'Eligible under current settings. Selected items are permanently deleted after review and confirmation.';
  if (isDriveRoot(node)) return 'Drive roots cannot be cleaned as a whole. Expand C: to select specific folders.';
  return 'Display-only item. It cannot be selected for cleanup in Analyzer.';
}

function nodeTypeTag(node: AnalyzerNodeDto): string | null {
  if (node.path.toLowerCase().endsWith('.vhdx')) return 'Virtual Disk';
  if (node.isLinkPoint) return 'Symlink';
  if (node.isCloudPlaceholder) return 'Cloud';
  return null;
}

function isDriveRoot(node: AnalyzerNodeDto): boolean {
  return /^[A-Za-z]:\\?$/.test(node.path.trim()) || /^[A-Za-z]:\\$/.test(normalizeDisplayPath(node.path));
}

function semanticNodeIcon(node: AnalyzerNodeDto) {
  if (node.path.toLowerCase().endsWith('.vhdx')) return <HardDriveIcon className="row-icon" aria-hidden="true" />;
  if (node.isLinkPoint) return <LinkIcon className="row-icon" aria-hidden="true" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="row-icon" aria-hidden="true" />;
  if (node.risk === 'Protected') return <ShieldIcon className="row-icon" aria-hidden="true" />;
  if (isDriveRoot(node)) return <DriveIcon className="row-icon" aria-hidden="true" />;
  return node.isDir ? <FolderIcon className="row-icon" aria-hidden="true" /> : <FileIcon className="row-icon" aria-hidden="true" />;
}

function leadingInspectorIcon(node: AnalyzerNodeDto | null) {
  if (!node) return <FolderIcon className="inspector-leading-icon" />;
  if (node.path.toLowerCase().endsWith('.vhdx')) return <HardDriveIcon className="inspector-leading-icon" />;
  if (node.isLinkPoint) return <LinkIcon className="inspector-leading-icon" />;
  if (node.isCloudPlaceholder) return <CloudIcon className="inspector-leading-icon" />;
  if (node.risk === 'Protected') return <ShieldIcon className="inspector-leading-icon" />;
  if (isDriveRoot(node)) return <DriveIcon className="inspector-leading-icon" />;
  return node.isDir ? <FolderIcon className="inspector-leading-icon" /> : <FileIcon className="inspector-leading-icon" />;
}

const ALLOC_COLORS = ['#3b82f6', '#10b981', '#f59e0b', '#8b5cf6', '#ef4444', '#64748b'];
const BROWSE_PATHS = ['C:\\', 'C:\\Users', 'C:\\Windows\\Temp'];

function shareBarFillPx(nodeBytes: bigint, rootTotalBytes: bigint): number {
  if (nodeBytes <= 0n || rootTotalBytes <= 0n) return 0;
  const shareRatio = Number(nodeBytes) / Number(rootTotalBytes);
  const sharePct = shareRatio * 100;
  if (sharePct < 0.1) return 0;
  const width = Math.round(Math.sqrt(Math.max(0, shareRatio)) * 48);
  return Math.max(2, Math.min(48, width));
}

function formatSharePercent(share: number): string {
  if (share <= 0) return '0%';
  if (share < 0.1) return '<0.1%';
  const pct1 = Math.floor(share * 10) / 10;
  return `${pct1.toFixed(1)}%`;
}

export function AnalyzerView({ client, settings, onOpenLog, onOpenSettings }: { client: SweeperClient; settings: Settings | null; onOpenLog: (logId: string) => void; onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void }) {
  const [rootPath, setRootPath] = useState('C:\\');
  const [analyzedPath, setAnalyzedPath] = useState<string | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [progress, setProgress] = useState<AnalyzerProgressEvent | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [, setStartedAt] = useState<number | null>(null);
  const [result, setResult] = useState<StartAnalyzerResult | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'size' | 'items' | 'name'>('size');
  const [statusFilter, setStatusFilter] = useState<AnalyzerStatusFilter>('all');
  const [showAllAnalyzerNodes, setShowAllAnalyzerNodes] = useState<Set<string>>(new Set());
  const previewRevisionRef = useRef(0);
  const activePreviewRequestRef = useRef(0);
  const inspectReqIdRef = useRef(0);
  const allowDeleteUnknown = settings?.allowDeleteUnknown ?? false;

  function invalidatePreviewRequest() {
    previewRevisionRef.current += 1;
    activePreviewRequestRef.current += 1;
    setPreviewLoading(false);
  }

  const nodesById = useMemo(() => {
    const map = new Map<string, AnalyzerNodeDto>();
    const walk = (n: AnalyzerNodeDto) => { map.set(n.id, n); for (const c of n.children ?? []) walk(c); };
    if (result?.root) walk(result.root);
    return map;
  }, [result]);

  const filteredRoot = useMemo(() => result ? filterTree(result.root, query, sort, statusFilter, allowDeleteUnknown) : null, [result, query, sort, statusFilter, allowDeleteUnknown]);
  const flatRows = useMemo(() => (filteredRoot ? buildFlat(filteredRoot, expanded, showAllAnalyzerNodes) : []), [filteredRoot, expanded, showAllAnalyzerNodes]);

  const statusCounts = useMemo(() => {
    const counts = { all: 0, safe: 0, caution: 0, unknown: 0, locked: 0 };
    if (!result) return counts;
    const kids = result.root.children ?? [];
    counts.all = kids.length;
    for (const n of kids) {
      const key = analyzerStatusKey(n);
      if (key === 'safe') counts.safe += 1;
      else if (key === 'caution') counts.caution += 1;
      else if (key === 'unknown') counts.unknown += 1;
      else counts.locked += 1;
    }
    return counts;
  }, [result]);

  const selectedNodes = useMemo(() => Array.from(selectedIds).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[], [selectedIds, nodesById]);
  const dedupedSelectedNodes = useMemo(() => dedupeAnalyzerNodesByCoverage(selectedNodes), [selectedNodes]);
  const selectedBytes = useMemo(() => dedupedSelectedNodes.reduce((a, n) => a + BigInt(n.sizeBytes || '0'), 0n).toString(), [dedupedSelectedNodes]);
  const requiresConfirmation = useMemo(() => analyzerSelectionNeedsConfirmation(dedupedSelectedNodes), [dedupedSelectedNodes]);

  function resetAnalyzerSession(msg?: string) {
    invalidatePreviewRequest();
    inspectReqIdRef.current += 1;
    setPreviewLoading(false);
    setSessionId(null); setStartedAt(null); setResult(null); setSelectedIds(new Set()); setExpanded(new Set()); setShowAllAnalyzerNodes(new Set());
    setPreview(null); setConfirmText(''); setCleanProgress(null); setInspected(null);
    if (msg) setInfo(msg);
  }

  async function inspectNode(itemId: string) {
    const reqId = inspectReqIdRef.current + 1;
    inspectReqIdRef.current = reqId;
    try {
      const result = await client.explainItem(itemId);
      if (inspectReqIdRef.current === reqId) {
        setInspected(result);
      }
    } catch { /* ignore */ }
  }

  async function runAnalyzer(pathOverride?: string) {
    const targetPath = (pathOverride ?? rootPath).trim() || 'C:\\';
    setRootPath(targetPath);
    invalidatePreviewRequest();
    inspectReqIdRef.current += 1;
    setPreviewLoading(false);
    setError(null); setInfo(null); setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds(new Set()); setResult(null); setExpanded(new Set()); setShowAllAnalyzerNodes(new Set()); setAnalyzing(true); setProgress(null); setInspected(null);
    try {
      const res = await client.startAnalyzer(targetPath, (p) => setProgress(p));
      setResult(res);
      setSessionId(res.sessionId);
      setStartedAt(Date.now());
      setExpanded(new Set());
      setAnalyzedPath(res.rootPath || targetPath);
      const autoInspectChild = [...(res.root.children ?? [])].sort((a, b) => {
        const sa = BigInt(a.sizeBytes || '0');
        const sb = BigInt(b.sizeBytes || '0');
        return sa === sb ? 0 : (sa > sb ? -1 : 1);
      })[0];
      void inspectNode((autoInspectChild ?? res.root).id);
      setInfo(null);
    } catch (e) {
      setError(formatContractError(e));
    } finally { setAnalyzing(false); }
  }

  function toggleExpand(node: AnalyzerNodeDto) {
    if (!node.isDir || !node.children || node.children.length === 0) return;
    setExpanded((prev) => { const next = new Set(prev); if (next.has(node.id)) next.delete(node.id); else next.add(node.id); return next; });
  }

  function toggleSelect(node: AnalyzerNodeDto) {
    if (!isNodeSelectable(node, allowDeleteUnknown)) { void inspectNode(node.id); return; }
    invalidatePreviewRequest();
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(node.id)) {
        next.delete(node.id);
        return next;
      }
      // Avoid overlapping coverage: drop descendants when selecting an ancestor.
      for (const id of Array.from(next)) {
        const existing = nodesById.get(id);
        if (existing && isAncestorAnalyzerPath(node.path, existing.path)) next.delete(id);
      }
      // If an ancestor is already selected, keep the ancestor (no double-counting).
      const hasSelectedAncestor = Array.from(next).some((id) => {
        const existing = nodesById.get(id);
        return !!existing && isAncestorAnalyzerPath(existing.path, node.path);
      });
      if (!hasSelectedAncestor) next.add(node.id);
      return next;
    });
    void inspectNode(node.id);
  }

  const safeSelectableNodeIds = useMemo(() => {
    const safeNodes: AnalyzerNodeDto[] = [];
    nodesById.forEach((node) => {
      const selectable = isNodeSelectable(node, allowDeleteUnknown);
      if (!selectable) return;
      const pill = analyzerStatusPill(node, selectable, allowDeleteUnknown);
      if (pill?.label === 'Safe') safeNodes.push(node);
    });
    return dedupeAnalyzerNodesByCoverage(safeNodes).map((n) => n.id);
  }, [nodesById, allowDeleteUnknown]);

  function selectSafeAnalyzer() {
    invalidatePreviewRequest();
    setPreview(null); setCleanSummary(null); setCleanProgress(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      const currentNodes = Array.from(next).map((id) => nodesById.get(id)).filter(Boolean) as AnalyzerNodeDto[];
      const additions = safeSelectableNodeIds
        .map((id) => nodesById.get(id))
        .filter(Boolean) as AnalyzerNodeDto[];
      // Keep existing non-overlapping selections and add non-overlapping safe roots.
      const merged = [...currentNodes];
      for (const node of additions) {
        if (merged.some((m) => m.id === node.id || isAncestorAnalyzerPath(m.path, node.path))) continue;
        // Remove descendants covered by this newly added safe ancestor.
        for (let i = merged.length - 1; i >= 0; i--) {
          if (isAncestorAnalyzerPath(node.path, merged[i].path)) merged.splice(i, 1);
        }
        merged.push(node);
      }
      return new Set(merged.map((n) => n.id));
    });
  }

  async function runPreview() {
    if (!sessionId) return;
    const requestId = activePreviewRequestRef.current + 1;
    activePreviewRequestRef.current = requestId;
    const requestRevision = previewRevisionRef.current;
    const requestSessionId = sessionId;
    const selectedForPreview = dedupeAnalyzerNodesByCoverage(selectedNodes);
    setError(null); setInfo(null); setPreviewLoading(true); setCleanSummary(null); setCleanProgress(null);
    try {
      const confirmations = requiresConfirmation ? { typedConfirmation: confirmText || null } : null;
      const res = await client.previewClean(requestSessionId, selectedForPreview.map((n) => n.id), confirmations);
      if (
        activePreviewRequestRef.current !== requestId ||
        previewRevisionRef.current !== requestRevision ||
        sessionId !== requestSessionId
      ) {
        return;
      }
      setPreview({ ...res, issuedAtMs: Date.now() });
    } catch (e) {
      if (
        activePreviewRequestRef.current !== requestId ||
        previewRevisionRef.current !== requestRevision ||
        sessionId !== requestSessionId
      ) {
        return;
      }
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') setPreview(null);
    } finally {
      if (activePreviewRequestRef.current === requestId) {
        setPreviewLoading(false);
      }
    }
  }

  async function runClean() {
    if (!preview) return;
    setError(null); setInfo(null); setCleaning(true); setCleanProgress(null); setCleanSummary(null);
    try {
      const res = await client.executeClean(preview.previewToken, (ev) => setCleanProgress(ev));
      setCleanSummary(res);
      setPreview(null);
      setSelectedIds(new Set());
      setConfirmText('');
      setInfo('Clean completed. Review the summary and Activity History below.');
    } catch (e) {
      const ce = e as ContractError | undefined;
      setError(formatContractError(e));
      if (ce?.code === 'sessionInvalid') resetAnalyzerSession('Analyzer session expired or was invalidated. Run Analyze again.');
      else if (ce?.code === 'previewTokenInvalid') { setPreview(null); setInfo('Preview token is invalid, expired, or already used. Run Review & Clean again.'); }
    } finally { setCleaning(false); }
  }

  const canPreview = !!sessionId && selectedIds.size > 0 && !analyzing && !previewLoading && !cleaning && (!requiresConfirmation || confirmText === TYPED_CONFIRMATION_PHRASE);
  const canClean = !!preview && !analyzing && !previewLoading && !cleaning;

  const rootTotalBytes = useMemo(() => BigInt(result?.root.sizeBytes || result?.totalBytes || '0') || 1n, [result]);

  const allocSegments = useMemo(() => {
    type AllocSeg = { id: string; name: string; bytes: bigint; pct: number; pctVisual: number; color: string; isOther?: boolean; focusNodeId?: string };
    if (!result) return [] as AllocSeg[];
    const kids = [...(result.root.children ?? [])].sort((a, b) => {
      const sa = BigInt(a.sizeBytes || '0'), sb = BigInt(b.sizeBytes || '0');
      return sa === sb ? 0 : (sa > sb ? -1 : 1);
    });
    const total = rootTotalBytes;
    const major: AllocSeg[] = [];
    let otherBytes = 0n;
    kids.forEach((k, i) => {
      const bytes = BigInt(k.sizeBytes || '0');
      const pct = total > 0n ? Number((bytes * 10000n) / total) / 100 : 0;
      if (pct >= 0.1) {
        major.push({ id: k.id, name: k.name, bytes, pct, pctVisual: 0, color: ALLOC_COLORS[i % ALLOC_COLORS.length], focusNodeId: k.id });
      } else {
        otherBytes += bytes;
      }
    });
    const raw = [...major];
    if (otherBytes > 0n) {
      const pct = total > 0n ? Number((otherBytes * 10000n) / total) / 100 : 0;
      raw.push({ id: 'alloc-other', name: 'Other', bytes: otherBytes, pct, pctVisual: 0, color: '#64748b', isOther: true, focusNodeId: undefined });
    }
    const gapPct = raw.length > 1 ? 0.2 : 0;
    const totalGap = gapPct * Math.max(0, raw.filter((s) => s.pct > 0).length - 1);
    const targetSum = Math.max(0, 100 - totalGap);
    const minTiny = 0.35;
    let adjusted = raw.map((s) => ({ ...s, pctVisual: s.pct <= 0 ? 0 : Math.max(minTiny, s.pct) }));
    let sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
    if (sum > targetSum) {
      const over = sum - targetSum;
      const reducible = adjusted.filter((s) => s.pctVisual > minTiny);
      const room = reducible.reduce((a, b) => a + (b.pctVisual - minTiny), 0);
      if (room > 0) {
        adjusted = adjusted.map((s) => {
          if (s.pctVisual <= minTiny) return s;
          const r = s.pctVisual - minTiny;
          return { ...s, pctVisual: s.pctVisual - (over * r) / room };
        });
      } else if (adjusted.length > 0) {
        const largest = [...adjusted].sort((a, b) => b.pct - a.pct)[0];
        adjusted = adjusted.map((s) => s.id === largest.id ? { ...s, pctVisual: Math.max(0, s.pctVisual - over) } : s);
      }
      sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
    }
    if (sum < targetSum && adjusted.length > 0) {
      const largest = [...adjusted].sort((a, b) => b.pct - a.pct)[0];
      adjusted = adjusted.map((s) => s.id === largest.id ? { ...s, pctVisual: s.pctVisual + (targetSum - sum) } : s);
    }
    return adjusted;
  }, [result, rootTotalBytes]);

  function focusNode(nodeId: string) {
    const node = nodesById.get(nodeId);
    if (!node) return;
    setExpanded((prev) => {
      const next = new Set(prev);
      const walk = (n: AnalyzerNodeDto, ancestors: string[]): boolean => {
        if (n.id === nodeId) { ancestors.forEach((a) => next.add(a)); return true; }
        if (!n.children) return false;
        for (const c of n.children) { if (walk(c, [...ancestors, n.id])) return true; }
        return false;
      };
      if (result) walk(result.root, []);
      return next;
    });
    void inspectNode(nodeId);
  }

  function cycleBrowsePath() {
    const idx = BROWSE_PATHS.findIndex((p) => p.toLowerCase() === rootPath.trim().toLowerCase());
    const next = BROWSE_PATHS[(idx + 1 + BROWSE_PATHS.length) % BROWSE_PATHS.length];
    setRootPath(next);
  }

  const inspectedNode = inspected ? nodesById.get(inspected.itemId) ?? null : null;
  const inspectedSelectable = inspectedNode ? isNodeSelectable(inspectedNode, allowDeleteUnknown) : false;
  const inspectedStatus = inspectedNode ? analyzerStatusText(inspectedNode, inspectedSelectable, allowDeleteUnknown) : null;
  const inspectedPill = inspectedNode ? analyzerStatusPill(inspectedNode, inspectedSelectable, allowDeleteUnknown) : null;
  const hasResult = !!result;
  const visibleExpandableIds = useMemo(() => {
    if (!filteredRoot) return [] as string[];
    const ids: string[] = [];
    const walk = (n: AnalyzerNodeDto, isRoot: boolean) => {
      if (!isRoot && n.isDir && (n.children?.length ?? 0) > 0) ids.push(n.id);
      for (const c of n.children ?? []) walk(c, false);
    };
    for (const c of filteredRoot.children ?? []) walk(c, false);
    return ids;
  }, [filteredRoot]);
  const allVisibleExpanded = visibleExpandableIds.length > 0 && visibleExpandableIds.every((id) => expanded.has(id));

  function toggleExpandCollapseAll() {
    if (!result) return;
    if (allVisibleExpanded) {
      setExpanded(new Set());
      return;
    }
    setExpanded(new Set(visibleExpandableIds));
  }

  const hasSelectedRisky = selectedNodes.some((n) => n.risk === 'Caution' || n.risk === 'Unknown');

  const displayPath = normalizeDisplayPath((analyzedPath || rootPath).trim() || 'C:\\');
  const isAtRoot = /^([A-Za-z]:\\)$/.test(displayPath);
  const breadcrumbParts = useMemo(() => {
    const p = displayPath;
    return p.split('\\').filter(Boolean).map((seg, i, arr) => ({ label: i === 0 ? `${seg}\\` : seg, path: arr.slice(0, i + 1).join('\\') + (i === 0 ? '\\' : '') }));
  }, [displayPath]);

  const [copiedVhdxCmd, setCopiedVhdxCmd] = useState(false);
  async function copyVhdxCommand(cmd: string) {
    await copyText(cmd);
    setCopiedVhdxCmd(true);
    window.setTimeout(() => setCopiedVhdxCmd(false), 1500);
  }

  const cleanProgressLabel = cleanProgress
    ? `Progress: ${cleanProgress.completedItems} / ${cleanProgress.totalItems} items — ${formatBytes(cleanProgress.bytesFreedSoFar)} freed so far (${cleanProgress.status})`
    : null;

  return (
    <div className="view-stack view-stack-fill">
      <header className="page-header" aria-label="Analyzer header">
        <div className="page-header-left analyzer-header-left">
          <div className="page-title-row">
            <h1 className="page-title" id="analyzer-heading">Analyzer</h1>
            <span className="header-subtitle num" aria-label="Analyzer subtitle">
              {result
                ? `${displayPath} · ${formatBytes(result.totalBytes)} across ${pluralize((result.root.children ?? []).length, 'entry', 'entries')} (${Number(result.totalItems || '0')} files total) · All files on disk`
                : `All files & disk hotspots under ${displayPath}`}
            </span>
          </div>
        </div>
        <div className="page-header-right">
          <button
            key={hasResult ? 'reanalyze' : 'analyze'}
            className={hasResult ? 'btn secondary-action' : 'btn primary'}
            onClick={() => void runAnalyzer()}
            disabled={analyzing || cleaning || !rootPath.trim()}
            aria-label="Analyze"
          >
            {hasResult ? <><RefreshIcon aria-hidden="true" /> <span>Re-analyze</span></> : 'Analyze'}
          </button>
        </div>
      </header>

      {error && <div className="notice error" role="alert">{error}</div>}
      {info && <div className="notice info" role="status">{info}</div>}

      <div className="analyzer-location-bar" aria-label="Analyzer location bar">
        <div className="analyzer-root-group analyzer-root-group-tight">
          <label htmlFor="analyzer-root" className="muted small-note">Root</label>
          <div className="analyzer-root-inline">
            <input id="analyzer-root" type="text" value={rootPath} onChange={(e) => setRootPath(e.target.value)} aria-label="Analyzer root path" disabled={analyzing || cleaning} className="analyzer-root-input" />
            <button type="button" className="btn small analyzer-go-btn" onClick={() => void runAnalyzer()} disabled={analyzing || cleaning || !rootPath.trim()} aria-label="Go">Go</button>
          </div>
          <button type="button" className="btn browse-btn" onClick={cycleBrowsePath} disabled={analyzing || cleaning} aria-label="Use preset root path"><BrowseIcon /> <span>Use Preset</span></button>
        </div>
        <div className="quick-roots quick-roots-inline" aria-label="Quick root path presets">
          <span className="muted small-note">Quick:</span>
          {(['C:\\', 'C:\\Users', 'Temp'] as const)
            .filter((label) => {
              const value = label === 'Temp' ? 'C:\\Windows\\Temp' : label;
              const norm = (p: string) => p.trim().replace(/[\\/]+$/, '').toLowerCase();
              return !(label === 'C:\\' && norm(rootPath) === norm('C:\\'));
            })
            .map((label) => {
              const value = label === 'Temp' ? 'C:\\Windows\\Temp' : label;
              return <button key={label} type="button" className="chip-btn pill-chip small-chip" onClick={() => { setRootPath(value); if (hasResult) void runAnalyzer(value); }} disabled={analyzing || cleaning}>{label}</button>;
            })}
        </div>
      </div>

      {result && allocSegments.length > 0 && (
        <section className="alloc-bar-wrap" aria-label="Analyzer allocation">
          <div className="alloc-bar-header">
            <div className="alloc-bar-header-left">
              <span className="alloc-hotspots-label">Hotspots under {displayPath} · Sorted by size</span>
              {!isAtRoot && (
                <nav className="analyzer-breadcrumb analyzer-breadcrumb-trail alloc-breadcrumb-trail" aria-label="Analyzed path breadcrumb">
                  {breadcrumbParts.map((part, i) => (
                    <span key={part.path} className="breadcrumb-seg">
                      {i > 0 && <span className="breadcrumb-sep" aria-hidden="true">&gt;</span>}
                      <button type="button" className="breadcrumb-btn" onClick={() => { setRootPath(part.path); void runAnalyzer(part.path); }} disabled={analyzing || cleaning}>{part.label.replace(/\\$/, '')}</button>
                    </span>
                  ))}
                </nav>
              )}
            </div>
            <span className="alloc-bar-header-meta muted num">{formatBytes(result.totalBytes)} · Top consumers</span>
          </div>
          <svg className="alloc-bar" viewBox="0 0 1000 14" preserveAspectRatio="none" role="img" aria-label="Top-level folder allocation">
            {(() => {
              const vbW = 1000;
              const visible = allocSegments.filter((s) => s.pctVisual > 0);
              const gapUnits = visible.length > 1 ? 1 : 0;
              const totalGaps = gapUnits * Math.max(0, visible.length - 1);
              const available = Math.max(0, vbW - totalGaps);
              let widths = allocSegments.map((s) => (s.pctVisual > 0 ? (s.pctVisual / 100) * vbW : 0));
              const widthSum = widths.reduce((a, b) => a + b, 0);
              if (widthSum > 0) {
                const scale = available / widthSum;
                widths = widths.map((w) => w * scale);
              } else {
                widths = widths.map(() => 0);
              }
              const minSegW = 4;
              const sizeableIdx = widths.map((w, i) => (w > 0 ? i : -1)).filter((i) => i >= 0);
              let extra = 0;
              widths = widths.map((w, i) => {
                if (w <= 0) return 0;
                const clamped = Math.max(minSegW, w);
                extra += clamped - w;
                return clamped;
              });
              if (extra > 0 && sizeableIdx.length > 0) {
                const reducible = sizeableIdx.map((i) => Math.max(0, widths[i] - minSegW));
                const reducibleTotal = reducible.reduce((a, b) => a + b, 0);
                if (reducibleTotal > 0) {
                  let remainingExtra = extra;
                  sizeableIdx.forEach((idx, j) => {
                    const room = reducible[j];
                    const take = Math.min(room, (extra * room) / reducibleTotal);
                    widths[idx] -= take;
                    remainingExtra -= take;
                  });
                  // Numerical safety: ensure we never exceed budget.
                  const finalSum = widths.reduce((a, b) => a + b, 0);
                  if (finalSum > available && finalSum > 0) {
                    const fixScale = available / finalSum;
                    widths = widths.map((w) => w * fixScale);
                  }
                } else {
                  const finalSum = widths.reduce((a, b) => a + b, 0);
                  if (finalSum > available && finalSum > 0) {
                    const fixScale = available / finalSum;
                    widths = widths.map((w) => w * fixScale);
                  }
                }
              }
              // Final guard: cap cumulative width to viewBox budget.
              let x = 0;
              let renderedVisible = 0;
              return allocSegments.map((s) => {
                const w = Math.max(0, widths[allocSegments.indexOf(s)] ?? 0);
                if (w <= 0) return null;
                const isLastVisible = renderedVisible === visible.length - 1;
                const gap = !isLastVisible && visible.length > 1 ? gapUnits : 0;
                const maxRectW = Math.max(0, vbW - x - gap);
                const rectW = Math.min(w, maxRectW);
                const clickable = !!s.focusNodeId;
                const rect = (
                  <rect
                    key={s.id}
                    x={x}
                    y={0}
                    width={Math.max(0, rectW)}
                    height={14}
                    fill={s.color}
                    className={`alloc-segment hotspot-seg${clickable ? '' : ' alloc-segment-static'}`}
                    onClick={clickable ? () => focusNode(s.focusNodeId!) : undefined}
                  >
                    <title>{`${s.name}: ${formatBytes(s.bytes.toString())} (${formatSharePercent(s.pct)})`}</title>
                  </rect>
                );
                x += rectW + gap;
                renderedVisible += 1;
                return rect;
              });
            })()}
          </svg>
          <div className="alloc-legend">
            {allocSegments.map((s) => {
              const labelPct = s.name === 'Other' && s.pct < 0.1 ? '<0.1%' : formatSharePercent(s.pct);
              const clickable = !!s.focusNodeId;
              const body = (
                <>
                  <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><rect x="0" y="0" width="8" height="8" fill={s.color} rx="2" /></svg>
                  <span>{s.name}</span>
                  <span className="muted num">{formatBytes(s.bytes.toString())} · {labelPct}</span>
                </>
              );
              return clickable ? (
                <button key={s.id} type="button" className="alloc-legend-item alloc-legend-btn" onClick={() => focusNode(s.focusNodeId!)} title={`Inspect ${s.name}`}>{body}</button>
              ) : (
                <span key={s.id} className="alloc-legend-item alloc-legend-static" aria-label={`${s.name} ${labelPct}`}>{body}</span>
              );
            })}
          </div>
        </section>
      )}

      {analyzing && (
        <div role="status" aria-live="polite" className="notice info scanning-pulse analyzer-progress-notice">
          <div className="analyzer-progress-row">
            <span className="analyzer-progress-label">Analyzing…</span>
            <span className="analyzer-progress-stats num">
              {progress ? `dirs ${progress.scannedDirs}, files ${progress.scannedFiles}, ${formatBytes(progress.scannedBytes)}` : 'Starting…'}
            </span>
          </div>
          {progress?.currentPath && (
            <div className="analyzer-progress-path-row">
              <span className="analyzer-progress-path-prefix muted">Current:</span>
              <span
                className="analyzer-progress-path mono"
                title={normalizeDisplayPath(progress.currentPath)}
              >
                {normalizeDisplayPath(progress.currentPath)}
              </span>
            </div>
          )}
          <div className="progress" aria-hidden="true"><div className="progress-bar progress-indeterminate" /></div>
        </div>
      )}

      {result && (
        <>
          <section className="master-detail analyzer-layout scan-layout-with-bar" aria-label="Analyzer results">
            <div className="master-pane">
              <h2 id="analyzer-tree-heading" className="sr-only">Analyzer Tree</h2>
              <div className="analyzer-table-toolbar analyzer-table-toolbar-compact analyzer-toolbar" aria-label="Analyzer table filters">
                <div className="analyzer-toolbar-row analyzer-toolbar-row-top">
                  <div className="segmented-toggle scan-status-segmented analyzer-status-segmented analyzer-status-segmented-compact" role="group" aria-label="Analyzer status filters">
                    <button type="button" className={`seg-btn ${statusFilter === 'all' ? 'active' : ''}`} aria-pressed={statusFilter === 'all'} onClick={() => setStatusFilter('all')}>All ({statusCounts.all})</button>
                    <button type="button" className={`seg-btn ${statusFilter === 'safe' ? 'active' : ''}`} aria-pressed={statusFilter === 'safe'} onClick={() => setStatusFilter('safe')}>Safe ({statusCounts.safe})</button>
                    <button type="button" className={`seg-btn ${statusFilter === 'caution' ? 'active' : ''}`} aria-pressed={statusFilter === 'caution'} onClick={() => setStatusFilter('caution')}>Caution ({statusCounts.caution})</button>
                    <button type="button" className={`seg-btn ${statusFilter === 'unknown' ? 'active' : ''}`} aria-pressed={statusFilter === 'unknown'} onClick={() => setStatusFilter('unknown')}>Unknown ({statusCounts.unknown})</button>
                    <button type="button" className={`seg-btn ${statusFilter === 'locked' ? 'active' : ''}`} aria-pressed={statusFilter === 'locked'} onClick={() => setStatusFilter('locked')}>Protected &amp; Read-Only ({statusCounts.locked})</button>
                  </div>
                </div>
                <div className="analyzer-toolbar-row analyzer-toolbar-row-actions">
                  <input type="search" className="analyzer-search-input analyzer-search-compact" placeholder="Search folders..." value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search analyzer folders" disabled={!result} />
                  <div className="analyzer-table-toolbar-actions">
                    <label className="toolbar-group analyzer-sort-group">Sort
                      <select value={sort} onChange={(e) => setSort(e.target.value as 'size' | 'items' | 'name')} aria-label="Sort analyzer" disabled={!result}>
                        <option value="size">Size</option>
                        <option value="items">Items</option>
                        <option value="name">Name</option>
                      </select>
                    </label>
                    <button type="button" className="btn small" onClick={selectSafeAnalyzer} disabled={!result || analyzing || cleaning} aria-label="Select Safe">Select Safe</button>
                    <button type="button" className="btn small subtle-btn" onClick={toggleExpandCollapseAll} disabled={analyzing || cleaning || !result}>{allVisibleExpanded ? 'Collapse All' : 'Expand All'}</button>
                  </div>
                </div>
              </div>
              <div className="analyzer-col-headers" aria-hidden="true">
                <span className="analyzer-col-h-name">
                  <span className="analyzer-header-master-slot">
                    <label className="checkbox analyzer-master-check" aria-label={(() => {
                      const selectableVisible = flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, allowDeleteUnknown));
                      const ids = selectableVisible.map((r) => r.node.id);
                      const allSel = ids.length > 0 && ids.every((id) => selectedIds.has(id));
                      return allSel ? 'Deselect all visible selectable nodes' : 'Select all visible selectable nodes';
                    })()}>
                      <input
                        type="checkbox"
                        checked={(() => { const ids = flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, allowDeleteUnknown)).map((r) => r.node.id); return ids.length > 0 && ids.every((id) => selectedIds.has(id)); })()}
                        ref={(el) => {
                          if (!el) return;
                          const ids = flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, allowDeleteUnknown)).map((r) => r.node.id);
                          const selCount = ids.filter((id) => selectedIds.has(id)).length;
                          el.indeterminate = selCount > 0 && selCount < ids.length;
                        }}
                        disabled={analyzing || cleaning || flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, allowDeleteUnknown)).length === 0}
                        onChange={() => {
                          const ids = flatRows.filter((r) => !isMoreRowId(r.node.id) && isNodeSelectable(r.node, allowDeleteUnknown)).map((r) => r.node.id);
                          const allSel = ids.length > 0 && ids.every((id) => selectedIds.has(id));
                          invalidatePreviewRequest();
                          setPreview(null); setCleanSummary(null); setCleanProgress(null);
                          setSelectedIds((prev) => {
                            const next = new Set(prev);
                            if (allSel) ids.forEach((id) => next.delete(id)); else ids.forEach((id) => next.add(id));
                            return next;
                          });
                        }}
                      />
                    </label>
                  </span>
                  <span>Name</span>
                </span>
                <span className="analyzer-col-h-share">Share</span>
                <span className="analyzer-col-h-items">Files</span>
                <span className="analyzer-col-h-size">Size</span>
                <span className="analyzer-col-h-status">Status</span>
              </div>
              <div className="pane-body flush">
                <div className="analyzer-viewport" role="tree" aria-label="Analyzer results tree" tabIndex={0}>
                  {flatRows.map(({ node, depth }) => {
                    if (isMoreRowId(node.id)) {
                      const parentId = moreRowParentId(node.id);
                      return (
                        <div key={node.id} className="analyzer-row analyzer-more-row" role="treeitem" aria-level={depth + 1} aria-selected={false} tabIndex={0}>
                          <div className="analyzer-col-main tree-cell-name">
                            <span className={`tree-indent${depth >= 1 ? ' has-guide' : ''}`} aria-hidden="true">
                              {depth >= 1 && <span className="tree-guide-line" />}
                              {Array.from({ length: depth }).map((_, i) => <span key={i} className="indent-step" />)}
                            </span>
                            <button type="button" className="btn small subtle-btn analyzer-show-more-btn" onClick={() => setShowAllAnalyzerNodes((prev) => new Set([...prev, parentId]))}>{node.name}</button>
                          </div>
                          <div className="analyzer-col-share tree-cell-share" />
                          <div className="analyzer-col-count muted num" />
                          <div className="analyzer-col-sizecell num" />
                          <div className="analyzer-col-guard" />
                        </div>
                      );
                    }
                    const selectable = isNodeSelectable(node, allowDeleteUnknown);
                    const checked = selectedIds.has(node.id);
                    const hasChildren = node.isDir && (node.children?.length ?? 0) > 0;
                    const isExpanded = expanded.has(node.id);
                    const status = analyzerStatusText(node, selectable, allowDeleteUnknown);
                    const pill = analyzerStatusPill(node, selectable, allowDeleteUnknown);
                    const typeTag = nodeTypeTag(node);
                    const nodeSize = BigInt(node.sizeBytes || '0');
                    const shareRatio = rootTotalBytes > 0n ? Number(nodeSize) / Number(rootTotalBytes) : 0;
                    const share = shareRatio * 100;
                    const shareLabel = formatSharePercent(share);
                    const shareFillWidthPx = shareBarFillPx(nodeSize, rootTotalBytes);
                    const pathDisplay = normalizeDisplayPath(node.path);
                    const pathDisplayShort = middleTruncatePath(pathDisplay, 64);
                    const inspectedMatch = inspected?.itemId === node.id;
                    const selectionSlot = selectable ? (
                      <label className="checkbox analyzer-checkbox" title="Select for Review & Clean">
                        <input type="checkbox" aria-label={`Select analyzer node ${node.path}`} checked={checked} disabled={cleaning} onChange={() => toggleSelect(node)} />
                      </label>
                    ) : (
                      <span className="analyzer-selection-fixed-slot analyzer-disabled-slot" title={status.text}>
                        <span className="scan-disabled-checkbox-outline analyzer-disabled-checkbox-outline" aria-hidden="true" />
                        <input className="sr-only" type="checkbox" aria-label={`Select analyzer node ${node.path}`} checked={checked} disabled readOnly tabIndex={-1} />
                      </span>
                    );
                    return (
                      <div
                        key={node.id}
                        role="treeitem"
                        aria-expanded={hasChildren ? isExpanded : undefined}
                        aria-selected={checked}
                        aria-level={depth + 1}
                        tabIndex={0}
                        className={`analyzer-row ${inspectedMatch ? 'inspected' : ''} ${depth >= 1 ? 'depth-child analyzer-child-row' : ''}`}
                        data-depth={depth}
                        title={hasChildren && depth > 0 ? 'Double-click to inspect folder as root' : undefined}
                        onClick={() => { void inspectNode(node.id); if (selectable && !cleaning) toggleSelect(node); }}
                        onDoubleClick={(e) => {
                          if (!node.isDir || !hasChildren || depth <= 0 || cleaning) return;
                          e.stopPropagation();
                          setRootPath(node.path);
                          void runAnalyzer(node.path);
                        }}
                        onKeyDown={(e) => {
                          if (e.target !== e.currentTarget) return;
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault();
                            void inspectNode(node.id);
                            if (selectable && !cleaning) toggleSelect(node);
                          }
                        }}
                      >
                        <div className="analyzer-col-main tree-cell-name">
                          <span className={`tree-indent depth-${depth}${depth >= 1 ? ' has-guide' : ''}`} aria-hidden="true">
                            {depth >= 1 && <span className="tree-guide-line" />}
                            {Array.from({ length: depth }).map((_, i) => <span key={i} className={`indent-step indent-step-d${depth}-${i}`} />)}
                          </span>
                          <span className="tree-slot tree-slot-chevron">
                            {hasChildren ? (
                              <button type="button" className="tree-toggle" onClick={(e) => { e.stopPropagation(); toggleExpand(node); }} aria-label={isExpanded ? `Collapse ${node.name}` : `Expand ${node.name}`}>
                                {isExpanded ? <ChevronDownIcon /> : <ChevronRightIcon />}
                              </button>
                            ) : (
                              <span className="tree-toggle-spacer" aria-hidden="true" />
                            )}
                          </span>
                          <span className={`tree-slot tree-slot-selection ${selectable ? '' : 'locked'}`} onClick={(e) => e.stopPropagation()}>
                            {selectionSlot}
                          </span>
                          <span className="tree-slot tree-slot-icon analyzer-type-slot" aria-hidden="true">{semanticNodeIcon(node)}</span>
                          <div className="analyzer-node-main">
                            <div className="analyzer-name-line">
                              <span className="analyzer-name" title={node.name}>{node.name}</span>
                              {typeTag && <span className="inline-type-tag muted">· {typeTag}</span>}
                              {node.isDir && (
                                <button
                                  type="button"
                                  className="analyzer-drill-btn"
                                  onClick={(e) => { e.stopPropagation(); if (cleaning) return; setRootPath(node.path); void runAnalyzer(node.path); }}
                                  aria-label={`Drill into ${node.name}`}
                                  title="Drill into folder"
                                  disabled={cleaning}
                                >
                                  Drill
                                </button>
                              )}
                            </div>
                            <div className="analyzer-path analyzer-node-path" title={pathDisplay}>{pathDisplayShort}</div>
                            <span className="sr-only">{status.text}</span>
                          </div>
                        </div>
                        <div className="analyzer-col-share tree-cell-share">
                          <div className="analyzer-share-cell">
                            {shareFillWidthPx <= 0 ? (
                              <span className="analyzer-share-empty muted" aria-hidden="true">—</span>
                            ) : (
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
                        <div className="analyzer-col-guard">
                          {pill ? <span className={pill.className}>{pill.label}</span> : <span className="status-muted-inline" />}
                        </div>
                      </div>
                    );
                  })}
                  {flatRows.length === 0 && <div className="muted analyzer-empty">No nodes.</div>}
                </div>
              </div>
            </div>

            <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="analyzer-preview-heading">
              <div className="pane-header">
                <h3 id="analyzer-preview-heading">Inspector</h3>
              </div>
              <div className="detail-scroll">
                <div className="inspector-section" aria-label="Analyzer inspector">
                  {inspected && inspectedNode ? (
                    <>
                      <InspectorTitleRow
                        icon={leadingInspectorIcon(inspectedNode)}
                        title={inspected.title || inspectedNode.name}
                        subtitle={nodeTypeTag(inspectedNode) ?? (isDriveRoot(inspectedNode) ? 'Drive' : 'Folder')}
                        badge={inspectedPill ? <span className={inspectedPill.className}>{inspectedPill.label}</span> : <span className="status-pill">{inspectedNode.risk}</span>}
                      />
                      <InspectorMetaGrid
                        size={inspectedNode.sizeBytes}
                        extraLabel="Items"
                        extraValue={<span className="num">{inspectedNode.itemCount}</span>}
                        eligibility={isDriveRoot(inspectedNode) ? 'Drive root — expand C: below to inspect and select specific folders.' : (inspectedStatus ? (inspectedSelectable ? 'Meets age limit (> 24h old)' : inspectedStatus.text) : '—')}
                      />
                      {!isDriveRoot(inspectedNode) && <p className="inspector-guidance">{analyzerGuidance(inspectedNode, inspectedSelectable, allowDeleteUnknown)}</p>}
                      {inspectedNode.path.toLowerCase().endsWith('.vhdx') && (() => {
                        const vhdxPath = normalizeDisplayPath(inspectedNode.path);
                        const quotedVhdxPath = `'${vhdxPath.replace(/['\u2018\u2019\u201A\u201B]/g, (q) => q + q)}'`;
                        const cmd = `wsl --shutdown\nOptimize-VHD -Path ${quotedVhdxPath} -Mode Full`;
                        return (
                          <div className="notice info analyzer-vhdx-callout" role="note">
                            <div><strong>Compaction (manual):</strong> shut down WSL/VM first, then run in an elevated PowerShell (Hyper-V).</div>
                            <pre className="analyzer-vhdx-cmd"><code>{cmd}</code></pre>
                            <button type="button" className="btn small subtle-btn analyzer-copy-cmd-btn" onClick={() => void copyVhdxCommand(cmd)} aria-label="Copy Command">{copiedVhdxCmd ? 'Copied' : 'Copy Command'}</button>
                          </div>
                        );
                      })()}
                      {inspectedStatus?.unknownGated && onOpenSettings && (
                        <div className="inspector-policy-links" aria-label="Inspector actions">
                          <button type="button" className="btn small inspector-inline-link" onClick={() => onOpenSettings('allowDeleteUnknown')}>Allow unknown folders in Settings →</button>
                        </div>
                      )}
                      <PathWell path={normalizeDisplayPath(inspectedNode.path)} />
                      <PathActions path={normalizeDisplayPath(inspectedNode.path)} />
                    </>
                  ) : (
                    <p className="inspector-empty">Select a node to see its purpose, risk, full path, and eligibility.</p>
                  )}
                </div>

                {requiresConfirmation && (
                  <div className="notice warn" role="note">
                    <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path nodes). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
                    <div className="field">
                      <label htmlFor="analyzer-confirm-delete">Typed confirmation</label>
                      <input id="analyzer-confirm-delete" type="text" value={confirmText} onChange={(e) => setConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={cleaning} />
                    </div>
                  </div>
                )}

                {preview && <PreviewBreakdownBlock preview={preview} cleaning={cleaning} cleanProgressLabel={cleanProgressLabel} />}
                {cleanSummary && <CleanSummaryBlock summary={cleanSummary} onOpenLog={onOpenLog} />}
              </div>
            </aside>
          </section>
          <div className="scan-bottom-bar" aria-label="Analyzer actions">
            <div className="scan-bottom-bar-left">
              <span className="scan-bottom-bar-summary"><strong className="num">{pluralize(selectedIds.size, 'item', 'items')} selected</strong> · <span className="num">{formatBytes(selectedBytes)}</span></span>
              {selectedIds.size > 0 && <button type="button" className="btn small subtle-btn" onClick={() => { invalidatePreviewRequest(); setSelectedIds(new Set()); setPreview(null); }} disabled={cleaning}>Clear</button>}
              {hasSelectedRisky && <span className="scan-bottom-bar-hint muted">Includes Caution/Unknown items · requires DELETE confirm</span>}
            </div>
            <div className="scan-bottom-bar-right">
              {!preview ? (
                <button className={`btn ${selectedIds.size > 0 ? 'primary' : ''}`} onClick={runPreview} disabled={!canPreview} aria-label="Review & Clean">{previewLoading ? 'Generating Preview…' : `Review & Clean${selectedIds.size > 0 ? ` (${formatBytes(selectedBytes)})` : ''}`}</button>
              ) : (
                <button className="btn danger primary" onClick={runClean} disabled={!canClean} aria-label="Clean Permanently">{cleaning ? 'Cleaning…' : 'Clean Permanently'}</button>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
