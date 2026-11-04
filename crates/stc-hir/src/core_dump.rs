//! Canonical, versioned JSON dump of the core HIR subset (P0.6).
//!
//! The dump is the input format of the Lean reference interpreter. It is
//! deliberately separate from [`crate::dump_hir`]: the debug text dump exposes
//! arena indices for humans, while this module emits semantic identities
//! (canonical uppercase names, declaration positions, GOT root keys) and a
//! feature tag for every construct outside the frozen core subset.
//!
//! The exact schema and versioning policy are normative in
//! `specification/Verification/HIR-Dump.md`. Unknown versions and unmodeled
//! features are rejected, never interpreted.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use stc_span::{FileId, SourceMap, Span};

use crate::hir::{
    ArgDirection, ArgHir, CalleeHir, CaseLabelHir, ConstValue, DefKind, ExprHir, ExprKindHir,
    HirCrate, LiteralHir, PlaceHir, StdFnKind, StmtKindHir, VarSectionKind,
};
use crate::id::{BodyId, DefId, ExprId, FieldId, LocalId, ParamId, StmtId, TyId};
use crate::ty::{CharTy, Ty};

/// Dump format family name.
pub const CORE_DUMP_FORMAT: &str = "stc-core-hir";
/// Frozen dump format version.
pub const CORE_DUMP_FORMAT_VERSION: &str = "0.1.0";
/// Semantics version recorded in every dump.
pub const CORE_DUMP_SEMANTICS_VERSION: &str = "1.0";
/// Conformance profile recorded in every dump.
pub const CORE_DUMP_PROFILE: &str = "stc-2013-oop-utf8-v1";

const MAX_VALIDATE_DEPTH: u32 = 512;

/// The stable header fields read back by the schema validator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreDumpSummary {
    /// Dump format family (`stc-core-hir`).
    pub format: String,
    /// Frozen dump format version.
    pub format_version: String,
    /// Semantics version the dump was produced against.
    pub semantics_version: String,
    /// Conformance profile.
    pub profile: String,
    /// Root package name.
    pub package: String,
    /// `ok` or `rejected`.
    pub status: String,
    /// Feature tags that caused rejection, in canonical order.
    pub features: Vec<String>,
}

impl CoreDumpSummary {
    /// Returns `true` when the dump has no out-of-core feature tags.
    #[must_use]
    pub fn accepted(&self) -> bool {
        self.features.is_empty()
    }
}

/// Renders the canonical core-HIR JSON dump for `hir`.
///
/// The returned text is one JSON object followed by a newline. It is
/// deterministic for a fixed HIR arena and source map.
#[must_use]
pub fn dump_core_hir(hir: &HirCrate, sources: &SourceMap) -> String {
    let value = Emitter::new(hir, sources).build();
    let mut text = serde_json::to_string(&value).expect("HIR dump is serializable");
    text.push('\n');
    text
}

/// Validates a core-HIR dump without executing it.
///
/// The validator checks the format family, the exact dump and semantics
/// versions, the profile, the complete node schema, declaration-scoped
/// variable positions, and feature/status consistency. A well-formed dump
/// with feature tags is returned as a rejected summary; it is never accepted.
///
/// # Errors
///
/// Returns a description when the text is not JSON, a required field is
/// missing or malformed, or the format/version/profile is unknown.
pub fn verify_core_dump(text: &str) -> Result<CoreDumpSummary, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    let root = expect_object(&value, "dump")?;
    let format = string_field(root, "format")?;
    if format != CORE_DUMP_FORMAT {
        return Err(format!(
            "unknown HIR dump format `{format}` (expected `{CORE_DUMP_FORMAT}`)"
        ));
    }
    let format_version = string_field(root, "formatVersion")?;
    if format_version != CORE_DUMP_FORMAT_VERSION {
        return Err(format!(
            "unknown HIR dump format version `{format_version}` (expected \
             `{CORE_DUMP_FORMAT_VERSION}`)"
        ));
    }
    let semantics_version = string_field(root, "semanticsVersion")?;
    if semantics_version != CORE_DUMP_SEMANTICS_VERSION {
        return Err(format!(
            "unknown semantics version `{semantics_version}` (expected \
             `{CORE_DUMP_SEMANTICS_VERSION}`)"
        ));
    }
    let profile = string_field(root, "profile")?;
    if profile != CORE_DUMP_PROFILE {
        return Err(format!(
            "unknown conformance profile `{profile}` (expected `{CORE_DUMP_PROFILE}`)"
        ));
    }
    let package = string_field(root, "package")?.to_owned();
    let status = string_field(root, "status")?;
    if status != "ok" && status != "rejected" {
        return Err(format!(
            "unknown dump status `{status}` (expected `ok` or `rejected`)"
        ));
    }

    let mut feature_tags = Vec::new();
    for (index, feature) in array_field(root, "features")?.iter().enumerate() {
        let feature = expect_object(feature, &format!("features[{index}]"))?;
        feature_tags.push(string_field(feature, "tag")?.to_owned());
        let _ = string_field(feature, "detail")?;
        let at = field(feature, "at")?;
        if !at.is_null() {
            check_span(at, &format!("features[{index}].at"))?;
        }
    }
    let expected_status = if feature_tags.is_empty() {
        "ok"
    } else {
        "rejected"
    };
    if status != expected_status {
        return Err(format!(
            "dump status `{status}` does not match its {} feature tag(s) (expected `{expected_status}`)",
            feature_tags.len()
        ));
    }

    let unit = expect_object(field(root, "unit")?, "unit")?;
    let globals = array_field(unit, "globals")?;
    for (index, global) in globals.iter().enumerate() {
        check_global(global, globals.len(), &format!("unit.globals[{index}]"))?;
    }
    let functions = array_field(unit, "functions")?;
    let mut function_names = BTreeSet::new();
    for (index, function) in functions.iter().enumerate() {
        let name = check_function(function, globals.len(), &format!("unit.functions[{index}]"))?;
        if !function_names.insert(name.clone()) {
            return Err(format!("duplicate function name `{name}` in the unit"));
        }
    }
    let program = field(unit, "program")?;
    if program.is_null() {
        if status == "ok" {
            return Err("an accepted core unit must contain exactly one program".into());
        }
    } else {
        check_program(program, globals.len(), "unit.program")?;
    }
    check_span(field(unit, "span")?, "unit.span")?;

    let mut global_names = BTreeSet::new();
    for (index, global) in globals.iter().enumerate() {
        let name = string_field(
            expect_object(global, &format!("unit.globals[{index}]"))?,
            "name",
        )?;
        if !global_names.insert(name.to_owned()) {
            return Err(format!("duplicate global name `{name}` in the unit"));
        }
    }

    if status == "ok" {
        check_accepted_dump(unit)?;
    }

    Ok(CoreDumpSummary {
        format: format.to_owned(),
        format_version: format_version.to_owned(),
        semantics_version: semantics_version.to_owned(),
        profile: profile.to_owned(),
        package,
        status: status.to_owned(),
        features: feature_tags,
    })
}

