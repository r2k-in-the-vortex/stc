//! A small textual LLVM module, function, block, and value builder.
//!
//! The builder validates operand widths where it can and guarantees that every
//! emitted block ends in exactly one terminator. Names are allocated from
//! monotonic counters so identical HIR produces byte-identical IR (D-081).
//!
//! Module-shaped IR additionally uses GOT slot-table access
//! ([`Function::slot_cell`], [`Function::cell_value`], [`Function::slot_value`]),
//! indirect calls ([`Function::call_ptr`]), module entry points with
//! [`SymbolAttrs::module_export`], and descriptor/function-table globals
//! ([`Module::add_descriptor`], [`Module::add_function_table`]). The builder
//! records structural facts while emitting; [`crate::verify`] checks them
//! before an artifact is rendered.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// An M4 LLVM type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IrType {
    /// `void`.
    Void,
    /// An integer of the given width.
    Int(u16),
    /// `float`.
    Float,
    /// `double`.
    Double,
    /// Opaque pointer (`ptr`).
    Ptr,
    /// The `{ iN, i1 }` result of an integer overflow intrinsic.
    Overflow(u16),
}

impl IrType {
    /// Renders the LLVM spelling.
    #[must_use]
    pub fn render(self) -> String {
        match self {
            Self::Void => "void".into(),
            Self::Int(bits) => format!("i{bits}"),
            Self::Float => "float".into(),
            Self::Double => "double".into(),
            Self::Ptr => "ptr".into(),
            Self::Overflow(bits) => format!("{{ i{bits}, i1 }}"),
        }
    }
}

/// LLVM linkage classes emitted by the builder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Linkage {
    /// Externally visible symbol.
    #[default]
    External,
    /// Module-local symbol with a stable name.
    Internal,
    /// Module-private symbol.
    Private,
}

impl Linkage {
    /// Renders the textual prefix, including the trailing space.
    #[must_use]
    pub const fn render(self) -> &'static str {
        match self {
            Self::External => "",
            Self::Internal => "internal ",
            Self::Private => "private ",
        }
    }
}

/// LLVM visibility for externally linked symbols.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    /// Default visibility.
    #[default]
    Default,
    /// Hidden visibility.
    Hidden,
    /// Protected visibility.
    Protected,
}

impl Visibility {
    const fn render(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Hidden => "hidden ",
            Self::Protected => "protected ",
        }
    }
}

/// Linkage and visibility attributes for one function or global.
///
/// Visibility and `dllexport` are rendered only for external linkage; LLVM
/// rejects both on internal or private symbols.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SymbolAttrs {
    /// Linkage class.
    pub linkage: Linkage,
    /// Symbol visibility.
    pub visibility: Visibility,
    /// Emit `dllexport` (module entry points).
    pub export: bool,
    /// Emit `dso_local` (non-preemptable on ELF targets).
    pub dso_local: bool,
}

impl SymbolAttrs {
    /// External linkage with default visibility.
    #[must_use]
    pub const fn external() -> Self {
        Self {
            linkage: Linkage::External,
            visibility: Visibility::Default,
            export: false,
            dso_local: false,
        }
    }

    /// Module-local symbol.
    #[must_use]
    pub const fn internal() -> Self {
        Self {
            linkage: Linkage::Internal,
            ..Self::external()
        }
    }

    /// Module-private symbol.
    #[must_use]
    pub const fn private() -> Self {
        Self {
            linkage: Linkage::Private,
            ..Self::external()
        }
    }

    /// Exported, non-preemptable module entry point (`dso_local dllexport`).
    ///
    /// `dso_local` makes the definition non-preemptable on ELF, and
    /// `dllexport` places the symbol in the export table on COFF; each target
    /// ignores the other's attribute.
    #[must_use]
    pub const fn module_export() -> Self {
        Self {
            linkage: Linkage::External,
            visibility: Visibility::Default,
            export: true,
            dso_local: true,
        }
    }

