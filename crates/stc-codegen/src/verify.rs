//! Structural verification of module-shaped LLVM IR.
//!
//! The verifier runs on the builder's [`Module`] before an artifact is
//! rendered. It enforces the GOT invariants of
//! [Online Change and GOT](../specification/Architecture/Online-Change-and-GOT.md)
//! §3 and [GOT Artifacts and Schemas](../specification/Architecture/GOT-Artifacts.md)
//! §7:
//!
//! - generated code never references a persistent root or a user callable
//!   directly; both are reached only through GOT cell values;
//! - an indirect call targets a `Function` or `Import` cell value;
//! - a load or store through a cell value targets a `Data` or `Direct` cell;
//! - direct global access is limited to allowlisted module-local globals;
//! - every required module entry point exists with the host ABI 2 signature and
//!   external linkage;
//! - the descriptor blob and the function-address table have the shape the
//!   host expects.
//!
//! The contract is supplied by the caller because slot numbering and the
//! identity sets are build-specific; the runtime repeats the manifest
//! validation on the actual bytes (GOT Artifacts §9.5, §11.4).

use std::collections::BTreeSet;
use std::fmt;

use crate::abi;
use crate::codes;
use crate::ir::{CellKind, CellStage, IrType, Linkage, Module, PointerOrigin, collect_symbols};

/// A required module entry point and its exact IR signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryPoint {
    /// Symbol name without `@`.
    pub symbol: String,
    /// Return type.
    pub ret: IrType,
    /// Parameter types in declaration order.
    pub params: Vec<IrType>,
}

/// A required descriptor blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorSpec {
    /// Global name without `@`.
    pub symbol: String,
    /// Exact blob length in bytes.
    pub size: usize,
}

/// A required function-address table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionTableSpec {
    /// Global name without `@`.
    pub symbol: String,
    /// Exact entry count.
    pub count: usize,
}

/// What a module-shaped IR document must satisfy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contract {
    /// Persistent root symbols. They are host-owned: a module neither defines
    /// them as globals nor references them from a body.
    pub roots: BTreeSet<String>,
    /// User callables. They are reached only through function-cell values.
    pub functions: BTreeSet<String>,
    /// Module-local globals that generated bodies may load from or store to.
    /// Persistent state is reached through `Data`/`Direct` cell values, so an
    /// access to any other global is a structural error.
    pub local_globals: BTreeSet<String>,
    /// GOT cell kind for each slot, indexed by slot number.
    pub slots: Vec<CellKind>,
    /// Required entry points and exact signatures.
    pub entry_points: Vec<EntryPoint>,
    /// Required descriptor blob.
    pub descriptor: Option<DescriptorSpec>,
    /// Required function-address table.
    pub function_table: Option<FunctionTableSpec>,
}

impl Contract {
    /// Creates the ABI 2 module contract for the given slot kinds.
    #[must_use]
    pub fn for_module(slots: Vec<CellKind>) -> Self {
        Self {
            slots,
            entry_points: module_entry_points(),
            ..Self::default()
        }
    }
}

/// Returns the ABI 2 module entry points and signatures (GOT Artifacts §7.3).
#[must_use]
pub fn module_entry_points() -> Vec<EntryPoint> {
    let entry = |symbol: &str, ret: IrType, params: Vec<IrType>| EntryPoint {
        symbol: symbol.into(),
        ret,
        params,
    };
    use IrType::{Int, Ptr, Void};
    vec![
        entry(abi::module::DESCRIPTOR, Ptr, vec![Ptr]),
        entry(abi::module::FUNCTIONS, Ptr, vec![Ptr]),
        entry(
            abi::module::ATTACH,
            Int(32),
            vec![Ptr, Ptr, Int(32), Int(64)],
        ),
        entry(
            abi::module::PREPARE,
            Int(32),
            vec![Int(32), Int(64), Ptr, Int(32)],
        ),
        entry(abi::module::CYCLE, Int(32), vec![Int(64)]),
        entry(abi::module::SHUTDOWN, Int(32), vec![Int(32)]),
        entry(abi::module::DETACH, Void, vec![Int(64)]),
    ]
}

/// One structural verification failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    /// Stable diagnostic code (`STC-E-CODEGEN-002`).
    pub code: &'static str,
    /// Short rule identifier, for tests and diagnostics.
    pub rule: &'static str,
    /// Human-readable message.
    pub message: String,
}

impl VerifyError {
    fn new(rule: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: codes::CODEGEN_INTERNAL,
            rule,
            message: message.into(),
        }
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {} [{}]", self.code, self.message, self.rule)
    }
}

impl std::error::Error for VerifyError {}