/// Resolves every function-call target name in an accepted unit.
fn check_accepted_dump(unit: &Map<String, Value>) -> Result<(), String> {
    let function_names = array_field(unit, "functions")?
        .iter()
        .map(|function| {
            let function = expect_object(function, "function")?;
            Ok(string_field(function, "name")?.to_owned())
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    if let Some(program) = unit.get("program")
        && !program.is_null()
    {
        let program = expect_object(program, "unit.program")?;
        check_calls_in_stmts(array_field(program, "body")?, &function_names, 1)?;
        for (index, local) in array_field(program, "locals")?.iter().enumerate() {
            let local = expect_object(local, &format!("program.locals[{index}]"))?;
            if let Some(init) = local.get("init")
                && !init.is_null()
            {
                check_calls_in_expr(init, &function_names, 1)?;
            }
        }
    }
    for (index, function) in array_field(unit, "functions")?.iter().enumerate() {
        let function = expect_object(function, &format!("function[{index}]"))?;
        check_calls_in_stmts(array_field(function, "body")?, &function_names, 1)?;
        for (local_index, local) in array_field(function, "locals")?.iter().enumerate() {
            let local = expect_object(local, &format!("function[{index}].locals[{local_index}]"))?;
            if let Some(init) = local.get("init")
                && !init.is_null()
            {
                check_calls_in_expr(init, &function_names, 1)?;
            }
        }
    }
    for (index, global) in array_field(unit, "globals")?.iter().enumerate() {
        let global = expect_object(global, &format!("global[{index}]"))?;
        if let Some(init) = global.get("init")
            && !init.is_null()
        {
            check_calls_in_expr(init, &function_names, 1)?;
        }
    }
    Ok(())
}

fn check_calls_in_expr(
    value: &Value,
    functions: &BTreeSet<String>,
    depth: u32,
) -> Result<(), String> {
    check_depth(depth, "expression")?;
    let object = expect_object(value, "expression")?;
    match string_field(object, "kind")? {
        "literal" | "var" => Ok(()),
        "unary" => check_calls_in_expr(field(object, "operand")?, functions, depth + 1),
        "binary" => {
            check_calls_in_expr(field(object, "lhs")?, functions, depth + 1)?;
            check_calls_in_expr(field(object, "rhs")?, functions, depth + 1)
        }
        "call" => {
            let callee = expect_object(field(object, "callee")?, "callee")?;
            if string_field(callee, "kind")? == "function" {
                let name = string_field(callee, "name")?;
                if !functions.contains(name) {
                    return Err(format!("call target `{name}` is not declared in the dump"));
                }
            }
            for (index, argument) in array_field(object, "args")?.iter().enumerate() {
                check_calls_in_expr(argument, functions, depth + 1)
                    .map_err(|error| format!("args[{index}]: {error}"))?;
            }
            Ok(())
        }
        "cast" => check_calls_in_expr(field(object, "operand")?, functions, depth + 1),
        "unsupported" => Ok(()),
        other => Err(format!("unknown expression kind `{other}`")),
    }
}

fn check_calls_in_stmts(
    stmts: &[Value],
    functions: &BTreeSet<String>,
    depth: u32,
) -> Result<(), String> {
    for (index, stmt) in stmts.iter().enumerate() {
        let object = expect_object(stmt, "statement")?;
        let result = match string_field(object, "kind")? {
            "assign" => check_calls_in_expr(field(object, "value")?, functions, depth + 1),
            "call" => check_calls_in_expr(field(object, "call")?, functions, depth + 1),
            "return" | "exit" | "continue" | "empty" | "unsupported" => Ok(()),
            "if" => {
                for arm in array_field(object, "arms")? {
                    let arm = expect_object(arm, "if arm")?;
                    check_calls_in_expr(field(arm, "cond")?, functions, depth + 1)?;
                    check_calls_in_stmts(array_field(arm, "body")?, functions, depth + 1)?;
                }
                check_calls_in_stmts(array_field(object, "else")?, functions, depth + 1)
            }
            "case" => {
                check_calls_in_expr(field(object, "selector")?, functions, depth + 1)?;
                for arm in array_field(object, "arms")? {
                    let arm = expect_object(arm, "case arm")?;
                    check_calls_in_stmts(array_field(arm, "body")?, functions, depth + 1)?;
                }
                check_calls_in_stmts(array_field(object, "else")?, functions, depth + 1)
            }
            "for" => {
                check_calls_in_expr(field(object, "start")?, functions, depth + 1)?;
                check_calls_in_expr(field(object, "stop")?, functions, depth + 1)?;
                if let Some(step) = object.get("step")
                    && !step.is_null()
                {
                    check_calls_in_expr(step, functions, depth + 1)?;
                }
                check_calls_in_stmts(array_field(object, "body")?, functions, depth + 1)
            }
            "while" => {
                check_calls_in_expr(field(object, "cond")?, functions, depth + 1)?;
                check_calls_in_stmts(array_field(object, "body")?, functions, depth + 1)
            }
            "repeat" => {
                check_calls_in_stmts(array_field(object, "body")?, functions, depth + 1)?;
                check_calls_in_expr(field(object, "cond")?, functions, depth + 1)
            }
            other => Err(format!("unknown statement kind `{other}`")),
        };
        result.map_err(|error| format!("statements[{index}]: {error}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FeatureSpan {
    span: Option<Span>,
}

#[derive(Clone)]
struct FunctionInfo {
    id: DefId,
    name: String,
    result: Option<TyId>,
    params: Vec<ParamId>,
    body: Option<BodyId>,
    span: Span,
}

#[derive(Clone)]
struct GlobalInfo {
    id: DefId,
    name: String,
    key: String,
    ty: TyId,
    init: Option<ExprId>,
    span: Span,
}

#[derive(Clone)]
struct ProgramInfo {
    id: DefId,
    name: String,
    fields: Vec<FieldId>,
    body: Option<BodyId>,
    span: Span,
}

#[derive(Clone)]
struct GlobalRef {
    position: usize,
    name: String,
    key: String,
}

struct BodyCtx {
    owner: DefId,
    function: Option<String>,
    params: BTreeMap<ParamId, usize>,
    locals: BTreeMap<LocalId, usize>,
    program_fields: Option<BTreeMap<FieldId, usize>>,
    globals: BTreeMap<DefId, GlobalRef>,
}

impl BodyCtx {
    fn for_global(owner: DefId, globals: BTreeMap<DefId, GlobalRef>) -> Self {
        Self {
            owner,
            function: None,
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            program_fields: None,
            globals,
        }
    }
}

struct Emitter<'a> {
    hir: &'a HirCrate,
    sources: &'a SourceMap,
    package: String,
    features: BTreeMap<(String, u32, u32, u32, String), FeatureSpan>,
    functions: BTreeMap<DefId, FunctionInfo>,
    function_order: Vec<DefId>,
    globals: Vec<GlobalInfo>,
    global_positions: BTreeMap<DefId, usize>,
    program: Option<ProgramInfo>,
}

impl<'a> Emitter<'a> {
    fn new(hir: &'a HirCrate, sources: &'a SourceMap) -> Self {
        Self {
            hir,
            sources,
            package: String::new(),
            features: BTreeMap::new(),
            functions: BTreeMap::new(),
            function_order: Vec::new(),
            globals: Vec::new(),
            global_positions: BTreeMap::new(),
            program: None,
        }
    }

    fn build(mut self) -> Value {
        let hir = self.hir;
        let raw_package = hir
            .packages
            .get(hir.root_package.index())
            .map_or("_root", |package| package.name.as_str());
        self.package = match normalize_package(raw_package) {
            Some(package) => package,
            None => {
                self.feature(
                    "package.invalid",
                    format!("root package name `{raw_package}` is not canonical"),
                    None,
                );
                raw_package.to_ascii_uppercase()
            }
        };
        self.collect();
        let globals = self.lower_globals();
        let functions = self.lower_functions();
        let program = self.lower_program();
        let unit_span = self.unit_span();
        let unit_span_json = self.span_json(unit_span);
        let features = self.features_json();
        let status = if features.is_empty() {
            "ok"
        } else {
            "rejected"
        };
        let feature_values: Vec<Value> = features
            .into_iter()
            .map(|(tag, detail, at)| {
                json!({
                    "tag": tag,
                    "detail": detail,
                    "at": at,
                })
            })
            .collect();
        json!({
            "format": CORE_DUMP_FORMAT,
            "formatVersion": CORE_DUMP_FORMAT_VERSION,
            "semanticsVersion": CORE_DUMP_SEMANTICS_VERSION,
            "profile": CORE_DUMP_PROFILE,
            "package": self.package,
            "unit": {
                "globals": globals,
                "functions": functions,
                "program": program,
                "span": unit_span_json,
            },
            "features": feature_values,
            "status": status,
        })
    }

    fn feature(&mut self, tag: &str, detail: impl Into<String>, span: Option<Span>) {
        let detail = detail.into();
        let (file, start, end) = span.map_or((u32::MAX, 0, 0), |span| {
            (span.file.raw(), span.start, span.end)
        });
        self.features.insert(
            (tag.to_owned(), file, start, end, detail),
            FeatureSpan { span },
        );
    }

    fn features_json(&self) -> Vec<(String, String, Value)> {
        self.features
            .iter()
            .map(|((tag, _, _, _, detail), entry)| {
                let at = entry
                    .span
                    .map_or(Value::Null, |span| self.span_json_raw(span));
                (tag.clone(), detail.clone(), at)
            })
            .collect()
    }

    fn span_json(&mut self, span: Span) -> Value {
        if self.sources.file_path(span.file).is_none() {
            self.feature(
                "span.unknown-file",
                "the span refers to a source file missing from the source map",
                Some(span),
            );
        }
        self.span_json_raw(span)
    }

    fn span_json_raw(&self, span: Span) -> Value {
        json!({
            "file": self.sources.file_path(span.file).unwrap_or(""),
            "start": span.start,
            "end": span.end,
        })
    }

    fn collect(&mut self) {
        let hir = self.hir;
        let root = hir.root_package;
        let mut function_ids = Vec::new();
        let mut global_ids = Vec::new();
        let mut program_ids = Vec::new();
        let mut external_packages = BTreeSet::new();
        for index in 0..hir.defs.len() {
            let id = DefId::new(index as u32);
            let def = &hir.defs[index];
            if def.span.start == 0 && def.span.end == 0 {
                continue;
            }
            if def.package != root {
                external_packages.insert(def.package);
                continue;
            }
            if def.test_only {
                self.feature(
                    "unit.test-only",
                    format!(
                        "definition `{}` is only compiled for tests",
                        hir.def_name(id)
                    ),
                    Some(def.span),
                );
                continue;
            }
            match &def.kind {
                DefKind::Namespace { .. } => {}
                DefKind::Function { .. } => function_ids.push(id),
                DefKind::Global { .. } => global_ids.push(id),
                DefKind::Program { .. } => program_ids.push(id),
                other => self.feature(
                    def_feature_tag(other),
                    "the declaration is outside the frozen core subset",
                    Some(def.span),
                ),
            }
        }
        for package in external_packages {
            let name = hir
                .packages
                .get(package.index())
                .map_or("", |package| package.name.as_str());
            self.feature(
                "unit.external-package",
                format!("package `{name}` is outside the dumped unit"),
                None,
            );
        }

        for id in function_ids {
            let def = &hir.defs[id.index()];
            let (result, params, body) = match &def.kind {
                DefKind::Function {
                    result,
                    params,
                    body,
                } => (*result, params.clone(), *body),
                _ => continue,
            };
            let name = self.canonical_def_name(id);
            self.functions.insert(
                id,
                FunctionInfo {
                    id,
                    name,
                    result,
                    params,
                    body,
                    span: def.span,
                },
            );
        }
        let mut order: Vec<DefId> = self.functions.keys().copied().collect();
        order.sort_by(|first, second| {
            let first = &self.functions[first];
            let second = &self.functions[second];
            first
                .name
                .cmp(&second.name)
                .then_with(|| first.id.raw().cmp(&second.id.raw()))
        });
        let mut duplicates = Vec::new();
        {
            let mut seen = BTreeSet::new();
            for id in &order {
                let info = &self.functions[id];
                if !seen.insert(info.name.clone()) {
                    duplicates.push((info.name.clone(), info.span));
                }
            }
        }
        for (name, span) in duplicates {
            self.feature(
                "function.duplicate-name",
                format!("function name `{name}` is not unique"),
                Some(span),
            );
        }
        self.function_order = order;

        let mut seen_globals = BTreeSet::new();
        for id in global_ids {
            let def = &hir.defs[id.index()];
            let (ty, section, retain, constant, at, io, init) = match &def.kind {
                DefKind::Global {
                    ty,
                    section,
                    retain,
                    constant,
                    at,
                    io,
                    init,
                } => (
                    *ty,
                    *section,
                    *retain,
                    *constant,
                    at.is_some(),
                    io.is_some(),
                    *init,
                ),
                _ => continue,
            };
            if section != VarSectionKind::Global {
                self.feature(
                    "global.section",
                    format!("global section `{}` is outside the core", section.as_str()),
                    Some(def.span),
                );
            }
            if retain != crate::hir::RetainPolicy::None {
                self.feature(
                    "global.retain",
                    "retained globals are outside the core",
                    Some(def.span),
                );
            }
            if constant {
                self.feature(
                    "global.constant",
                    "constant globals are outside the core",
                    Some(def.span),
                );
            }
            if at {
                self.feature(
                    "global.direct-address",
                    "direct-address globals are outside the core",
                    Some(def.span),
                );
            }
            if io {
                self.feature(
                    "global.io",
                    "I/O-bound globals are outside the core",
                    Some(def.span),
                );
            }
            let name = self.canonical_def_name(id);
            if !seen_globals.insert(name.clone()) {
                self.feature(
                    "global.duplicate-name",
                    format!("global name `{name}` is not unique"),
                    Some(def.span),
                );
            }
            let key = got_root_key(&self.package, &name)
                .unwrap_or_else(|| format!("{}::{}", self.package, name));
            self.global_positions.insert(id, self.globals.len());
            self.globals.push(GlobalInfo {
                id,
                name,
                key,
                ty,
                init,
                span: def.span,
            });
        }

        match program_ids.as_slice() {
            [id] => {
                let def = &hir.defs[id.index()];
                let (fields, body, var_access_empty) = match &def.kind {
                    DefKind::Program {
                        fields,
                        body,
                        var_access,
                    } => (fields.clone(), *body, var_access.is_empty()),
                    _ => (Vec::new(), None, true),
                };
                if !var_access_empty {
                    self.feature(
                        "program.var-access",
                        "VAR_ACCESS declarations are outside the core",
                        Some(def.span),
                    );
                }
                let name = self.canonical_def_name(*id);
                self.program = Some(ProgramInfo {
                    id: *id,
                    name,
                    fields,
                    body,
                    span: def.span,
                });
            }
            [] => self.feature(
                "program.missing",
                "the unit has no PROGRAM declaration",
                None,
            ),
            many => self.feature(
                "program.multiple",
                format!("the unit has {} PROGRAM declarations", many.len()),
                None,
            ),
        }
    }

    fn canonical_def_name(&mut self, id: DefId) -> String {
        let hir = self.hir;
        let mut parts = Vec::new();
        let mut current = Some(id);
        let mut first = true;
        while let Some(def_id) = current {
            let def = &hir.defs[def_id.index()];
            if !first && matches!(def.kind, DefKind::Namespace { .. }) && def.parent.is_none() {
                break;
            }
            if let Some(symbol) = def.name {
                let text = hir.interner.text(symbol);
                match normalize_ident(text) {
                    Some(name) => parts.push(name),
                    None => {
                        self.feature(
                            "name.invalid",
                            format!("definition name `{text}` is not canonical"),
                            Some(def.span),
                        );
                        parts.push(sanitize_ident(text));
                    }
                }
            } else {
                self.feature(
                    "def.anonymous",
                    "a definition in the core unit has no name",
                    Some(def.span),
                );
                if first {
                    parts.push("_ANONYMOUS".into());
                } else {
                    break;
                }
            }
            first = false;
            current = def.parent;
            if let Some(parent) = current {
                let parent = &hir.defs[parent.index()];
                if !matches!(parent.kind, DefKind::Namespace { .. }) {
                    self.feature(
                        "name.non-namespace-scope",
                        "a definition is nested in a non-namespace scope",
                        Some(def.span),
                    );
                }
            }
        }
        parts.reverse();
        parts.join(".")
    }

    fn canonical_ident(&mut self, text: &str, span: Span, what: &str) -> String {
        match normalize_ident(text) {
            Some(name) => name,
            None => {
                self.feature(
                    "name.invalid",
                    format!("{what} name `{text}` is not canonical"),
                    Some(span),
                );
                sanitize_ident(text)
            }
        }
    }

    fn global_refs(&self) -> BTreeMap<DefId, GlobalRef> {
        self.globals
            .iter()
            .enumerate()
            .map(|(position, global)| {
                (
                    global.id,
                    GlobalRef {
                        position,
                        name: global.name.clone(),
                        key: global.key.clone(),
                    },
                )
            })
            .collect()
    }

    fn unit_span(&self) -> Span {
        self.program
            .as_ref()
            .map(|program| program.span)
            .or_else(|| {
                self.function_order
                    .first()
                    .map(|id| self.functions[id].span)
            })
            .or_else(|| self.globals.first().map(|global| global.span))
            .unwrap_or_else(|| Span::empty(FileId::new(0), 0))
    }

    fn lower_globals(&mut self) -> Vec<Value> {
        let globals = self.globals.clone();
        let global_refs = self.global_refs();
        globals
            .iter()
            .map(|global| {
                let ctx = BodyCtx::for_global(global.id, global_refs.clone());
                let init = global
                    .init
                    .map_or(Value::Null, |expr| self.lower_expr(expr, &ctx));
                json!({
                    "name": global.name,
                    "gotKey": global.key,
                    "type": self.type_json(global.ty),
                    "init": init,
                    "span": self.span_json(global.span),
                })
            })
            .collect()
    }

    fn lower_functions(&mut self) -> Vec<Value> {
        let infos: Vec<FunctionInfo> = self
            .function_order
            .iter()
            .filter_map(|id| self.functions.get(id).cloned())
            .collect();
        infos.iter().map(|info| self.lower_function(info)).collect()
    }

    fn lower_function(&mut self, info: &FunctionInfo) -> Value {
        let hir = self.hir;
        let global_refs = self.global_refs();
        let body = info.body.map(|body| &hir.bodies[body.index()]);
        let body_locals: Vec<LocalId> = body.map_or_else(Vec::new, |body| body.locals.clone());
        let body_params: Vec<ParamId> = body.map_or_else(Vec::new, |body| body.params.clone());
        if info.body.is_none() {
            self.feature(
                "function.no-body",
                "a core function has no body",
                Some(info.span),
            );
        }
        if let Some(body) = body {
            if body.owner != info.id {
                self.feature(
                    "function.body-owner",
                    "the body owner does not match the function",
                    Some(info.span),
                );
            }
            if !body_params.is_empty() && body_params != info.params {
                self.feature(
                    "function.body-params",
                    "the body parameter list does not match the signature",
                    Some(body.span),
                );
            }
        }
        let ctx = BodyCtx {
            owner: info.id,
            function: Some(info.name.clone()),
            params: info
                .params
                .iter()
                .enumerate()
                .map(|(position, param)| (*param, position))
                .collect(),
            locals: body_locals
                .iter()
                .enumerate()
                .map(|(position, local)| (*local, position))
                .collect(),
            program_fields: None,
            globals: global_refs,
        };

        let mut seen_params = BTreeSet::new();
        let params: Vec<Value> = info
            .params
            .iter()
            .map(|param| {
                let param = hir.param(*param);
                let name =
                    self.canonical_ident(hir.interner.text(param.name), param.span, "parameter");
                if !seen_params.insert(name.clone()) {
                    self.feature(
                        "param.duplicate-name",
                        format!("parameter name `{name}` is not unique"),
                        Some(param.span),
                    );
                }
                if param.owner != info.id {
                    self.feature(
                        "param.owner-mismatch",
                        "the parameter owner does not match the function",
                        Some(param.span),
                    );
                }
                if param.direction != ArgDirection::Input {
                    self.feature(
                        "param.direction",
                        "only VAR_INPUT parameters are in the core",
                        Some(param.span),
                    );
                }
                if param.default.is_some() {
                    self.feature(
                        "param.default",
                        "parameter defaults are outside the core",
                        Some(param.span),
                    );
                }
                json!({
                    "name": name,
                    "type": self.type_json(param.ty),
                    "span": self.span_json(param.span),
                })
            })
            .collect();

        let mut seen_locals = BTreeSet::new();
        let locals: Vec<Value> = body_locals
            .iter()
            .map(|local| {
                let local = hir.local(*local);
                let name = self.canonical_ident(hir.interner.text(local.name), local.span, "local");
                if !seen_locals.insert(name.clone()) {
                    self.feature(
                        "local.duplicate-name",
                        format!("local name `{name}` is not unique"),
                        Some(local.span),
                    );
                }
                if local.owner != info.id {
                    self.feature(
                        "local.owner-mismatch",
                        "the local owner does not match the function",
                        Some(local.span),
                    );
                }
                if local.is_temp {
                    self.feature(
                        "local.temp",
                        "VAR_TEMP locals are outside the core",
                        Some(local.span),
                    );
                }
                if local.retain != crate::hir::RetainPolicy::None {
                    self.feature(
                        "local.retain",
                        "retained locals are outside the core",
                        Some(local.span),
                    );
                }
                if local.external_target.is_some() {
                    self.feature(
                        "local.external",
                        "VAR_EXTERNAL aliases are outside the core",
                        Some(local.span),
                    );
                }
                let init = local
                    .init
                    .map_or(Value::Null, |expr| self.lower_expr(expr, &ctx));
                json!({
                    "name": name,
                    "type": self.type_json(local.ty),
                    "init": init,
                    "span": self.span_json(local.span),
                })
            })
            .collect();

        let body_values = body.map_or_else(Vec::new, |body| self.lower_stmts(&body.stmts, &ctx));
        json!({
            "name": info.name,
            "result": info.result.map_or(Value::Null, |ty| self.type_json(ty)),
            "params": params,
            "locals": locals,
            "body": body_values,
            "span": self.span_json(info.span),
        })
    }

    fn lower_program(&mut self) -> Value {
        let Some(info) = self.program.clone() else {
            return Value::Null;
        };
        let hir = self.hir;
        let global_refs = self.global_refs();
        let body = info.body.map(|body| &hir.bodies[body.index()]);
        if info.body.is_none() {
            self.feature(
                "program.no-body",
                "the core program has no cycle body",
                Some(info.span),
            );
        }
        if let Some(body) = body {
            if body.owner != info.id {
                self.feature(
                    "program.body-owner",
                    "the body owner does not match the program",
                    Some(info.span),
                );
            }
            if !body.params.is_empty() {
                self.feature(
                    "program.body-param",
                    "program bodies do not declare parameters in the core",
                    Some(body.span),
                );
            }
            if !body.locals.is_empty() {
                self.feature(
                    "program.body-local",
                    "program bodies do not declare extra locals in the core",
                    Some(body.span),
                );
            }
        }
        let mut field_positions = BTreeMap::new();
        for (position, field) in info.fields.iter().enumerate() {
            field_positions.insert(*field, position);
        }
        let ctx = BodyCtx {
            owner: info.id,
            function: None,
            params: BTreeMap::new(),
            locals: BTreeMap::new(),
            program_fields: Some(field_positions),
            globals: global_refs,
        };

        let mut seen = BTreeSet::new();
        let locals: Vec<Value> = info
            .fields
            .iter()
            .map(|field| {
                let field = hir.field(*field);
                let name = self.canonical_ident(
                    hir.interner.text(field.name),
                    field.span,
                    "program field",
                );
                if !seen.insert(name.clone()) {
                    self.feature(
                        "program.field-duplicate-name",
                        format!("program field name `{name}` is not unique"),
                        Some(field.span),
                    );
                }
                if field.owner != info.id {
                    self.feature(
                        "program.field-owner",
                        "the field owner does not match the program",
                        Some(field.span),
                    );
                }
                if field.section != VarSectionKind::Var {
                    self.feature(
                        "program.field-section",
                        format!(
                            "program field section `{}` is outside the core",
                            field.section.as_str()
                        ),
                        Some(field.span),
                    );
                }
                if field.constant {
                    self.feature(
                        "program.field-constant",
                        "constant program fields are outside the core",
                        Some(field.span),
                    );
                }
                if field.at.is_some() {
                    self.feature(
                        "program.field-at",
                        "direct-address program fields are outside the core",
                        Some(field.span),
                    );
                }
                if field.io.is_some() {
                    self.feature(
                        "program.field-io",
                        "I/O-bound program fields are outside the core",
                        Some(field.span),
                    );
                }
                if field.retain != crate::hir::RetainPolicy::None {
                    self.feature(
                        "program.field-retain",
                        "retained program fields are outside the core",
                        Some(field.span),
                    );
                }
                let init = field
                    .init
                    .map_or(Value::Null, |expr| self.lower_expr(expr, &ctx));
                json!({
                    "name": name,
                    "type": self.type_json(field.ty),
                    "init": init,
                    "span": self.span_json(field.span),
                })
            })
            .collect();
        let body_values = body.map_or_else(Vec::new, |body| self.lower_stmts(&body.stmts, &ctx));
        json!({
            "name": info.name,
            "locals": locals,
            "body": body_values,
            "span": self.span_json(info.span),
        })
    }

    fn lower_stmts(&mut self, stmts: &[StmtId], ctx: &BodyCtx) -> Vec<Value> {
        stmts
            .iter()
            .map(|stmt| self.lower_stmt(*stmt, ctx))
            .collect()
    }

    fn lower_stmt(&mut self, id: StmtId, ctx: &BodyCtx) -> Value {
        let hir = self.hir;
        let stmt = hir.stmt(id);
        let span = self.span_json(stmt.span);
        match &stmt.kind {
            StmtKindHir::Assign { place, value } => {
                let place = self.lower_place_expr(*place, ctx);
                let value = self.lower_expr(*value, ctx);
                json!({"kind": "assign", "place": place, "value": value, "span": span})
            }
            StmtKindHir::Call { call } => {
                let call = self.lower_expr(*call, ctx);
                json!({"kind": "call", "call": call, "span": span})
            }
            StmtKindHir::Return => json!({"kind": "return", "span": span}),
            StmtKindHir::Exit => json!({"kind": "exit", "span": span}),
            StmtKindHir::Continue => json!({"kind": "continue", "span": span}),
            StmtKindHir::Empty => json!({"kind": "empty", "span": span}),
            StmtKindHir::If { arms, else_body } => {
                let arms: Vec<Value> = arms
                    .iter()
                    .map(|(condition, body)| {
                        let condition = self.lower_expr(*condition, ctx);
                        let body = self.lower_stmts(body, ctx);
                        json!({
                            "cond": condition,
                            "body": body,
                            "span": span,
                        })
                    })
                    .collect();
                let else_body = self.lower_stmts(else_body, ctx);
                json!({
                    "kind": "if",
                    "arms": arms,
                    "else": else_body,
                    "span": span,
                })
            }
            StmtKindHir::Case {
                selector,
                arms,
                else_body,
            } => {
                let selector = self.lower_expr(*selector, ctx);
                let arms: Vec<Value> = arms
                    .iter()
                    .map(|arm| {
                        let labels: Vec<Value> = arm
                            .labels
                            .iter()
                            .map(|label| self.lower_case_label(label))
                            .collect();
                        let body = self.lower_stmts(&arm.body, ctx);
                        json!({
                            "labels": labels,
                            "body": body,
                            "span": span,
                        })
                    })
                    .collect();
                let else_body = self.lower_stmts(else_body, ctx);
                json!({
                    "kind": "case",
                    "selector": selector,
                    "arms": arms,
                    "else": else_body,
                    "span": span,
                })
            }
            StmtKindHir::For {
                var,
                start,
                end,
                step,
                body,
            } => {
                let var = self.lower_place_expr(*var, ctx);
                let start = self.lower_expr(*start, ctx);
                let stop = self.lower_expr(*end, ctx);
                let step = self.lower_expr(*step, ctx);
                let body = self.lower_stmts(body, ctx);
                json!({
                    "kind": "for",
                    "var": var,
                    "start": start,
                    "stop": stop,
                    "step": step,
                    "body": body,
                    "span": span,
                })
            }
            StmtKindHir::While { cond, body } => {
                let cond = self.lower_expr(*cond, ctx);
                let body = self.lower_stmts(body, ctx);
                json!({"kind": "while", "cond": cond, "body": body, "span": span})
            }
            StmtKindHir::Repeat { body, cond } => {
                let body = self.lower_stmts(body, ctx);
                let cond = self.lower_expr(*cond, ctx);
                json!({"kind": "repeat", "body": body, "cond": cond, "span": span})
            }
            StmtKindHir::RefAssign { .. } => {
                self.feature(
                    "stmt.ref-assign",
                    "reference assignment is outside the core",
                    Some(stmt.span),
                );
                self.unsupported("stmt.ref-assign", stmt.span)
            }
            StmtKindHir::Attempt { .. } => {
                self.feature(
                    "stmt.attempt",
                    "assignment attempt is outside the core",
                    Some(stmt.span),
                );
                self.unsupported("stmt.attempt", stmt.span)
            }
        }
    }

    fn lower_case_label(&mut self, label: &CaseLabelHir) -> Value {
        let low = match label_value(&label.lower) {
            Some(value) => value,
            None => {
                self.feature(
                    "case.label",
                    "a CASE label is not an integer or character value",
                    Some(label.span),
                );
                0
            }
        };
        let high = match &label.upper {
            None => Value::Null,
            Some(value) => match label_value(value) {
                Some(value) => int_json(value),
                None => {
                    self.feature(
                        "case.label",
                        "a CASE range bound is not an integer or character value",
                        Some(label.span),
                    );
                    Value::Null
                }
            },
        };
        json!({
            "low": int_json(low),
            "high": high,
            "span": self.span_json(label.span),
        })
    }

    fn lower_place_expr(&mut self, id: ExprId, ctx: &BodyCtx) -> Value {
        let hir = self.hir;
        let expr = hir.expr(id);
        match &expr.kind {
            ExprKindHir::Place(place) => match self.lower_place(place, expr.span, ctx) {
                Ok(place) => place,
                Err(tag) => self.unsupported(tag, expr.span),
            },
            _ => {
                self.feature(
                    "stmt.place-expected",
                    "an assignment target is not a resolved place",
                    Some(expr.span),
                );
                self.unsupported("stmt.place-expected", expr.span)
            }
        }
    }

    fn lower_place(
        &mut self,
        place: &PlaceHir,
        span: Span,
        ctx: &BodyCtx,
    ) -> Result<Value, &'static str> {
        let hir = self.hir;
        match place {
            PlaceHir::Local(id) => {
                let local = hir.local(*id);
                if local.owner != ctx.owner {
                    self.feature(
                        "place.foreign-local",
                        "a local is referenced outside its owning body",
                        Some(span),
                    );
                    return Err("place.foreign-local");
                }
                let Some(&position) = ctx.locals.get(id) else {
                    self.feature(
                        "place.foreign-local",
                        "a local is not part of the owning body's declaration list",
                        Some(span),
                    );
                    return Err("place.foreign-local");
                };
                let name = self.canonical_ident(hir.interner.text(local.name), local.span, "local");
                Ok(json!({
                    "kind": "var",
                    "varKind": "local",
                    "position": position,
                    "name": name,
                    "span": self.span_json(span),
                }))
            }
            PlaceHir::Param(id) => {
                let param = hir.param(*id);
                if param.owner != ctx.owner {
                    self.feature(
                        "place.foreign-param",
                        "a parameter is referenced outside its owning body",
                        Some(span),
                    );
                    return Err("place.foreign-param");
                }
                let Some(&position) = ctx.params.get(id) else {
                    self.feature(
                        "place.foreign-param",
                        "a parameter is not part of the owning body's signature",
                        Some(span),
                    );
                    return Err("place.foreign-param");
                };
                let name =
                    self.canonical_ident(hir.interner.text(param.name), param.span, "parameter");
                Ok(json!({
                    "kind": "var",
                    "varKind": "param",
                    "position": position,
                    "name": name,
                    "span": self.span_json(span),
                }))
            }
            PlaceHir::Global(id) => {
                let Some(global) = ctx.globals.get(id) else {
                    self.feature(
                        "place.foreign-global",
                        "a global is not part of the dumped unit",
                        Some(span),
                    );
                    return Err("place.foreign-global");
                };
                Ok(json!({
                    "kind": "var",
                    "varKind": "global",
                    "position": global.position,
                    "name": global.name,
                    "gotKey": global.key,
                    "span": self.span_json(span),
                }))
            }
            PlaceHir::Result(id) => {
                if *id != ctx.owner || ctx.function.is_none() {
                    self.feature(
                        "place.result",
                        "the result variable is referenced outside its function",
                        Some(span),
                    );
                    return Err("place.result");
                }
                let name = ctx.function.clone().unwrap_or_default();
                Ok(json!({
                    "kind": "var",
                    "varKind": "result",
                    "name": name,
                    "span": self.span_json(span),
                }))
            }
            PlaceHir::This(_) => {
                self.feature(
                    "place.this",
                    "THIS outside a program field access is outside the core",
                    Some(span),
                );
                Err("place.this")
            }
            PlaceHir::Field { base, field } => {
                let base = hir.expr(*base);
                let is_this = matches!(
                    &base.kind,
                    ExprKindHir::Place(PlaceHir::This(owner)) if *owner == ctx.owner
                );
                if is_this
                    && let Some(fields) = &ctx.program_fields
                    && let Some(&position) = fields.get(field)
                {
                    let field = hir.field(*field);
                    let name = self.canonical_ident(
                        hir.interner.text(field.name),
                        field.span,
                        "program field",
                    );
                    return Ok(json!({
                        "kind": "var",
                        "varKind": "local",
                        "position": position,
                        "name": name,
                        "span": self.span_json(span),
                    }));
                }
                self.feature(
                    "place.field",
                    "field, member, or aggregate access is outside the core",
                    Some(span),
                );
                Err("place.field")
            }
            PlaceHir::Index { .. } => {
                self.feature(
                    "place.index",
                    "array or string subscripting is outside the core",
                    Some(span),
                );
                Err("place.index")
            }
            PlaceHir::Deref(_) => {
                self.feature("place.deref", "dereference is outside the core", Some(span));
                Err("place.deref")
            }
            PlaceHir::BitField { .. } => {
                self.feature(
                    "place.bit-field",
                    "partial bit access is outside the core",
                    Some(span),
                );
                Err("place.bit-field")
            }
            PlaceHir::Direct(_) => {
                self.feature(
                    "place.direct",
                    "direct process-image access is outside the core",
                    Some(span),
                );
                Err("place.direct")
            }
            PlaceHir::InstanceField { .. } => {
                self.feature(
                    "place.instance-field",
                    "configured program instance fields are outside the core",
                    Some(span),
                );
                Err("place.instance-field")
            }
        }
    }

    fn lower_expr(&mut self, id: ExprId, ctx: &BodyCtx) -> Value {
        let hir = self.hir;
        let expr = hir.expr(id);
        match &expr.kind {
            ExprKindHir::Literal(literal) => self.lower_literal(expr, literal),
            ExprKindHir::Place(place) => match self.lower_place(place, expr.span, ctx) {
                Ok(place) => place,
                Err(tag) => self.unsupported(tag, expr.span),
            },
            ExprKindHir::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr(*lhs, ctx);
                let rhs = self.lower_expr(*rhs, ctx);
                json!({
                    "kind": "binary",
                    "op": bin_op_name(*op),
                    "lhs": lhs,
                    "rhs": rhs,
                    "type": self.type_json(expr.ty),
                    "span": self.span_json(expr.span),
                })
            }
            ExprKindHir::Unary { op, operand } => {
                let operand = self.lower_expr(*operand, ctx);
                json!({
                    "kind": "unary",
                    "op": un_op_name(*op),
                    "operand": operand,
                    "type": self.type_json(expr.ty),
                    "span": self.span_json(expr.span),
                })
            }
            ExprKindHir::Call { callee, args } => self.lower_call(expr, callee, args, ctx),
            ExprKindHir::Cast { operand, to, kind } => {
                let operand = self.lower_expr(*operand, ctx);
                json!({
                    "kind": "cast",
                    "operand": operand,
                    "to": self.type_json(*to),
                    "castKind": cast_kind_name(*kind),
                    "type": self.type_json(expr.ty),
                    "span": self.span_json(expr.span),
                })
            }
            ExprKindHir::Ref { .. } => {
                self.feature(
                    "expr.ref",
                    "reference construction is outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.ref", expr.span)
            }
            ExprKindHir::AddressOf { .. } => {
                self.feature(
                    "expr.address-of",
                    "ADR is outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.address-of", expr.span)
            }
            ExprKindHir::SizeOf { .. } => {
                self.feature(
                    "expr.size-of",
                    "SIZEOF is outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.size-of", expr.span)
            }
            ExprKindHir::PointerConversion { .. } => {
                self.feature(
                    "expr.pointer-conversion",
                    "pointer conversion is outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.pointer-conversion", expr.span)
            }
            ExprKindHir::Null => {
                self.feature("expr.null", "NULL is outside the core", Some(expr.span));
                self.unsupported("expr.null", expr.span)
            }
            ExprKindHir::ArrayInit { .. } => {
                self.feature(
                    "expr.array-init",
                    "array initializers are outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.array-init", expr.span)
            }
            ExprKindHir::StructInit { .. } => {
                self.feature(
                    "expr.struct-init",
                    "struct initializers are outside the core",
                    Some(expr.span),
                );
                self.unsupported("expr.struct-init", expr.span)
            }
            ExprKindHir::Error => {
                self.feature(
                    "expr.error",
                    "an invalid expression reached the HIR dump",
                    Some(expr.span),
                );
                self.unsupported("expr.error", expr.span)
            }
        }
    }

    fn lower_literal(&mut self, expr: &ExprHir, literal: &LiteralHir) -> Value {
        let ty = self.type_json(expr.ty);
        let payload = match &literal.value {
            ConstValue::Int(value) => Some(*value),
            ConstValue::Bool(value) => Some(i128::from(*value)),
            ConstValue::Char(value) => Some(i128::from(*value)),
            ConstValue::Real(_) => {
                self.feature(
                    "literal.real",
                    "real literals are outside the core",
                    Some(expr.span),
                );
                None
            }
            ConstValue::String(_) => {
                self.feature(
                    "literal.string",
                    "string literals are outside the core",
                    Some(expr.span),
                );
                None
            }
            ConstValue::Time(_) => {
                self.feature(
                    "literal.time",
                    "time literals are outside the core",
                    Some(expr.span),
                );
                None
            }
            ConstValue::Date { .. } => {
                self.feature(
                    "literal.date",
                    "date/time literals are outside the core",
                    Some(expr.span),
                );
                None
            }
            ConstValue::Enum { .. } => {
                self.feature(
                    "literal.enum",
                    "enumeration literals are outside the core",
                    Some(expr.span),
                );
                None
            }
            ConstValue::Null => {
                self.feature(
                    "literal.null",
                    "NULL literals are outside the core",
                    Some(expr.span),
                );
                None
            }
        };
        match payload {
            Some(value) => {
                if let Some(core) = core_type(self.hir.types.get(expr.ty))
                    && !literal_fits(&core, value)
                {
                    self.feature(
                        "literal.out-of-range",
                        format!("literal payload {value} is not representable in its type"),
                        Some(expr.span),
                    );
                }
                json!({
                    "kind": "literal",
                    "type": ty,
                    "value": int_json(value),
                    "span": self.span_json(expr.span),
                })
            }
            None => {
                let tag = match &literal.value {
                    ConstValue::Real(_) => "literal.real",
                    ConstValue::String(_) => "literal.string",
                    ConstValue::Time(_) => "literal.time",
                    ConstValue::Date { .. } => "literal.date",
                    ConstValue::Enum { .. } => "literal.enum",
                    ConstValue::Null => "literal.null",
                    ConstValue::Int(_) | ConstValue::Bool(_) | ConstValue::Char(_) => {
                        "literal.unsupported"
                    }
                };
                self.unsupported(tag, expr.span)
            }
        }
    }

    fn lower_call(
        &mut self,
        expr: &ExprHir,
        callee: &CalleeHir,
        args: &[ArgHir],
        ctx: &BodyCtx,
    ) -> Value {
        let (callee_json, args_json) = match callee {
            CalleeHir::Function(def) => {
                let Some(info) = self.functions.get(def).cloned() else {
                    self.feature(
                        "call.target-missing",
                        format!(
                            "call target `{}` is not in the dumped function list",
                            self.hir.def_name(*def)
                        ),
                        Some(expr.span),
                    );
                    return self.unsupported("call.target-missing", expr.span);
                };
                let Some(ordered) = self.order_function_args(&info, args, expr.span) else {
                    let values: Vec<Value> = args
                        .iter()
                        .filter_map(|argument| argument.value)
                        .map(|value| self.lower_expr(value, ctx))
                        .collect();
                    return json!({
                        "kind": "call",
                        "callee": {
                            "kind": "function",
                            "name": info.name,
                            "span": self.span_json(expr.span),
                        },
                        "args": values,
                        "type": self.type_json(expr.ty),
                        "span": self.span_json(expr.span),
                    });
                };
                let values: Vec<Value> = ordered
                    .into_iter()
                    .map(|value| self.lower_expr(value, ctx))
                    .collect();
                (
                    json!({
                        "kind": "function",
                        "name": info.name,
                        "span": self.span_json(expr.span),
                    }),
                    values,
                )
            }
            CalleeHir::StdFunction(id) => {
                let hir = self.hir;
                let Some(std) = hir.std_fns.get(id.index()) else {
                    self.feature(
                        "call.std-function-missing",
                        "standard-function signature is not in the crate",
                        Some(expr.span),
                    );
                    return self.unsupported("call.std-function-missing", expr.span);
                };
                let Some(name) = core_std_name(std.kind) else {
                    let tag = std_feature_tag(std.kind);
                    self.feature(
                        tag,
                        format!(
                            "standard function `{}` is outside the core",
                            hir.interner.text(std.name)
                        ),
                        Some(expr.span),
                    );
                    return self.unsupported(tag, expr.span);
                };
                let mut values = Vec::new();
                for argument in args {
                    if argument.direction != ArgDirection::Input
                        || argument.field.is_some()
                        || argument.target.is_some()
                        || argument.negated
                    {
                        self.feature(
                            "call.output-binding",
                            "output and in-out call bindings are outside the core",
                            Some(argument.span),
                        );
                    }
                    if let Some(value) = argument.value {
                        values.push(self.lower_expr(value, ctx));
                    } else {
                        self.feature(
                            "call.missing-value",
                            "a standard-function argument has no input value",
                            Some(argument.span),
                        );
                    }
                }
                (
                    json!({
                        "kind": "std",
                        "name": name,
                        "span": self.span_json(expr.span),
                    }),
                    values,
                )
            }
            CalleeHir::Conversion { .. } => {
                self.feature(
                    "call.conversion",
                    "a conversion callee is not a lowered Cast node",
                    Some(expr.span),
                );
                return self.unsupported("call.conversion", expr.span);
            }
            CalleeHir::Method { .. } => {
                self.feature(
                    "callee.method",
                    "method calls are outside the core",
                    Some(expr.span),
                );
                return self.unsupported("callee.method", expr.span);
            }
            CalleeHir::SuperMethod { .. } => {
                self.feature(
                    "callee.super-method",
                    "super method calls are outside the core",
                    Some(expr.span),
                );
                return self.unsupported("callee.super-method", expr.span);
            }
            CalleeHir::InterfaceMethod { .. } => {
                self.feature(
                    "callee.interface-method",
                    "interface method calls are outside the core",
                    Some(expr.span),
                );
                return self.unsupported("callee.interface-method", expr.span);
            }
            CalleeHir::Fb { .. } => {
                self.feature(
                    "callee.fb",
                    "function-block invocation is outside the core",
                    Some(expr.span),
                );
                return self.unsupported("callee.fb", expr.span);
            }
            CalleeHir::SuperBody { .. } => {
                self.feature(
                    "callee.super-body",
                    "SUPER body invocation is outside the core",
                    Some(expr.span),
                );
                return self.unsupported("callee.super-body", expr.span);
            }
        };
        json!({
            "kind": "call",
            "callee": callee_json,
            "args": args_json,
            "type": self.type_json(expr.ty),
            "span": self.span_json(expr.span),
        })
    }

    /// Returns call arguments in formal-parameter order when every argument
    /// is a plain input bound to a distinct formal, or `None` after tagging
    /// the reason the call binding is outside the modeled call shape.
    fn order_function_args(
        &mut self,
        info: &FunctionInfo,
        args: &[ArgHir],
        span: Span,
    ) -> Option<Vec<ExprId>> {
        let mut positions = BTreeMap::new();
        for (position, param) in info.params.iter().enumerate() {
            positions.insert(*param, position);
        }
        let mut ordered: Vec<(usize, ExprId)> = Vec::new();
        let mut ok = true;
        let mut seen_formals = BTreeSet::new();
        for argument in args {
            if argument.direction != ArgDirection::Input
                || argument.field.is_some()
                || argument.target.is_some()
                || argument.negated
            {
                ok = false;
                self.feature(
                    "call.output-binding",
                    "output and in-out call bindings are outside the core",
                    Some(argument.span),
                );
            }
            let Some(value) = argument.value else {
                ok = false;
                self.feature(
                    "call.missing-value",
                    "a call argument has no input value",
                    Some(argument.span),
                );
                continue;
            };
            let Some(formal) = argument.formal else {
                ok = false;
                self.feature(
                    "call.argument-formal",
                    "a function-call argument has no formal parameter",
                    Some(argument.span),
                );
                continue;
            };
            let Some(&position) = positions.get(&formal) else {
                ok = false;
                self.feature(
                    "call.argument-formal",
                    "a call argument names a formal outside the target signature",
                    Some(argument.span),
                );
                continue;
            };
            if !seen_formals.insert(position) {
                ok = false;
                self.feature(
                    "call.argument-duplicate",
                    "a formal parameter is bound more than once",
                    Some(argument.span),
                );
            }
            ordered.push((position, value));
        }
        if ok && ordered.len() != info.params.len() {
            self.feature(
                "call.arity",
                format!(
                    "call binds {} argument(s) to {} parameter(s)",
                    ordered.len(),
                    info.params.len()
                ),
                Some(span),
            );
            ok = false;
        }
        if ok {
            ordered.sort_by_key(|(position, _)| *position);
            Some(ordered.into_iter().map(|(_, value)| value).collect())
        } else {
            None
        }
    }

    fn unsupported(&mut self, tag: &str, span: Span) -> Value {
        json!({
            "kind": "unsupported",
            "feature": tag,
            "span": self.span_json(span),
        })
    }

    fn type_json(&mut self, id: TyId) -> Value {
        let hir = self.hir;
        let ty = hir.types.get(id);
        if let Some(core) = core_type(ty) {
            return core_type_json(&core);
        }
        let (tag, detail) = match ty {
            Ty::Error => ("type.error", "the type is invalid".to_owned()),
            Ty::Real(_) => (
                "type.real",
                "REAL and LREAL are outside the core".to_owned(),
            ),
            Ty::Time(_) => (
                "type.time",
                "TIME and LTIME are outside the core".to_owned(),
            ),
            Ty::Date(_) => (
                "type.date",
                "date and time-of-day types are outside the core".to_owned(),
            ),
            Ty::String(_) => (
                "type.string",
                "string types are outside the core".to_owned(),
            ),
            Ty::Array(_) => ("type.array", "array types are outside the core".to_owned()),
            Ty::Ref(_) => (
                "type.ref",
                "reference types are outside the core".to_owned(),
            ),
            Ty::Ptr(_) => ("type.ptr", "pointer types are outside the core".to_owned()),
            Ty::Named(_) => (
                "type.named",
                format!("named type `{}` is outside the core", hir.type_name(id)),
            ),
            Ty::Null => ("type.null", "the NULL type is outside the core".to_owned()),
            Ty::Generic(_) => (
                "type.generic",
                "generic standard-function types are outside the core".to_owned(),
            ),
            Ty::Int(_) | Ty::BitString(_) => (
                "type.width",
                format!("type `{}` is not a core width", hir.type_name(id)),
            ),
            Ty::Bool | Ty::Char(_) => {
                // Covered by `core_type`; kept for exhaustiveness.
                return core_type_json(&core_type(ty).expect("core type"));
            }
        };
        self.feature(tag, detail, None);
        json!({"kind": "unsupported", "feature": tag})
    }
}

fn def_feature_tag(kind: &DefKind) -> &'static str {
    match kind {
        DefKind::Namespace { .. } => "def.namespace",
        DefKind::Alias { .. } => "def.alias",
        DefKind::Subrange { .. } => "def.subrange",
        DefKind::Enum { .. } => "def.enum",
        DefKind::NamedValue { .. } => "def.named-value",
        DefKind::Struct { .. } => "def.struct",
        DefKind::Function { .. } => "def.function",
        DefKind::FunctionBlock { .. } => "def.function-block",
        DefKind::Program { .. } => "def.program",
        DefKind::Class { .. } => "def.class",
        DefKind::Interface { .. } => "def.interface",
        DefKind::Method { .. } => "def.method",
        DefKind::Global { .. } => "def.global",
        DefKind::Configuration { .. } => "def.configuration",
    }
}

fn core_type(ty: &Ty) -> Option<CoreTy> {
    match ty {
        Ty::Bool => Some(CoreTy::Bool),
        Ty::Int(int) if core_width(int.width) => Some(CoreTy::Int {
            width: int.width,
            signed: int.signed,
        }),
        Ty::BitString(width) if core_width(*width) => Some(CoreTy::Bits(*width)),
        Ty::Char(kind) => Some(CoreTy::Char(*kind)),
        _ => None,
    }
}

fn core_width(width: u8) -> bool {
    matches!(width, 8 | 16 | 32 | 64)
}

#[derive(Clone, Copy)]
enum CoreTy {
    Bool,
    Int { width: u8, signed: bool },
    Bits(u8),
    Char(CharTy),
}

fn core_type_json(ty: &CoreTy) -> Value {
    match ty {
        CoreTy::Bool => json!({"kind": "bool"}),
        CoreTy::Int { width, signed } => {
            json!({"kind": "int", "width": width, "signed": signed})
        }
        CoreTy::Bits(width) => json!({"kind": "bits", "width": width}),
        CoreTy::Char(kind) => json!({
            "kind": "char",
            "charKind": match kind {
                CharTy::Char => "char",
                CharTy::WChar => "wchar",
                CharTy::UChar => "uchar",
            },
        }),
    }
}

fn literal_fits(ty: &CoreTy, value: i128) -> bool {
    match ty {
        CoreTy::Bool => value == 0 || value == 1,
        CoreTy::Int { width, signed } => {
            let bits = u32::from(*width);
            if *signed {
                let min = -(1_i128 << (bits - 1));
                let max = (1_i128 << (bits - 1)) - 1;
                (min..=max).contains(&value)
            } else {
                (0..=(1_i128 << bits) - 1).contains(&value)
            }
        }
        CoreTy::Bits(width) => (0..=(1_i128 << u32::from(*width)) - 1).contains(&value),
        CoreTy::Char(kind) => {
            let width = match kind {
                CharTy::Char => 8,
                CharTy::WChar => 16,
                CharTy::UChar => 32,
            };
            (0..=(1_i128 << width) - 1).contains(&value)
        }
    }
}

fn int_json(value: i128) -> Value {
    if let Ok(value) = i64::try_from(value) {
        Value::from(value)
    } else if let Ok(value) = u64::try_from(value) {
        Value::from(value)
    } else {
        // No core literal or CASE label is outside the 64-bit range. This
        // fallback keeps a malformed HIR dump structurally valid; the
        // surrounding feature tag rejects it.
        Value::from(0)
    }
}

fn label_value(value: &ConstValue) -> Option<i128> {
    match value {
        ConstValue::Int(value) => Some(*value),
        ConstValue::Bool(value) => Some(i128::from(*value)),
        ConstValue::Char(value) => Some(i128::from(*value)),
        _ => None,
    }
}

fn bin_op_name(op: crate::hir::BinOp) -> &'static str {
    match op {
        crate::hir::BinOp::Add => "add",
        crate::hir::BinOp::Sub => "sub",
        crate::hir::BinOp::Mul => "mul",
        crate::hir::BinOp::Div => "div",
        crate::hir::BinOp::Mod => "mod",
        crate::hir::BinOp::Power => "pow",
        crate::hir::BinOp::And => "and",
        crate::hir::BinOp::Or => "or",
        crate::hir::BinOp::Xor => "xor",
        crate::hir::BinOp::Eq => "eq",
        crate::hir::BinOp::Ne => "ne",
        crate::hir::BinOp::Lt => "lt",
        crate::hir::BinOp::Le => "le",
        crate::hir::BinOp::Gt => "gt",
        crate::hir::BinOp::Ge => "ge",
    }
}