    /// Renders the attribute prefix including a trailing space.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = String::from(self.linkage.render());
        if self.linkage == Linkage::External {
            if self.dso_local {
                text.push_str("dso_local ");
            }
            text.push_str(self.visibility.render());
            if self.export {
                text.push_str("dllexport ");
            }
        }
        text
    }
}

/// One GOT cell kind (GOT Artifacts §7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CellKind {
    /// Pointer to persistent host-owned root storage.
    Data,
    /// Address of a function implementation in the active module.
    Function,
    /// Address of a runtime-owned descriptor or vtable.
    Descriptor,
    /// Pointer to a process-image direct-variable location.
    Direct,
    /// Address of a host service entry point.
    Import,
}

/// A typed SSA value or literal.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    /// Value type.
    pub ty: IrType,
    /// SSA name (`%t0`) or literal text (`0`, `true`, `0x3F800000`).
    pub repr: String,
}

impl Value {
    /// Renders `type repr` for use as an instruction operand.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{} {}", self.ty.render(), self.repr)
    }

    /// Creates an integer literal in two's complement for `bits`.
    #[must_use]
    pub fn int(bits: u16, value: i128) -> Self {
        Self {
            ty: IrType::Int(bits),
            repr: int_literal(value, bits),
        }
    }

    /// Creates a boolean (`i1`) literal.
    #[must_use]
    pub fn bool(value: bool) -> Self {
        Self {
            ty: IrType::Int(1),
            repr: if value { "true" } else { "false" }.into(),
        }
    }

    /// Creates an exact floating-point literal.
    #[must_use]
    pub fn float32(value: f32) -> Self {
        Self {
            ty: IrType::Float,
            repr: format!("0x{:08X}", value.to_bits()),
        }
    }

    /// Creates an exact double literal.
    #[must_use]
    pub fn double(value: f64) -> Self {
        Self {
            ty: IrType::Double,
            repr: format!("0x{:016X}", value.to_bits()),
        }
    }

    /// Creates a null pointer literal.
    #[must_use]
    pub fn null_ptr() -> Self {
        Self {
            ty: IrType::Ptr,
            repr: "null".into(),
        }
    }

    /// Creates a symbol pointer literal (`@name`).
    #[must_use]
    pub fn symbol(name: &str) -> Self {
        Self {
            ty: IrType::Ptr,
            repr: format!("@{name}"),
        }
    }

    /// Returns the zero value of a scalar type.
    #[must_use]
    pub fn zero(ty: IrType) -> Self {
        match ty {
            IrType::Void => Self {
                ty,
                repr: String::new(),
            },
            IrType::Int(bits) => Self::int(bits, 0),
            IrType::Float => Self::float32(0.0),
            IrType::Double => Self::double(0.0),
            IrType::Ptr => Self::null_ptr(),
            IrType::Overflow(bits) => Self {
                ty: IrType::Overflow(bits),
                repr: "zeroinitializer".into(),
            },
        }
    }
}

/// Renders an integer literal accepted by LLVM for an `iN` type.
#[must_use]
pub fn int_literal(value: i128, bits: u16) -> String {
    debug_assert!(bits > 0 && bits <= 64);
    let modulus = 1_i128 << bits;
    let wrapped = value.rem_euclid(modulus);
    let half = 1_i128 << (bits - 1);
    if wrapped >= half {
        (wrapped - modulus).to_string()
    } else {
        wrapped.to_string()
    }
}

#[derive(Clone, Debug)]
struct Block {
    label: String,
    instructions: Vec<String>,
    terminator: Option<String>,
}

/// The stage of a tracked slot access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellStage {
    /// The value is the address of a host cell (`slots[slot]`).
    Address,
    /// The value is the pointer-sized value stored in a host cell.
    Value,
}

/// One tracked slot access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellAccess {
    /// Slot number.
    pub(crate) slot: u32,
    /// Access stage.
    pub(crate) stage: CellStage,
}

