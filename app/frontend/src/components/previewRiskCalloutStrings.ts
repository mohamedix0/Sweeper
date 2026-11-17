/**
 * Presentation-only copy for the Preview per-item Risk callouts.
 *
 * All user-facing strings for the callouts live here so they can be localised
 * in one place later. v1 strings remain English and use the canonical Sweeper
 * Risk vocabulary from GLOSSARY.md (Safe / Caution / Unknown / Protected) plus
 * the canonical Custom Path term.
 */

export type PreviewCalloutRisk = 'Safe' | 'Caution' | 'Unknown' | 'Protected' | 'Custom Path';

export type CalloutCopy = {
  /** Text shown on the Risk badge (also the non-colour-only risk signal). */
  badge: string;
  /** The single plain-language consequence sentence for this risk. */
  consequence: string;
  /** Existing Risk design-token class applied to the badge. */
  badgeClass: string;
  /** Existing Risk design-token class applied to the callout surface. */
  surfaceClass: string;
};

export const PREVIEW_RISK_CALLOUT_STRINGS = {
  /** Accessible name of the callout group. */
  groupLabel: 'Elevated risk in this selection',
  /** Visible heading above the callout list. */
  heading: 'Review elevated risk before confirming',
  /** Visible, non-colour-only warning marker on every callout. */
  warningLabel: 'Warning',
  /** Visible label introducing the item identity. */
  identityLabel: 'Item',
  /** Accessible name template for an individual callout. */
  accessibleName: (args: { riskLabel: string; identity: string }): string =>
    `${args.riskLabel} risk: ${args.identity}`,
} as const;

const CAUTION_COPY: CalloutCopy = {
  badge: 'Caution',
  consequence: 'Removing this item is permanent and it may be slow or costly to restore.',
  badgeClass: 'risk-Caution',
  surfaceClass: 'preview-risk-callout--caution',
};

const UNKNOWN_COPY: CalloutCopy = {
  badge: 'Unknown',
  consequence: 'Sweeper does not recognise this item; removing it permanently may delete data it cannot verify.',
  badgeClass: 'risk-Unknown',
  surfaceClass: 'preview-risk-callout--unknown',
};

const CUSTOM_PATH_COPY: CalloutCopy = {
  badge: 'Custom Path',
  consequence: 'You added this path; only its contents will be removed, permanently.',
  badgeClass: 'risk-CustomPath',
  surfaceClass: 'preview-risk-callout--custom',
};

/**
 * Total Risk -> callout copy mapping. `Safe` and `Protected` have no callout,
 * so they map to null. `Protected` items are non-selectable upstream and never
 * reach the callout list.
 */
export function calloutCopyFor(risk: PreviewCalloutRisk): CalloutCopy | null {
  if (risk === 'Caution') return CAUTION_COPY;
  if (risk === 'Unknown') return UNKNOWN_COPY;
  if (risk === 'Custom Path') return CUSTOM_PATH_COPY;
  return null;
}
