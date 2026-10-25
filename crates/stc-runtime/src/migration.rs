//! Online-change plan validation against the active baseline.
//!
//! The compiler's plan is a candidate, not an authorization (GOT Artifacts
//! §10). The host applies an online plan only when the plan's parent is the
//! exact active manifest and every `Preserve` record still matches its
//! baseline shape. This module performs that validation; phase 6 wires it to
//! the active-manifest store and the module loader.

use std::collections::{BTreeMap, BTreeSet};

use stc_got::catalog::root_kind_identity;
use stc_got::error::{Error, Result, codes};
use stc_got::keys::IdentityKind;
use stc_got::{
    ActionRecord, DescriptorRecord, ExportRecord, FunctionRecord, GotId, InstanceRecord, Manifest,
    PlanAction, PlanVerdict, RootKind, RootRecord, Severity, TaskRecord, TypeRecord,
};

/// The host's disposition for a validated plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanDecision {
    /// No usable baseline; the host must cold-download.
    Cold,
    /// The plan names the exact active parent; an online attempt is permitted.
    Online,
}

/// Validates a manifest as a deployable candidate.
///
/// The host runs this before either a cold or an online load. A rejected
/// verdict or an error diagnostic makes the artifacts undeployable regardless
/// of the active baseline.
///
/// # Errors
///
/// Rejects a malformed manifest, a rejected plan, or an error diagnostic.
pub fn validate_deployable(manifest: &Manifest) -> Result<()> {
    manifest.validate()?;
    if manifest.plan.verdict == PlanVerdict::Rejected {
        return Err(Error::plan(
            "the compiler rejected this candidate; a rejected plan is never applied",
        ));
    }
    if manifest
        .plan
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error.tag())
    {
        return Err(Error::plan(
            "the candidate carries an error diagnostic and must not be applied",
        ));
    }
    Ok(())
}

/// Validates a manifest's online-change plan against an optional baseline.
///
/// A cold plan never requires a baseline. An online-candidate plan requires the
/// exact baseline named by its parent fields, compatible preserved shapes, and
/// full retirement coverage. A rejected plan is terminal and never applied.
///
/// # Errors
///
/// Rejects a malformed plan, a rejected plan, a missing or stale baseline, a
/// preserve action whose shape changed, an allocate action that reuses a
/// baseline identity, and a removed identity without a retirement action.
pub fn validate_plan(manifest: &Manifest, baseline: Option<&Manifest>) -> Result<PlanDecision> {
    validate_deployable(manifest)?;
    if manifest.plan.verdict == PlanVerdict::Cold {
        return Ok(PlanDecision::Cold);
    }

    let baseline = baseline.ok_or_else(|| {
        Error::plan("an online-candidate plan requires the active baseline manifest")
    })?;
    baseline.validate()?;
    validate_headers(manifest, baseline)?;
    validate_parent(manifest, baseline)?;

    let current = Index::new(manifest);
    let previous = Index::new(baseline);
    validate_history_consistency(manifest, &current)?;
    validate_history_consistency(baseline, &previous)?;
    let actions: BTreeMap<GotId, &ActionRecord> = manifest
        .plan
        .actions
        .iter()
        .map(|action| (action.got_id, action))
        .collect();
    for action in &manifest.plan.actions {
        validate_action(action, &current, &previous)?;
    }
    validate_retirement_coverage(&current, &previous, &actions)?;
    Ok(PlanDecision::Online)
}

fn validate_headers(candidate: &Manifest, baseline: &Manifest) -> Result<()> {
    let candidate = &candidate.header;
    let baseline = &baseline.header;
    let equal = candidate.profile == baseline.profile
        && candidate.target_triple == baseline.target_triple
        && candidate.endianness == baseline.endianness
        && candidate.pointer_width == baseline.pointer_width
        && candidate.module_format == baseline.module_format
        && candidate.host_abi_major == baseline.host_abi_major;
    if !equal {
        return Err(Error::plan(
            "the candidate profile, target, layout, or ABI major does not match the baseline",
        ));
    }
    if candidate.host_abi_minor < baseline.host_abi_minor {
        return Err(Error::plan(
            "the candidate requires an older host ABI minor than its baseline",
        ));
    }
    Ok(())
}

