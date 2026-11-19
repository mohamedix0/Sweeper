import type { Page } from '@playwright/test';

/**
 * Test-only wrapper around the existing browser mock backend.
 *
 * The genuine Caution Risk item now lives in the mock Knowledge Base
 * (`CAUTION_VENDOR_LOGS` in src/api/mock/data.ts), so the callout E2E selects it
 * through the real Scan -> Preview journey with no `explainItem` override. This
 * helper stubs only the optional skipped-preview metadata so the skipped notice
 * can be exercised alongside the callout. Scan results, explanations, preview
 * token, typed confirmation and breakdown stay on the real mock backend.
 */
export type PreviewSkippedMockConfig = {
  /** Optional skipped count returned by Preview so the skipped notice renders. */
  skippedCount?: string;
  /** Optional skipped reasons returned by Preview. */
  skippedReasons?: string[];
};

type PreviewResultLike = {
  previewToken: string;
  breakdown: unknown[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  skippedCount?: string;
  skippedReasons?: string[];
};
type MockBackend = {
  previewClean: (
    sessionId: string,
    itemIds: string[],
    confirmations?: unknown,
  ) => Promise<PreviewResultLike>;
};

export async function installPreviewSkippedMock(page: Page, config: PreviewSkippedMockConfig): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { previewClean?: unknown } }).__sweeperMock;
    return typeof mock?.previewClean === 'function';
  });

  await page.evaluate((cfg) => {
    const mock = (window as unknown as { __sweeperMock: MockBackend }).__sweeperMock;
    const originalPreview = mock.previewClean.bind(mock);
    mock.previewClean = async (sessionId, itemIds, confirmations) => {
      const result = await originalPreview(sessionId, itemIds, confirmations);
      return {
        ...result,
        skippedCount: cfg.skippedCount ?? result.skippedCount,
        skippedReasons: cfg.skippedReasons ?? result.skippedReasons,
      };
    };
  }, config);
}
