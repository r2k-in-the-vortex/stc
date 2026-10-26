//! Resolved HIR nodes and the crate arena.

use stc_span::Span;

use crate::id::{
    BodyId, DefId, ExprId, FieldId, LocalId, PackageId, ParamId, StdFnId, StmtId, TyId,
};
use crate::interner::{Interner, Symbol};
use crate::ty::{DateTy, GenericKind, Ty, TyArena};

/// Visibility of a definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    /// Visible across packages.
    Public,
    /// Visible in the declaring type and derived types.
    Protected,
    /// Visible only in the declaring type.
    Private,
    /// Visible in the declaring package/namespace subtree.
    Internal,
}

impl Visibility {
    /// Returns the source spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "PUBLIC",
            Self::Protected => "PROTECTED",
            Self::Private => "PRIVATE",
            Self::Internal => "INTERNAL",
        }
    }
}

/// Variable section kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VarSectionKind {
    /// `VAR`.
    Var,
    /// `VAR_INPUT`.
    Input,
    /// `VAR_OUTPUT`.
    Output,
    /// `VAR_IN_OUT`.
    InOut,
    /// `VAR_TEMP`.
    Temp,
    /// `VAR_EXTERNAL`.
    External,
    /// `VAR_GLOBAL`.
    Global,
}

impl VarSectionKind {
    /// Returns the source spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Var => "VAR",
            Self::Input => "VAR_INPUT",
            Self::Output => "VAR_OUTPUT",
            Self::InOut => "VAR_IN_OUT",
            Self::Temp => "VAR_TEMP",
            Self::External => "VAR_EXTERNAL",
            Self::Global => "VAR_GLOBAL",
        }
    }

    /// Returns the parameter direction for call binding.
    #[must_use]
    pub const fn direction(self) -> Option<ArgDirection> {
        match self {
            Self::Input => Some(ArgDirection::Input),
            Self::Output => Some(ArgDirection::Output),
            Self::InOut => Some(ArgDirection::InOut),
            _ => None,
        }
    }
}

/// Parameter direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArgDirection {
    /// `VAR_INPUT`.
    Input,
    /// `VAR_OUTPUT`.
    Output,
    /// `VAR_IN_OUT`.
    InOut,
}

/// Effective retention policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RetainPolicy {
    /// No explicit or inherited retention.
    None,
    /// Persist across warm restarts.
    Retain,
    /// Explicitly not retained.
    NonRetain,
}

/// Process-image area of a direct variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectArea {
    /// `%I`.
    Input,
    /// `%Q`.
    Output,
    /// `%M`.
    Memory,
}

impl DirectArea {
    /// Returns the area letter.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Input => 'I',
            Self::Output => 'Q',
            Self::Memory => 'M',
        }
    }
}

/// Size prefix of a direct variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DirectSize {
    /// Bit (`X`).
    Bit,
    /// Byte (`B`).
    Byte,
    /// Word (`W`).
    Word,
    /// Double word (`D`).
    DWord,
    /// Long word (`L`).
    LWord,
}

impl DirectSize {
    /// Returns the size letter.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Bit => 'X',
            Self::Byte => 'B',
            Self::Word => 'W',
            Self::DWord => 'D',
            Self::LWord => 'L',
        }
    }

    /// Returns the storage width in bits.
    #[must_use]
    pub const fn width(self) -> u8 {
        match self {
            Self::Bit => 1,
            Self::Byte => 8,
            Self::Word => 16,
            Self::DWord => 32,
            Self::LWord => 64,
        }
    }
}

/// Channel direction declared by a `{stc:io}` hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IoDirection {
    /// `input`.
    Input,
    /// `output`.
    Output,
    /// `inout`.
    InOut,
}

impl IoDirection {
    /// Returns the lowercase metadata spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::InOut => "inout",
        }
    }
}

/// A `{stc:io}` binding hint attached to a variable or field (D-085).
#[derive(Clone, Debug, PartialEq)]
pub struct IoHint {
    /// Device name, when present.
    pub device: Option<String>,
    /// Channel number, when present.
    pub channel: Option<u32>,
    /// Declared direction, when present.
    pub direction: Option<IoDirection>,
    /// Canonical JSON text of the validated payload.
    pub payload: String,
    /// Pragma span.
    pub span: Span,
}

/// A resolved direct-variable address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectAddress {
    /// Area letter, when present.
    pub area: Option<DirectArea>,
    /// Size letter, when present.
    pub size: Option<DirectSize>,
    /// Wildcard address (declarations only).
    pub wildcard: bool,
    /// Dot-separated indices.
    pub indices: Vec<u32>,
    /// Source span.
    pub span: Span,
}

/// One named value of an enum or named-value type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumValueHir {
    /// Value name.
    pub name: Symbol,
    /// Integer value.
    pub value: i128,
    /// Declaration span.
    pub span: Span,
}

/// One resolved definition.
#[derive(Clone, Debug)]
pub struct Def {
    /// Definition kind and payload.
    pub kind: DefKind,
    /// Name, when the definition has one.
    pub name: Option<Symbol>,
    /// Declaration span.
    pub span: Span,
    /// Enclosing definition (namespace, POU, or type).
    pub parent: Option<DefId>,
    /// Owning package.
    pub package: PackageId,
    /// Declared visibility.
    pub visibility: Visibility,
    /// `{stc:test}` marker.
    pub test_only: bool,
    /// `{stc:export}` marker.
    pub export: bool,
    /// Final C symbol recorded by the analyzer for an export (D-086).
    pub export_name: Option<Symbol>,
    /// `{stc:no_mangle}` marker.
    pub no_mangle: bool,
    /// `USING` directives attached to this scope.
    pub using: Vec<DefId>,
}

impl Def {
    /// Returns the definition name when present.
    #[must_use]
    pub fn name_text<'a>(&self, interner: &'a Interner) -> &'a str {
        self.name.map_or("", |symbol| interner.text(symbol))
    }

    /// Returns the field list for aggregate-like definitions.
    #[must_use]
    pub fn field_ids(&self) -> &[FieldId] {
        match &self.kind {
            DefKind::Struct { fields, .. }
            | DefKind::FunctionBlock { fields, .. }
            | DefKind::Class { fields, .. }
            | DefKind::Program { fields, .. } => fields,
            _ => &[],
        }
    }

    /// Returns the method list for type-like definitions.
    #[must_use]
    pub fn method_ids(&self) -> &[DefId] {
        match &self.kind {
            DefKind::FunctionBlock { methods, .. }
            | DefKind::Class { methods, .. }
            | DefKind::Interface { methods, .. } => methods,
            _ => &[],
        }
    }

    /// Returns the base type of a class or FB.
    #[must_use]
    pub fn base(&self) -> Option<DefId> {
        match &self.kind {
            DefKind::FunctionBlock { base, .. } | DefKind::Class { base, .. } => *base,
            _ => None,
        }
    }

    /// Returns the implemented or extended interface list.
    #[must_use]
    pub fn interface_ids(&self) -> &[DefId] {
        match &self.kind {
            DefKind::FunctionBlock { interfaces, .. } | DefKind::Class { interfaces, .. } => {
                interfaces
            }
            DefKind::Interface { extends, .. } => extends,
            _ => &[],
        }
    }
}