fn validate_parent(candidate: &Manifest, baseline: &Manifest) -> Result<()> {
    let parent_build_id = baseline.header.build_id;
    let parent_manifest_digest = baseline.digest()?;
    if candidate.header.parent_build_id != Some(parent_build_id) {
        return Err(Error::plan(format!(
            "the plan parent build ID {:?} is not the active build ID {parent_build_id}",
            candidate.header.parent_build_id
        )));
    }
    if candidate.header.parent_manifest_digest != Some(parent_manifest_digest) {
        return Err(Error::plan(format!(
            "the plan parent manifest digest {:?} is not the active manifest digest {parent_manifest_digest}",
            candidate.header.parent_manifest_digest
        )));
    }
    Ok(())
}

fn validate_action(action: &ActionRecord, current: &Index<'_>, previous: &Index<'_>) -> Result<()> {
    let kind = IdentityKind::from_tag(action.kind).ok_or_else(|| {
        Error::plan(format!(
            "plan action for {} has invalid identity kind {}",
            action.got_id, action.kind
        ))
    })?;
    match action.action {
        PlanAction::Preserve => {
            if kind == IdentityKind::Direct {
                return Err(Error::plan(format!(
                    "direct-variable identity `{}` is per generation and cannot be preserved",
                    action.key
                )));
            }
            let Some(previous_record) = previous.get(action.got_id) else {
                return Err(Error::plan(format!(
                    "preserve action for `{}` has no baseline record",
                    action.key
                )));
            };
            let Some(current_record) = current.get(action.got_id) else {
                return Err(Error::plan(format!(
                    "preserve action for `{}` has no active record",
                    action.key
                )));
            };
            ensure_compatible(previous_record, current_record)
        }
        PlanAction::Allocate => {
            if kind == IdentityKind::Direct {
                // Direct cells are rebuilt for every generation; an address
                // remap is reported as STC-W-ONLINE-003 by the compiler.
                return Ok(());
            }
            if previous.retired.contains(&action.got_id) {
                return Err(Error::plan(format!(
                    "allocate action for `{}` reintroduces an identity retired by the baseline",
                    action.key
                )));
            }
            if previous.get(action.got_id).is_some() {
                return Err(Error::plan(format!(
                    "allocate action for `{}` already exists in the baseline",
                    action.key
                )));
            }
            if previous.keys.contains_key(&(kind, action.key.as_str())) {
                return Err(Error::plan(format!(
                    "allocate action for `{}` reuses a baseline key with a new identity",
                    action.key
                )));
            }
            Ok(())
        }
        PlanAction::Drop => {
            let Some(previous_record) = previous.get(action.got_id) else {
                return Err(Error::plan(format!(
                    "drop action for `{}` has no baseline record",
                    action.key
                )));
            };
            if previous_record.kind() != kind {
                return Err(Error::plan(format!(
                    "drop action for `{}` does not match the baseline identity kind",
                    action.key
                )));
            }
            match kind {
                IdentityKind::Direct => Err(Error::plan(format!(
                    "direct-variable identity `{}` is never dropped",
                    action.key
                ))),
                IdentityKind::Export => Err(Error::new(
                    codes::ONLINE_EXPORT,
                    format!(
                        "export `{}` was removed; export removal is a rejection",
                        action.key
                    ),
                )),
                _ => Ok(()),
            }
        }
        PlanAction::Migrate | PlanAction::Reset => Err(Error::plan(format!(
            "`{}` action for `{}` is not supported by M4.5",
            action.action.name(),
            action.key
        ))),
        PlanAction::Reject => Err(Error::plan(format!(
            "rejected action for `{}`; the candidate cannot be applied",
            action.key
        ))),
    }
}

fn ensure_compatible(previous: RecordRef<'_>, current: RecordRef<'_>) -> Result<()> {
    match (previous, current) {
        (RecordRef::Root(previous), RecordRef::Root(current)) => compare_roots(previous, current),
        (RecordRef::Function(previous), RecordRef::Function(current)) => {
            compare_functions(previous, current)
        }
        (RecordRef::Type(previous), RecordRef::Type(current)) => compare_types(previous, current),
        (RecordRef::Descriptor(previous), RecordRef::Descriptor(current)) => {
            compare_descriptors(previous, current)
        }
        (RecordRef::Instance(previous), RecordRef::Instance(current)) => {
            compare_instances(previous, current)
        }
        (RecordRef::Task(previous), RecordRef::Task(current)) => compare_tasks(previous, current),
        (RecordRef::Export(previous), RecordRef::Export(current)) => {
            compare_exports(previous, current)
        }
        _ => Err(Error::plan("a preserve action changes the identity kind")),
    }
}

