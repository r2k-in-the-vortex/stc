//! Manifest-only compatibility comparison for `stc online diff`.
//!
//! This entry point never loads or executes a module: it validates the
//! candidate manifest, runs the same plan validation the host applies, and
//! returns the comparison data (verdict, decision, error, actions, and
//! diagnostics).

use stc_got::Manifest;

use crate::host::ManifestDiff;
use crate::migration::validate_plan;

/// Compares a candidate manifest with an optional baseline.
///
/// # Errors
///
/// Rejects a malformed candidate manifest. A plan rejection is reported in
/// [`ManifestDiff::error`] so the caller can print the compatibility report.
pub fn compare_manifests(
    candidate: &Manifest,
    baseline: Option<&Manifest>,
) -> stc_got::error::Result<ManifestDiff> {
    candidate.validate()?;
    let decision = validate_plan(candidate, baseline);
    let (decision, error) = match decision {
        Ok(decision) => (Some(decision), None),
        Err(error) => (None, Some(error)),
    };
    Ok(ManifestDiff {
        baseline_build_id: baseline.map(|manifest| manifest.header.build_id),
        candidate_build_id: candidate.header.build_id,
        verdict: candidate.plan.verdict,
        decision,
        error,
        actions: candidate.plan.actions.clone(),
        diagnostics: candidate.plan.diagnostics.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support;

    #[test]
    fn compares_a_candidate_with_its_baseline_without_loading_a_module() {
        let first = test_support::artifacts(test_support::SIMPLE, "simple");
        let second = test_support::artifacts_with(
            test_support::SIMPLE_ALTERNATE,
            "simple",
            first.manifest.clone(),
            false,
        );
        let baseline = first.manifest.clone();
        let candidate = second.manifest.clone();
        let diff = compare_manifests(&candidate, Some(&baseline)).expect("compare");
        assert_eq!(diff.baseline_build_id, Some(baseline.header.build_id));
        assert_eq!(diff.candidate_build_id, candidate.header.build_id);
        assert_eq!(diff.verdict, candidate.plan.verdict);
        assert!(diff.error.is_none(), "{:?}", diff.error);
        assert_eq!(diff.decision, Some(crate::migration::PlanDecision::Online));
    }
}
