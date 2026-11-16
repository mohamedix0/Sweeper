import type { AnalyzerNodeDto } from '../../contract/bindings';
import type { AnalyzerStatusFilter, Pill } from './analyzerHelpersTypes';
import { isDriveRoot, isVhdxPath } from './analyzerHelpersUtils';

type NodeArg = { node: AnalyzerNodeDto };
type SelectableArg = NodeArg & { allowDeleteUnknown: boolean };
type EligibleArg = NodeArg & { selectable: boolean };
type GuidanceArg = EligibleArg & { allowDeleteUnknown: boolean };
type StatusText = { text: string; locked: boolean; unknownGated: boolean };

function blockedByFlags(args: NodeArg): boolean {
  if (args.node.displayOnly) return true;
  if (args.node.isLinkPoint) return true;
  return args.node.isCloudPlaceholder;
}

function unknownBlockedForSelect(args: SelectableArg): boolean {
  if (args.node.risk !== 'Unknown') return false;
  return !args.allowDeleteUnknown;
}

export function isNodeSelectable(args: SelectableArg): boolean {
  if (blockedByFlags(args)) return false;
  if (args.node.risk === 'Protected') return false;
  if (isVhdxPath({ path: args.node.path })) return false;
  if (unknownBlockedForSelect(args)) return false;
  return Boolean(args.node.selectable);
}

function tierEligible(args: { tier: AnalyzerNodeDto['tier'] }): boolean {
  if (args.tier === 'Tier1') return true;
  if (args.tier === 'Tier2') return true;
  return args.tier === 'Tier3';
}

export function isCleanableEligible(args: EligibleArg): boolean {
  if (!args.selectable) return false;
  return tierEligible({ tier: args.node.tier });
}

const PILL_PROTECTED = { label: 'Protected', className: 'status-pill status-pill-protected' };
const PILL_READONLY = { label: 'Read-Only', className: 'status-pill status-pill-readonly' };
const PILL_DRIVE = { label: 'Drive Root', className: 'status-pill status-pill-readonly' };
const PILL_UNKNOWN = { label: 'Unknown', className: 'status-pill status-pill-unknown' };
const PILL_CAUTION = { label: 'Caution', className: 'status-pill status-pill-caution' };
const PILL_SAFE = { label: 'Safe', className: 'status-pill status-pill-safe' };

function readOnlyStatus(args: NodeArg): boolean {
  if (isVhdxPath({ path: args.node.path })) return true;
  if (args.node.isLinkPoint) return true;
  if (args.node.isCloudPlaceholder) return true;
  return Boolean(args.node.displayOnly);
}

function pillForRisk(args: NodeArg): Pill {
  if (args.node.risk === 'Protected') return PILL_PROTECTED;
  if (args.node.risk === 'Unknown') return PILL_UNKNOWN;
  if (args.node.risk === 'Caution') return PILL_CAUTION;
  return null;
}

function pillForLocked(args: NodeArg): Pill {
  if (isDriveRoot(args)) return PILL_DRIVE;
  if (readOnlyStatus(args)) return PILL_READONLY;
  return pillForRisk(args);
}

function pillForSelectable(args: EligibleArg): Pill {
  if (isCleanableEligible(args)) return PILL_SAFE;
  if (args.selectable) return PILL_SAFE;
  return null;
}

export function analyzerStatusPill(args: EligibleArg): Pill {
  const locked = pillForLocked(args);
  if (locked) return locked;
  return pillForSelectable(args);
}

function lockedStatusKey(args: NodeArg): boolean {
  if (isDriveRoot(args)) return true;
  if (args.node.risk === 'Protected') return true;
  return readOnlyStatus(args);
}

export function analyzerStatusKey(args: NodeArg): AnalyzerStatusFilter {
  if (lockedStatusKey(args)) return 'locked';
  if (args.node.risk === 'Unknown') return 'unknown';
  if (args.node.risk === 'Caution') return 'caution';
  if (args.node.risk === 'Safe') return 'safe';
  return 'locked';
}

