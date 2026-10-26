//! The online-change compatibility report (CLI §1.5).
//!
//! The report is derived from the candidate manifest's plan and history and
//! the baseline manifest, so `stc build`, `stc online diff`, and the watch
//! deploy path all describe a candidate the same way.

use stc_codegen::Json;
use stc_got::{IdentityKind, Manifest, PlanAction, PlanVerdict};

use crate::report::{Log, Reporter};

/// The `stc build`/`stc online` compatibility summary.
pub(crate) struct CompatibilityReport {
    /// `compatible`, `incompatible`, or `cold`.
    pub(crate) verdict: &'static str,
    /// Baseline build ID, when a baseline was used.
    pub(crate) baseline_build_id: Option<String>,
    /// Candidate build ID.
    pub(crate) candidate_build_id: String,
    /// Preserved (including migrated) records.
    pub(crate) preserved: usize,
    /// Newly allocated or reset records.
    pub(crate) allocated: usize,
    /// Dropped roots.
    pub(crate) dropped: usize,
    /// Newly retired records relative to the baseline.
    pub(crate) retired: usize,
    /// Rejected records.
    pub(crate) rejected: usize,
    /// Non-fatal plan diagnostics.
    pub(crate) warnings: Vec<(String, String)>,
    /// Rejection diagnostics.
    pub(crate) rejections: Vec<(String, String)>,
    /// Per-record actions, printed with `-v`.
    pub(crate) actions: Vec<(String, String, String)>,
}

impl CompatibilityReport {
    /// Builds a report from a candidate manifest and an optional baseline.
    #[must_use]
    pub(crate) fn new(candidate: &Manifest, baseline: Option<&Manifest>) -> Self {
        let verdict = match candidate.plan.verdict {
            PlanVerdict::Cold => "cold",
            PlanVerdict::OnlineCandidate => "compatible",
            PlanVerdict::Rejected => "incompatible",
        };
        let mut preserved = 0;
        let mut allocated = 0;
        let mut dropped = 0;
        let mut rejected = 0;
        let mut actions = Vec::new();
        for action in &candidate.plan.actions {
            match action.action {
                PlanAction::Preserve | PlanAction::Migrate => preserved += 1,
                PlanAction::Allocate | PlanAction::Reset => allocated += 1,
                PlanAction::Drop => dropped += 1,
                PlanAction::Reject => rejected += 1,
            }
            actions.push((
                action.got_id.to_hex(),
                identity_kind_name(action.kind),
                action.key.clone(),
            ));
        }
        let baseline_retired = baseline.map_or(0, |baseline| baseline.history.retired.len());
        let retired = candidate
            .history
            .retired
            .len()
            .saturating_sub(baseline_retired);
        let mut warnings = Vec::new();
        let mut rejections = Vec::new();
        for diagnostic in &candidate.plan.diagnostics {
            let entry = (diagnostic.code.clone(), diagnostic.message.clone());
            if diagnostic.severity >= 2 {
                rejections.push(entry);
            } else {
                warnings.push(entry);
            }
        }
        Self {
            verdict,
            baseline_build_id: baseline
                .map(|baseline| baseline.header.build_id)
                .or(candidate.plan.parent_build_id)
                .map(stc_got::GotId::to_hex),
            candidate_build_id: candidate.header.build_id.to_hex(),
            preserved,
            allocated,
            dropped,
            retired,
            rejected,
            warnings,
            rejections,
            actions,
        }
    }

    /// Returns `true` when the verdict permits an online change.
    #[must_use]
    pub(crate) fn is_compatible(&self) -> bool {
        self.verdict == "compatible" && self.rejections.is_empty() && self.rejected == 0
    }

    /// Projects the report to the documented JSON object.
    #[must_use]
    pub(crate) fn to_json(&self, verbose: bool) -> Json {
        let mut fields = vec![
            ("verdict".into(), Json::String(self.verdict.into())),
            (
                "baseline_build_id".into(),
                self.baseline_build_id
                    .clone()
                    .map_or(Json::Null, Json::String),
            ),
            (
                "candidate_build_id".into(),
                Json::String(self.candidate_build_id.clone()),
            ),
            ("counts".into(), self.counts_json()),
            ("warnings".into(), diagnostics_json(&self.warnings)),
            ("rejections".into(), diagnostics_json(&self.rejections)),
        ];
        if verbose {
            fields.push((
                "actions".into(),
                Json::Array(
                    self.actions
                        .iter()
                        .map(|(got_id, kind, key)| {
                            Json::object([
                                ("got_id".into(), Json::String(got_id.clone())),
                                ("kind".into(), Json::String(kind.clone())),
                                ("key".into(), Json::String(key.clone())),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        Json::object(fields)
    }

    fn counts_json(&self) -> Json {
        Json::object([
            (
                "preserved".into(),
                Json::Number(i128::try_from(self.preserved).unwrap_or(i128::MAX)),
            ),
            (
                "allocated".into(),
                Json::Number(i128::try_from(self.allocated).unwrap_or(i128::MAX)),
            ),
            (
                "dropped".into(),
                Json::Number(i128::try_from(self.dropped).unwrap_or(i128::MAX)),
            ),
            (
                "retired".into(),
                Json::Number(i128::try_from(self.retired).unwrap_or(i128::MAX)),
            ),
            (
                "rejected".into(),
                Json::Number(i128::try_from(self.rejected).unwrap_or(i128::MAX)),
            ),
        ])
    }

    /// Prints the report in the selected format.
    pub(crate) fn print(&self, reporter: &Reporter, log: Log) {
        if reporter.is_json() {
            println!("{}", self.to_json(log.verbose()).render());
            return;
        }
        let baseline = self.baseline_build_id.as_deref().unwrap_or("<none>");
        log.status(format!(
            "compatibility {}: baseline {baseline} candidate {}",
            self.verdict, self.candidate_build_id
        ));
        log.status(format!(
            "  preserved {}  allocated {}  dropped {}  retired {}  rejected {}",
            self.preserved, self.allocated, self.dropped, self.retired, self.rejected
        ));
        for (code, message) in &self.warnings {
            log.status(format!("  warning[{code}]: {message}"));
        }
        for (code, message) in &self.rejections {
            log.status(format!("  rejection[{code}]: {message}"));
        }
        if log.verbose() {
            for (got_id, kind, key) in &self.actions {
                log.info(format!("  action {kind} {key} {got_id}"));
            }
        }
    }
}

fn identity_kind_name(tag: u8) -> String {
    IdentityKind::from_tag(tag).map_or_else(
        || format!("kind-{tag}"),
        |kind| kind.name().replace(' ', "-"),
    )
}

fn diagnostics_json(entries: &[(String, String)]) -> Json {
    Json::Array(
        entries
            .iter()
            .map(|(code, message)| {
                Json::object([
                    ("code".into(), Json::String(code.clone())),
                    ("message".into(), Json::String(message.clone())),
                ])
            })
            .collect(),
    )
}