/// Definition payloads.
#[derive(Clone, Debug)]
pub enum DefKind {
    /// A namespace scope.
    Namespace {
        /// `INTERNAL` marker.
        internal: bool,
    },
    /// A transparent type alias (simple, array, string, or reference).
    Alias {
        /// Aliased type.
        target: TyId,
    },
    /// A subrange type.
    Subrange {
        /// Base integer type.
        base: TyId,
        /// Inclusive lower bound.
        lower: i128,
        /// Inclusive upper bound.
        upper: i128,
        /// Declared default value.
        default: Option<i128>,
    },
    /// An enumeration.
    Enum {
        /// Underlying integer type (`DINT` in this profile).
        base: TyId,
        /// Values in declaration order.
        values: Vec<EnumValueHir>,
        /// Default value, when declared.
        default: Option<Symbol>,
    },
    /// An elementary type with named values.
    NamedValue {
        /// Underlying elementary type.
        base: TyId,
        /// Values in declaration order.
        values: Vec<EnumValueHir>,
        /// Default value, when declared.
        default: Option<Symbol>,
    },
    /// A named struct type.
    Struct {
        /// Fields in declaration order.
        fields: Vec<FieldId>,
        /// `OVERLAP` marker.
        overlap: bool,
    },
    /// A function.
    Function {
        /// Result type.
        result: Option<TyId>,
        /// Input/output/in-out parameters.
        params: Vec<ParamId>,
        /// Body, when present.
        body: Option<BodyId>,
    },
    /// A function block.
    FunctionBlock {
        /// Base FB or class.
        base: Option<DefId>,
        /// Implemented interfaces.
        interfaces: Vec<DefId>,
        /// Instance fields (including inputs/outputs).
        fields: Vec<FieldId>,
        /// Methods.
        methods: Vec<DefId>,
        /// Body, when present.
        body: Option<BodyId>,
        /// `FINAL` marker.
        is_final: bool,
        /// `ABSTRACT` marker.
        is_abstract: bool,
        /// Stable vtable slot order.
        vtable: Vec<DefId>,
    },
    /// A program.
    Program {
        /// Instance fields.
        fields: Vec<FieldId>,
        /// Body.
        body: Option<BodyId>,
        /// `VAR_ACCESS` declarations.
        var_access: Vec<VarAccessHir>,
    },
    /// A class.
    Class {
        /// Base class.
        base: Option<DefId>,
        /// Implemented interfaces.
        interfaces: Vec<DefId>,
        /// Instance fields.
        fields: Vec<FieldId>,
        /// Methods.
        methods: Vec<DefId>,
        /// `FINAL` marker.
        is_final: bool,
        /// `ABSTRACT` marker.
        is_abstract: bool,
        /// Stable vtable slot order.
        vtable: Vec<DefId>,
    },
    /// An interface.
    Interface {
        /// Extended interfaces.
        extends: Vec<DefId>,
        /// Method prototypes.
        methods: Vec<DefId>,
        /// Stable prototype slot order.
        vtable: Vec<DefId>,
    },
    /// A method or interface prototype.
    Method {
        /// Enclosing class, FB, or interface.
        owner: DefId,
        /// Result type.
        result: Option<TyId>,
        /// Input/output/in-out parameters.
        params: Vec<ParamId>,
        /// Body; `None` for prototypes and abstract methods.
        body: Option<BodyId>,
        /// Access specifier.
        access: Visibility,
        /// `OVERRIDE` marker.
        is_override: bool,
        /// `FINAL` marker.
        is_final: bool,
        /// `ABSTRACT` marker.
        is_abstract: bool,
        /// Vtable slot, when virtual.
        slot: Option<u32>,
    },
    /// A global variable.
    Global {
        /// Declared type.
        ty: TyId,
        /// Section kind (`VAR_GLOBAL`).
        section: VarSectionKind,
        /// Effective retention policy.
        retain: RetainPolicy,
        /// `CONSTANT` marker.
        constant: bool,
        /// `AT` address.
        at: Option<DirectAddress>,
        /// `{stc:io}` binding hint.
        io: Option<IoHint>,
        /// Initializer expression.
        init: Option<ExprId>,
    },
    /// A configuration declaration.
    Configuration {
        /// Resolved configuration contents.
        config: Box<ConfigurationHir>,
    },
}

/// One parameter.
#[derive(Clone, Debug)]
pub struct ParamHir {
    /// Parameter name.
    pub name: Symbol,
    /// Parameter type.
    pub ty: TyId,
    /// Direction.
    pub direction: ArgDirection,
    /// Owning POU.
    pub owner: DefId,
    /// Declaration span.
    pub span: Span,
    /// Default value, when declared.
    pub default: Option<ExprId>,
}

/// One local variable.
#[derive(Clone, Debug)]
pub struct LocalHir {
    /// Local name.
    pub name: Symbol,
    /// Local type.
    pub ty: TyId,
    /// Owning POU.
    pub owner: DefId,
    /// `VAR_TEMP` marker.
    pub is_temp: bool,
    /// Effective retention policy.
    pub retain: RetainPolicy,
    /// Declaration span.
    pub span: Span,
    /// Initializer expression.
    pub init: Option<ExprId>,
    /// Global targeted by a `VAR_EXTERNAL` alias.
    pub external_target: Option<DefId>,
}

/// One field of a struct, class, FB, or program.
#[derive(Clone, Debug)]
pub struct FieldHir {
    /// Field name.
    pub name: Symbol,
    /// Field type.
    pub ty: TyId,
    /// Owning definition.
    pub owner: DefId,
    /// Declared visibility.
    pub visibility: Visibility,
    /// Section kind; `Var` for struct fields.
    pub section: VarSectionKind,
    /// Effective retention policy.
    pub retain: RetainPolicy,
    /// `CONSTANT` marker.
    pub constant: bool,
    /// `AT` address.
    pub at: Option<DirectAddress>,
    /// `{stc:io}` binding hint.
    pub io: Option<IoHint>,
    /// Initializer expression.
    pub init: Option<ExprId>,
    /// Declaration span.
    pub span: Span,
}

impl FieldHir {
    /// Returns the direction when the field is an FB/program parameter.
    #[must_use]
    pub fn direction(&self) -> Option<ArgDirection> {
        self.section.direction()
    }
}

/// One body.
#[derive(Clone, Debug)]
pub struct Body {
    /// Owning POU or method.
    pub owner: DefId,
    /// Parameters.
    pub params: Vec<ParamId>,
    /// Locals.
    pub locals: Vec<LocalId>,
    /// Statements in order.
    pub stmts: Vec<StmtId>,
    /// Source span.
    pub span: Span,
}

/// A constant value embedded in HIR.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    /// Signed integer value.
    Int(i128),
    /// Floating-point value.
    Real(f64),
    /// Boolean value.
    Bool(bool),
    /// Character/scalar value.
    Char(u32),
    /// String code units or scalars.
    String(Vec<u32>),
    /// Duration in nanoseconds.
    Time(i128),
    /// Date/time value with its category.
    Date {
        /// Category.
        kind: DateTy,
        /// Encoded value.
        value: i128,
    },
    /// Enumeration value.
    Enum {
        /// Enumeration type.
        def: DefId,
        /// Integer value.
        value: i128,
    },
    /// Null reference.
    Null,
}

