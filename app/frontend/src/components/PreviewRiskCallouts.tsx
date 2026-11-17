import {
  calloutCopyFor,
  PREVIEW_RISK_CALLOUT_STRINGS,
  type PreviewCalloutRisk,
} from './previewRiskCalloutStrings';

export type { PreviewCalloutRisk } from './previewRiskCalloutStrings';

/** Identity and existing Knowledge Base explanation for one selected Preview item. */
export type PreviewRiskCalloutItem = {
  id: string;
  identity: string;
  risk: PreviewCalloutRisk;
  explanation: string;
};

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