fn un_op_name(op: crate::hir::UnOp) -> &'static str {
    match op {
        crate::hir::UnOp::Neg => "neg",
        crate::hir::UnOp::Plus => "pos",
        crate::hir::UnOp::Not => "not",
    }
}

fn cast_kind_name(kind: crate::hir::CastKind) -> &'static str {
    match kind {
        crate::hir::CastKind::Implicit => "implicit",
        crate::hir::CastKind::Explicit => "explicit",
    }
}

fn core_std_name(kind: StdFnKind) -> Option<&'static str> {
    match kind {
        StdFnKind::Abs => Some("ABS"),
        StdFnKind::Sel => Some("SEL"),
        StdFnKind::Min => Some("MIN"),
        StdFnKind::Max => Some("MAX"),
        StdFnKind::Limit => Some("LIMIT"),
        StdFnKind::Mux => Some("MUX"),
        _ => None,
    }
}

fn std_feature_tag(kind: StdFnKind) -> &'static str {
    match kind {
        StdFnKind::Sqrt => "call.std-function-sqrt",
        StdFnKind::Ln => "call.std-function-ln",
        StdFnKind::Log => "call.std-function-log",
        StdFnKind::Exp => "call.std-function-exp",
        StdFnKind::Sin => "call.std-function-sin",
        StdFnKind::Cos => "call.std-function-cos",
        StdFnKind::Tan => "call.std-function-tan",
        StdFnKind::Asin => "call.std-function-asin",
        StdFnKind::Acos => "call.std-function-acos",
        StdFnKind::Atan => "call.std-function-atan",
        StdFnKind::Atan2 => "call.std-function-atan2",
        StdFnKind::Expt => "call.std-function-expt",
        StdFnKind::Trunc => "call.std-function-trunc",
        StdFnKind::Add => "call.std-function-add",
        StdFnKind::Sub => "call.std-function-sub",
        StdFnKind::Mul => "call.std-function-mul",
        StdFnKind::Div => "call.std-function-div",
        StdFnKind::Mod => "call.std-function-mod",
        StdFnKind::Move => "call.std-function-move",
        StdFnKind::Shl => "call.std-function-shl",
        StdFnKind::Shr => "call.std-function-shr",
        StdFnKind::Rol => "call.std-function-rol",
        StdFnKind::Ror => "call.std-function-ror",
        StdFnKind::And => "call.std-function-and",
        StdFnKind::Or => "call.std-function-or",
        StdFnKind::Xor => "call.std-function-xor",
        StdFnKind::Not => "call.std-function-not",
        StdFnKind::Gt => "call.std-function-gt",
        StdFnKind::Ge => "call.std-function-ge",
        StdFnKind::Eq => "call.std-function-eq",
        StdFnKind::Le => "call.std-function-le",
        StdFnKind::Lt => "call.std-function-lt",
        StdFnKind::Ne => "call.std-function-ne",
        StdFnKind::Len => "call.std-function-len",
        StdFnKind::LenMax => "call.std-function-len-max",
        StdFnKind::LenCodeUnit => "call.std-function-len-code-unit",
        StdFnKind::Left => "call.std-function-left",
        StdFnKind::Right => "call.std-function-right",
        StdFnKind::Mid => "call.std-function-mid",
        StdFnKind::Concat => "call.std-function-concat",
        StdFnKind::Insert => "call.std-function-insert",
        StdFnKind::Delete => "call.std-function-delete",
        StdFnKind::Replace => "call.std-function-replace",
        StdFnKind::Find => "call.std-function-find",
        StdFnKind::Assert => "call.std-function-assert",
        StdFnKind::ClockAdvance => "call.std-function-clock-advance",
        StdFnKind::IoSetInput => "call.std-function-io-set-input",
        StdFnKind::IoGetOutput => "call.std-function-io-get-output",
        StdFnKind::Abs
        | StdFnKind::Sel
        | StdFnKind::Min
        | StdFnKind::Max
        | StdFnKind::Limit
        | StdFnKind::Mux => "call.std-function",
    }
}