/// A literal HIR expression payload.
#[derive(Clone, Debug, PartialEq)]
pub struct LiteralHir {
    /// Typed value.
    pub value: ConstValue,
}

/// A resolved place.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceHir {
    /// Local variable.
    Local(LocalId),
    /// Parameter.
    Param(ParamId),
    /// Global variable.
    Global(DefId),
    /// Function result (the function's own name).
    Result(DefId),
    /// `THIS`.
    This(DefId),
    /// Field or member of a value.
    Field {
        /// Base expression (a place).
        base: ExprId,
        /// Resolved field.
        field: FieldId,
    },
    /// Array or string subscript.
    Index {
        /// Base expression.
        base: ExprId,
        /// Subscripts.
        indices: Vec<ExprId>,
    },
    /// Dereference of a reference.
    Deref(ExprId),
    /// Partial bit access.
    BitField {
        /// Base expression.
        base: ExprId,
        /// Bit index.
        bit: u16,
    },
    /// Direct process-image variable.
    Direct(DirectAddress),
    /// A field path inside a configured program instance.
    InstanceField {
        /// Program type.
        program: DefId,
        /// Configuration instance name.
        instance: Symbol,
        /// Field path, outermost first.
        fields: Vec<FieldId>,
    },
}

/// A resolved callee.
#[derive(Clone, Debug, PartialEq)]
pub enum CalleeHir {
    /// User function.
    Function(DefId),
    /// Standard function signature.
    StdFunction(StdFnId),
    /// Explicit conversion (`X_TO_Y`).
    Conversion {
        /// Source type.
        from: TyId,
        /// Destination type.
        to: TyId,
    },
    /// Method call with an explicit receiver.
    Method {
        /// Receiver expression.
        receiver: ExprId,
        /// Resolved method.
        def: DefId,
    },
    /// `SUPER.method()` direct call to the base implementation.
    SuperMethod {
        /// Current instance.
        receiver: ExprId,
        /// Base method.
        def: DefId,
    },
    /// Interface method call through the interface vtable.
    InterfaceMethod {
        /// Receiver expression.
        receiver: ExprId,
        /// Interface type.
        interface: DefId,
        /// Prototype method.
        method: DefId,
    },
    /// Function-block invocation.
    Fb {
        /// Instance place.
        instance: ExprId,
        /// FB type.
        def: DefId,
    },
    /// `SUPER()` base FB body invocation on the current instance.
    SuperBody {
        /// Current instance.
        instance: ExprId,
        /// Base FB.
        base: DefId,
    },
}

/// One call argument.
#[derive(Clone, Debug)]
pub struct ArgHir {
    /// Formal parameter, when known.
    pub formal: Option<ParamId>,
    /// FB field bound to this argument, when known.
    pub field: Option<FieldId>,
    /// Direction.
    pub direction: ArgDirection,
    /// Value expression (absent for output-only bindings).
    pub value: Option<ExprId>,
    /// Output destination place.
    pub target: Option<ExprId>,
    /// `NOT` output binding.
    pub negated: bool,
    /// Source span.
    pub span: Span,
}

/// One expression.
#[derive(Clone, Debug)]
pub struct ExprHir {
    /// Expression kind.
    pub kind: ExprKindHir,
    /// Result type.
    pub ty: TyId,
    /// Source span.
    pub span: Span,
}

/// Expression payloads.
#[derive(Clone, Debug)]
pub enum ExprKindHir {
    /// Literal value.
    Literal(LiteralHir),
    /// Resolved place.
    Place(PlaceHir),
    /// Binary operation.
    Binary {
        /// Operator.
        op: BinOp,
        /// Left operand.
        lhs: ExprId,
        /// Right operand.
        rhs: ExprId,
    },
    /// Unary operation.
    Unary {
        /// Operator.
        op: UnOp,
        /// Operand.
        operand: ExprId,
    },
    /// Call.
    Call {
        /// Resolved callee.
        callee: CalleeHir,
        /// Arguments.
        args: Vec<ArgHir>,
    },
    /// Reference value to a place.
    Ref {
        /// Referenced place.
        target: ExprId,
    },
    /// `ADR(place)`.
    AddressOf {
        /// Addressed place.
        target: ExprId,
    },
    /// `SIZEOF(type | place)`.
    SizeOf {
        /// Resolved operand.
        operand: SizeOfHir,
    },
    /// Explicit `REF_TO T(expr)` / `POINTER TO T(expr)` conversion.
    PointerConversion {
        /// Converted expression.
        operand: ExprId,
        /// Result (or target) type.
        to: TyId,
        /// Written explicitly on the surface.
        kind: CastKind,
    },
    /// `NULL`.
    Null,
    /// Explicit or implicit conversion.
    Cast {
        /// Operand.
        operand: ExprId,
        /// Destination type.
        to: TyId,
        /// Conversion kind.
        kind: CastKind,
    },
    /// Array initializer.
    ArrayInit {
        /// Element initializers.
        elements: Vec<ExprId>,
    },
    /// Struct initializer.
    StructInit {
        /// Field initializers.
        fields: Vec<(FieldId, ExprId)>,
    },
    /// A recovered or invalid expression.
    Error,
}

/// The resolved operand of a `SIZEOF` expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeOfHir {
    /// A type operand.
    Type(TyId),
    /// A place operand.
    Place(ExprId),
}

/// Binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `MOD`.
    Mod,
    /// `**`.
    Power,
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `AND` / `&`.
    And,
    /// `OR`.
    Or,
    /// `XOR`.
    Xor,
}

impl BinOp {
    /// Returns the canonical source spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "MOD",
            Self::Power => "**",
            Self::Eq => "=",
            Self::Ne => "<>",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Xor => "XOR",
        }
    }

    /// Returns `true` for comparisons.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge
        )
    }
}

/// Unary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnOp {
    /// `+`.
    Plus,
    /// `-`.
    Neg,
    /// `NOT`.
    Not,
}

impl UnOp {
    /// Returns the canonical source spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plus => "+",
            Self::Neg => "-",
            Self::Not => "NOT",
        }
    }
}

/// Conversion category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CastKind {
    /// Inserted by the analyzer for an implicit conversion.
    Implicit,
    /// Written explicitly (conversion function).
    Explicit,
}

/// One statement.
#[derive(Clone, Debug)]
pub struct StmtHir {
    /// Statement kind.
    pub kind: StmtKindHir,
    /// Source span.
    pub span: Span,
}

