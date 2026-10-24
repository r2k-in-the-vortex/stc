//! A small textual LLVM module, function, block, and value builder.
//!
//! The builder validates operand widths where it can and guarantees that every
//! emitted block ends in exactly one terminator. Names are allocated from
//! monotonic counters so identical HIR produces byte-identical IR (D-081).

use std::collections::BTreeMap;
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

/// One function being built.
#[derive(Clone, Debug)]
pub struct Function {
    /// Symbol name without `@`.
    pub name: String,
    /// Linkage prefix (`internal ` or empty).
    pub linkage: String,
    /// Return type.
    pub ret: IrType,
    /// Parameters as `(type, %name)`.
    pub params: Vec<(IrType, String)>,
    blocks: Vec<Block>,
    current: usize,
    temp_counter: u32,
    block_counter: u32,
}

impl Function {
    /// Creates an empty function. Call [`Self::start_block`] before emitting.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        linkage: impl Into<String>,
        ret: IrType,
        params: Vec<(IrType, String)>,
    ) -> Self {
        Self {
            name: name.into(),
            linkage: linkage.into(),
            ret,
            params,
            blocks: Vec::new(),
            current: 0,
            temp_counter: 0,
            block_counter: 0,
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
        let block = &mut self.blocks[self.current];
        assert!(
            block.terminator.is_none(),
            "block {} already terminated",
            block.label
        );
        block.terminator = Some(instruction);
    }

    fn push(&mut self, instruction: String) {
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
        let mut instruction = format!("store {}, ptr {}", value.render(), pointer.repr);
        if let Some(align) = align {
            let _ = write!(instruction, ", align {align}");
        }
        self.emit(instruction);
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
        value
    }

    /// Emits a GEP by constant byte offset.
    pub fn gep_bytes(&mut self, base: &Value, offset: i64) -> Value {
        self.gep("i8", base, &[format!("i64 {offset}")], false)
    }

    /// Emits a call. Returns `None` for `void` calls.
    pub fn call(&mut self, ret: IrType, callee: &str, args: &[Value]) -> Option<Value> {
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
            self.linkage,
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
    /// Linkage prefix (`internal `, `private `, or empty).
    pub linkage: String,
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
                    global.name, global.linkage, global.ty, global.init
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

/// Renders an LLVM `c"..."` string constant with a terminating NUL.
#[must_use]
pub fn c_string(bytes: &[u8]) -> String {
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
    out.push_str("\\00\"");
    out
}