fn normalize_ident(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    for (index, character) in text.chars().enumerate() {
        let valid = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_'
        };
        if !valid {
            return None;
        }
        out.push(character.to_ascii_uppercase());
    }
    Some(out)
}

/// Deterministic, schema-valid fallback for a name that is not canonical.
///
/// The dump is rejected whenever this is used; the sanitized spelling only
/// keeps the document structurally valid so the rejection can be read.
fn sanitize_ident(text: &str) -> String {
    let mut out = String::with_capacity(text.len().max(1));
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            out.push(character.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

fn normalize_package(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    for (index, character) in text.chars().enumerate() {
        let valid = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        };
        if !valid {
            return None;
        }
        out.push(character.to_ascii_uppercase());
    }
    Some(out)
}

/// Mirrors `stc_got::global_root_key` without making the HIR crate depend on
/// the whole identity/manifest layer.
fn got_root_key(package: &str, path: &str) -> Option<String> {
    let package = normalize_package(package)?;
    let mut segments = Vec::new();
    for segment in path.split('.') {
        segments.push(normalize_ident(segment)?);
    }
    Some(format!("{package}::{}", segments.join(".")))
}

// ---------------------------------------------------------------------------
// Validator
// ---------------------------------------------------------------------------

fn expect_object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{what} must be a JSON object"))
}

