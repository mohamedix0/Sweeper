import type { ScanItemDto, TargetScanResultDto } from '../../contract/bindings';
import { VISIBLE_CHILD_CAP } from './scanHelpers';
import { ScanGroupRow } from './ScanGroupRow';
import { ScanItemRow } from './ScanItemRow';

interface Props {
  target: TargetScanResultDto;
  visibleItems: ScanItemDto[];
  allowDeleteUnknown: boolean;
  minAgeHours?: number | null;
  selectedIds: Set<string>;
  collapsed: boolean;
  showAll: boolean;
  focusedItemId: string | null;
  groupActive: boolean;
  cleaning: boolean;
  onFocusGroup: () => void;
  onToggleCollapsed: () => void;
  onToggleGroupSelect: () => void;
  onToggleItem: (item: ScanItemDto) => void;
  onToggleShowAll: () => void;
  onOpenSettings?: (focus?: 'allowDeleteUnknown' | 'enableTier3' | null, prefillCustomPath?: string | null) => void;
}

function cardClass(collapsed: boolean): string {
  if (collapsed) return 'target-card collapsed';
  return 'target-card expanded';
}

function limitedItems(args: { showAll: boolean; visibleItems: ScanItemDto[] }) {
  if (args.showAll) return args.visibleItems;
  if (args.visibleItems.length <= VISIBLE_CHILD_CAP) return args.visibleItems;
  return args.visibleItems.slice(0, VISIBLE_CHILD_CAP);
}

function hiddenCount(args: { showAll: boolean; visibleItems: ScanItemDto[]; limited: number }): number {
  if (args.showAll) return 0;
  return Math.max(0, args.visibleItems.length - args.limited);
}

function EmptyRow(props: { show: boolean; text: string }) {
  if (!props.show) return null;
  return <div className="scan-row scan-item-empty"><span className="muted">{props.text}</span></div>;
}

function ShowAllRow(props: { onClick: () => void; label: string }) {
  return <div className="scan-row scan-item-empty scan-show-all-row"><button type="button" className="btn small subtle-btn" onClick={props.onClick}>{props.label}</button></div>;
}

function hasNoTargetItems(props: Props): boolean {
  return props.target.items.length === 0;
}

function hasNoVisible(props: Props): boolean {
  if (props.target.items.length === 0) return false;
  return props.visibleItems.length === 0;
}

function showFewer(args: { showAll: boolean; visible: number }): boolean {
  if (!args.showAll) return false;
  return args.visible > VISIBLE_CHILD_CAP;
}

function ShowMoreBlock(props: { hidden: number; onClick: () => void; total: number }) {
  if (props.hidden <= 0) return null;
  return <ShowAllRow onClick={props.onClick} label={`Showing top ${VISIBLE_CHILD_CAP} of ${props.total} files · Show more`} />;
}

function ShowFewerBlock(props: { show: boolean; onClick: () => void }) {
  if (!props.show) return null;
  return <ShowAllRow onClick={props.onClick} label="Show fewer files" />;
}

function ItemsBody(props: Props) {
  if (props.collapsed) return null;
  const limited = limitedItems({ showAll: props.showAll, visibleItems: props.visibleItems });
  const hidden = hiddenCount({ showAll: props.showAll, visibleItems: props.visibleItems, limited: limited.length });
  const noTarget = hasNoTargetItems(props);
  const noVisible = hasNoVisible(props);
  return (
    <div className="items" role="group" aria-label={`${props.target.targetName} items`}>
      <div className="scan-items-guide" aria-hidden="true" />
      <EmptyRow show={noTarget} text="No items." />
      <EmptyRow show={noVisible} text="No items shown for current filters." />
      {limited.map((item) => <ScanItemRow key={item.id} item={item} target={props.target} allowDeleteUnknown={props.allowDeleteUnknown} minAgeHours={props.minAgeHours} selected={props.selectedIds.has(item.id)} inspected={props.focusedItemId === item.id} cleaning={props.cleaning} onToggle={() => props.onToggleItem(item)} />)}
      <ShowMoreBlock hidden={hidden} onClick={props.onToggleShowAll} total={props.visibleItems.length} />
      <ShowFewerBlock show={showFewer({ showAll: props.showAll, visible: props.visibleItems.length })} onClick={props.onToggleShowAll} />
    </div>
  );
}

export function ScanTargetCard(props: Props) {
  return (
    <article className={cardClass(props.collapsed)} aria-labelledby={`target-${props.target.targetId}-name`}>
      <ScanGroupRow target={props.target} allowDeleteUnknown={props.allowDeleteUnknown} selectedIds={props.selectedIds} collapsed={props.collapsed} groupActive={props.groupActive} cleaning={props.cleaning} onFocusGroup={props.onFocusGroup} onToggleCollapsed={props.onToggleCollapsed} onToggleGroupSelect={props.onToggleGroupSelect} onOpenSettings={props.onOpenSettings} />
      <ItemsBody {...props} />
    </article>
  );
}