fn compare_roots(previous: &RootRecord, current: &RootRecord) -> Result<()> {
    if previous.root_kind == RootKind::Direct || current.root_kind == RootKind::Direct {
        return Err(Error::plan(format!(
            "direct-variable identity `{}` cannot be preserved",
            current.key
        )));
    }
    if previous.root_kind != current.root_kind
        || previous.type_key != current.type_key
        || previous.layout_hash != current.layout_hash
        || previous.size != current.size
        || previous.align != current.align
    {
        return Err(Error::new(
            codes::ONLINE_LAYOUT,
            format!(
                "root `{}` no longer matches the baseline type/layout/size/alignment",
                current.key
            ),
        ));
    }
    Ok(())
}

fn compare_functions(previous: &FunctionRecord, current: &FunctionRecord) -> Result<()> {
    if previous.signature_key != current.signature_key {
        return Err(Error::new(
            codes::ONLINE_EXPORT,
            format!(
                "callable `{}` changed its signature from `{}` to `{}`",
                current.key, previous.signature_key, current.signature_key
            ),
        ));
    }
    Ok(())
}

fn compare_types(previous: &TypeRecord, current: &TypeRecord) -> Result<()> {
    if previous.vtable_shape != current.vtable_shape {
        return Err(Error::new(
            codes::ONLINE_VTABLE,
            format!("type `{}` changed its vtable shape", current.key),
        ));
    }
    if previous.type_key != current.type_key
        || previous.layout_hash != current.layout_hash
        || previous.size != current.size
        || previous.align != current.align
        || previous.base != current.base
        || previous.interfaces != current.interfaces
    {
        return Err(Error::new(
            codes::ONLINE_LAYOUT,
            format!(
                "type `{}` no longer matches the baseline layout",
                current.key
            ),
        ));
    }
    Ok(())
}

fn compare_descriptors(previous: &DescriptorRecord, current: &DescriptorRecord) -> Result<()> {
    if previous.descriptor_kind != current.descriptor_kind
        || previous.owner_type != current.owner_type
    {
        return Err(Error::new(
            codes::ONLINE_VTABLE,
            format!(
                "descriptor `{}` changed its owner or descriptor kind",
                current.key
            ),
        ));
    }
    if previous.type_key != current.type_key
        || previous.layout_hash != current.layout_hash
        || previous.size != current.size
        || previous.align != current.align
    {
        return Err(Error::new(
            codes::ONLINE_LAYOUT,
            format!(
                "descriptor `{}` no longer matches the baseline layout",
                current.key
            ),
        ));
    }
    Ok(())
}

fn compare_instances(previous: &InstanceRecord, current: &InstanceRecord) -> Result<()> {
    if previous.program_type != current.program_type
        || previous.program_type_key != current.program_type_key
        || previous.configuration != current.configuration
        || previous.resource != current.resource
        || previous.instance_path != current.instance_path
        || previous.task_key != current.task_key
    {
        return Err(Error::new(
            codes::ONLINE_TOPOLOGY,
            format!("instance `{}` changed its topology", current.key),
        ));
    }
    Ok(())
}

fn compare_tasks(previous: &TaskRecord, current: &TaskRecord) -> Result<()> {
    if previous.configuration != current.configuration
        || previous.resource != current.resource
        || previous.name != current.name
        || previous.single != current.single
        || previous.interval_ns != current.interval_ns
        || previous.priority != current.priority
    {
        return Err(Error::new(
            codes::ONLINE_TOPOLOGY,
            format!("task `{}` changed its topology", current.key),
        ));
    }
    Ok(())
}

