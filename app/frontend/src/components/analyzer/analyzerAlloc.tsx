import type { StartAnalyzerResult } from '../../api/client';
import { formatBytes, normalizeDisplayPath } from '../../utils/format';
import { ALLOC_COLORS, formatSharePercent } from './analyzerHelpers';

export type AllocSeg = {
  id: string;
  name: string;
  bytes: bigint;
  pct: number;
  pctVisual: number;
  color: string;
  isOther?: boolean;
  focusNodeId?: string;
};

function pctOf(args: { bytes: bigint; total: bigint }): number {
  if (args.total <= 0n) return 0;
  return Number((args.bytes * 10000n) / args.total) / 100;
}

function compareNodeBytes(a: { sizeBytes: string }, b: { sizeBytes: string }): number {
  const sa = BigInt(a.sizeBytes || '0');
  const sb = BigInt(b.sizeBytes || '0');
  if (sa === sb) return 0;
  if (sa > sb) return -1;
  return 1;
}

function majorSegments(args: { result: StartAnalyzerResult; total: bigint }) {
  const kids = [...(args.result.root.children ?? [])].sort(compareNodeBytes);
  const major: AllocSeg[] = [];
  let otherBytes = 0n;
  kids.forEach((k, i) => {
    const bytes = BigInt(k.sizeBytes || '0');
    const pct = pctOf({ bytes, total: args.total });
    if (pct >= 0.1) {
      major.push({ id: k.id, name: k.name, bytes, pct, pctVisual: 0, color: ALLOC_COLORS[i % ALLOC_COLORS.length], focusNodeId: k.id });
      return;
    }
    otherBytes += bytes;
  });
  return { major, otherBytes };
}

function withOther(args: { major: AllocSeg[]; otherBytes: bigint; total: bigint }): AllocSeg[] {
  const raw = [...args.major];
  if (args.otherBytes <= 0n) return raw;
  const pct = pctOf({ bytes: args.otherBytes, total: args.total });
  raw.push({ id: 'alloc-other', name: 'Other', bytes: args.otherBytes, pct, pctVisual: 0, color: '#64748b', isOther: true });
  return raw;
}

function initialVisuals(raw: AllocSeg[], minTiny: number): AllocSeg[] {
  return raw.map((s) => {
    let pctVisual = 0;
    if (s.pct > 0) pctVisual = Math.max(minTiny, s.pct);
    return { ...s, pctVisual };
  });
}

function shrinkAdjusted(input: { adjusted: AllocSeg[]; minTiny: number; sum: number; targetSum: number; room: number }): AllocSeg[] {
  return input.adjusted.map((s) => {
    if (s.pctVisual <= input.minTiny) return s;
    const delta = ((input.sum - input.targetSum) * (s.pctVisual - input.minTiny)) / input.room;
    return { ...s, pctVisual: s.pctVisual - delta };
  });
}

function growAdjusted(input: { adjusted: AllocSeg[]; sum: number; targetSum: number }): AllocSeg[] {
  if (input.adjusted.length === 0) return input.adjusted;
  const largest = [...input.adjusted].sort((a, b) => b.pct - a.pct)[0];
  return input.adjusted.map((s) => {
    if (s.id !== largest.id) return s;
    return { ...s, pctVisual: s.pctVisual + (input.targetSum - input.sum) };
  });
}

function adjustVisuals(raw: AllocSeg[]): AllocSeg[] {
  const visible = raw.filter((s) => s.pct > 0);
  let gapPct = 0;
  if (visible.length > 1) gapPct = 0.2;
  const targetSum = Math.max(0, 100 - gapPct * Math.max(0, visible.length - 1));
  const minTiny = 0.35;
  let adjusted = initialVisuals(raw, minTiny);
  let sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
  const room = adjusted.reduce((a, s) => {
    if (s.pctVisual <= minTiny) return a;
    return a + (s.pctVisual - minTiny);
  }, 0);
  if (sum > targetSum && room > 0) adjusted = shrinkAdjusted({ adjusted, minTiny, sum, targetSum, room });
  sum = adjusted.reduce((a, b) => a + b.pctVisual, 0);
  if (sum < targetSum) adjusted = growAdjusted({ adjusted, sum, targetSum });
  return adjusted;
}