fn field<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a Value, String> {
    object
        .get(name)
        .ok_or_else(|| format!("missing required field `{name}`"))
}

fn string_field<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    field(object, name)?
        .as_str()
        .ok_or_else(|| format!("field `{name}` must be a string"))
}

fn bool_field(object: &Map<String, Value>, name: &str) -> Result<bool, String> {
    field(object, name)?
        .as_bool()
        .ok_or_else(|| format!("field `{name}` must be a boolean"))
}

fn u64_field(object: &Map<String, Value>, name: &str) -> Result<u64, String> {
    field(object, name)?
        .as_u64()
        .ok_or_else(|| format!("field `{name}` must be a non-negative integer"))
}

fn array_field<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a [Value], String> {
    field(object, name)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("field `{name}` must be an array"))
}

fn int_value(value: &Value) -> Result<i128, String> {
    if let Some(value) = value.as_i64() {
        Ok(i128::from(value))
    } else if let Some(value) = value.as_u64() {
        Ok(i128::from(value))
    } else {
        Err("expected an integer".into())
    }
}

fn check_depth(depth: u32, what: &str) -> Result<(), String> {
    if depth > MAX_VALIDATE_DEPTH {
        Err(format!("{what} exceeds the nesting limit"))
    } else {
        Ok(())
    }
}

