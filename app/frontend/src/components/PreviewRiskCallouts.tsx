import {
  calloutCopyFor,
  PREVIEW_RISK_CALLOUT_STRINGS,
  type PreviewCalloutRisk,
} from './previewRiskCalloutStrings';
import type { ExplainState } from './cleanSharedCore';

export type { PreviewCalloutRisk } from './previewRiskCalloutStrings';

/** Identity and existing Knowledge Base explanation for one selected Preview item. */
export type PreviewRiskCalloutItem = {
  id: string;
  identity: string;
  risk: PreviewCalloutRisk;
  explanation: string;
};

/**
 * An explanation for one item that may be part of the Preview selection.
 *
 * It is the existing `explainItem` result (`ExplainState`, which already carries
 * the Knowledge Base description and canonical Risk) plus the owning target id
 * so a Custom Path target can be promoted to the canonical Custom Path Risk.
 */
export type PreviewRiskCalloutSource = NonNullable<ExplainState> & {
  targetId?: string | null;
};

/**
 * Derive one callout per elevated-Risk item in the FULL selected set.
 *
 * The derivation is driven by `selectedIds`, never by the single inspected
 * item: a source is emitted only when its id is selected, so an inspected but
 * unselected item never produces a callout, and every selected elevated-Risk
 * item produces exactly one. Safe and Protected items have no callout copy and
 * are skipped. A Custom Path target is promoted to the canonical Custom Path
 * Risk via `isCustomPath`.
 *
 * This is a pure, per-render derivation: when the selection or the available
 * explanations change the caller recomputes it, so the list follows the
 * selection. No state is held and no IPC is issued here.
 */
export function deriveCalloutItemsFromSelection(args: {
  selectedIds: ReadonlySet<string>;
  sources: readonly PreviewRiskCalloutSource[];
  isCustomPath?: (targetId: string) => boolean;
}): PreviewRiskCalloutItem[] {
  const items: PreviewRiskCalloutItem[] = [];
  for (const source of args.sources) {
    if (!args.selectedIds.has(source.itemId)) continue;
    const isCustomPath = Boolean(source.targetId && args.isCustomPath?.(source.targetId));
    const risk: PreviewCalloutRisk = isCustomPath ? 'Custom Path' : source.risk;
    if (calloutCopyFor(risk) === null) continue;
    items.push({
      id: source.itemId,
      identity: source.title,
      risk,
      explanation: source.description,
    });
  }
  return items;
}

/**
 * Pure presentation of one callout per elevated-Risk item in the current
 * Preview selection. Renders nothing for a Safe-only selection and never
 * presents Protected items. Holds no state and issues no IPC.
 */
export function PreviewRiskCallouts(props: { items: PreviewRiskCalloutItem[] }) {
  const elevated = props.items.filter((item) => calloutCopyFor(item.risk) !== null);
  if (elevated.length === 0) return null;

  return (
    <section
      className="preview-risk-callouts"
      role="group"
      aria-label={PREVIEW_RISK_CALLOUT_STRINGS.groupLabel}
    >
      <p className="preview-risk-callouts-heading subheading-xs">{PREVIEW_RISK_CALLOUT_STRINGS.heading}</p>
      <ul className="preview-risk-callouts-list">
        {elevated.map((item) => {
          const copy = calloutCopyFor(item.risk)!;
          return (
            <li
              key={item.id}
              className={`preview-risk-callout ${copy.surfaceClass}`}
              role="note"
              aria-label={PREVIEW_RISK_CALLOUT_STRINGS.accessibleName({ riskLabel: copy.badge, identity: item.identity })}
              tabIndex={0}
            >
              <div className="preview-risk-callout-head">
                <span className={`badge preview-risk-callout-badge ${copy.badgeClass}`}>{copy.badge}</span>
                <span className="preview-risk-callout-warning">{PREVIEW_RISK_CALLOUT_STRINGS.warningLabel}</span>
              </div>
              <div className="preview-risk-callout-identity">
                <span className="preview-risk-callout-identity-label">{PREVIEW_RISK_CALLOUT_STRINGS.identityLabel}:</span>{' '}
                <span className="preview-risk-callout-identity-text">{item.identity}</span>
              </div>
              <p className="preview-risk-callout-explanation">{item.explanation}</p>
              <p className="preview-risk-callout-consequence">{copy.consequence}</p>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