/// Statement payloads.
#[derive(Clone, Debug)]
pub enum StmtKindHir {
    /// Value assignment.
    Assign {
        /// Destination place.
        place: ExprId,
        /// Value.
        value: ExprId,
    },
    /// Reference assignment.
    RefAssign {
        /// Destination reference place.
        place: ExprId,
        /// Reference value.
        value: ExprId,
    },
    /// Assignment attempt.
    Attempt {
        /// Destination.
        place: ExprId,
        /// Source.
        value: ExprId,
    },
    /// Call with discarded result.
    Call {
        /// Call expression.
        call: ExprId,
    },
    /// `RETURN`.
    Return,
    /// `IF`/`ELSIF`/`ELSE`.
    If {
        /// `(condition, body)` arms.
        arms: Vec<(ExprId, Vec<StmtId>)>,
        /// `ELSE` body.
        else_body: Vec<StmtId>,
    },
    /// `CASE`.
    Case {
        /// Selector.
        selector: ExprId,
        /// Selections.
        arms: Vec<CaseArmHir>,
        /// `ELSE` body.
        else_body: Vec<StmtId>,
    },
    /// `FOR`.
    For {
        /// Control variable place.
        var: ExprId,
        /// Start value.
        start: ExprId,
        /// Final value.
        end: ExprId,
        /// Step.
        step: ExprId,
        /// Body.
        body: Vec<StmtId>,
    },
    /// `WHILE`.
    While {
        /// Condition.
        cond: ExprId,
        /// Body.
        body: Vec<StmtId>,
    },
    /// `REPEAT`.
    Repeat {
        /// Body.
        body: Vec<StmtId>,
        /// Until condition.
        cond: ExprId,
    },
    /// `EXIT`.
    Exit,
    /// `CONTINUE`.
    Continue,
    /// Empty statement.
    Empty,
}

/// One `CASE` selection.
#[derive(Clone, Debug)]
pub struct CaseArmHir {
    /// Labels.
    pub labels: Vec<CaseLabelHir>,
    /// Body.
    pub body: Vec<StmtId>,
}

/// One `CASE` label or range.
#[derive(Clone, Debug)]
pub struct CaseLabelHir {
    /// Lower bound.
    pub lower: ConstValue,
    /// Upper bound for a range.
    pub upper: Option<ConstValue>,
    /// Source span.
    pub span: Span,
}

/// A `VAR_ACCESS` declaration.
#[derive(Clone, Debug)]
pub struct VarAccessHir {
    /// Access name.
    pub name: Symbol,
    /// Resolved place.
    pub place: ExprId,
    /// Declared type.
    pub ty: TyId,
    /// `READ_ONLY` marker.
    pub read_only: bool,
    /// Source span.
    pub span: Span,
}

/// A resolved configuration.
#[derive(Clone, Debug)]
pub struct ConfigurationHir {
    /// Configuration name.
    pub name: Symbol,
    /// Resources (or the implicit single resource).
    pub resources: Vec<ResourceHir>,
    /// Configuration-level `VAR_ACCESS`.
    pub var_access: Vec<VarAccessHir>,
    /// `VAR_CONFIG` declarations.
    pub config_inits: Vec<ConfigInitHir>,
    /// Source span.
    pub span: Span,
}

/// A resource.
#[derive(Clone, Debug)]
pub struct ResourceHir {
    /// Resource name.
    pub name: Symbol,
    /// `ON <name>` processor/host.
    pub on: Symbol,
    /// Tasks.
    pub tasks: Vec<TaskHir>,
    /// Program instances.
    pub programs: Vec<ProgramInstanceHir>,
    /// Resource-level `VAR_ACCESS`.
    pub var_access: Vec<VarAccessHir>,
}

/// A task configuration.
#[derive(Clone, Debug)]
pub struct TaskHir {
    /// Task name.
    pub name: Symbol,
    /// `SINGLE :=` value.
    pub single: Option<ExprId>,
    /// `INTERVAL :=` value.
    pub interval: Option<ExprId>,
    /// `PRIORITY :=` value.
    pub priority: Option<ExprId>,
    /// Folded `SINGLE` value.
    pub single_value: Option<bool>,
    /// Folded `INTERVAL` duration in nanoseconds.
    pub interval_ns: Option<i128>,
    /// Folded `PRIORITY` value.
    pub priority_value: Option<i128>,
    /// Source span.
    pub span: Span,
}

/// A program instance.
#[derive(Clone, Debug)]
pub struct ProgramInstanceHir {
    /// Instance name.
    pub name: Symbol,
    /// Program type.
    pub program: DefId,
    /// Task binding from `WITH`.
    pub task: Option<Symbol>,
    /// Input/output bindings.
    pub bindings: Vec<BindingHir>,
    /// Source span.
    pub span: Span,
}

/// One program-instance binding.
#[derive(Clone, Debug)]
pub struct BindingHir {
    /// Destination place.
    pub target: ExprId,
    /// Source expression.
    pub source: ExprId,
    /// `=>` marker (output binding).
    pub output: bool,
    /// Source span.
    pub span: Span,
}

/// One `VAR_CONFIG` instance initializer.
#[derive(Clone, Debug)]
pub struct ConfigInitHir {
    /// Program type.
    pub program: DefId,
    /// Configuration instance name.
    pub instance: Symbol,
    /// Field path, outermost first.
    pub fields: Vec<FieldId>,
    /// Optional `AT` override.
    pub at: Option<DirectAddress>,
    /// Declared type.
    pub ty: TyId,
    /// Initializer.
    pub init: Option<ExprId>,
    /// Source span.
    pub span: Span,
}

/// Standard-function behavior category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StdFnKind {
    /// `ABS`.
    Abs,
    /// `SQRT`.
    Sqrt,
    /// `LN`.
    Ln,
    /// `LOG`.
    Log,
    /// `EXP`.
    Exp,
    /// `SIN`.
    Sin,
    /// `COS`.
    Cos,
    /// `TAN`.
    Tan,
    /// `ASIN`.
    Asin,
    /// `ACOS`.
    Acos,
    /// `ATAN`.
    Atan,
    /// `ATAN2`.
    Atan2,
    /// `EXPT`.
    Expt,
    /// `TRUNC`.
    Trunc,
    /// `ADD`.
    Add,
    /// `SUB`.
    Sub,
    /// `MUL`.
    Mul,
    /// `DIV`.
    Div,
    /// `MOD`.
    Mod,
    /// `MOVE`.
    Move,
    /// `SHL`.
    Shl,
    /// `SHR`.
    Shr,
    /// `ROL`.
    Rol,
    /// `ROR`.
    Ror,
    /// `AND`.
    And,
    /// `OR`.
    Or,
    /// `XOR`.
    Xor,
    /// `NOT`.
    Not,
    /// `SEL`.
    Sel,
    /// `MAX`.
    Max,
    /// `MIN`.
    Min,
    /// `LIMIT`.
    Limit,
    /// `MUX`.
    Mux,
    /// `GT`.
    Gt,
    /// `GE`.
    Ge,
    /// `EQ`.
    Eq,
    /// `LE`.
    Le,
    /// `LT`.
    Lt,
    /// `NE`.
    Ne,
    /// `LEN`.
    Len,
    /// `LEN_MAX`.
    LenMax,
    /// `LEN_CODE_UNIT`.
    LenCodeUnit,
    /// `LEFT`.
    Left,
    /// `RIGHT`.
    Right,
    /// `MID`.
    Mid,
    /// `CONCAT`.
    Concat,
    /// `INSERT`.
    Insert,
    /// `DELETE`.
    Delete,
    /// `REPLACE`.
    Replace,
    /// `FIND`.
    Find,
    /// `STC_ASSERT` (test-only prelude).
    Assert,
    /// `STC_CLOCK_ADVANCE` (test-only prelude).
    ClockAdvance,
    /// `STC_IO_SET_INPUT` (test-only prelude).
    IoSetInput,
    /// `STC_IO_GET_OUTPUT` (test-only prelude).
    IoGetOutput,
}