/// Where an SSA pointer value comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PointerOrigin {
    /// A host cell address or cell value.
    Cell(CellAccess),
    /// A module-level symbol.
    Global(String),
}

/// Structural facts recorded while an instruction is emitted.
#[derive(Clone, Debug, Default)]
pub(crate) struct FunctionFacts {
    /// Direct calls by callee symbol.
    pub(crate) direct_calls: Vec<String>,
    /// Indirect calls by callee SSA value.
    pub(crate) indirect_calls: Vec<String>,
    /// Origin of each SSA pointer value that carries one.
    pub(crate) origins: BTreeMap<String, PointerOrigin>,
    /// Loads and stores through a cell value as `(slot, is_store)`.
    pub(crate) memory_accesses: Vec<(u32, bool)>,
    /// Stores through a cell address, which would write a host cell.
    pub(crate) cell_writes: Vec<u32>,
    /// Loads and stores through a module global as `(symbol, is_store)`.
    pub(crate) memory_globals: Vec<(String, bool)>,
    /// `@symbol` references in emitted instructions and terminators.
    pub(crate) symbol_refs: BTreeSet<String>,
}

/// One function being built.
#[derive(Clone, Debug)]
pub struct Function {
    /// Symbol name without `@`.
    pub name: String,
    /// Linkage and visibility attributes.
    pub attrs: SymbolAttrs,
    /// Return type.
    pub ret: IrType,
    /// Parameters as `(type, %name)`.
    pub params: Vec<(IrType, String)>,
    blocks: Vec<Block>,
    current: usize,
    temp_counter: u32,
    block_counter: u32,
    pub(crate) facts: FunctionFacts,
}