fn compare_exports(previous: &ExportRecord, current: &ExportRecord) -> Result<()> {
    if previous.c_name != current.c_name
        || previous.function_got_id != current.function_got_id
        || previous.signature_key != current.signature_key
        || previous.no_mangle != current.no_mangle
    {
        return Err(Error::new(
            codes::ONLINE_EXPORT,
            format!("export `{}` changed its C ABI", current.key),
        ));
    }
    Ok(())
}

fn validate_history_consistency(manifest: &Manifest, index: &Index<'_>) -> Result<()> {
    for record in &manifest.history.retired {
        if index.get(record.got_id).is_some() {
            return Err(Error::plan(format!(
                "retired identity `{}` is also active in the same manifest",
                record.key
            )));
        }
    }
    Ok(())
}

fn validate_retirement_coverage(
    current: &Index<'_>,
    previous: &Index<'_>,
    actions: &BTreeMap<GotId, &ActionRecord>,
) -> Result<()> {
    for (got_id, record) in &previous.records {
        if current.get(*got_id).is_some() {
            continue;
        }
        match record.kind() {
            IdentityKind::Direct => {}
            IdentityKind::Export => {
                return Err(Error::new(
                    codes::ONLINE_EXPORT,
                    format!(
                        "export `{}` was removed; export removal is a rejection",
                        record.key()
                    ),
                ));
            }
            _ => {
                let Some(action) = actions.get(got_id) else {
                    return Err(Error::plan(format!(
                        "baseline identity `{}` was removed without a drop action",
                        record.key()
                    )));
                };
                if action.action != PlanAction::Drop {
                    return Err(Error::plan(format!(
                        "baseline identity `{}` was removed but the plan does not drop it",
                        record.key()
                    )));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum RecordRef<'a> {
    Root(&'a RootRecord),
    Function(&'a FunctionRecord),
    Type(&'a TypeRecord),
    Descriptor(&'a DescriptorRecord),
    Instance(&'a InstanceRecord),
    Task(&'a TaskRecord),
    Export(&'a ExportRecord),
}

impl<'a> RecordRef<'a> {
    fn kind(self) -> IdentityKind {
        match self {
            Self::Root(record) => root_kind_identity(record.root_kind),
            Self::Function(_) => IdentityKind::Function,
            Self::Type(_) => IdentityKind::Type,
            Self::Descriptor(_) => IdentityKind::Descriptor,
            Self::Instance(_) => IdentityKind::Instance,
            Self::Task(_) => IdentityKind::Task,
            Self::Export(_) => IdentityKind::Export,
        }
    }

    fn key(self) -> &'a str {
        match self {
            Self::Root(record) => &record.key,
            Self::Function(record) => &record.key,
            Self::Type(record) => &record.key,
            Self::Descriptor(record) => &record.key,
            Self::Instance(record) => &record.key,
            Self::Task(record) => &record.key,
            Self::Export(record) => &record.key,
        }
    }
}

struct Index<'a> {
    records: BTreeMap<GotId, RecordRef<'a>>,
    keys: BTreeMap<(IdentityKind, &'a str), GotId>,
    retired: BTreeSet<GotId>,
}

impl<'a> Index<'a> {
    fn new(manifest: &'a Manifest) -> Self {
        let mut index = Self {
            records: BTreeMap::new(),
            keys: BTreeMap::new(),
            retired: manifest
                .history
                .retired
                .iter()
                .map(|record| record.got_id)
                .collect(),
        };
        for root in &manifest.roots {
            index.insert(
                root.got_id,
                root_kind_identity(root.root_kind),
                &root.key,
                RecordRef::Root(root),
            );
        }
        for function in &manifest.functions {
            index.insert(
                function.got_id,
                IdentityKind::Function,
                &function.key,
                RecordRef::Function(function),
            );
        }
        for ty in &manifest.types {
            index.insert(ty.got_id, IdentityKind::Type, &ty.key, RecordRef::Type(ty));
        }
        for descriptor in &manifest.descriptors {
            index.insert(
                descriptor.got_id,
                IdentityKind::Descriptor,
                &descriptor.key,
                RecordRef::Descriptor(descriptor),
            );
        }
        for instance in &manifest.instances {
            index.insert(
                instance.got_id,
                IdentityKind::Instance,
                &instance.key,
                RecordRef::Instance(instance),
            );
        }
        for task in &manifest.tasks {
            index.insert(
                task.got_id,
                IdentityKind::Task,
                &task.key,
                RecordRef::Task(task),
            );
        }
        for export in &manifest.exports {
            index.insert(
                export.got_id,
                IdentityKind::Export,
                &export.key,
                RecordRef::Export(export),
            );
        }
        index
    }

    fn insert(&mut self, got_id: GotId, kind: IdentityKind, key: &'a str, record: RecordRef<'a>) {
        self.records.insert(got_id, record);
        self.keys.insert((kind, key), got_id);
    }

    fn get(&self, got_id: GotId) -> Option<RecordRef<'a>> {
        self.records.get(&got_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use stc_got::{PlanDiagnostic, RetiredCause, RetiredRecord};

    use crate::test_support::{self, artifacts_with, artifacts_with_renames, reseal_manifest};

    const COUNTER_SOURCE: &str = r#"
VAR_GLOBAL
  Counter : DINT;
  Limit : INT;
END_VAR

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
  Counter := ticks;
END_PROGRAM
"#;

    fn cold() -> stc_codegen::module::ModuleArtifacts {
        test_support::artifacts(COUNTER_SOURCE, "counter")
    }

    fn code_only_change() -> stc_codegen::module::ModuleArtifacts {
        let source = COUNTER_SOURCE.replace("ticks := ticks + 1;", "ticks := ticks + 2;");
        artifacts_with(&source, "counter", cold().manifest.clone(), false)
    }

    fn remove_limit() -> stc_codegen::module::ModuleArtifacts {
        let source = COUNTER_SOURCE.replace("  Limit : INT;\n", "");
        let artifacts = artifacts_with_renames(
            &source,
            "counter",
            cold().manifest.clone(),
            false,
            Vec::new(),
            true,
        );
        assert!(artifacts.is_deployable());
        // The build above produces the drop plan; keep the manifest addressable
        // for tests that remove or rewrite the action.
        artifacts.manifest.validate().expect("valid drop manifest");
        artifacts
    }

    #[test]
    fn cold_plan_needs_no_baseline() {
        let artifacts = cold();
        assert_eq!(artifacts.verdict(), PlanVerdict::Cold);
        assert_eq!(
            validate_plan(&artifacts.manifest, None).expect("cold"),
            PlanDecision::Cold
        );
    }

    #[test]
    fn code_only_online_plan_validates_against_its_parent() {
        let first = cold();
        let second = code_only_change();
        assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
        let preserves = second
            .manifest
            .plan
            .actions
            .iter()
            .filter(|action| action.action == PlanAction::Preserve)
            .count();
        assert!(preserves > 0, "a code-only change preserves identities");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).expect("online"),
            PlanDecision::Online
        );
    }

    #[test]
    fn online_plan_requires_a_baseline() {
        let second = code_only_change();
        assert_eq!(
            validate_plan(&second.manifest, None).unwrap_err().code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_a_stale_baseline() {
        let first = cold();
        let second = code_only_change();
        let error = validate_plan(&second.manifest, Some(&second.manifest)).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PLAN);
        let other = test_support::artifacts(
            &COUNTER_SOURCE.replace("Limit : INT", "Limit : DINT"),
            "other",
        );
        let error = validate_plan(&second.manifest, Some(&other.manifest)).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PLAN);
        // The exact baseline remains valid after the failed comparisons.
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).unwrap(),
            PlanDecision::Online
        );
    }

    #[test]
    fn rejects_a_rejected_verdict() {
        let first = cold();
        let mut second = code_only_change();
        second.manifest.plan.verdict = PlanVerdict::Rejected;
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_error_diagnostics_in_an_online_plan() {
        let first = cold();
        let mut second = code_only_change();
        second.manifest.plan.diagnostics.push(PlanDiagnostic {
            code: codes::W_ONLINE_REMOVED.into(),
            severity: Severity::Error.tag(),
            message: "tampered severity".into(),
            got_id: None,
            old_key: None,
            new_key: None,
        });
        second
            .manifest
            .plan
            .diagnostics
            .sort_by(stc_got::model::compare_plan_diagnostics);
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_an_allocate_action_for_a_baseline_identity() {
        let first = cold();
        let mut second = code_only_change();
        let action = second
            .manifest
            .plan
            .actions
            .iter_mut()
            .find(|action| action.action == PlanAction::Preserve)
            .expect("preserve action");
        action.action = PlanAction::Allocate;
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_a_preserve_action_with_changed_layout() {
        let first = cold();
        let mut second = code_only_change();
        let action_id = second
            .manifest
            .plan
            .actions
            .iter()
            .find(|action| action.action == PlanAction::Preserve && action.kind == 1)
            .expect("preserved root action")
            .got_id;
        let root = second
            .manifest
            .roots
            .iter_mut()
            .find(|root| root.got_id == action_id)
            .expect("root");
        root.type_key = "LINT".into();
        root.size = 8;
        root.align = 8;
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::ONLINE_LAYOUT
        );
    }

    #[test]
    fn rejects_preserve_for_a_direct_identity() {
        let first = cold();
        let mut second = code_only_change();
        let action = second
            .manifest
            .plan
            .actions
            .iter_mut()
            .find(|action| action.action == PlanAction::Preserve)
            .expect("preserve action");
        action.kind = IdentityKind::Direct.as_u8();
        reseal_manifest(&mut second.manifest).expect("reseal");
        let error = validate_plan(&second.manifest, Some(&first.manifest)).unwrap_err();
        assert!(
            error.code == codes::MANIFEST_PLAN || error.code == codes::ONLINE_LAYOUT,
            "unexpected code {}",
            error.code
        );
    }

    #[test]
    fn rejects_migrate_and_reset_actions() {
        for action_kind in [PlanAction::Migrate, PlanAction::Reset] {
            let first = cold();
            let mut second = code_only_change();
            let action = second
                .manifest
                .plan
                .actions
                .iter_mut()
                .find(|action| action.action == PlanAction::Preserve)
                .expect("preserve action");
            action.action = action_kind;
            reseal_manifest(&mut second.manifest).expect("reseal");
            assert_eq!(
                validate_plan(&second.manifest, Some(&first.manifest))
                    .unwrap_err()
                    .code,
                codes::MANIFEST_PLAN
            );
        }
    }

    #[test]
    fn rejects_a_reject_action_in_an_online_plan() {
        let first = cold();
        let mut second = code_only_change();
        let action = second
            .manifest
            .plan
            .actions
            .iter_mut()
            .find(|action| action.action == PlanAction::Preserve)
            .expect("preserve action");
        action.action = PlanAction::Reject;
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn accepts_a_drop_of_a_removed_root() {
        let first = cold();
        let second = remove_limit();
        assert!(
            second
                .manifest
                .plan
                .actions
                .iter()
                .any(|action| action.action == PlanAction::Drop)
        );
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).expect("online"),
            PlanDecision::Online
        );
    }

    #[test]
    fn rejects_a_removed_root_without_a_drop_action() {
        let first = cold();
        let mut second = remove_limit();
        let removed_key = first
            .manifest
            .roots
            .iter()
            .find(|root| root.key == "_ROOT::LIMIT")
            .expect("limit root")
            .key
            .clone();
        second
            .manifest
            .plan
            .actions
            .retain(|action| action.key != removed_key);
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_a_manifest_that_retires_an_active_identity() {
        let first = cold();
        let mut second = code_only_change();
        let active = second
            .manifest
            .roots
            .iter()
            .find(|root| root.key == "_ROOT::COUNTER")
            .expect("counter root");
        second.manifest.history.retired.push(RetiredRecord {
            got_id: active.got_id,
            kind: IdentityKind::DataRoot.as_u8(),
            key: active.key.clone(),
            previous_keys: Vec::new(),
            retired_build_id: first.manifest.header.build_id,
            retired_manifest_digest: first.manifest.digest().expect("first digest"),
            cause: RetiredCause::Dropped,
        });
        second.manifest.history.retired.sort_by(|left, right| {
            (left.retired_build_id, left.got_id).cmp(&(right.retired_build_id, right.got_id))
        });
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_reintroducing_an_identity_retired_by_the_baseline() {
        let first = cold();
        let removed = remove_limit();
        let retired = removed
            .manifest
            .history
            .retired
            .iter()
            .find(|record| record.key == "_ROOT::LIMIT")
            .expect("retired limit root");

        // Model the previous generation as the exact parent of the original
        // build, with every surviving identity preserved and the removed root
        // allocated again under its retired identity.
        let mut candidate = first;
        candidate.manifest.plan.verdict = PlanVerdict::OnlineCandidate;
        candidate.manifest.header.parent_build_id = Some(removed.manifest.header.build_id);
        candidate.manifest.header.parent_manifest_digest =
            Some(removed.manifest.digest().expect("baseline digest"));
        candidate.manifest.plan.parent_build_id = candidate.manifest.header.parent_build_id;
        candidate.manifest.plan.parent_manifest_digest =
            candidate.manifest.header.parent_manifest_digest;
        for action in &mut candidate.manifest.plan.actions {
            if action.got_id != retired.got_id {
                action.action = PlanAction::Preserve;
            }
        }
        reseal_manifest(&mut candidate.manifest).expect("reseal");

        let error = validate_plan(&candidate.manifest, Some(&removed.manifest)).unwrap_err();
        assert_eq!(error.code, codes::MANIFEST_PLAN);
        assert!(
            error.message.contains("retired"),
            "unexpected message: {}",
            error.message
        );
    }

    #[test]
    fn rejects_dropping_a_direct_identity() {
        let source = r#"
PROGRAM Main
VAR
  input AT %IX1.2 : BOOL;
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;
        let first = test_support::artifacts(source, "direct");
        let mut second =
            test_support::artifacts(&source.replace("  input AT %IX1.2 : BOOL;\n", ""), "direct");
        let direct = first
            .manifest
            .roots
            .iter()
            .find(|root| root.root_kind == RootKind::Direct)
            .expect("direct root");
        second.manifest.plan.actions.push(ActionRecord {
            got_id: direct.got_id,
            kind: IdentityKind::Direct.as_u8(),
            key: direct.key.clone(),
            old_key: None,
            action: PlanAction::Drop,
            reason_code: "drop.tampered".into(),
            detail: "tampered".into(),
        });
        second
            .manifest
            .plan
            .actions
            .sort_by_key(|action| action.got_id);
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn accepts_a_tracked_rename_with_a_preserved_identity() {
        let first = cold();
        let source = COUNTER_SOURCE
            .replace("Counter : DINT", "Count : DINT")
            .replace("Counter := ticks;", "Count := ticks;");
        let second = artifacts_with_renames(
            &source,
            "counter",
            first.manifest.clone(),
            false,
            vec![stc_got::Rename::new("_ROOT::COUNTER", "_ROOT::COUNT")],
            false,
        );
        let counter = first
            .manifest
            .roots
            .iter()
            .find(|root| root.key == "_ROOT::COUNTER")
            .expect("counter root");
        let renamed = second
            .manifest
            .roots
            .iter()
            .find(|root| root.key == "_ROOT::COUNT")
            .expect("renamed root");
        assert_eq!(counter.got_id, renamed.got_id);
        let action = second
            .manifest
            .plan
            .actions
            .iter()
            .find(|action| action.got_id == counter.got_id)
            .expect("rename action");
        assert_eq!(action.action, PlanAction::Preserve);
        assert_eq!(action.old_key.as_deref(), Some("_ROOT::COUNTER"));
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).expect("online"),
            PlanDecision::Online
        );
    }

    #[test]
    fn rejects_a_baseline_with_a_different_target() {
        let first = cold();
        let mut second = code_only_change();
        second.manifest.header.target_triple = "aarch64-unknown-linux-gnu".into();
        reseal_manifest(&mut second.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn root_registry_and_got_substrate_support_preserved_state() {
        use std::collections::BTreeMap;

        use crate::got::Got;
        use crate::memory::RootRegistry;

        let first = cold();
        let second = code_only_change();
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).unwrap(),
            PlanDecision::Online
        );

        let mut roots = RootRegistry::new();
        for root in first
            .manifest
            .roots
            .iter()
            .filter(|root| root.root_kind != RootKind::Direct)
        {
            roots.allocate(root, 1).expect("allocate");
        }
        let counter = first
            .manifest
            .roots
            .iter()
            .find(|root| root.key == "_ROOT::COUNTER")
            .expect("counter");
        roots
            .get_mut(counter.got_id)
            .unwrap()
            .storage_mut()
            .copy_from_slice(&41_u32.to_le_bytes());
        let address = roots.address(counter.got_id).unwrap();

        for action in &second.manifest.plan.actions {
            if action.kind != IdentityKind::DataRoot.as_u8() {
                continue;
            }
            match action.action {
                PlanAction::Preserve => {
                    let root = second
                        .manifest
                        .roots
                        .iter()
                        .find(|root| root.got_id == action.got_id)
                        .expect("preserved root");
                    roots.preserve(root).expect("preserve");
                }
                PlanAction::Allocate => {
                    if let Some(root) = second
                        .manifest
                        .roots
                        .iter()
                        .find(|root| root.got_id == action.got_id)
                        && root.root_kind != RootKind::Direct
                    {
                        roots.allocate(root, 2).expect("allocate");
                    }
                }
                PlanAction::Drop => {
                    let _ = roots.quarantine(action.got_id, stc_got::RetiredCause::Dropped);
                }
                _ => {}
            }
        }

        assert_eq!(roots.address(counter.got_id), Some(address));
        assert_eq!(
            roots.storage(counter.got_id).unwrap(),
            &41_u32.to_le_bytes()
        );

        let mut got = Got::new(&second.manifest.slots).expect("got");
        got.bind_data_roots(&roots).expect("bind roots");
        let functions: BTreeMap<GotId, usize> = second
            .manifest
            .functions
            .iter()
            .map(|function| (function.got_id, 0x1000))
            .collect();
        got.bind_functions(&functions).expect("bind functions");
        got.bind_imports(|service| Some(0x2000 + usize::from(service)))
            .expect("bind imports");
        assert!(got.unbound_slots().is_empty());
    }

    const FUNCTION_SOURCE: &str = r#"
FUNCTION Helper : DINT
VAR_INPUT
  value : DINT;
END_VAR
  Helper := value + 1;
END_FUNCTION

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := Helper(ticks);
END_PROGRAM
"#;

    const FUNCTION_REMOVED: &str = r#"
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

    const EXPORT_SOURCE: &str = r#"
{stc:export}
FUNCTION Read : DINT
  Read := 1;
END_FUNCTION

PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

    const EXPORT_REMOVED: &str = r#"
PROGRAM Main
VAR
  ticks : DINT;
END_VAR
  ticks := ticks + 1;
END_PROGRAM
"#;

    #[test]
    fn rejects_a_removed_function_without_a_drop_action() {
        let first = test_support::artifacts(FUNCTION_SOURCE, "function");
        let second = artifacts_with(FUNCTION_REMOVED, "function", first.manifest.clone(), false);
        assert_eq!(second.verdict(), PlanVerdict::OnlineCandidate);
        assert_eq!(
            validate_plan(&second.manifest, Some(&first.manifest)).expect("online"),
            PlanDecision::Online
        );

        let helper = first
            .manifest
            .functions
            .iter()
            .find(|function| function.key.contains("HELPER"))
            .expect("helper function");
        let mut tampered = second;
        tampered
            .manifest
            .plan
            .actions
            .retain(|action| action.got_id != helper.got_id);
        reseal_manifest(&mut tampered.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&tampered.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::MANIFEST_PLAN
        );
    }

    #[test]
    fn rejects_a_removed_export_even_without_a_plan_action() {
        let first = test_support::artifacts(EXPORT_SOURCE, "export");
        let second = artifacts_with(EXPORT_REMOVED, "export", first.manifest.clone(), false);
        assert_eq!(second.verdict(), PlanVerdict::Rejected);

        // Model a tampered plan that hides the compiler's export rejection.
        let export = first.manifest.exports.first().expect("export record");
        let mut tampered = second;
        tampered.manifest.plan.verdict = PlanVerdict::OnlineCandidate;
        tampered
            .manifest
            .plan
            .actions
            .retain(|action| action.got_id != export.got_id && action.action != PlanAction::Reject);
        tampered
            .manifest
            .plan
            .diagnostics
            .retain(|diagnostic| diagnostic.severity != Severity::Error.tag());
        reseal_manifest(&mut tampered.manifest).expect("reseal");
        assert_eq!(
            validate_plan(&tampered.manifest, Some(&first.manifest))
                .unwrap_err()
                .code,
            codes::ONLINE_EXPORT
        );
    }
}