impl StdFnKind {
    /// Returns `true` for the test-only prelude functions.
    ///
    /// They resolve only in test builds and only inside test-only units
    /// (Testing and Verification §2.2, §2.5).
    #[must_use]
    pub const fn is_test_prelude(self) -> bool {
        matches!(
            self,
            Self::Assert | Self::ClockAdvance | Self::IoSetInput | Self::IoGetOutput
        )
    }

    /// Returns the canonical name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Abs => "ABS",
            Self::Sqrt => "SQRT",
            Self::Ln => "LN",
            Self::Log => "LOG",
            Self::Exp => "EXP",
            Self::Sin => "SIN",
            Self::Cos => "COS",
            Self::Tan => "TAN",
            Self::Asin => "ASIN",
            Self::Acos => "ACOS",
            Self::Atan => "ATAN",
            Self::Atan2 => "ATAN2",
            Self::Expt => "EXPT",
            Self::Trunc => "TRUNC",
            Self::Add => "ADD",
            Self::Sub => "SUB",
            Self::Mul => "MUL",
            Self::Div => "DIV",
            Self::Mod => "MOD",
            Self::Move => "MOVE",
            Self::Shl => "SHL",
            Self::Shr => "SHR",
            Self::Rol => "ROL",
            Self::Ror => "ROR",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Xor => "XOR",
            Self::Not => "NOT",
            Self::Sel => "SEL",
            Self::Max => "MAX",
            Self::Min => "MIN",
            Self::Limit => "LIMIT",
            Self::Mux => "MUX",
            Self::Gt => "GT",
            Self::Ge => "GE",
            Self::Eq => "EQ",
            Self::Le => "LE",
            Self::Lt => "LT",
            Self::Ne => "NE",
            Self::Len => "LEN",
            Self::LenMax => "LEN_MAX",
            Self::LenCodeUnit => "LEN_CODE_UNIT",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Mid => "MID",
            Self::Concat => "CONCAT",
            Self::Insert => "INSERT",
            Self::Delete => "DELETE",
            Self::Replace => "REPLACE",
            Self::Find => "FIND",
            Self::Assert => "STC_ASSERT",
            Self::ClockAdvance => "STC_CLOCK_ADVANCE",
            Self::IoSetInput => "STC_IO_SET_INPUT",
            Self::IoGetOutput => "STC_IO_GET_OUTPUT",
        }
    }
}

/// One parameter of a standard function signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdParam {
    /// Parameter name.
    pub name: &'static str,
    /// Accepted generic category.
    pub generic: GenericKind,
    /// Variadic tail marker (must be last).
    pub variadic: bool,
    /// Whether the parameter is an output bound with `=>`.
    pub output: bool,
}

/// Standard-function result rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdResult {
    /// No result.
    Void,
    /// Same type as the given argument index.
    SameAs(usize),
    /// A concrete builtin type selected by the resolver.
    Concrete(StdRetType),
    /// The string type of argument 0 (string functions).
    StringOf(usize),
    /// `DINT`.
    Dint,
    /// `BOOL`.
    Bool,
    /// `REAL`.
    Real,
    /// `LREAL` when any argument is `LREAL`, otherwise `REAL`.
    RealOfArgs,
}

/// Concrete standard-function result categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdRetType {
    /// `BOOL`.
    Bool,
    /// `DINT`.
    Dint,
    /// `REAL`.
    Real,
    /// `LREAL`.
    Lreal,
}

/// A standard-function signature.
#[derive(Clone, Debug)]
pub struct StdFunction {
    /// Name.
    pub name: Symbol,
    /// Behavior category.
    pub kind: StdFnKind,
    /// Parameters.
    pub params: Vec<StdParam>,
    /// Result rule.
    pub result: StdResult,
}

/// One package.
#[derive(Clone, Debug)]
pub struct PackageInfo {
    /// Package ID.
    pub id: PackageId,
    /// Manifest name (the M3 session uses `_root`).
    pub name: String,
    /// Root namespace name.
    pub root_namespace: String,
}

/// The resolved HIR for one build.
#[derive(Debug)]
pub struct HirCrate {
    /// Interned identifiers.
    pub interner: Interner,
    /// Canonical types.
    pub types: TyArena,
    /// Definitions.
    pub defs: Vec<Def>,
    /// Fields.
    pub fields: Vec<FieldHir>,
    /// Parameters.
    pub params: Vec<ParamHir>,
    /// Locals.
    pub locals: Vec<LocalHir>,
    /// Bodies.
    pub bodies: Vec<Body>,
    /// Expressions.
    pub exprs: Vec<ExprHir>,
    /// Statements.
    pub stmts: Vec<StmtHir>,
    /// Packages.
    pub packages: Vec<PackageInfo>,
    /// The root package whose configurations and programs select the build.
    pub root_package: PackageId,
    /// Standard-function signatures.
    pub std_fns: Vec<StdFunction>,
    /// Selected configuration, when exactly one exists and no manifest
    /// overrides it.
    pub entry_configuration: Option<DefId>,
}

impl HirCrate {
    /// Creates an empty crate with the builtin type arena.
    #[must_use]
    pub fn new(package_name: impl Into<String>, root_namespace: impl Into<String>) -> Self {
        Self::with_packages(
            vec![PackageInfo {
                id: PackageId::new(0),
                name: package_name.into(),
                root_namespace: root_namespace.into(),
            }],
            PackageId::new(0),
        )
    }

    /// Creates an empty crate for a merged multi-package build.
    ///
    /// `packages` are in package order (dependencies before dependents) and
    /// every [`PackageInfo::id`] must equal its index; `root_package` selects
    /// the package whose `CONFIGURATION` and `PROGRAM` declarations provide
    /// the build entry point.
    #[must_use]
    pub fn with_packages(packages: Vec<PackageInfo>, root_package: PackageId) -> Self {
        Self {
            interner: Interner::new(),
            types: TyArena::new(),
            defs: Vec::new(),
            fields: Vec::new(),
            params: Vec::new(),
            locals: Vec::new(),
            bodies: Vec::new(),
            exprs: Vec::new(),
            stmts: Vec::new(),
            packages,
            root_package,
            std_fns: Vec::new(),
            entry_configuration: None,
        }
    }

    /// Returns the root package.
    ///
    /// # Panics
    ///
    /// Panics if the crate has no packages.
    #[must_use]
    pub fn root_package(&self) -> &PackageInfo {
        &self.packages[self.root_package.index()]
    }

    /// Interns an identifier.
    pub fn intern(&mut self, text: &str) -> Symbol {
        self.interner.intern(text)
    }