function displayOnlyReason(args: NodeArg): string {
  if (isDriveRoot(args)) return 'Drive roots cannot be cleaned as a whole. Expand to select specific folders.';
  return 'Display-only. Not selectable.';
}

function lockedUnknown(args: SelectableArg): StatusText | null {
  if (args.node.risk !== 'Unknown') return null;
  if (args.allowDeleteUnknown) return null;
  return { text: 'Unknown gated by Settings (Allow deleting unknown folders).', locked: true, unknownGated: true };
}

function flagLockedText(args: SelectableArg): string | null {
  if (args.node.risk === 'Protected') return 'Protected. Not selectable.';
  if (args.node.isLinkPoint) return 'Link Point. Not selectable.';
  if (args.node.isCloudPlaceholder) return 'Cloud placeholder (Skipped). Not selectable.';
  if (isVhdxPath({ path: args.node.path })) return 'Read-only virtual disk. Not selectable.';
  return null;
}

function otherLockedText(args: SelectableArg): string {
  if (args.node.displayOnly) return displayOnlyReason(args);
  if (args.node.reason) return `${args.node.reason}. Not selectable.`;
  return 'Not selectable.';
}

function lockedText(args: SelectableArg): string {
  const text = flagLockedText(args);
  if (text) return text;
  return otherLockedText(args);
}

function lockedReason(args: SelectableArg): StatusText {
  const unknown = lockedUnknown(args);
  if (unknown) return unknown;
  return { text: lockedText(args), locked: true, unknownGated: false };
}

function confirmText(args: NodeArg): string {
  if (args.node.risk === 'Unknown') return 'Selectable (typed confirmation required)';
  if (!args.node.targetId) return 'Selectable (typed confirmation required)';
  return 'Meets age limit (> 24h old)';
}

export function analyzerStatusText(args: GuidanceArg): StatusText {
  if (args.selectable) return { text: confirmText(args), locked: false, unknownGated: false };
  return lockedReason(args);
}

function unknownGuidance(args: { allowDeleteUnknown: boolean }): string {
  if (!args.allowDeleteUnknown) return 'Unknown folder is locked by Settings. Enable "Allow deleting unknown folders" to allow cleanup after typed confirmation.';
  return 'Unknown folder. Review carefully; typed DELETE confirmation is required before cleaning.';
}

function specialGuidanceText(args: SelectableArg): string | null {
  if (isVhdxPath({ path: args.node.path })) return 'Virtual disk (WSL/Hyper-V). Read-only in Sweeper — compaction is manual and safe to do when the VM/WSL is shut down.';
  if (args.node.risk === 'Protected') return 'Protected system location. Sweeper never removes protected paths.';
  if (args.node.isCloudPlaceholder) return 'Cloud placeholder file. Read-only in Analyzer; content may be remote and is not removable here.';
  if (args.node.isLinkPoint) return 'Link point (symlink/junction). Read-only in Analyzer to avoid traversing or removing linked targets.';
  return null;
}

function lockedGuidanceText(args: SelectableArg): string | null {
  const special = specialGuidanceText(args);
  if (special) return special;
  if (args.node.risk === 'Unknown') return unknownGuidance({ allowDeleteUnknown: args.allowDeleteUnknown });
  if (isDriveRoot(args)) return 'Drive roots cannot be cleaned as a whole. Expand C: to select specific folders.';
  return null;
}

function eligibleGuidance(args: EligibleArg): string {
  if (isCleanableEligible(args)) return 'Eligible cache/build artifact under current settings. Selected items are permanently deleted after review and confirmation.';
  return 'Eligible under current settings. Selected items are permanently deleted after review and confirmation.';
}

export function analyzerGuidance(args: GuidanceArg): string {
  const locked = lockedGuidanceText(args);
  if (locked) return locked;
  if (args.selectable) return eligibleGuidance(args);
  return 'Display-only item. It cannot be selected for cleanup in Analyzer.';
}
