import { useRef, useState } from 'react';
import type { CleanProgressEvent, ExecuteCleanResponse, ScanResults } from '../../api/client';
import type { ExplainState, PreviewResult } from '../cleanShared';
import type { FocusState, ScanToolbarState } from './scanHelpers';

export function useScanState() {
  const [tier1, setTier1] = useState(true);
  const [tier2, setTier2] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanStatus, setScanStatus] = useState('');
  const [scanProgress, setScanProgress] = useState<{ completedTargets: number; totalTargets: number; currentTargetName: string | null; scannedItems: string; scannedBytes: string } | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [scanStartedAt, setScanStartedAt] = useState<number | null>(null);
  const [results, setResults] = useState<ScanResults | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [collapsedTargets, setCollapsedTargets] = useState<Set<string>>(new Set());
  const [toolbar, setToolbar] = useState<ScanToolbarState>({ query: '', sort: 'size', filter: 'all' });
  const [focus, setFocus] = useState<FocusState>(null);
  const [inspected, setInspected] = useState<ExplainState>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [confirmText, setConfirmText] = useState('');
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<CleanProgressEvent | null>(null);
  const [cleanSummary, setCleanSummary] = useState<ExecuteCleanResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [relaunching, setRelaunching] = useState(false);
  const [relaunchInfo, setRelaunchInfo] = useState<string | null>(null);
  const [relaunchError, setRelaunchError] = useState<string | null>(null);
  const [showAllGroups, setShowAllGroups] = useState<Set<string>>(new Set());
  const [scanSettingsVersion, setScanSettingsVersion] = useState<number | null>(null);
  const [, setTick] = useState(0);
  const previewRevisionRef = useRef(0);
  const activePreviewRequestRef = useRef(0);
  const inspectReqIdRef = useRef(0);
  const activeScanRunRef = useRef(0);
  return {
    tier1, setTier1, tier2, setTier2, scanning, setScanning, scanStatus, setScanStatus, scanProgress, setScanProgress,
    sessionId, setSessionId, scanStartedAt, setScanStartedAt, results, setResults, selectedIds, setSelectedIds,
    collapsedTargets, setCollapsedTargets, toolbar, setToolbar, focus, setFocus, inspected, setInspected,
    preview, setPreview, previewLoading, setPreviewLoading, confirmText, setConfirmText, cleaning, setCleaning,
    cleanProgress, setCleanProgress, cleanSummary, setCleanSummary, error, setError, info, setInfo,
    relaunching, setRelaunching, relaunchInfo, setRelaunchInfo, relaunchError, setRelaunchError,
    showAllGroups, setShowAllGroups, scanSettingsVersion, setScanSettingsVersion, setTick,
    previewRevisionRef, activePreviewRequestRef, inspectReqIdRef, activeScanRunRef,
  };
}