    /// Allocates a definition.
    pub fn alloc_def(
        &mut self,
        kind: DefKind,
        name: Option<Symbol>,
        span: Span,
        parent: Option<DefId>,
        visibility: Visibility,
    ) -> DefId {
        let raw = u32::try_from(self.defs.len()).expect("definition limit exceeded");
        let id = DefId::new(raw);
        self.defs.push(Def {
            kind,
            name,
            span,
            parent,
            package: PackageId::new(0),
            visibility,
            test_only: false,
            export: false,
            export_name: None,
            no_mangle: false,
            using: Vec::new(),
        });
        id
    }

    /// Allocates a field.
    pub fn alloc_field(&mut self, field: FieldHir) -> FieldId {
        let raw = u32::try_from(self.fields.len()).expect("field limit exceeded");
        let id = FieldId::new(raw);
        self.fields.push(field);
        id
    }

    /// Allocates a parameter.
    pub fn alloc_param(&mut self, param: ParamHir) -> ParamId {
        let raw = u32::try_from(self.params.len()).expect("parameter limit exceeded");
        let id = ParamId::new(raw);
        self.params.push(param);
        id
    }

    /// Allocates a local.
    pub fn alloc_local(&mut self, local: LocalHir) -> LocalId {
        let raw = u32::try_from(self.locals.len()).expect("local limit exceeded");
        let id = LocalId::new(raw);
        self.locals.push(local);
        id
    }

    /// Allocates a body.
    pub fn alloc_body(&mut self, body: Body) -> BodyId {
        let raw = u32::try_from(self.bodies.len()).expect("body limit exceeded");
        let id = BodyId::new(raw);
        self.bodies.push(body);
        id
    }

    /// Allocates an expression.
    pub fn alloc_expr(&mut self, kind: ExprKindHir, ty: TyId, span: Span) -> ExprId {
        let raw = u32::try_from(self.exprs.len()).expect("expression limit exceeded");
        let id = ExprId::new(raw);
        self.exprs.push(ExprHir { kind, ty, span });
        id
    }

    /// Allocates a statement.
    pub fn alloc_stmt(&mut self, kind: StmtKindHir, span: Span) -> StmtId {
        let raw = u32::try_from(self.stmts.len()).expect("statement limit exceeded");
        let id = StmtId::new(raw);
        self.stmts.push(StmtHir { kind, span });
        id
    }

    /// Returns a definition by ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not in this crate.
    #[must_use]
    pub fn def(&self, id: DefId) -> &Def {
        &self.defs[id.index()]
    }

    /// Returns the name of a definition, or `""` for anonymous definitions.
    #[must_use]
    pub fn def_name(&self, id: DefId) -> &str {
        self.defs[id.index()].name_text(&self.interner)
    }

    /// Interning helper for callers that already hold `&mut self`.
    pub fn symbol(&mut self, text: &str) -> Symbol {
        self.interner.intern(text)
    }

    /// Returns the nominal type ID for a definition.
    pub fn named_type(&mut self, def: DefId) -> TyId {
        self.types.intern(Ty::Named(def))
    }

    /// Returns the expression for an ID.
    #[must_use]
    pub fn expr(&self, id: ExprId) -> &ExprHir {
        &self.exprs[id.index()]
    }

    /// Returns the statement for an ID.
    #[must_use]
    pub fn stmt(&self, id: StmtId) -> &StmtHir {
        &self.stmts[id.index()]
    }

    /// Returns the field for an ID.
    #[must_use]
    pub fn field(&self, id: FieldId) -> &FieldHir {
        &self.fields[id.index()]
    }

    /// Returns the parameter for an ID.
    #[must_use]
    pub fn param(&self, id: ParamId) -> &ParamHir {
        &self.params[id.index()]
    }

    /// Returns the local for an ID.
    #[must_use]
    pub fn local(&self, id: LocalId) -> &LocalHir {
        &self.locals[id.index()]
    }

    /// Returns a human-readable type name.
    #[must_use]
    pub fn type_name(&self, ty: TyId) -> String {
        let mut out = String::new();
        self.write_type(ty, &mut out, 0);
        out
    }

