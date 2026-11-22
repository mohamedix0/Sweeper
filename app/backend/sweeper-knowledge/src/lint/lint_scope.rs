//! Single responsibility: validate an allowlist target's tier, risk and scope combination.
//!
//! In particular, issue #41 forbids per-file age-based deletion of package
//! contents: a `whole_cache` target must have `min_age_hours = 0`. All lint
//! messages are verbatim so lint verdicts are unchanged by this extraction.

use super::{terr, LintError, LintTarget};

/// Validate tier, risk and scope together before path-based checks run.
pub(super) fn validate_tier_risk_and_scope(t: &LintTarget) -> Result<(), LintError> {
    match t.tier.as_str() {
        "Tier1" | "Tier2" | "Tier3" => {}
        _ => return Err(terr(&t.id, "tier must be Tier1, Tier2, or Tier3")),
    }
    match t.risk.as_str() {
        "Safe" | "Caution" | "Unknown" | "Protected" => {}
        _ => {
            return Err(terr(
                &t.id,
                "risk must be Safe, Caution, Unknown, or Protected",
            ))
        }
    }
    validate_scope_and_age(t)
}

/// Validate the scope and its age semantics. Whole-cache targets are removed
/// as one unit; per-file age-based deletion of package contents is prohibited
/// (issue #41), so no minimum age may be configured for them.
fn validate_scope_and_age(t: &LintTarget) -> Result<(), LintError> {
    match t.scope.as_str() {
        "contents" | "whole_cache" => {}
        other => return Err(terr(&t.id, format!("invalid scope `{other}`"))),
    }
    if is_whole_cache_with_age_filter(t) {
        return Err(terr(
            &t.id,
            "whole_cache scope forbids per-file age-based deletion (min_age_hours must be 0)",
        ));
    }
    Ok(())
}

/// Predicate for the forbidden whole-cache + age-filter combination.
fn is_whole_cache_with_age_filter(t: &LintTarget) -> bool {
    t.scope == "whole_cache" && t.min_age_hours != 0
}