impl Function {
    /// Creates an empty function. Call [`Self::start_block`] before emitting.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        attrs: SymbolAttrs,
        ret: IrType,
        params: Vec<(IrType, String)>,
    ) -> Self {
        Self {
            name: name.into(),
            attrs,
            ret,
            params,
            blocks: Vec::new(),
            current: 0,
            temp_counter: 0,
            block_counter: 0,
            facts: FunctionFacts::default(),
        }
    }

    /// Returns the current block label.
    #[must_use]
    pub fn current_label(&self) -> &str {
        self.blocks
            .get(self.current)
            .map_or("", |block| block.label.as_str())
    }

    /// Starts a new block.
    pub fn start_block(&mut self, label: &str) {
        if let Some(block) = self.blocks.get(self.current)
            && block.terminator.is_none()
            && !block.instructions.is_empty()
        {
            panic!("cannot start block {label}: current block is not terminated");
        }
        self.blocks.push(Block {
            label: label.into(),
            instructions: Vec::new(),
            terminator: None,
        });
        self.current = self.blocks.len() - 1;
    }

    /// Starts a fresh generated block and returns its label.
    pub fn start_fresh_block(&mut self) -> String {
        let label = self.next_block_label();
        self.start_block(&label);
        label
    }

    /// Reserves a fresh block label without starting the block.
    pub fn next_block_label(&mut self) -> String {
        let label = format!("bb{}", self.block_counter);
        self.block_counter += 1;
        label
    }

    /// Returns a function parameter as a value.
    #[must_use]
    pub fn param_value(&self, index: usize) -> Value {
        let (ty, name) = self.params[index].clone();
        Value { ty, repr: name }
    }

    /// Starts a new block when the current one is already terminated.
    pub fn ensure_open(&mut self) {
        if self.is_terminated() {
            self.start_fresh_block();
        }
    }

    /// Allocates a temporary SSA name.
    pub fn temp(&mut self, ty: IrType) -> Value {
        let value = Value {
            ty,
            repr: format!("%t{}", self.temp_counter),
        };
        self.temp_counter += 1;
        value
    }

    /// Appends an instruction that produces `value`.
    pub fn emit_value(&mut self, value: &Value, instruction: impl AsRef<str>) {
        self.push(format!("{} = {}", value.repr, instruction.as_ref()));
    }

    /// Appends a non-producing instruction.
    pub fn emit(&mut self, instruction: impl AsRef<str>) {
        self.push(instruction.as_ref().to_owned());
    }

    /// Appends a terminator.
    pub fn terminate(&mut self, instruction: impl Into<String>) {
        let instruction = instruction.into();
        collect_symbols(&instruction, &mut self.facts.symbol_refs);
        let block = &mut self.blocks[self.current];
        assert!(
            block.terminator.is_none(),
            "block {} already terminated",
            block.label
        );
        block.terminator = Some(instruction);
    }

    fn push(&mut self, instruction: String) {
        collect_symbols(&instruction, &mut self.facts.symbol_refs);
        let block = &mut self.blocks[self.current];
        assert!(
            block.terminator.is_none(),
            "cannot emit into terminated block {}",
            block.label
        );
        block.instructions.push(instruction);
    }

    /// Returns `true` when the current block has a terminator.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.blocks
            .get(self.current)
            .is_some_and(|block| block.terminator.is_some())
    }

    /// Emits an unconditional branch.
    pub fn branch(&mut self, target: &str) {
        self.terminate(format!("br label %{target}"));
    }

    /// Emits a conditional branch.
    pub fn cond_branch(&mut self, condition: &Value, then: &str, otherwise: &str) {
        debug_assert_eq!(condition.ty, IrType::Int(1));
        self.terminate(format!(
            "br i1 {}, label %{then}, label %{otherwise}",
            condition.repr
        ));
    }

    /// Emits `ret`.
    pub fn ret(&mut self, value: Option<&Value>) {
        match value {
            Some(value) => self.terminate(format!("ret {}", value.render())),
            None => self.terminate("ret void"),
        }
    }

    /// Emits `unreachable`.
    pub fn unreachable(&mut self) {
        self.terminate("unreachable");
    }

    /// Emits an `alloca`.
    pub fn alloca(&mut self, ty: IrType) -> Value {
        let pointer = self.temp(IrType::Ptr);
        self.emit_value(&pointer, format!("alloca {}", ty.render()));
        pointer
    }

    /// Emits a typed load. `align` is emitted only when explicitly known.
    pub fn load(&mut self, ty: IrType, pointer: &Value, align: Option<u32>) -> Value {
        debug_assert_eq!(pointer.ty, IrType::Ptr);
        self.note_memory_access(pointer, false);
        let value = self.temp(ty);
        let mut instruction = format!("load {}, ptr {}", ty.render(), pointer.repr);
        if let Some(align) = align {
            let _ = write!(instruction, ", align {align}");
        }
        self.emit_value(&value, instruction);
        value
    }

    /// Emits a typed store.
    pub fn store(&mut self, value: &Value, pointer: &Value, align: Option<u32>) {
        debug_assert_eq!(pointer.ty, IrType::Ptr);
        self.note_memory_access(pointer, true);
        let mut instruction = format!("store {}, ptr {}", value.render(), pointer.repr);
        if let Some(align) = align {
            let _ = write!(instruction, ", align {align}");
        }
        self.emit(instruction);
    }

    /// Records a load or store that goes through a slot-table cell value or a
    /// module global.
    fn note_memory_access(&mut self, pointer: &Value, store: bool) {
        if let Some(symbol) = pointer.repr.strip_prefix('@') {
            self.facts.memory_globals.push((symbol.to_owned(), store));
            return;
        }
        match self.facts.origins.get(&pointer.repr) {
            Some(PointerOrigin::Cell(access)) if access.stage == CellStage::Value => {
                self.facts.memory_accesses.push((access.slot, store));
            }
            Some(PointerOrigin::Cell(access)) if store => {
                self.facts.cell_writes.push(access.slot);
            }
            Some(PointerOrigin::Global(symbol)) => {
                self.facts.memory_globals.push((symbol.clone(), store));
            }
            _ => {}
        }
    }

    /// Emits an integer or floating-point binary operation.
    pub fn binop(&mut self, op: &str, lhs: &Value, rhs: &Value) -> Value {
        debug_assert_eq!(lhs.ty, rhs.ty);
        let value = self.temp(lhs.ty);
        self.emit_value(
            &value,
            format!("{op} {} {}, {}", lhs.ty.render(), lhs.repr, rhs.repr),
        );
        value
    }

    /// Emits an integer or floating-point unary operation.
    pub fn unop(&mut self, op: &str, operand: &Value) -> Value {
        let value = self.temp(operand.ty);
        self.emit_value(
            &value,
            format!("{op} {} {}", operand.ty.render(), operand.repr),
        );
        value
    }

    /// Emits an integer comparison; returns `i1`.
    pub fn icmp(&mut self, predicate: &str, lhs: &Value, rhs: &Value) -> Value {
        debug_assert_eq!(lhs.ty, rhs.ty);
        let value = self.temp(IrType::Int(1));
        self.emit_value(
            &value,
            format!(
                "icmp {predicate} {} {}, {}",
                lhs.ty.render(),
                lhs.repr,
                rhs.repr
            ),
        );
        value
    }

    /// Emits a floating-point comparison; returns `i1`.
    pub fn fcmp(&mut self, predicate: &str, lhs: &Value, rhs: &Value) -> Value {
        debug_assert_eq!(lhs.ty, rhs.ty);
        let value = self.temp(IrType::Int(1));
        self.emit_value(
            &value,
            format!(
                "fcmp {predicate} {} {}, {}",
                lhs.ty.render(),
                lhs.repr,
                rhs.repr
            ),
        );
        value
    }

    /// Emits `select`.
    pub fn select(&mut self, condition: &Value, then: &Value, otherwise: &Value) -> Value {
        debug_assert_eq!(condition.ty, IrType::Int(1));
        debug_assert_eq!(then.ty, otherwise.ty);
        let value = self.temp(then.ty);
        self.emit_value(
            &value,
            format!(
                "select i1 {}, {}, {}",
                condition.repr,
                then.render(),
                otherwise.render()
            ),
        );
        value
    }

    /// Emits a `getelementptr` with rendered indices.
    pub fn gep(
        &mut self,
        pointee: &str,
        base: &Value,
        indices: &[String],
        inbounds: bool,
    ) -> Value {
        debug_assert_eq!(base.ty, IrType::Ptr);
        let value = self.temp(IrType::Ptr);
        let keyword = if inbounds {
            "getelementptr inbounds"
        } else {
            "getelementptr"
        };
        self.emit_value(
            &value,
            format!(
                "{keyword} {pointee}, ptr {}, {}",
                base.repr,
                indices.join(", ")
            ),
        );
        if let Some(symbol) = base.repr.strip_prefix('@') {
            self.facts
                .origins
                .insert(value.repr.clone(), PointerOrigin::Global(symbol.to_owned()));
        } else if let Some(origin) = self.facts.origins.get(&base.repr).cloned() {
            self.facts.origins.insert(value.repr.clone(), origin);
        }
        value
    }

    /// Emits a GEP by constant byte offset.
    pub fn gep_bytes(&mut self, base: &Value, offset: i64) -> Value {
        self.gep("i8", base, &[format!("i64 {offset}")], false)
    }

    /// Emits a direct call. Returns `None` for `void` calls.
    pub fn call(&mut self, ret: IrType, callee: &str, args: &[Value]) -> Option<Value> {
        self.facts.direct_calls.push(callee.to_owned());
        let arguments = args
            .iter()
            .map(Value::render)
            .collect::<Vec<_>>()
            .join(", ");
        let instruction = format!("call {} @{callee}({arguments})", ret.render());
        if ret == IrType::Void {
            self.emit(instruction);
            None
        } else {
            let value = self.temp(ret);
            self.emit_value(&value, instruction);
            Some(value)
        }
    }

    /// Emits an indirect call through a function or import cell value.
    ///
    /// The LLVM function type is inferred from the result and argument types,
    /// which is sufficient for the M4.5 scalar surface. Returns `None` for
    /// `void` calls.
    pub fn call_ptr(&mut self, ret: IrType, callee: &Value, args: &[Value]) -> Option<Value> {
        debug_assert_eq!(callee.ty, IrType::Ptr);
        self.facts.indirect_calls.push(callee.repr.clone());
        let arguments = args
            .iter()
            .map(Value::render)
            .collect::<Vec<_>>()
            .join(", ");
        let instruction = format!("call {} {}({arguments})", ret.render(), callee.repr);
        if ret == IrType::Void {
            self.emit(instruction);
            None
        } else {
            let value = self.temp(ret);
            self.emit_value(&value, instruction);
            Some(value)
        }
    }

    /// Emits a slot-table entry address: `getelementptr ptr, ptr %slots, i64 slot`.
    ///
    /// `slots` is the host-provided array of cell pointers (`void* const*`).
    /// The result is the address of the host cell, not the cell value.
    pub fn slot_cell(&mut self, slots: &Value, slot: u32) -> Value {
        debug_assert_eq!(slots.ty, IrType::Ptr);
        let cell = self.gep("ptr", slots, &[format!("i64 {slot}")], false);
        self.facts.origins.insert(
            cell.repr.clone(),
            PointerOrigin::Cell(CellAccess {
                slot,
                stage: CellStage::Address,
            }),
        );
        cell
    }

    /// Loads the pointer-sized value of a cell produced by [`Self::slot_cell`].
    ///
    /// A cell value is a storage pointer for data/direct/descriptor cells, a
    /// code address for function cells, and a host service address for import
    /// cells.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `cell` is not a [`Self::slot_cell`] result.
    #[must_use]
    pub fn cell_value(&mut self, cell: &Value) -> Value {
        debug_assert_eq!(cell.ty, IrType::Ptr);
        let access = match self.facts.origins.get(&cell.repr) {
            Some(PointerOrigin::Cell(access)) => Some(*access),
            _ => None,
        };
        debug_assert!(
            access.is_some_and(|access| access.stage == CellStage::Address),
            "cell_value requires a slot_cell result"
        );
        let value = self.load(IrType::Ptr, cell, Some(8));
        if let Some(access) = access {
            self.facts.origins.insert(
                value.repr.clone(),
                PointerOrigin::Cell(CellAccess {
                    slot: access.slot,
                    stage: CellStage::Value,
                }),
            );
        }
        value
    }

    /// Emits both loads for one slot and returns the cell value.
    pub fn slot_value(&mut self, slots: &Value, slot: u32) -> Value {
        let cell = self.slot_cell(slots, slot);
        self.cell_value(&cell)
    }

    /// Emits `extractvalue`.
    pub fn extract_value(&mut self, aggregate: &Value, index: u32, ty: IrType) -> Value {
        let value = self.temp(ty);
        self.emit_value(
            &value,
            format!("extractvalue {}, {index}", aggregate.render()),
        );
        value
    }

    /// Emits a `phi` with `(value, predecessor label)` inputs.
    pub fn phi(&mut self, ty: IrType, incoming: &[(Value, String)]) -> Value {
        let value = self.temp(ty);
        let parts = incoming
            .iter()
            .map(|(value, label)| format!("[ {}, %{label} ]", value.repr))
            .collect::<Vec<_>>()
            .join(", ");
        self.emit_value(&value, format!("phi {} {parts}", ty.render()));
        value
    }

    /// Renders the function in LLVM textual form.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let params = self
            .params
            .iter()
            .map(|(ty, name)| format!("{} {name}", ty.render()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "define {}{} @{}({params}) {{",
            self.attrs.render(),
            self.ret.render(),
            self.name
        );
        for block in &self.blocks {
            let _ = writeln!(out, "{}:", block.label);
            for instruction in &block.instructions {
                let _ = writeln!(out, "  {instruction}");
            }
            if let Some(terminator) = &block.terminator {
                let _ = writeln!(out, "  {terminator}");
            }
        }
        out.push_str("}\n");
        out
    }

    /// Checks structural invariants used by tests.
    ///
    /// # Errors
    ///
    /// Returns the first violated invariant.
    pub fn check(&self) -> Result<(), String> {
        if self.blocks.is_empty() {
            return Err(format!("function {} has no blocks", self.name));
        }
        let mut labels = std::collections::BTreeSet::new();
        for block in &self.blocks {
            if !labels.insert(block.label.clone()) {
                return Err(format!(
                    "duplicate block label {} in {}",
                    block.label, self.name
                ));
            }
            if block.terminator.is_none() {
                return Err(format!(
                    "block {} in {} is not terminated",
                    block.label, self.name
                ));
            }
        }
        Ok(())
    }
}