export function buildAllocSegments(result: StartAnalyzerResult | null, rootTotalBytes: bigint): AllocSeg[] {
  if (!result) return [];
  const { major, otherBytes } = majorSegments({ result, total: rootTotalBytes });
  const raw = withOther({ major, otherBytes, total: rootTotalBytes });
  return adjustVisuals(raw);
}

function segmentWidths(segments: AllocSeg[]) {
  const visible = segments.filter((s) => s.pctVisual > 0);
  let gapUnits = 0;
  if (visible.length > 1) gapUnits = 1;
  const available = Math.max(0, 1000 - gapUnits * Math.max(0, visible.length - 1));
  let widths = segments.map((s) => {
    if (s.pctVisual > 0) return (s.pctVisual / 100) * 1000;
    return 0;
  });
  const widthSum = widths.reduce((a, b) => a + b, 0);
  if (widthSum > 0) widths = widths.map((w) => (w * available) / widthSum);
  return { widths, visible, gapUnits, available };
}

function clampWidths(args: { widths: number[]; available: number }) {
  const minSegW = 4;
  let extra = 0;
  let widths = args.widths.map((w) => {
    if (w <= 0) return 0;
    const clamped = Math.max(minSegW, w);
    extra += clamped - w;
    return clamped;
  });
  if (extra <= 0) return widths;
  const sizeableIdx = widths.map((w, i) => {
    if (w > 0) return i;
    return -1;
  }).filter((i) => i >= 0);
  const reducible = sizeableIdx.map((i) => Math.max(0, widths[i] - minSegW));
  const reducibleTotal = reducible.reduce((a, b) => a + b, 0);
  if (reducibleTotal > 0) {
    sizeableIdx.forEach((idx, j) => {
      widths[idx] -= Math.min(reducible[j], (extra * reducible[j]) / reducibleTotal);
    });
  }
  const finalSum = widths.reduce((a, b) => a + b, 0);
  if (finalSum > args.available && finalSum > 0) widths = widths.map((w) => (w * args.available) / finalSum);
  return widths;
}

function legendLabel(s: AllocSeg) {
  if (!s.isOther && s.name !== 'Other') return formatSharePercent({ share: s.pct });
  if (s.pct < 0.1) return '<0.1%';
  return formatSharePercent({ share: s.pct });
}

const LEGEND_CAP = 12;

function cappedLegendSegments(segments: AllocSeg[]): AllocSeg[] {
  if (segments.length <= LEGEND_CAP) return segments;
  const top = segments.slice(0, LEGEND_CAP);
  const rest = segments.slice(LEGEND_CAP);
  const otherBytes = rest.reduce((a, s) => a + s.bytes, 0n);
  const otherPct = rest.reduce((a, s) => a + s.pct, 0);
  top.push({
    id: 'alloc-legend-other-more',
    name: `Other (${rest.length} more)`,
    bytes: otherBytes,
    pct: otherPct,
    pctVisual: 0,
    color: '#64748b',
    isOther: true,
  });
  return top;
}

type AllocProps = {
  segments: AllocSeg[];
  result: StartAnalyzerResult;
  displayPath: string;
  analyzing: boolean;
  cleaning: boolean;
  onRun: (args: { path: string }) => void;
  onFocus: (args: { id: string }) => void;
};

function AllocBreadcrumb(props: { displayPath: string; analyzing: boolean; cleaning: boolean; onRun: (args: { path: string }) => void }) {
  if (/^([A-Za-z]:\\)$/.test(props.displayPath)) return null;
  const parts = props.displayPath.split('\\').filter(Boolean).map((seg, index, arr) => {
    let label = seg;
    if (index === 0) label = `${seg}\\`;
    let path = arr.slice(0, index + 1).join('\\');
    if (index === 0) path = `${path}\\`;
    return { label, path };
  });
  let busy = false;
  if (props.analyzing) busy = true;
  if (props.cleaning) busy = true;
  return (
    <nav className="analyzer-breadcrumb analyzer-breadcrumb-trail alloc-breadcrumb-trail" aria-label="Analyzed path breadcrumb">
      {parts.map((part, i) => {
        let sep: JSX.Element | null = null;
        if (i > 0) sep = <span className="breadcrumb-sep" aria-hidden="true">&gt;</span>;
        return (
          <span key={part.path} className="breadcrumb-seg">
            {sep}
            <button type="button" className="breadcrumb-btn" onClick={() => props.onRun({ path: part.path })} disabled={busy}>{part.label.replace(/\\$/, '')}</button>
          </span>
        );
      })}
    </nav>
  );
}

