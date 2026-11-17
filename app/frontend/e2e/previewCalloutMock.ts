import type { Page } from '@playwright/test';

/**
 * Test-only wrapper around the existing browser mock backend.
 *
 * The mock Scan dataset contains only Safe targets plus the Unknown Custom Path
 * target, so an E2E test cannot otherwise exercise an elevated-Risk callout in
 * the Scan -> Preview journey. This wrapper stubs only the Knowledge Base
 * explanation (explainItem) for one selected item, presenting it as Caution in
 * exactly the way a real Caution Target would. Scan results, preview token,
 * typed confirmation, breakdown and every other backend behavior stay on the
 * real mock backend. No production code is changed.
 */
export type PreviewCalloutMockConfig = {
  /** Scan item id whose explanation should be reported as Caution. */
  elevatedItemId: string;
  /** Identity shown in the callout for the elevated item. */
  elevatedTitle: string;
  /** Knowledge Base explanation shown in the callout. */
  elevatedDescription: string;
  /** Optional skipped count returned by Preview so the skipped notice renders. */
  skippedCount?: string;
  /** Optional skipped reasons returned by Preview. */
  skippedReasons?: string[];
};

type Explanation = { itemId: string; title: string; description: string; risk: string };
type PreviewResultLike = {
  previewToken: string;
  breakdown: unknown[];
  totalSelectedCount: string;
  totalSelectedBytes: string;
  skippedCount?: string;
  skippedReasons?: string[];
};
type MockBackend = {
  explainItem: (itemId: string) => Promise<Explanation> | Explanation;
  previewClean: (
    sessionId: string,
    itemIds: string[],
    confirmations?: unknown,
  ) => Promise<PreviewResultLike>;
};

export async function installPreviewCalloutMock(page: Page, config: PreviewCalloutMockConfig): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { explainItem?: unknown } }).__sweeperMock;
    return typeof mock?.explainItem === 'function';
  });

  await page.evaluate((cfg) => {
    const mock = (window as unknown as { __sweeperMock: MockBackend }).__sweeperMock;

    const originalExplain = mock.explainItem.bind(mock);
    mock.explainItem = async (itemId: string): Promise<Explanation> => {
      if (itemId === cfg.elevatedItemId) {
        return {
          itemId,
          title: cfg.elevatedTitle,
          description: cfg.elevatedDescription,
          risk: 'Caution',
        };
      }
      return originalExplain(itemId);
    };

    const hasSkips = cfg.skippedCount != null || (cfg.skippedReasons?.length ?? 0) > 0;
    if (hasSkips) {
      const originalPreview = mock.previewClean.bind(mock);
      mock.previewClean = async (sessionId, itemIds, confirmations) => {
        const result = await originalPreview(sessionId, itemIds, confirmations);
        return {
          ...result,
          skippedCount: cfg.skippedCount ?? result.skippedCount,
          skippedReasons: cfg.skippedReasons ?? result.skippedReasons,
        };
      };
    }
  }, config);
}