/// One module-level global.
#[derive(Clone, Debug)]
pub struct GlobalDef {
    /// Global name without `@`.
    pub name: String,
    /// Linkage and visibility attributes.
    pub attrs: SymbolAttrs,
    /// Rendered pointee/global type, for example `i32` or `[4 x i8]`.
    pub ty: String,
    /// Initializer text.
    pub init: String,
    /// `true` for `constant`, `false` for `global`.
    pub constant: bool,
    /// Optional explicit alignment.
    pub align: Option<u32>,
}

/// A textual LLVM module.
#[derive(Debug, Default)]
pub struct Module {
    /// Header comment and target lines.
    pub header: Vec<String>,
    /// Struct type definitions by name (without `%`).
    pub type_defs: BTreeMap<String, String>,
    /// Globals by symbol name.
    pub globals: BTreeMap<String, GlobalDef>,
    /// Declarations by symbol name.
    pub declarations: BTreeMap<String, String>,
    /// Function definitions by symbol name.
    pub functions: BTreeMap<String, Function>,
}

impl Module {
    /// Creates a module with the standard deterministic header.
    #[must_use]
    pub fn new(artifact_name: &str, target: Option<&str>) -> Self {
        let mut module = Self::default();
        module.header.push(format!(
            "; stc {} - profile stc-2013-oop-utf8-v1",
            env!("CARGO_PKG_VERSION")
        ));
        module
            .header
            .push(format!("source_filename = {}", quote(artifact_name)));
        if let Some(target) = target {
            module
                .header
                .push(format!("target triple = {}", quote(target)));
        }
        module
    }