/// Verifies a module-shaped IR document against a GOT contract.
///
/// # Errors
///
/// Returns every violated invariant in deterministic module order.
pub fn verify(module: &Module, contract: &Contract) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();
    if let Err(message) = module.check() {
        errors.push(VerifyError::new("module", message));
    }
    verify_function_bodies(module, contract, &mut errors);
    verify_pre_attach(module, &mut errors);
    verify_globals(module, contract, &mut errors);
    verify_entry_points(module, contract, &mut errors);
    verify_descriptor(module, contract, &mut errors);
    verify_function_table(module, contract, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn verify_function_bodies(module: &Module, contract: &Contract, errors: &mut Vec<VerifyError>) {
    for function in module.functions.values() {
        let facts = &function.facts;
        for symbol in &facts.symbol_refs {
            if contract.roots.contains(symbol) {
                errors.push(VerifyError::new(
                    "direct-root-reference",
                    format!(
                        "function `{}` references persistent root `@{symbol}` directly",
                        function.name
                    ),
                ));
            } else if contract.functions.contains(symbol) {
                errors.push(VerifyError::new(
                    "direct-function-reference",
                    format!(
                        "function `{}` references user callable `@{symbol}` directly",
                        function.name
                    ),
                ));
            }
        }
        for callee in &facts.direct_calls {
            if contract.functions.contains(callee) {
                errors.push(VerifyError::new(
                    "direct-user-call",
                    format!(
                        "function `{}` calls user callable `@{callee}` directly; \
                         user calls use a function-cell value",
                        function.name
                    ),
                ));
            }
        }
        for callee in &facts.indirect_calls {
            let Some(origin) = facts.origins.get(callee) else {
                errors.push(VerifyError::new(
                    "indirect-call",
                    format!(
                        "function `{}` calls through `{callee}`, which is not a GOT cell value \
                         or a VTABLE code pointer",
                        function.name
                    ),
                ));
                continue;
            };
            if matches!(origin, PointerOrigin::VtableCode) {
                // VTABLE dispatch loads the host cell named by a TYPEINFO/class
                // or per-pair VTABLE entry and then the cell's code address
                // (GOT Artifacts §8.5.3, §8.5.8).
                continue;
            }
            let PointerOrigin::Cell(access) = origin else {
                errors.push(VerifyError::new(
                    "indirect-call",
                    format!(
                        "function `{}` calls through `{callee}`, which is not a GOT cell value \
                         or a VTABLE code pointer",
                        function.name
                    ),
                ));
                continue;
            };
            if access.stage != CellStage::Value {
                errors.push(VerifyError::new(
                    "indirect-call",
                    format!(
                        "function `{}` calls through a cell address rather than a cell value",
                        function.name
                    ),
                ));
                continue;
            }
            match contract.slots.get(access.slot as usize) {
                Some(CellKind::Function | CellKind::Import) => {}
                Some(kind) => errors.push(VerifyError::new(
                    "slot-kind",
                    format!(
                        "function `{}` calls through slot {} ({kind:?}); \
                         calls use Function or Import cells",
                        function.name, access.slot
                    ),
                )),
                None => errors.push(VerifyError::new(
                    "slot-range",
                    format!(
                        "function `{}` uses slot {} but the slot table has {} entries",
                        function.name,
                        access.slot,
                        contract.slots.len()
                    ),
                )),
            }
        }
        for &(slot, store) in &facts.memory_accesses {
            let operation = if store { "store to" } else { "load from" };
            match contract.slots.get(slot as usize) {
                Some(CellKind::Data | CellKind::Direct) => {}
                Some(kind) => errors.push(VerifyError::new(
                    "slot-kind",
                    format!(
                        "function `{}` performs a {operation} slot {slot} ({kind:?}); \
                         persistent storage uses Data or Direct cells",
                        function.name
                    ),
                )),
                None => errors.push(VerifyError::new(
                    "slot-range",
                    format!(
                        "function `{}` accesses slot {slot} but the slot table has {} entries",
                        function.name,
                        contract.slots.len()
                    ),
                )),
            }
        }
        for (symbol, store) in &facts.memory_globals {
            if !contract.local_globals.contains(symbol) {
                let operation = if *store { "store to" } else { "load from" };
                errors.push(VerifyError::new(
                    "global-access",
                    format!(
                        "function `{}` performs a {operation} `@{symbol}` directly; \
                         persistent state is reached through Data or Direct cells",
                        function.name
                    ),
                ));
            }
        }
        for slot in &facts.cell_writes {
            errors.push(VerifyError::new(
                "cell-write",
                format!(
                    "function `{}` stores into the GOT cell for slot {slot}; \
                     only the host writes cells",
                    function.name
                ),
            ));
        }
    }
}

/// Pre-attach entry points run before the host binds the slot table, so they
/// must not read or write any cell.
fn verify_pre_attach(module: &Module, errors: &mut Vec<VerifyError>) {
    for name in [abi::module::DESCRIPTOR, abi::module::FUNCTIONS] {
        let Some(function) = module.functions.get(name) else {
            continue;
        };
        let uses_cells = function
            .facts
            .origins
            .values()
            .any(|origin| matches!(origin, PointerOrigin::Cell(_)));
        if uses_cells || !function.facts.indirect_calls.is_empty() {
            errors.push(VerifyError::new(
                "pre-attach",
                format!("pre-attach entry point `@{name}` uses the GOT slot table"),
            ));
        }
    }
}

fn verify_globals(module: &Module, contract: &Contract, errors: &mut Vec<VerifyError>) {
    for root in &contract.roots {
        if module.globals.contains_key(root) {
            errors.push(VerifyError::new(
                "root-global",
                format!("persistent root `@{root}` is defined as a module global"),
            ));
        }
    }
    let function_table = contract
        .function_table
        .as_ref()
        .map(|spec| spec.symbol.as_str());
    for global in module.globals.values() {
        let mut symbols = BTreeSet::new();
        collect_symbols(&global.init, &mut symbols);
        for symbol in symbols {
            if contract.roots.contains(&symbol) {
                errors.push(VerifyError::new(
                    "direct-root-reference",
                    format!(
                        "global `@{}` references persistent root `@{symbol}` in its initializer",
                        global.name
                    ),
                ));
            } else if contract.functions.contains(&symbol)
                && function_table != Some(global.name.as_str())
            {
                errors.push(VerifyError::new(
                    "direct-function-reference",
                    format!(
                        "global `@{}` references user callable `@{symbol}`; \
                         only the function-address table names callables",
                        global.name
                    ),
                ));
            }
        }
    }
}

fn verify_entry_points(module: &Module, contract: &Contract, errors: &mut Vec<VerifyError>) {
    for entry in &contract.entry_points {
        let Some(function) = module.functions.get(&entry.symbol) else {
            errors.push(VerifyError::new(
                "entry-point",
                format!("required entry point `@{}` is missing", entry.symbol),
            ));
            continue;
        };
        if function.attrs.linkage != Linkage::External {
            errors.push(VerifyError::new(
                "entry-point",
                format!(
                    "entry point `@{}` has {:?} linkage; entry points are external",
                    entry.symbol, function.attrs.linkage
                ),
            ));
        }
        let params: Vec<IrType> = function
            .params
            .iter()
            .map(|(ty, _name)| ty.clone())
            .collect();
        if function.ret != entry.ret || params != entry.params {
            errors.push(VerifyError::new(
                "entry-point",
                format!(
                    "entry point `@{}` has signature {:?} ({params:?}), expected {:?} ({:?})",
                    entry.symbol, function.ret, entry.ret, entry.params
                ),
            ));
        }
    }
}

fn verify_descriptor(module: &Module, contract: &Contract, errors: &mut Vec<VerifyError>) {
    let Some(spec) = &contract.descriptor else {
        return;
    };
    verify_readonly_global(module, &spec.symbol, "descriptor", "descriptor", errors);
    match module.globals.get(&spec.symbol) {
        Some(global) => {
            let expected = format!("[{} x i8]", spec.size);
            if global.ty != expected {
                errors.push(VerifyError::new(
                    "descriptor",
                    format!(
                        "descriptor `@{}` has type `{}`, expected `{expected}`",
                        spec.symbol, global.ty
                    ),
                ));
            }
        }
        None => errors.push(VerifyError::new(
            "descriptor",
            format!("descriptor global `@{}` is missing", spec.symbol),
        )),
    }
    for function in module.functions.values() {
        if function.name != abi::module::DESCRIPTOR
            && function.facts.symbol_refs.contains(&spec.symbol)
        {
            errors.push(VerifyError::new(
                "descriptor",
                format!(
                    "function `{}` references descriptor `@{}`; \
                     only `{}` exposes it",
                    function.name,
                    spec.symbol,
                    abi::module::DESCRIPTOR
                ),
            ));
        }
    }
}

fn verify_function_table(module: &Module, contract: &Contract, errors: &mut Vec<VerifyError>) {
    let Some(spec) = &contract.function_table else {
        return;
    };
    verify_readonly_global(
        module,
        &spec.symbol,
        "function-table",
        "function table",
        errors,
    );
    match module.globals.get(&spec.symbol) {
        Some(global) => {
            let expected = format!("[{} x ptr]", spec.count);
            if global.ty != expected {
                errors.push(VerifyError::new(
                    "function-table",
                    format!(
                        "function table `@{}` has type `{}`, expected `{expected}`",
                        spec.symbol, global.ty
                    ),
                ));
            }
        }
        None => errors.push(VerifyError::new(
            "function-table",
            format!("function table global `@{}` is missing", spec.symbol),
        )),
    }
}

/// Checks that a module-owned read-only global exists with local linkage.
fn verify_readonly_global(
    module: &Module,
    symbol: &str,
    rule: &'static str,
    description: &str,
    errors: &mut Vec<VerifyError>,
) {
    let Some(global) = module.globals.get(symbol) else {
        return;
    };
    if !global.constant {
        errors.push(VerifyError::new(
            rule,
            format!("{description} `@{symbol}` must be constant"),
        ));
    }
    if global.attrs.linkage == Linkage::External {
        errors.push(VerifyError::new(
            rule,
            format!("{description} `@{symbol}` must have internal or private linkage"),
        ));
    }
}