    fn write_type(&self, ty: TyId, out: &mut String, depth: usize) {
        if depth > 24 {
            out.push_str("...");
            return;
        }
        match self.types.get(ty) {
            Ty::Error => out.push_str("<error>"),
            Ty::Bool => out.push_str("BOOL"),
            Ty::Int(int) => out.push_str(int_name(*int)),
            Ty::Real(real) => out.push_str(match real {
                crate::RealTy::Real => "REAL",
                crate::RealTy::LReal => "LREAL",
            }),
            Ty::BitString(8) => out.push_str("BYTE"),
            Ty::BitString(16) => out.push_str("WORD"),
            Ty::BitString(32) => out.push_str("DWORD"),
            Ty::BitString(64) => out.push_str("LWORD"),
            Ty::BitString(width) => {
                out.push_str("BITSTRING");
                out.push_str(&width.to_string());
            }
            Ty::Time(time) => out.push_str(match time {
                crate::TimeTy::Time => "TIME",
                crate::TimeTy::LTime => "LTIME",
            }),
            Ty::Date(date) => out.push_str(date_name(*date)),
            Ty::Char(ch) => out.push_str(match ch {
                crate::CharTy::Char => "CHAR",
                crate::CharTy::WChar => "WCHAR",
                crate::CharTy::UChar => "UCHAR",
            }),
            Ty::String(string) => {
                out.push_str(match string.kind {
                    crate::StringKind::Sbcs => "STRING",
                    crate::StringKind::Dbcs => "WSTRING",
                    crate::StringKind::Unicode => "USTRING",
                });
                out.push('[');
                out.push_str(&string.capacity.to_string());
                out.push(']');
            }
            Ty::Array(array) => {
                out.push_str("ARRAY [");
                for (index, bound) in array.bounds.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    if array.conformant {
                        out.push('*');
                    } else {
                        out.push_str(&bound.lower.to_string());
                        out.push_str("..");
                        out.push_str(&bound.upper.to_string());
                    }
                }
                out.push_str("] OF ");
                self.write_type(array.element, out, depth + 1);
            }
            Ty::Ref(reference) => {
                out.push_str("REF_TO ");
                self.write_type(reference.target, out, depth + 1);
            }
            Ty::Ptr(pointer) => {
                out.push_str("POINTER TO ");
                self.write_type(pointer.target, out, depth + 1);
            }
            Ty::Named(def) => {
                let def = &self.defs[def.index()];
                if let Some(name) = def.name {
                    out.push_str(self.interner.text(name));
                } else {
                    out.push_str(anonymous_name(&def.kind));
                }
            }
            Ty::Null => out.push_str("NULL"),
            Ty::Generic(generic) => out.push_str(generic.as_str()),
        }
    }

    /// Checks arena and reference invariants (used by the fuzz target and tests).
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated invariant.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.packages.is_empty() {
            return Err("HIR crate has no packages".into());
        }
        if self.root_package.index() >= self.packages.len() {
            return Err(format!(
                "root package {} out of range",
                self.root_package.raw()
            ));
        }
        for (index, package) in self.packages.iter().enumerate() {
            if package.id.index() != index {
                return Err(format!(
                    "package {} has mismatched id {}",
                    index,
                    package.id.raw()
                ));
            }
        }
        for (index, def) in self.defs.iter().enumerate() {
            if def.package.index() >= self.packages.len() {
                return Err(format!(
                    "definition {index} has out-of-range package {}",
                    def.package.raw()
                ));
            }
        }
        for (_, ty) in self.types.iter() {
            match ty {
                Ty::Ref(reference) => self.check_ty_id(reference.target)?,
                Ty::Ptr(pointer) => self.check_ty_id(pointer.target)?,
                Ty::Array(array) => self.check_ty_id(array.element)?,
                _ => {}
            }
        }
        for def in &self.defs {
            for &field in def.field_ids() {
                self.check_field_id(field)?;
            }
            for &method in def.method_ids() {
                self.check_def_id(method)?;
            }
            for &interface in def.interface_ids() {
                self.check_def_id(interface)?;
            }
            if let Some(base) = def.base() {
                self.check_def_id(base)?;
            }
            match &def.kind {
                DefKind::Alias { target } => self.check_ty_id(*target)?,
                DefKind::Subrange { base, .. } => self.check_ty_id(*base)?,
                DefKind::Enum { base, .. } | DefKind::NamedValue { base, .. } => {
                    self.check_ty_id(*base)?;
                }
                DefKind::Function {
                    result,
                    params,
                    body,
                } => {
                    if let Some(result) = result {
                        self.check_ty_id(*result)?;
                    }
                    for &param in params {
                        self.check_param_id(param)?;
                    }
                    if let Some(body) = body {
                        self.check_body_id(*body)?;
                    }
                }
                DefKind::FunctionBlock {
                    fields,
                    methods,
                    body,
                    vtable,
                    ..
                } => {
                    for &field in fields {
                        self.check_field_id(field)?;
                    }
                    for &method in methods.iter().chain(vtable.iter()) {
                        self.check_def_id(method)?;
                    }
                    if let Some(body) = body {
                        self.check_body_id(*body)?;
                    }
                }
                DefKind::Class {
                    fields,
                    methods,
                    vtable,
                    ..
                } => {
                    for &field in fields {
                        self.check_field_id(field)?;
                    }
                    for &method in methods.iter().chain(vtable.iter()) {
                        self.check_def_id(method)?;
                    }
                }
                DefKind::Program {
                    fields,
                    body,
                    var_access,
                } => {
                    for &field in fields {
                        self.check_field_id(field)?;
                    }
                    if let Some(body) = body {
                        self.check_body_id(*body)?;
                    }
                    for access in var_access {
                        self.check_expr_id(access.place)?;
                        self.check_ty_id(access.ty)?;
                    }
                }
                DefKind::Interface {
                    extends,
                    methods,
                    vtable,
                } => {
                    for &interface in extends {
                        self.check_def_id(interface)?;
                    }
                    for &method in methods.iter().chain(vtable.iter()) {
                        self.check_def_id(method)?;
                    }
                }
                DefKind::Method {
                    owner,
                    result,
                    params,
                    body,
                    ..
                } => {
                    self.check_def_id(*owner)?;
                    if let Some(result) = result {
                        self.check_ty_id(*result)?;
                    }
                    for &param in params {
                        self.check_param_id(param)?;
                    }
                    if let Some(body) = body {
                        self.check_body_id(*body)?;
                    }
                }
                DefKind::Global { ty, init, .. } => {
                    self.check_ty_id(*ty)?;
                    if let Some(init) = init {
                        self.check_expr_id(*init)?;
                    }
                }
                DefKind::Struct { fields, .. } => {
                    for &field in fields {
                        self.check_field_id(field)?;
                    }
                }
                DefKind::Configuration { config } => {
                    for access in &config.var_access {
                        self.check_expr_id(access.place)?;
                        self.check_ty_id(access.ty)?;
                    }
                    for init in &config.config_inits {
                        self.check_def_id(init.program)?;
                        for &field in &init.fields {
                            self.check_field_id(field)?;
                        }
                        self.check_ty_id(init.ty)?;
                        if let Some(init) = init.init {
                            self.check_expr_id(init)?;
                        }
                    }
                    for resource in &config.resources {
                        for access in &resource.var_access {
                            self.check_expr_id(access.place)?;
                            self.check_ty_id(access.ty)?;
                        }
                        for program in &resource.programs {
                            self.check_def_id(program.program)?;
                            for binding in &program.bindings {
                                self.check_expr_id(binding.target)?;
                                self.check_expr_id(binding.source)?;
                            }
                        }
                    }
                }
                DefKind::Namespace { .. } => {}
            }
        }
        for field in &self.fields {
            self.check_ty_id(field.ty)?;
            self.check_def_id(field.owner)?;
            if let Some(init) = field.init {
                self.check_expr_id(init)?;
            }
        }
        for param in &self.params {
            self.check_ty_id(param.ty)?;
            self.check_def_id(param.owner)?;
            if let Some(default) = param.default {
                self.check_expr_id(default)?;
            }
        }
        for local in &self.locals {
            self.check_ty_id(local.ty)?;
            self.check_def_id(local.owner)?;
            if let Some(init) = local.init {
                self.check_expr_id(init)?;
            }
            if let Some(target) = local.external_target {
                self.check_def_id(target)?;
            }
        }
        for (index, body) in self.bodies.iter().enumerate() {
            self.check_def_id(body.owner)?;
            for &param in &body.params {
                self.check_param_id(param)?;
            }
            for &local in &body.locals {
                self.check_local_id(local)?;
            }
            for &stmt in &body.stmts {
                self.check_stmt_id(stmt)?;
            }
            let _ = index;
        }
        for (index, expr) in self.exprs.iter().enumerate() {
            self.check_ty_id(expr.ty)?;
            self.check_expr_kind(&expr.kind)?;
            let _ = index;
        }
        for (index, stmt) in self.stmts.iter().enumerate() {
            self.check_stmt_kind(&stmt.kind)?;
            let _ = index;
        }
        Ok(())
    }

    fn check_ty_id(&self, id: TyId) -> Result<(), String> {
        if id.index() >= self.types.len() {
            return Err(format!("type {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_def_id(&self, id: DefId) -> Result<(), String> {
        if id.index() >= self.defs.len() {
            return Err(format!("definition {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_field_id(&self, id: FieldId) -> Result<(), String> {
        if id.index() >= self.fields.len() {
            return Err(format!("field {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_param_id(&self, id: ParamId) -> Result<(), String> {
        if id.index() >= self.params.len() {
            return Err(format!("parameter {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_local_id(&self, id: LocalId) -> Result<(), String> {
        if id.index() >= self.locals.len() {
            return Err(format!("local {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_body_id(&self, id: BodyId) -> Result<(), String> {
        if id.index() >= self.bodies.len() {
            return Err(format!("body {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_expr_id(&self, id: ExprId) -> Result<(), String> {
        if id.index() >= self.exprs.len() {
            return Err(format!("expression {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_stmt_id(&self, id: StmtId) -> Result<(), String> {
        if id.index() >= self.stmts.len() {
            return Err(format!("statement {} out of range", id.raw()));
        }
        Ok(())
    }

    fn check_expr_kind(&self, kind: &ExprKindHir) -> Result<(), String> {
        match kind {
            ExprKindHir::Literal(literal) => {
                if let ConstValue::Enum { def, .. } = literal.value {
                    self.check_def_id(def)?;
                }
            }
            ExprKindHir::Place(place) => self.check_place(place)?,
            ExprKindHir::Binary { lhs, rhs, .. } => {
                self.check_expr_id(*lhs)?;
                self.check_expr_id(*rhs)?;
            }
            ExprKindHir::Unary { operand, .. } => self.check_expr_id(*operand)?,
            ExprKindHir::Call { callee, args } => {
                self.check_callee(callee)?;
                for arg in args {
                    if let Some(formal) = arg.formal {
                        self.check_param_id(formal)?;
                    }
                    if let Some(field) = arg.field {
                        self.check_field_id(field)?;
                    }
                    if let Some(value) = arg.value {
                        self.check_expr_id(value)?;
                    }
                    if let Some(target) = arg.target {
                        self.check_expr_id(target)?;
                    }
                }
            }
            ExprKindHir::Ref { target } => self.check_expr_id(*target)?,
            ExprKindHir::AddressOf { target } => self.check_expr_id(*target)?,
            ExprKindHir::SizeOf { operand } => match operand {
                SizeOfHir::Type(ty) => self.check_ty_id(*ty)?,
                SizeOfHir::Place(place) => self.check_expr_id(*place)?,
            },
            ExprKindHir::PointerConversion { operand, to, .. } => {
                self.check_expr_id(*operand)?;
                self.check_ty_id(*to)?;
            }
            ExprKindHir::Null | ExprKindHir::Error => {}
            ExprKindHir::Cast { operand, to, .. } => {
                self.check_expr_id(*operand)?;
                self.check_ty_id(*to)?;
            }
            ExprKindHir::ArrayInit { elements } => {
                for &element in elements {
                    self.check_expr_id(element)?;
                }
            }
            ExprKindHir::StructInit { fields } => {
                for (field, value) in fields {
                    self.check_field_id(*field)?;
                    self.check_expr_id(*value)?;
                }
            }
        }
        Ok(())
    }

    fn check_place(&self, place: &PlaceHir) -> Result<(), String> {
        match place {
            PlaceHir::Local(id) => self.check_local_id(*id),
            PlaceHir::Param(id) => self.check_param_id(*id),
            PlaceHir::Global(id) | PlaceHir::Result(id) | PlaceHir::This(id) => {
                self.check_def_id(*id)
            }
            PlaceHir::Field { base, field } => {
                self.check_expr_id(*base)?;
                self.check_field_id(*field)
            }
            PlaceHir::Index { base, indices } => {
                self.check_expr_id(*base)?;
                for &index in indices {
                    self.check_expr_id(index)?;
                }
                Ok(())
            }
            PlaceHir::Deref(expr) => self.check_expr_id(*expr),
            PlaceHir::BitField { base, .. } => self.check_expr_id(*base),
            PlaceHir::Direct(_) => Ok(()),
            PlaceHir::InstanceField {
                program, fields, ..
            } => {
                self.check_def_id(*program)?;
                for &field in fields {
                    self.check_field_id(field)?;
                }
                Ok(())
            }
        }
    }

    fn check_callee(&self, callee: &CalleeHir) -> Result<(), String> {
        match callee {
            CalleeHir::Function(id) => self.check_def_id(*id),
            CalleeHir::StdFunction(id) => {
                if id.index() >= self.std_fns.len() {
                    return Err(format!("std function {} out of range", id.raw()));
                }
                Ok(())
            }
            CalleeHir::Conversion { from, to } => {
                self.check_ty_id(*from)?;
                self.check_ty_id(*to)
            }
            CalleeHir::Method { receiver, def } | CalleeHir::SuperMethod { receiver, def } => {
                self.check_expr_id(*receiver)?;
                self.check_def_id(*def)
            }
            CalleeHir::InterfaceMethod {
                receiver,
                interface,
                method,
            } => {
                self.check_expr_id(*receiver)?;
                self.check_def_id(*interface)?;
                self.check_def_id(*method)
            }
            CalleeHir::Fb { instance, def }
            | CalleeHir::SuperBody {
                instance,
                base: def,
            } => {
                self.check_expr_id(*instance)?;
                self.check_def_id(*def)
            }
        }
    }

    fn check_stmt_kind(&self, kind: &StmtKindHir) -> Result<(), String> {
        match kind {
            StmtKindHir::Assign { place, value }
            | StmtKindHir::RefAssign { place, value }
            | StmtKindHir::Attempt { place, value } => {
                self.check_expr_id(*place)?;
                self.check_expr_id(*value)?;
            }
            StmtKindHir::Call { call } => self.check_expr_id(*call)?,
            StmtKindHir::If { arms, else_body } => {
                for (condition, body) in arms {
                    self.check_expr_id(*condition)?;
                    for &stmt in body {
                        self.check_stmt_id(stmt)?;
                    }
                }
                for &stmt in else_body {
                    self.check_stmt_id(stmt)?;
                }
            }
            StmtKindHir::Case {
                selector,
                arms,
                else_body,
            } => {
                self.check_expr_id(*selector)?;
                for arm in arms {
                    for &stmt in &arm.body {
                        self.check_stmt_id(stmt)?;
                    }
                }
                for &stmt in else_body {
                    self.check_stmt_id(stmt)?;
                }
            }
            StmtKindHir::For {
                var,
                start,
                end,
                step,
                body,
            } => {
                for expr in [var, start, end, step] {
                    self.check_expr_id(*expr)?;
                }
                for &stmt in body {
                    self.check_stmt_id(stmt)?;
                }
            }
            StmtKindHir::While { cond, body } => {
                self.check_expr_id(*cond)?;
                for &stmt in body {
                    self.check_stmt_id(stmt)?;
                }
            }
            StmtKindHir::Repeat { body, cond } => {
                for &stmt in body {
                    self.check_stmt_id(stmt)?;
                }
                self.check_expr_id(*cond)?;
            }
            StmtKindHir::Return
            | StmtKindHir::Exit
            | StmtKindHir::Continue
            | StmtKindHir::Empty => {}
        }
        Ok(())
    }
}

fn int_name(int: crate::IntTy) -> &'static str {
    match (int.signed, int.width) {
        (true, 8) => "SINT",
        (true, 16) => "INT",
        (true, 32) => "DINT",
        (true, 64) => "LINT",
        (false, 8) => "USINT",
        (false, 16) => "UINT",
        (false, 32) => "UDINT",
        (false, 64) => "ULINT",
        _ => "INT",
    }
}

fn date_name(date: DateTy) -> &'static str {
    match date {
        DateTy::Date => "DATE",
        DateTy::LDate => "LDATE",
        DateTy::Tod => "TOD",
        DateTy::LTod => "LTOD",
        DateTy::DateTime => "DT",
        DateTy::LDateTime => "LDT",
    }
}

fn anonymous_name(kind: &DefKind) -> &'static str {
    match kind {
        DefKind::Struct { .. } => "STRUCT",
        DefKind::Enum { .. } => "ENUM",
        DefKind::Subrange { .. } => "SUBRANGE",
        DefKind::NamedValue { .. } => "NAMED_VALUE",
        _ => "<anonymous>",
    }
}