    /// Adds or replaces a struct type definition.
    pub fn add_type(&mut self, name: &str, body: &str) {
        self.type_defs.insert(name.into(), body.into());
    }

    /// Adds or replaces a global.
    pub fn add_global(&mut self, global: GlobalDef) {
        self.globals.insert(global.name.clone(), global);
    }

    /// Adds an exact byte-array constant global (module read-only data).
    pub fn add_constant_bytes(
        &mut self,
        name: &str,
        attrs: SymbolAttrs,
        bytes: &[u8],
        align: Option<u32>,
    ) {
        self.add_global(GlobalDef {
            name: name.into(),
            attrs,
            ty: format!("[{} x i8]", bytes.len()),
            init: byte_string(bytes),
            constant: true,
            align,
        });
    }

    /// Adds a private, eight-byte-aligned module descriptor blob.
    pub fn add_descriptor(&mut self, name: &str, bytes: &[u8]) {
        self.add_constant_bytes(name, SymbolAttrs::private(), bytes, Some(8));
    }

    /// Adds a module-local constant table of function code addresses.
    ///
    /// This is the array returned by `stc_module_functions`; entries are
    /// module code addresses, never GOT cells.
    pub fn add_function_table(&mut self, name: &str, functions: &[&str]) {
        let entries = functions
            .iter()
            .map(|function| format!("ptr @{function}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.add_global(GlobalDef {
            name: name.into(),
            attrs: SymbolAttrs::internal(),
            ty: format!("[{} x ptr]", functions.len()),
            init: format!("[{entries}]"),
            constant: true,
            align: Some(8),
        });
    }