fn check_span(value: &Value, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let _ = string_field(object, "file")?;
    let start = u64_field(object, "start")?;
    let end = u64_field(object, "end")?;
    if start > end {
        return Err(format!("{what} has start > end"));
    }
    Ok(())
}

fn check_ident(value: &str, what: &str, allow_path: bool) -> Result<(), String> {
    let parts: Vec<&str> = if allow_path {
        value.split('.').collect()
    } else {
        vec![value]
    };
    if parts.is_empty() {
        return Err(format!("{what} is empty"));
    }
    for part in parts {
        let mut characters = part.chars();
        let Some(first) = characters.next() else {
            return Err(format!("{what} has an empty name segment"));
        };
        if !(first.is_ascii_uppercase() || first == '_') {
            return Err(format!("{what} `{value}` is not uppercase canonical"));
        }
        if !characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        }) {
            return Err(format!("{what} `{value}` is not uppercase canonical"));
        }
    }
    Ok(())
}

fn check_type(value: &Value, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    match string_field(object, "kind")? {
        "bool" => Ok(()),
        "int" => {
            let width = u64_field(object, "width")?;
            if !matches!(width, 8 | 16 | 32 | 64) {
                return Err(format!("{what} has non-core integer width {width}"));
            }
            let _ = bool_field(object, "signed")?;
            Ok(())
        }
        "bits" => {
            let width = u64_field(object, "width")?;
            if !matches!(width, 8 | 16 | 32 | 64) {
                return Err(format!("{what} has non-core bit-string width {width}"));
            }
            Ok(())
        }
        "char" => match string_field(object, "charKind")? {
            "char" | "wchar" | "uchar" => Ok(()),
            other => Err(format!("{what} has unknown character kind `{other}`")),
        },
        "unsupported" => {
            let _ = string_field(object, "feature")?;
            Ok(())
        }
        other => Err(format!("{what} has unknown type kind `{other}`")),
    }
}

