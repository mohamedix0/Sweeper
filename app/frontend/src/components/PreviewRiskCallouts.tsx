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
 * Derive the callout items for the item currently inspected in a panel.
 *
 * The item identity, Risk and Knowledge Base explanation all come from the
 * existing `explainItem` result carried by `ExplainState`. Callers set
 * `isCustomPath` from the item's owning target so a Custom Path selection is
 * surfaced with the canonical Custom Path Risk. Safe and Protected items have
 * no callout copy and therefore produce no entry.
 *
 * This is a pure, per-render derivation: when the inspected selection changes
 * the caller recomputes it, so the rendered callout list follows the selection.
 */
export function deriveCalloutItemsFromInspection(args: {
  inspected: ExplainState;
  isCustomPath?: boolean;
}): PreviewRiskCalloutItem[] {
  const { inspected } = args;
  if (!inspected) return [];
  const risk: PreviewCalloutRisk = args.isCustomPath ? 'Custom Path' : inspected.risk;
  if (calloutCopyFor(risk) === null) return [];
  return [{ id: inspected.itemId, identity: inspected.title, risk, explanation: inspected.description }];
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