    /// Adds a declaration when it is not already present.
    pub fn declare(&mut self, name: &str, signature: &str) {
        self.declarations
            .entry(name.into())
            .or_insert_with(|| format!("declare {signature}"));
    }

    /// Adds a function definition.
    pub fn add_function(&mut self, function: Function) {
        self.functions.insert(function.name.clone(), function);
    }

    /// Renders the whole module with a stable section order.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.header {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        if !self.type_defs.is_empty() {
            out.push_str("; types\n");
            for (name, body) in &self.type_defs {
                let _ = writeln!(out, "%{name} = type {body}");
            }
            out.push('\n');
        }
        if !self.globals.is_empty() {
            out.push_str("; globals\n");
            for global in self.globals.values() {
                let keyword = if global.constant {
                    "constant"
                } else {
                    "global"
                };
                let _ = write!(
                    out,
                    "@{} = {}{keyword} {} {}",
                    global.name,
                    global.attrs.render(),
                    global.ty,
                    global.init
                );
                if let Some(align) = global.align {
                    let _ = write!(out, ", align {align}");
                }
                out.push('\n');
            }
            out.push('\n');
        }
        if !self.declarations.is_empty() {
            out.push_str("; declarations\n");
            for declaration in self.declarations.values() {
                out.push_str(declaration);
                out.push('\n');
            }
            out.push('\n');
        }
        if !self.functions.is_empty() {
            out.push_str("; functions\n");
            for function in self.functions.values() {
                out.push_str(&function.render());
                out.push('\n');
            }
        }
        out
    }

    /// Checks every function's structural invariants.
    ///
    /// # Errors
    ///
    /// Returns the first violated invariant.
    pub fn check(&self) -> Result<(), String> {
        for function in self.functions.values() {
            function.check()?;
        }
        Ok(())
    }
}