function AllocLegendItem(props: { seg: AllocSeg; onFocus: (args: { id: string }) => void }) {
  const body = (
    <>
      <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><rect x="0" y="0" width="8" height="8" fill={props.seg.color} rx="2" /></svg>
      <span>{props.seg.name}</span>
      <span className="muted num">{formatBytes({ bytes: props.seg.bytes.toString() })} · {legendLabel(props.seg)}</span>
    </>
  );
  if (props.seg.focusNodeId) {
    const id = props.seg.focusNodeId;
    return <button type="button" className="alloc-legend-item alloc-legend-btn" onClick={() => props.onFocus({ id })} title={`Inspect ${props.seg.name}`}>{body}</button>;
  }
  return <span className="alloc-legend-item alloc-legend-static" aria-label={`${props.seg.name} ${legendLabel(props.seg)}`}>{body}</span>;
}

type Layout = { widths: number[]; visible: AllocSeg[]; gapUnits: number; available: number };

function renderSegments(input: { segments: AllocSeg[]; widths: number[]; layout: Layout; onFocus: (args: { id: string }) => void }) {
  let x = 0;
  let renderedVisible = 0;
  const out: JSX.Element[] = [];
  input.segments.forEach((s, i) => {
    const w = Math.max(0, input.widths[i] ?? 0);
    if (w <= 0) return;
    const isLastVisible = renderedVisible === input.layout.visible.length - 1;
    let gap = 0;
    if (!isLastVisible) gap = input.layout.gapUnits;
    if (input.layout.visible.length <= 1) gap = 0;
    const rectW = Math.min(w, Math.max(0, 1000 - x - gap));
    const title = `${s.name}: ${formatBytes({ bytes: s.bytes.toString() })} (${formatSharePercent({ share: s.pct })})`;
    const clickable = Boolean(s.focusNodeId);
    function onClick() {
      if (!s.focusNodeId) return;
      input.onFocus({ id: s.focusNodeId });
    }
    let clickHandler: (() => void) | undefined;
    if (clickable) clickHandler = onClick;
    let className = 'alloc-segment hotspot-seg';
    if (!clickable) className = 'alloc-segment hotspot-seg alloc-segment-static';
    out.push(<rect key={s.id} x={x} y={0} width={Math.max(0, rectW)} height={14} fill={s.color} className={className} onClick={clickHandler}><title>{title}</title></rect>);
    x += rectW + gap;
    renderedVisible += 1;
  });
  return out;
}

export function AllocBar(props: AllocProps) {
  const layout = segmentWidths(props.segments);
  const widths = clampWidths({ widths: layout.widths, available: layout.available });
  return (
    <section className="alloc-bar-wrap" aria-label="Analyzer allocation">
      <div className="alloc-bar-header">
        <div className="alloc-bar-header-left">
          <span className="alloc-hotspots-label">Hotspots under {props.displayPath} · Sorted by size</span>
          <AllocBreadcrumb displayPath={props.displayPath} analyzing={props.analyzing} cleaning={props.cleaning} onRun={props.onRun} />
        </div>
        <span className="alloc-bar-header-meta muted num">{formatBytes({ bytes: props.result.totalBytes })} · Top consumers</span>
      </div>
      <svg className="alloc-bar" viewBox="0 0 1000 14" preserveAspectRatio="none" role="img" aria-label="Top-level folder allocation">
        {renderSegments({ segments: props.segments, widths, layout, onFocus: props.onFocus })}
      </svg>
      <div className="alloc-legend">
        {cappedLegendSegments(props.segments).map((s) => <AllocLegendItem key={s.id} seg={s} onFocus={props.onFocus} />)}
      </div>
    </section>
  );
}