#[derive(Clone, Copy)]
struct Scope {
    params: usize,
    locals: usize,
    globals: usize,
    result: bool,
}

fn check_var(object: &Map<String, Value>, scope: Scope, what: &str) -> Result<(), String> {
    let kind = string_field(object, "varKind")?;
    match kind {
        "param" => {
            let position = u64_field(object, "position")?;
            if position >= scope.params as u64 {
                return Err(format!(
                    "{what} refers to parameter {position} outside the {} parameter(s)",
                    scope.params
                ));
            }
        }
        "local" => {
            let position = u64_field(object, "position")?;
            if position >= scope.locals as u64 {
                return Err(format!(
                    "{what} refers to local {position} outside the {} local(s)",
                    scope.locals
                ));
            }
        }
        "global" => {
            let position = u64_field(object, "position")?;
            if position >= scope.globals as u64 {
                return Err(format!(
                    "{what} refers to global {position} outside the {} global(s)",
                    scope.globals
                ));
            }
            let key = string_field(object, "gotKey")?;
            if !key.contains("::") {
                return Err(format!("{what} has a malformed GOT root key"));
            }
        }
        "result" => {
            if !scope.result {
                return Err(format!(
                    "{what} refers to a result variable that does not exist"
                ));
            }
        }
        other => return Err(format!("{what} has unknown variable kind `{other}`")),
    }
    let name = string_field(object, "name")?;
    check_ident(
        name,
        &format!("{what}.name"),
        matches!(kind, "global" | "result"),
    )?;
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_place(value: &Value, scope: Scope, what: &str, depth: u32) -> Result<(), String> {
    check_depth(depth, what)?;
    let object = expect_object(value, what)?;
    match string_field(object, "kind")? {
        "var" => check_var(object, scope, what),
        "unsupported" => {
            let _ = string_field(object, "feature")?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        other => Err(format!("{what} has unknown place kind `{other}`")),
    }
}

fn check_expr(value: &Value, scope: Scope, what: &str, depth: u32) -> Result<(), String> {
    check_depth(depth, what)?;
    let object = expect_object(value, what)?;
    let kind = string_field(object, "kind")?;
    match kind {
        "literal" => {
            check_type(field(object, "type")?, &format!("{what}.type"))?;
            let _ = int_value(field(object, "value")?)?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "var" => check_var(object, scope, what),
        "unary" => {
            match string_field(object, "op")? {
                "neg" | "pos" | "not" => {}
                other => return Err(format!("{what} has unknown unary operator `{other}`")),
            }
            check_expr(
                field(object, "operand")?,
                scope,
                &format!("{what}.operand"),
                depth + 1,
            )?;
            check_type(field(object, "type")?, &format!("{what}.type"))?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "binary" => {
            match string_field(object, "op")? {
                "add" | "sub" | "mul" | "div" | "mod" | "pow" | "and" | "or" | "xor" | "eq"
                | "ne" | "lt" | "le" | "gt" | "ge" => {}
                other => return Err(format!("{what} has unknown binary operator `{other}`")),
            }
            check_expr(
                field(object, "lhs")?,
                scope,
                &format!("{what}.lhs"),
                depth + 1,
            )?;
            check_expr(
                field(object, "rhs")?,
                scope,
                &format!("{what}.rhs"),
                depth + 1,
            )?;
            check_type(field(object, "type")?, &format!("{what}.type"))?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "call" => {
            check_callee(field(object, "callee")?, &format!("{what}.callee"))?;
            for (index, argument) in array_field(object, "args")?.iter().enumerate() {
                check_expr(argument, scope, &format!("{what}.args[{index}]"), depth + 1)?;
            }
            check_type(field(object, "type")?, &format!("{what}.type"))?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "cast" => {
            check_expr(
                field(object, "operand")?,
                scope,
                &format!("{what}.operand"),
                depth + 1,
            )?;
            check_type(field(object, "to")?, &format!("{what}.to"))?;
            match string_field(object, "castKind")? {
                "implicit" | "explicit" => {}
                other => return Err(format!("{what} has unknown cast kind `{other}`")),
            }
            check_type(field(object, "type")?, &format!("{what}.type"))?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "unsupported" => {
            let _ = string_field(object, "feature")?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        other => Err(format!("{what} has unknown expression kind `{other}`")),
    }
}

fn check_callee(value: &Value, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    match string_field(object, "kind")? {
        "function" => {
            let name = string_field(object, "name")?;
            check_ident(name, &format!("{what}.name"), true)?;
        }
        "std" => match string_field(object, "name")? {
            "ABS" | "SEL" | "MIN" | "MAX" | "LIMIT" | "MUX" => {}
            other => return Err(format!("{what} has non-core standard function `{other}`")),
        },
        other => return Err(format!("{what} has unknown callee kind `{other}`")),
    }
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_label(value: &Value, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let _ = int_value(field(object, "low")?)?;
    let high = field(object, "high")?;
    if !high.is_null() {
        let _ = int_value(high)?;
    }
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_stmt(value: &Value, scope: Scope, what: &str, depth: u32) -> Result<(), String> {
    check_depth(depth, what)?;
    let object = expect_object(value, what)?;
    match string_field(object, "kind")? {
        "assign" => {
            check_place(
                field(object, "place")?,
                scope,
                &format!("{what}.place"),
                depth + 1,
            )?;
            check_expr(
                field(object, "value")?,
                scope,
                &format!("{what}.value"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "call" => {
            let call = field(object, "call")?;
            check_expr(call, scope, &format!("{what}.call"), depth + 1)?;
            let call_kind = call
                .as_object()
                .and_then(|call| call.get("kind"))
                .and_then(Value::as_str);
            // An out-of-core invocation (for example a function-block call
            // statement) is represented as `unsupported` rather than a `call`
            // expression; both are valid call statements.
            if !matches!(call_kind, Some("call" | "unsupported")) {
                return Err(format!("{what}.call must be a call expression"));
            }
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "return" | "exit" | "continue" | "empty" => {
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "if" => {
            for (index, arm) in array_field(object, "arms")?.iter().enumerate() {
                let arm = expect_object(arm, &format!("{what}.arms[{index}]"))?;
                check_expr(
                    field(arm, "cond")?,
                    scope,
                    &format!("{what}.arms[{index}].cond"),
                    depth + 1,
                )?;
                check_stmts(
                    array_field(arm, "body")?,
                    scope,
                    &format!("{what}.arms[{index}].body"),
                    depth + 1,
                )?;
                check_span(field(arm, "span")?, &format!("{what}.arms[{index}].span"))?;
            }
            check_stmts(
                array_field(object, "else")?,
                scope,
                &format!("{what}.else"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "case" => {
            check_expr(
                field(object, "selector")?,
                scope,
                &format!("{what}.selector"),
                depth + 1,
            )?;
            for (index, arm) in array_field(object, "arms")?.iter().enumerate() {
                let arm = expect_object(arm, &format!("{what}.arms[{index}]"))?;
                for (label_index, label) in array_field(arm, "labels")?.iter().enumerate() {
                    check_label(
                        label,
                        &format!("{what}.arms[{index}].labels[{label_index}]"),
                    )?;
                }
                check_stmts(
                    array_field(arm, "body")?,
                    scope,
                    &format!("{what}.arms[{index}].body"),
                    depth + 1,
                )?;
                check_span(field(arm, "span")?, &format!("{what}.arms[{index}].span"))?;
            }
            check_stmts(
                array_field(object, "else")?,
                scope,
                &format!("{what}.else"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "for" => {
            check_place(
                field(object, "var")?,
                scope,
                &format!("{what}.var"),
                depth + 1,
            )?;
            check_expr(
                field(object, "start")?,
                scope,
                &format!("{what}.start"),
                depth + 1,
            )?;
            check_expr(
                field(object, "stop")?,
                scope,
                &format!("{what}.stop"),
                depth + 1,
            )?;
            let step = field(object, "step")?;
            if step.is_null() {
                return Err(format!(
                    "{what}.step must be an expression (the default step is materialized)"
                ));
            }
            check_expr(step, scope, &format!("{what}.step"), depth + 1)?;
            check_stmts(
                array_field(object, "body")?,
                scope,
                &format!("{what}.body"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "while" => {
            check_expr(
                field(object, "cond")?,
                scope,
                &format!("{what}.cond"),
                depth + 1,
            )?;
            check_stmts(
                array_field(object, "body")?,
                scope,
                &format!("{what}.body"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "repeat" => {
            check_stmts(
                array_field(object, "body")?,
                scope,
                &format!("{what}.body"),
                depth + 1,
            )?;
            check_expr(
                field(object, "cond")?,
                scope,
                &format!("{what}.cond"),
                depth + 1,
            )?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        "unsupported" => {
            let _ = string_field(object, "feature")?;
            check_span(field(object, "span")?, &format!("{what}.span"))
        }
        other => Err(format!("{what} has unknown statement kind `{other}`")),
    }
}

fn check_stmts(stmts: &[Value], scope: Scope, what: &str, depth: u32) -> Result<(), String> {
    for (index, stmt) in stmts.iter().enumerate() {
        check_stmt(stmt, scope, &format!("{what}[{index}]"), depth + 1)?;
    }
    Ok(())
}

fn check_param(value: &Value, what: &str, seen: &mut BTreeSet<String>) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let name = string_field(object, "name")?;
    check_ident(name, &format!("{what}.name"), false)?;
    if !seen.insert(name.to_owned()) {
        return Err(format!("duplicate parameter name `{name}` in {what}"));
    }
    check_type(field(object, "type")?, &format!("{what}.type"))?;
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_local(
    value: &Value,
    scope: Scope,
    what: &str,
    seen: &mut BTreeSet<String>,
) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let name = string_field(object, "name")?;
    check_ident(name, &format!("{what}.name"), false)?;
    if !seen.insert(name.to_owned()) {
        return Err(format!("duplicate local name `{name}` in {what}"));
    }
    check_type(field(object, "type")?, &format!("{what}.type"))?;
    let init = field(object, "init")?;
    if !init.is_null() {
        check_expr(init, scope, &format!("{what}.init"), 1)?;
    }
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_global(value: &Value, globals: usize, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let name = string_field(object, "name")?;
    check_ident(name, &format!("{what}.name"), true)?;
    let key = string_field(object, "gotKey")?;
    if !key.contains("::") {
        return Err(format!("{what}.gotKey is not a GOT root key"));
    }
    check_type(field(object, "type")?, &format!("{what}.type"))?;
    let scope = Scope {
        params: 0,
        locals: 0,
        globals,
        result: false,
    };
    let init = field(object, "init")?;
    if !init.is_null() {
        check_expr(init, scope, &format!("{what}.init"), 1)?;
    }
    check_span(field(object, "span")?, &format!("{what}.span"))
}

fn check_function(value: &Value, globals: usize, what: &str) -> Result<String, String> {
    let object = expect_object(value, what)?;
    let name = string_field(object, "name")?;
    check_ident(name, &format!("{what}.name"), true)?;
    let result = field(object, "result")?;
    if !result.is_null() {
        check_type(result, &format!("{what}.result"))?;
    }
    let mut seen_params = BTreeSet::new();
    for (index, param) in array_field(object, "params")?.iter().enumerate() {
        check_param(param, &format!("{what}.params[{index}]"), &mut seen_params)?;
    }
    let mut seen_locals = BTreeSet::new();
    let locals = array_field(object, "locals")?;
    let scope = Scope {
        params: array_field(object, "params")?.len(),
        locals: locals.len(),
        globals,
        result: !result.is_null(),
    };
    for (index, local) in locals.iter().enumerate() {
        check_local(
            local,
            scope,
            &format!("{what}.locals[{index}]"),
            &mut seen_locals,
        )?;
    }
    check_stmts(
        array_field(object, "body")?,
        scope,
        &format!("{what}.body"),
        1,
    )?;
    check_span(field(object, "span")?, &format!("{what}.span"))?;
    Ok(name.to_owned())
}

fn check_program(value: &Value, globals: usize, what: &str) -> Result<(), String> {
    let object = expect_object(value, what)?;
    let name = string_field(object, "name")?;
    check_ident(name, &format!("{what}.name"), true)?;
    let locals = array_field(object, "locals")?;
    let scope = Scope {
        params: 0,
        locals: locals.len(),
        globals,
        result: false,
    };
    let mut seen_locals = BTreeSet::new();
    for (index, local) in locals.iter().enumerate() {
        check_local(
            local,
            scope,
            &format!("{what}.locals[{index}]"),
            &mut seen_locals,
        )?;
    }
    check_stmts(
        array_field(object, "body")?,
        scope,
        &format!("{what}.body"),
        1,
    )?;
    check_span(field(object, "span")?, &format!("{what}.span"))
}
