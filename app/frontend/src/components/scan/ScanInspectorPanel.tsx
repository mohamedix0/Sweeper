import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { formatBytes, normalizeDisplayPath, pluralize } from '../../utils/format';
import { CleanSummaryBlock, InspectorMetaGrid, InspectorTitleRow, PathActions, PathWell, PreviewBreakdownBlock, TYPED_CONFIRMATION_PHRASE, formatModifiedAge, type ExplainState, type PreviewResult } from '../cleanShared';
import { FileIcon, FolderIcon } from '../Icons';
import type { ExecuteCleanResponse } from '../../api/client';
import { childStatusForScan, customDisplayName, formatMinAgeLabel, humanizeSkippedReason, isCustomPathTargetId, isSelectableScanItem, itemDisplayName, relativeItemName, resolvedRootPath, scanGuidance, statusClassForKind, statusDisplayLabel, sumDecimalStrings } from './scanHelpers';

function GroupInspector(props: {
  target: TargetScanResultDto;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}) {
  const t = props.target;
  const selectableItems = t.items.filter((i) => isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: props.allowDeleteUnknown }));
  const selectedInTarget = selectableItems.filter((i) => props.selectedIds.has(i.id)).length;
  const skippedInTarget = t.items.length - selectableItems.length;
  const isCustomUnknownLocked = isCustomPathTargetId(t.targetId) && t.targetName === 'Custom Path (Unknown)' && t.risk === 'Unknown' && !props.allowDeleteUnknown;
  const groupStatusLabel = isCustomUnknownLocked ? 'Skipped · Unverified' : (selectedInTarget > 0 ? 'Eligible' : t.risk);
  const groupStatusKind = isCustomUnknownLocked ? 'skipped' : (selectedInTarget > 0 ? 'eligible' : (t.risk === 'Protected' ? 'protected' : t.risk === 'Caution' ? 'caution' : t.risk === 'Unknown' ? 'unknown' : 'safe'));
  const display = customDisplayName(t);
  const topFiles = [...t.items].sort((a, b) => BigInt(b.size) === BigInt(a.size) ? 0 : (BigInt(b.size) > BigInt(a.size) ? 1 : -1)).slice(0, 5);
  const eligibleBytes = sumDecimalStrings(selectableItems.map((i) => i.size));
  const skippedBytes = sumDecimalStrings(t.items.filter((i) => !isSelectableScanItem({ item: i, target: t, allowDeleteUnknown: props.allowDeleteUnknown })).map((i) => i.size));
  const ageLabel = props.minAgeHours == null ? 'configured minimum age' : (props.minAgeHours === 0 ? 'no minimum age' : `${props.minAgeHours}h`);
  const policyText = t.targetName === 'User Temp' ? `Policy: Minimum age ${props.minAgeHours === 0 ? '(none)' : `> ${ageLabel}`} · Auto-recreated by Windows/apps` : `Policy: Minimum age ${props.minAgeHours === 0 ? '(none)' : `> ${ageLabel}`} · Governed by ${t.targetName} safety rules`;
  const desc = t.items[0] ? (t.targetName === 'User Temp' ? 'Temporary files Windows and apps can recreate automatically. Safe to remove when not in active use.' : (t.statusReason || 'Category-level cleanup target governed by Sweeper safety rules.')) : (t.statusReason || 'Category-level cleanup target governed by Sweeper safety rules.');
  return (
    <>
      <InspectorTitleRow icon={<FolderIcon className="inspector-leading-icon" />} title={display.title} subtitle="Category Overview" badge={<span className={statusClassForKind(groupStatusKind as any, groupStatusLabel)}>{groupStatusLabel}</span>} />
      <p className="inspector-guidance">{desc}</p>
      <dl className="inspector-meta-grid inspector-meta-grid-v19" aria-label="Category metadata">
        <div><dt>Reclaimable</dt><dd className="num">{formatBytes(eligibleBytes || '0')}</dd></div>
        <div><dt>Skipped</dt><dd className="num">{formatBytes(skippedBytes || '0')}</dd></div>
        <div className="inspector-meta-eligibility inspector-meta-eligibility-full"><dt>Items</dt><dd>{selectedInTarget} selected · {skippedInTarget} skipped · {pluralize(t.items.length, 'file', 'files')} total</dd></div>
      </dl>
      <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety"><div className="inspector-policy-title">Rule &amp; Safety</div><div className="inspector-policy-text"><strong>{policyText}</strong></div></div>
      <div className="inspector-top-files" aria-label="Top files preview">
        <strong className="subheading-xs">Top files</strong>
        {topFiles.length === 0 ? <p className="muted small-note">No files in this category.</p> : (
          <ul className="inspector-top-files-list">
            {topFiles.map((item) => {
              const name = itemDisplayName({ itemPath: item.path, resolvedPaths: t.resolvedPaths }).name;
              const st = childStatusForScan({ target: t, item, selectable: isSelectableScanItem({ item, target: t, allowDeleteUnknown: props.allowDeleteUnknown }), allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
              return <li key={item.id}><span className="inspector-top-file-name" title={normalizeDisplayPath(item.path)}>{name}</span><span className="num muted">{formatBytes(item.size)}</span><span className="muted">{statusDisplayLabel(st.kind, st.label)}</span></li>;
            })}
          </ul>
        )}
      </div>
      {isCustomUnknownLocked && props.onOpenSettings && (
        <div className="inspector-policy-links inspector-allow-settings-prominent" aria-label="Inspector actions">
          <div className="inspector-rule-safety-block"><div className="inspector-rule-safety-title">ELIGIBILITY / RULE &amp; SAFETY</div><p className="muted small-note">Unverified custom folder is locked by Settings. Enable custom path cleanup to make it eligible.</p></div>
          <button type="button" className="btn small secondary-action inspector-inline-link prominent-allow-link" onClick={() => props.onOpenSettings?.('allowDeleteUnknown', resolvedRootPath(t))}>Allow in Settings →</button>
        </div>
      )}
    </>
  );
}

function ItemInspector(props: { target: TargetScanResultDto; item: ScanItemDto; inspected: ExplainState; allowDeleteUnknown: boolean; minAgeHours?: number | null; onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void }) {
  const { target, item } = props;
  const selectable = isSelectableScanItem({ item, target, allowDeleteUnknown: props.allowDeleteUnknown });
  const fullPath = normalizeDisplayPath(item.path);
  const itemTitle = relativeItemName({ itemPath: item.path, resolvedPaths: target.resolvedPaths }).split(/[\\/]/).pop() || props.inspected?.title || '';
  const status = childStatusForScan({ target, item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  const eligibility = selectable ? (props.minAgeHours == null ? 'Meets age limit' : props.minAgeHours === 0 ? 'Eligible (no minimum age)' : `Meets age limit (> ${props.minAgeHours}h old)`) : humanizeSkippedReason({ target, item, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours });
  const modifiedAge = formatModifiedAge(item.mtimeUnixSecs, item.ageEligible, props.minAgeHours ?? undefined);
  const isVhdx = fullPath.toLowerCase().endsWith('.vhdx');
  const isCustomUnknownLocked = isCustomPathTargetId(target.targetId) && target.risk === 'Unknown' && !props.allowDeleteUnknown;
  const isSkippedByAge = !selectable && !item.ageEligible;
  const display = customDisplayName(target);
  const policyAge = props.minAgeHours === 0 ? '(none)' : `> ${props.minAgeHours == null ? 'configured' : `${props.minAgeHours}h`}`;
  const policy = target.targetName === 'User Temp' ? `Minimum age ${policyAge} · Auto-recreated by Windows/apps` : `Minimum age ${policyAge} · Governed by ${target.targetName} safety rules`;
  return (
    <>
      <InspectorTitleRow icon={<FileIcon className="inspector-leading-icon" />} title={itemTitle} subtitle={display.tag ? display.title : target.targetName} badge={<span className={statusClassForKind(status.kind, status.label)} aria-label={status.label.startsWith('Skipped') || status.kind === 'skipped' ? 'Status Skipped' : `Status ${status.label}`}>{statusDisplayLabel(status.kind, status.label)}</span>} />
      <p className="inspector-guidance">{scanGuidance({ target, item, selectable, allowDeleteUnknown: props.allowDeleteUnknown, minAgeHours: props.minAgeHours })}</p>
      <InspectorMetaGrid size={item.size} modifiedAge={modifiedAge} eligibility={eligibility} />
      <div className="inspector-policy-footer inspector-rule-safety" aria-label="Rule and safety"><div className="inspector-policy-title">Rule &amp; Safety</div><div className="inspector-policy-text"><strong>Policy:</strong> {policy}</div></div>
      {isVhdx && <div className="notice info" role="note"><div><strong>.vhdx is display-only.</strong> To compact: shut down WSL/VM (<code>wsl --shutdown</code>) then run <code>Optimize-VHD</code> (Hyper-V) or your VM tooling.</div></div>}
      <PathWell path={fullPath} /><PathActions path={fullPath} />
      {(isCustomUnknownLocked || isSkippedByAge) && props.onOpenSettings && (
        <div className="inspector-policy-links" aria-label="Inspector actions">
          {isCustomUnknownLocked && <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.('allowDeleteUnknown', resolvedRootPath(target))}>Allow in Settings →</button>}
          {isSkippedByAge && <button type="button" className="btn small inspector-inline-link" onClick={() => props.onOpenSettings?.()}>Adjust minimum age in Settings →</button>}
        </div>
      )}
    </>
  );
}

export function ScanInspectorPanel(props: {
  focus: { kind: 'item'; itemId: string } | { kind: 'group'; targetId: string } | null;
  focusedGroup: TargetScanResultDto | null;
  inspected: ExplainState;
  inspectedCtx: { target: TargetScanResultDto; item: ScanItemDto } | null;
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  requiresConfirmation: boolean;
  confirmText: string;
  onConfirmText: (v: string) => void;
  cleaning: boolean;
  preview: PreviewResult | null;
  cleanProgressLabel: string | null;
  cleanSummary: ExecuteCleanResponse | null;
  onOpenLog: (id: string) => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}) {
  return (
    <aside className="detail-pane selection-pane scan-inspector-pane" aria-labelledby="inspector-heading">
      <div className="pane-header"><h3 id="inspector-heading">Inspector</h3></div>
      <div className="detail-scroll">
        <div className="inspector-section" aria-label="Item inspector">
          {props.focus?.kind === 'group' && props.focusedGroup ? (
            <GroupInspector target={props.focusedGroup} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} selectedIds={props.selectedIds} onOpenSettings={props.onOpenSettings} />
          ) : props.inspected && props.inspectedCtx && props.focus?.kind === 'item' ? (
            <ItemInspector target={props.inspectedCtx.target} item={props.inspectedCtx.item} inspected={props.inspected} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} onOpenSettings={props.onOpenSettings} />
          ) : <p className="inspector-empty">Select an item to see its purpose, risk, full path, and eligibility.</p>}
        </div>
        {props.requiresConfirmation && (
          <div className="notice warn" role="note">
            <div><strong>Typed confirmation required</strong> for this selection (Unknown and/or Custom Path items). Enter <code>{TYPED_CONFIRMATION_PHRASE}</code> to request Preview.</div>
            <div className="field"><label htmlFor="confirm-delete">Typed confirmation</label><input id="confirm-delete" type="text" value={props.confirmText} onChange={(e) => props.onConfirmText(e.target.value)} placeholder={`Type ${TYPED_CONFIRMATION_PHRASE}`} autoComplete="off" disabled={props.cleaning} /></div>
          </div>
        )}
        {props.preview && <PreviewBreakdownBlock preview={props.preview} cleaning={props.cleaning} cleanProgressLabel={props.cleanProgressLabel} />}
        {props.cleanSummary && <CleanSummaryBlock summary={props.cleanSummary} onOpenLog={props.onOpenLog} />}
      </div>
    </aside>
  );
}