/// Quotes an LLVM string.
#[must_use]
pub fn quote(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\22")
        .replace('\n', "\\0A");
    format!("\"{escaped}\"")
}

/// Renders an exact LLVM `c"..."` byte-string constant.
///
/// No terminator is appended, so the rendered element count equals
/// `bytes.len()`; use [`c_string`] for NUL-terminated strings.
#[must_use]
pub fn byte_string(bytes: &[u8]) -> String {
    let mut out = String::from("c\"");
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\5C"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7E => out.push(char::from(*byte)),
            _ => {
                let _ = write!(out, "\\{byte:02X}");
            }
        }
    }
    out.push('"');
    out
}

/// Renders an LLVM `c"..."` string constant with a terminating NUL.
#[must_use]
pub fn c_string(bytes: &[u8]) -> String {
    let mut terminated = Vec::with_capacity(bytes.len() + 1);
    terminated.extend_from_slice(bytes);
    terminated.push(0);
    byte_string(&terminated)
}

/// Collects `@symbol` references from one instruction, terminator, or
/// initializer.
///
/// Quoted string bodies are skipped so descriptor, metadata, and export-name
/// constants never look like symbols. LLVM accepts quoted symbol names, but
/// the builder only emits unquoted C-identifier-style names.
pub(crate) fn collect_symbols(text: &str, out: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            match byte {
                b'\\' => index += 2,
                b'"' => {
                    in_string = false;
                    index += 1;
                }
                _ => index += 1,
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                index += 1;
            }
            b'@' => {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len() && symbol_char(bytes[end]) {
                    end += 1;
                }
                if end > start {
                    out.insert(text[start..end].to_owned());
                }
                index = end.max(start);
            }
            _ => index += 1,
        }
    }
}

fn symbol_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$' | b'-')
}
