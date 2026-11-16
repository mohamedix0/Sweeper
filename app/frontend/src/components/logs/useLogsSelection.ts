import type { SweeperClient } from '../../api/client';
import { formatGroupBytes, groupSkippedRows } from './useLogsSelectionHelpers';
import { useLogsSelectionModel } from './useLogsSelectionModel';

export function useLogsSelection(args: { client: SweeperClient; focusLogId?: string | null; onFocusConsumed?: () => void }) {
  return useLogsSelectionModel(args);
}
export { formatGroupBytes, groupSkippedRows };
