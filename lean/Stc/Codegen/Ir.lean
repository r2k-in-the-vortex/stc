/-
The core emitted-IR fragment and its well-formedness predicate (P0.4).

`IRFrag` covers the production module `stc-codegen` emits for the core subset:
type definitions, globals and constants, declarations, functions, basic blocks,
instructions, terminators, the slot table, and the function-address table.
`WellFormed` records type correctness, SSA/dominance, block terminators, `phi`
shape and predecessor agreement, GOT cell kinds, the grouped slot table, the
host ABI 2 production entry points, and the no-direct-root-or-callable,
pre-attach, local-global, and cell-write rules checked by the Rust verifier in
`crates/stc-codegen/src/verify.rs`.

The fragment is the debug-free production module form. Debug-info builds, test
instrumentation, coverage counters, checkpoint hooks, `switch`, floating point,
OOP descriptors/vtables, and the process image are outside it.

`FunctionFacts` mirrors the builder's recorded GOT facts. The Rust verifier
trusts those facts; deriving them from the textual IR is Phase 5's checker
obligation. P0.4 checks the facts and the intrinsic IR structure together.

The printer and parser are out of scope; the semantics is defined over the
in-memory fragment.
-/
namespace Stc.Codegen

/-! ## Types and signatures -/

/-- An IR type of the core fragment. `overflow n` is the `{ iN, i1 }` result of
an `llvm.*.with.overflow` intrinsic. `array` covers the function table, the
root-pointer arrays, and the descriptor, metadata, and retain blobs. -/
inductive IrType where
  /-- `void`. -/
  | void
  /-- An integer of the given width. -/
  | int (bits : Nat)
  /-- Opaque `ptr`. -/
  | ptr
  /-- The `{ iN, i1 }` overflow-intrinsic result. -/
  | overflow (bits : Nat)
  /-- A fixed array `[N x T]`. -/
  | array (elem : IrType) (count : Nat)
deriving Repr, DecidableEq

namespace IrType

/-- Whether the type is in the frozen core fragment. -/
def isCore : IrType → Bool
  | .void => true
  | .int bits => bits == 1 || bits == 8 || bits == 16 || bits == 32 || bits == 64
  | .ptr => true
  | .overflow bits => bits == 8 || bits == 16 || bits == 32 || bits == 64
  | .array elem _ => elem.isCore

/-- Whether the type is a core value type: core and not `void`. -/
def isValue : IrType → Bool
  | .void => false
  | ty => ty.isCore

/-- Whether the type is an integer. -/
def isInt : IrType → Bool
  | .int _ => true
  | _ => false

/-- The integer width, when the type is an integer. -/
def bits? : IrType → Option Nat
  | .int bits => some bits
  | _ => none

end IrType

/-- An exact function signature. -/
structure Signature where
  /-- Symbol name without `@`. -/
  name : String
  /-- Return type. -/
  returnType : IrType
  /-- Parameter types in declaration order. -/
  params : List IrType
deriving Repr, DecidableEq

/-! ## Operands and instructions -/

/-- An SSA local name. -/
abbrev Ssa := String

/-- A basic-block label. -/
abbrev Label := String

/-- One instruction operand. -/
inductive Operand where
  /-- An SSA local. -/
  | local (name : Ssa)
  /-- An integer literal of the given type. -/
  | intLit (ty : IrType) (value : Int)
  /-- A null pointer. -/
  | nullPtr
  /-- A module symbol reference. -/
  | symbol (name : String)
deriving Repr, DecidableEq

namespace Operand

/-- The local names used by an operand. -/
def localNames : Operand → List String
  | .local name => [name]
  | _ => []

end Operand

/-- Integer binary operators. -/
inductive IntBinOp where
  | add
  | sub
  | mul
  | sdiv
  | udiv
  | srem
  | urem
  | band
  | bor
  | bxor
  | shl
  | lshr
  | ashr
deriving Repr, DecidableEq

/-- Integer comparison predicates. -/
inductive ICmpPred where
  | eq
  | ne
  | sgt
  | sge
  | slt
  | sle
  | ugt
  | uge
  | ult
  | ule
deriving Repr, DecidableEq

/-- Integer cast operators. -/
inductive CastOp where
  | zext
  | sext
  | trunc
deriving Repr, DecidableEq

namespace CastOp

/-- The operand and result types satisfy the cast's width relation. -/
def Valid : CastOp → IrType → IrType → Prop
  | .zext, fromTy, toTy =>
      ∃ fromBits toBits, fromTy.bits? = some fromBits ∧
        toTy.bits? = some toBits ∧ fromBits < toBits
  | .sext, fromTy, toTy =>
      ∃ fromBits toBits, fromTy.bits? = some fromBits ∧
        toTy.bits? = some toBits ∧ fromBits < toBits
  | .trunc, fromTy, toTy =>
      ∃ fromBits toBits, fromTy.bits? = some fromBits ∧
        toTy.bits? = some toBits ∧ toBits < fromBits

end CastOp

/-- One `phi` incoming pair. -/
structure PhiIncoming where
  /-- The incoming value. -/
  value : Operand
  /-- The predecessor label. -/
  predecessor : Label
deriving Repr, DecidableEq

/-- One instruction of the core fragment. -/
inductive Instr where
  /-- `alloca T`. -/
  | alloca (dest : Ssa) (ty : IrType)
  /-- `load T, ptr P`. -/
  | load (dest : Ssa) (ty : IrType) (ptr : Operand) (align : Option Nat)
  /-- `store T V, ptr P`. -/
  | store (ty : IrType) (value : Operand) (ptr : Operand) (align : Option Nat)
  /-- `getelementptr T, ptr B, ...`. -/
  | gep (dest : Ssa) (pointee : IrType) (base : Operand)
      (indices : List (IrType × Operand)) (inbounds : Bool)
  /-- An integer binary operation. -/
  | binop (dest : Ssa) (op : IntBinOp) (ty : IrType) (lhs rhs : Operand)
  /-- An integer comparison. -/
  | icmp (dest : Ssa) (pred : ICmpPred) (ty : IrType) (lhs rhs : Operand)
  /-- `select i1 C, T A, T B`. -/
  | select (dest : Ssa) (ty : IrType) (cond thenValue elseValue : Operand)
  /-- An integer cast. -/
  | cast (dest : Ssa) (op : CastOp) (value : Operand) (fromTy toTy : IrType)
  /-- A `phi`. -/
  | phi (dest : Ssa) (ty : IrType) (incoming : List PhiIncoming)
  /-- A direct call to a module function, declaration, or intrinsic. -/
  | call (dest : Option Ssa) (ret : IrType) (callee : String) (args : List Operand)
  /-- An indirect call through a function or import cell value. -/
  | callPtr (dest : Option Ssa) (ret : IrType) (callee : Operand) (args : List Operand)
  /-- `extractvalue { iN, i1 } V, 0/1`. -/
  | extractValue (dest : Ssa) (aggregate : Operand) (index : Nat) (ty : IrType)
deriving Repr, DecidableEq

namespace Instr

/-- The defined local, when the instruction defines one. -/
def dest? : Instr → Option Ssa
  | .alloca dest _ => some dest
  | .load dest _ _ _ => some dest
  | .store .. => none
  | .gep dest .. => some dest
  | .binop dest .. => some dest
  | .icmp dest .. => some dest
  | .select dest .. => some dest
  | .cast dest .. => some dest
  | .phi dest .. => some dest
  | .call dest _ _ _ => dest
  | .callPtr dest _ _ _ => dest
  | .extractValue dest .. => some dest

/-- The result type, when the instruction produces a value. -/
def resultType? : Instr → Option IrType
  | .alloca _ _ => some .ptr
  | .load _ ty _ _ => some ty
  | .store .. => none
  | .gep .. => some .ptr
  | .binop _ _ ty _ _ => some ty
  | .icmp .. => some (.int 1)
  | .select _ ty _ _ _ => some ty
  | .cast _ _ _ _ toTy => some toTy
  | .phi _ ty _ => some ty
  | .call dest ret _ _ => if dest.isSome then (if ret = .void then none else some ret) else none
  | .callPtr dest ret _ _ => if dest.isSome then (if ret = .void then none else some ret) else none
  | .extractValue _ _ _ ty => some ty

/-- The local names read by the instruction. `phi` inputs are handled at the
predecessor edge by `Block.philist`. -/
def localUses : Instr → List String
  | .alloca .. => []
  | .load _ _ ptr _ => ptr.localNames
  | .store _ value ptr _ => value.localNames ++ ptr.localNames
  | .gep _ _ base indices _ => base.localNames ++ indices.flatMap (·.2.localNames)
  | .binop _ _ _ lhs rhs => lhs.localNames ++ rhs.localNames
  | .icmp _ _ _ lhs rhs => lhs.localNames ++ rhs.localNames
  | .select _ _ cond thenValue elseValue =>
      cond.localNames ++ thenValue.localNames ++ elseValue.localNames
  | .cast _ _ value _ _ => value.localNames
  | .phi .. => []
  | .call _ _ _ args => args.flatMap Operand.localNames
  | .callPtr _ _ callee args => callee.localNames ++ args.flatMap Operand.localNames
  | .extractValue _ aggregate _ _ => aggregate.localNames

/-- Whether the instruction is a `phi`. -/
def isPhi : Instr → Bool
  | .phi .. => true
  | _ => false

end Instr

/-- One terminator of the core fragment. -/
inductive Terminator where
  /-- `ret` / `ret T V`. -/
  | ret (value : Option Operand)
  /-- An unconditional branch. -/
  | br (target : Label)
  /-- A conditional branch. -/
  | condBr (cond : Operand) (thenTarget elseTarget : Label)
  /-- `unreachable`. -/
  | unreachable
deriving Repr, DecidableEq

/-- Linkage classes emitted by the builder. -/
inductive Linkage where
  | external
  | internal
  | hidden
deriving Repr, DecidableEq

/-- The first index at which a predicate holds. -/
def firstIndex? {α : Type} (p : α → Bool) : List α → Option Nat
  | [] => none
  | value :: rest => if p value then some 0 else (firstIndex? p rest).map (· + 1)

/-- Pointwise relation on two lists of equal length. -/
def listForall₂ {α β : Type} (rel : α → β → Prop) : List α → List β → Prop
  | [], [] => True
  | a :: as, b :: bs => rel a b ∧ listForall₂ rel as bs
  | _, _ => False

/-- Where an SSA pointer value comes from. -/
inductive PointerOrigin where
  /-- A host cell address or cell value. -/
  | cell (slot : Nat) (isValue : Bool)
  /-- A module-level symbol. -/
  | global (name : String)
  /-- Function code loaded from a VTABLE entry. -/
  | vtableCode
deriving Repr, DecidableEq

/-- The builder-recorded structural facts checked by `WellFormed`. -/
structure FunctionFacts where
  /-- Origin of each SSA pointer value that carries one. -/
  origins : List (String × PointerOrigin)
  /-- Direct calls by callee symbol. -/
  directCalls : List String
  /-- Indirect calls by callee SSA value. -/
  indirectCalls : List String
  /-- Loads and stores through a cell value as `(slot, isStore)`. -/
  memoryAccesses : List (Nat × Bool)
  /-- Stores through a cell address. -/
  cellWrites : List Nat
  /-- Loads and stores through a module global as `(symbol, isStore)`. -/
  memoryGlobals : List (String × Bool)
  /-- `@symbol` references in instructions and terminators. -/
  symbolRefs : List String

namespace FunctionFacts

/-- The origin of an SSA value, when recorded. -/
def originOf? (facts : FunctionFacts) (name : String) : Option PointerOrigin :=
  (facts.origins.find? (fun entry => entry.1 == name)).map (·.2)

/-- Every slot number the facts refer to. -/
def slotRefs (facts : FunctionFacts) : List Nat :=
  facts.origins.filterMap (fun entry =>
    match entry.2 with
    | .cell slot _ => some slot
    | _ => none) ++
  facts.memoryAccesses.map (·.1) ++ facts.cellWrites

end FunctionFacts

/-! ## Blocks and functions -/

/-- One basic block. The terminator is optional so `WellFormed` can state that
it exists; the builder always emits one. -/
structure Block where
  /-- Block label. -/
  label : Label
  /-- The instructions, including `phi`s at the front. -/
  instrs : List Instr
  /-- The terminator. -/
  terminator : Option Terminator
deriving Repr, DecidableEq

namespace Block

/-- The CFG successor labels. -/
def successors (block : Block) : List Label :=
  match block.terminator with
  | some (.br target) => [target]
  | some (.condBr _ thenTarget elseTarget) => [thenTarget, elseTarget]
  | _ => []

/-- The `phi` types and incoming lists, in block order. -/
def philist (block : Block) : List (IrType × List PhiIncoming) :=
  block.instrs.filterMap fun instr =>
    match instr with
    | .phi _ ty incoming => some (ty, incoming)
    | _ => none

/-- Every `phi` precedes every non-`phi` instruction. -/
def phisFirst (block : Block) : Prop :=
  ∃ phis rest, block.instrs = phis ++ rest ∧
    (∀ instr ∈ phis, instr.isPhi = true) ∧
    (∀ instr ∈ rest, instr.isPhi = false)

end Block

/-- One module function. -/
structure Function where
  /-- Symbol name without `@`. -/
  name : String
  /-- Linkage class. -/
  linkage : Linkage
  /-- Return type. -/
  returnType : IrType
  /-- Parameters as `(type, %name)`. -/
  params : List (IrType × String)
  /-- Basic blocks; the first is the entry. -/
  blocks : List Block
  /-- Builder-recorded GOT facts. -/
  facts : FunctionFacts

namespace Function

/-- Block labels in order. -/
def labels (fn : Function) : List Label :=
  fn.blocks.map (·.label)

/-- The entry label relation. -/
def Entry (fn : Function) (label : Label) : Prop :=
  ∃ block rest, fn.blocks = block :: rest ∧ block.label = label

/-- One CFG edge. -/
def Edge (fn : Function) (src dst : Label) : Prop :=
  ∃ block ∈ fn.blocks, block.label = src ∧ dst ∈ block.successors

/-- A CFG path, as the list of labels from the start to the end. -/
inductive Path (fn : Function) : Label → Label → List Label → Prop
  | nil {label : Label} (h : label ∈ fn.labels) : Path fn label label [label]
  | cons {src middle dst : Label} {rest : List Label}
      (edge : Function.Edge fn src middle) (path : Path fn middle dst rest) :
      Path fn src dst (src :: rest)

/-- `block` dominates `use` when every entry-to-use path passes through it. -/
def Dominates (fn : Function) (block use : Label) : Prop :=
  ∀ entry, fn.Entry entry → ∀ path, Function.Path fn entry use path → block ∈ path

/-- The signature of a function. -/
def signature (fn : Function) : Signature :=
  ⟨fn.name, fn.returnType, fn.params.map (·.1)⟩

/-- The local definitions, with their result types. -/
def localDefs (fn : Function) : List (String × IrType) :=
  fn.blocks.flatMap fun block =>
    block.instrs.filterMap fun instr =>
      match instr.dest?, instr.resultType? with
      | some dest, some ty => some (dest, ty)
      | _, _ => none

/-- The type of a local, searching parameters first and then definitions. -/
def localType? (fn : Function) (name : String) : Option IrType :=
  match fn.params.find? (fun param => param.2 == name) with
  | some param => some param.1
  | none => (fn.localDefs.find? (fun entry => entry.1 == name)).map (·.2)

/-- The block label and instruction index of a local definition. -/
def defSite? (fn : Function) (name : String) : Option (Label × Nat) :=
  (fn.blocks.filterMap fun block =>
    (firstIndex? (fun instr => instr.dest? == some name) block.instrs).map
      fun index => (block.label, index)).head?

/-- The block with the given label. -/
def block? (fn : Function) (label : Label) : Option Block :=
  fn.blocks.find? (fun block => block.label == label)

/-- The number of instructions in a block, or zero when it is absent. -/
def blockLength (fn : Function) (label : Label) : Nat :=
  match fn.block? label with
  | some block => block.instrs.length
  | none => 0

/-- A local use is a parameter or a definition that dominates it. -/
def DefinedAt (fn : Function) (name : String) (useBlock : Label) (useIndex : Nat) : Prop :=
  name ∈ fn.params.map (·.2) ∨
    ∃ defBlock defIndex, fn.defSite? name = some (defBlock, defIndex) ∧
      fn.Dominates defBlock useBlock ∧ (defBlock = useBlock → defIndex < useIndex)

/-- The CFG predecessor edges of a block, in block order. A repeated target
produces a repeated predecessor. -/
def predecessorEdges (fn : Function) (label : Label) : List Label :=
  fn.blocks.flatMap fun block =>
    (block.successors.filter (fun target => target == label)).map (fun _ => block.label)

/-- `phi` incoming predecessor lists match the block's CFG predecessors. -/
def phiPredecessorsMatch (fn : Function) (block : Block) : Prop :=
  ∀ entry ∈ block.philist, (entry.2.map (·.predecessor)).Perm (fn.predecessorEdges block.label)

/-- Every branch target is a block in the function. -/
def CfgClosed (fn : Function) : Prop :=
  ∀ target ∈ fn.blocks.flatMap Block.successors, target ∈ fn.labels

/-- The entry block has no `phi`s. -/
def entryHasNoPhis (fn : Function) : Prop :=
  match fn.blocks with
  | [] => True
  | block :: _ => block.philist = []

end Function

/-- The operand's type in the function's locals. -/
def operandTypeOf? (fn : Function) : Operand → Option IrType
  | .local name => fn.localType? name
  | .intLit ty _ => some ty
  | .nullPtr => some .ptr
  | .symbol _ => some .ptr

/-- The operand has the given type in the function's locals. -/
def operandHasType (fn : Function) (ty : IrType) (operand : Operand) : Prop :=
  operandTypeOf? fn operand = some ty

/-- The result of a call must be `void` exactly when no SSA name is defined. -/
def CallResultOK (dest : Option Ssa) (ret : IrType) : Prop :=
  match dest with
  | some _ => ret ≠ .void
  | none => ret = .void

/-- One external declaration (an LLVM intrinsic). -/
structure Declaration where
  /-- Symbol name without `@`. -/
  name : String
  /-- Return type. -/
  returnType : IrType
  /-- Parameter types in declaration order. -/
  params : List IrType

namespace Declaration

/-- The declaration's signature. -/
def signature (decl : Declaration) : Signature :=
  ⟨decl.name, decl.returnType, decl.params⟩

end Declaration

/-- The checked-arithmetic intrinsic signatures emitted by `stc-codegen`. -/
def overflowIntrinsics : List Signature :=
  let widths : List Nat := [8, 16, 32, 64]
  let ops : List String := ["sadd", "ssub", "smul", "uadd", "usub", "umul"]
  widths.flatMap fun width =>
    ops.map fun op =>
      { name := s!"llvm.{op}.with.overflow.i{width}"
        returnType := .overflow width
        params := [.int width, .int width] }

/-- The rotate intrinsic signatures emitted by `stc-codegen`. `BOOL` rotates at
`i1`, so the width set includes 1. -/
def rotateIntrinsics : List Signature :=
  let widths : List Nat := [1, 8, 16, 32, 64]
  widths.flatMap fun width =>
    ["fshl", "fshr"].map fun op =>
      { name := s!"llvm.{op}.i{width}"
        returnType := .int width
        params := [.int width, .int width, .int width] }

/-- Every intrinsic signature the fragment resolves a direct call against. -/
def intrinsicSignatures : List Signature :=
  overflowIntrinsics ++ rotateIntrinsics

/-- The type correctness of one instruction. -/
def Instr.WellTyped (env : List Signature) (fn : Function) : Instr → Prop
  | .alloca _ ty =>
      ty.isValue = true
  | .load _ ty ptr _ =>
      ty.isValue = true ∧ operandHasType fn .ptr ptr
  | .store ty value ptr _ =>
      ty.isValue = true ∧ operandHasType fn ty value ∧ operandHasType fn .ptr ptr
  | .gep _ pointee base indices _ =>
      pointee.isCore = true ∧ operandHasType fn .ptr base ∧
        ∀ index ∈ indices,
          (index.1 = .int 32 ∨ index.1 = .int 64) ∧
            operandHasType fn index.1 index.2
  | .binop _ _ ty lhs rhs =>
      ty.isInt = true ∧ operandHasType fn ty lhs ∧ operandHasType fn ty rhs
  | .icmp _ _ ty lhs rhs =>
      ty.isInt = true ∧ operandHasType fn ty lhs ∧ operandHasType fn ty rhs
  | .select _ ty cond thenValue elseValue =>
      ty.isValue = true ∧ operandHasType fn (.int 1) cond ∧
        operandHasType fn ty thenValue ∧ operandHasType fn ty elseValue
  | .cast _ op value fromTy toTy =>
      fromTy.isInt = true ∧ toTy.isInt = true ∧ operandHasType fn fromTy value ∧
        op.Valid fromTy toTy
  | .phi _ ty incoming =>
      ty.isValue = true ∧ ∀ inc ∈ incoming, operandHasType fn ty inc.value
  | .call dest ret callee args =>
      ret.isCore = true ∧ CallResultOK dest ret ∧
        ∃ sig ∈ env, sig.name = callee ∧ sig.returnType = ret ∧
          listForall₂ (fun arg ty => operandHasType fn ty arg) args sig.params
  | .callPtr dest ret callee args =>
      ret.isCore = true ∧ CallResultOK dest ret ∧ operandHasType fn .ptr callee ∧
        ∀ arg ∈ args, ∃ ty, operandHasType fn ty arg
  | .extractValue _ aggregate index ty =>
      ∃ bits, operandHasType fn (.overflow bits) aggregate ∧
        ((index = 0 ∧ ty = .int bits) ∨ (index = 1 ∧ ty = .int 1))

/-- The type correctness of one terminator. -/
def Terminator.WellTyped (fn : Function) (returnType : IrType) : Terminator → Prop
  | .ret none => returnType = .void
  | .ret (some value) => returnType ≠ .void ∧ operandHasType fn returnType value
  | .br _ => True
  | .condBr cond _ _ => operandHasType fn (.int 1) cond
  | .unreachable => True

/-- Type correctness of a whole function against the module's direct-call environment. -/
structure FunctionTypesCorrect (env : List Signature) (fn : Function) : Prop where
  /-- The function has at least one block. -/
  nonempty : fn.blocks ≠ []
  /-- The return type is a core type. -/
  returnCore : fn.returnType.isCore = true
  /-- Every parameter is a core value type. -/
  paramsCore : ∀ param ∈ fn.params, param.1.isValue = true
  /-- Block labels are unique. -/
  labelsUnique : fn.labels.Nodup
  /-- Branch targets are defined. -/
  cfgClosed : fn.CfgClosed
  /-- The entry block has no phis. -/
  entryNoPhis : fn.entryHasNoPhis
  /-- Every instruction is type-correct. -/
  instructions : ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs, instr.WellTyped env fn
  /-- Every terminator is type-correct. -/
  terminators : ∀ block ∈ fn.blocks, ∀ terminator,
    block.terminator = some terminator → terminator.WellTyped fn fn.returnType

/-- SSA uniqueness, dominance, and `phi`-edge definedness for one function. -/
structure FunctionUsesDefined (fn : Function) : Prop where
  /-- Parameter and destination names are jointly unique. -/
  allNodup :
    ((fn.params.map (·.2)) ++
      (fn.blocks.flatMap fun block => block.instrs.filterMap Instr.dest?)).Nodup
  /-- Every instruction operand is defined and dominates the use. -/
  uses : ∀ block ∈ fn.blocks, ∀ (index : Nat) (instr : Instr),
    block.instrs[index]? = some instr →
    ∀ name ∈ instr.localUses, fn.DefinedAt name block.label index
  /-- Every `phi` incoming value is defined on its predecessor edge. -/
  phiUses : ∀ block ∈ fn.blocks, ∀ entry ∈ block.philist, ∀ inc ∈ entry.2,
    ∀ name ∈ inc.value.localNames,
      fn.DefinedAt name inc.predecessor (fn.blockLength inc.predecessor)

/-! ## GOT cell kinds and the slot table -/

/-- One GOT cell kind (GOT Artifacts §7.1). -/
inductive CellKind where
  | data
  | function
  | descriptor
  | direct
  | importCell
deriving Repr, DecidableEq

namespace CellKind

/-- The canonical group index. -/
def groupIndex : CellKind → Nat
  | .data => 0
  | .function => 1
  | .descriptor => 2
  | .direct => 3
  | .importCell => 4

/-- Whether the kind holds persistent storage. -/
def isStorage : CellKind → Bool
  | .data | .direct => true
  | _ => false

/-- Whether the kind is callable. -/
def isCallable : CellKind → Bool
  | .function | .importCell => true
  | _ => false

end CellKind

/-- The slot table: one cell kind per slot. -/
abbrev SlotTable := List CellKind

namespace SlotTable

/-- The canonical kind grouping: kinds are non-decreasing in the order
`Data`, `Function`, `Descriptor`, `Direct`, `Import`. Identity sorting within a
group is Phase 3's `slots_canonical`. -/
def Canonical : SlotTable → Prop
  | [] => True
  | [_] => True
  | first :: second :: rest =>
      first.groupIndex ≤ second.groupIndex ∧ Canonical (second :: rest)

end SlotTable

/-! ## Module globals and the fragment -/

/-- One global initializer. -/
inductive GlobalInit where
  /-- An integer constant. -/
  | int (ty : IrType) (value : Int)
  /-- A null pointer. -/
  | null
  /-- A symbol reference. -/
  | symbol (name : String)
  /-- An opaque byte string. -/
  | bytes (data : ByteArray)
  /-- An array initializer. -/
  | array (elements : List GlobalInit)
  /-- `zeroinitializer`. -/
  | zeroinitializer

namespace GlobalInit

/-- The symbols referenced by an initializer. -/
def symbols : GlobalInit → List String
  | .symbol name => [name]
  | .array elements => elements.flatMap symbols
  | _ => []

/-- The initializer is type-correct for the global's type. -/
def WellTyped (ty : IrType) : GlobalInit → Prop
  | .int initTy _ => initTy = ty ∧ ty.isInt = true
  | .null => ty = .ptr
  | .symbol _ => ty = .ptr
  | .bytes data => ∃ count, ty = .array (.int 8) count ∧ data.size = count
  | .array elements => ∃ elem count, ty = .array elem count ∧
      elements.length = count ∧ ∀ element ∈ elements, element.WellTyped elem
  | .zeroinitializer => ty.isCore = true

end GlobalInit

/-- One module-level global. -/
structure Global where
  /-- Symbol name without `@`. -/
  name : String
  /-- Linkage class. -/
  linkage : Linkage
  /-- `true` for `constant`. -/
  constant : Bool
  /-- The global's type. -/
  ty : IrType
  /-- The initializer. -/
  init : GlobalInit

/-- One named type definition. Core production modules carry none. -/
structure TypeDef where
  /-- Type name without `%`. -/
  name : String
  /-- The defined type. -/
  body : IrType

/-- The whole emitted production module plus the slot and function tables. -/
structure IRFrag where
  /-- Named type definitions. -/
  typeDefs : List TypeDef
  /-- Module globals. -/
  globals : List Global
  /-- External declarations (LLVM intrinsics). -/
  declarations : List Declaration
  /-- Function definitions. -/
  functions : List Function
  /-- The per-generation slot table. -/
  slots : SlotTable
  /-- The function-address table, in descriptor order. -/
  functionTable : List String

/-- The module's name uniqueness and type-correctness invariants. -/
structure ModuleShapes (frag : IRFrag) : Prop where
  /-- Type-definition names are unique. -/
  typeDefNames : (frag.typeDefs.map (·.name)).Nodup
  /-- Type definitions are core types. -/
  typeDefsWellFormed : ∀ typeDef ∈ frag.typeDefs, typeDef.body.isCore = true
  /-- Global names are unique. -/
  globalNames : (frag.globals.map (·.name)).Nodup
  /-- Global initializers match their declared types. -/
  globalsWellFormed : ∀ global ∈ frag.globals, global.init.WellTyped global.ty
  /-- Declaration names are unique. -/
  declarationNames : (frag.declarations.map (·.name)).Nodup
  /-- Declarations are well-typed, known intrinsics. -/
  declarationsWellFormed : ∀ decl ∈ frag.declarations,
    decl.returnType.isCore = true ∧
      (∀ param ∈ decl.params, param.isCore = true) ∧
      decl.signature ∈ intrinsicSignatures
  /-- Function names are unique. -/
  functionNames : (frag.functions.map (·.name)).Nodup

namespace IRFrag

/-- The direct-call resolution environment. -/
def directCallEnv (frag : IRFrag) : List Signature :=
  frag.functions.map Function.signature ++
  frag.declarations.map Declaration.signature ++
  intrinsicSignatures

/-- The function with the given name. -/
def function? (frag : IRFrag) (name : String) : Option Function :=
  frag.functions.find? (fun fn => fn.name == name)

/-- Whether the fragment defines the named global. -/
def hasGlobal (frag : IRFrag) (name : String) : Prop :=
  ∃ global ∈ frag.globals, global.name = name

end IRFrag

/-! ## The build contract -/

/-- A required module entry point and its exact signature. -/
structure EntryPointSpec where
  /-- Symbol name without `@`. -/
  symbol : String
  /-- Return type. -/
  returnType : IrType
  /-- Parameter types in declaration order. -/
  params : List IrType
deriving Repr, DecidableEq

/-- A required descriptor blob. -/
structure DescriptorSpec where
  /-- Global name without `@`. -/
  symbol : String
  /-- Exact blob length in bytes. -/
  size : Nat

/-- A required function-address table. -/
structure FunctionTableSpec where
  /-- Global name without `@`. -/
  symbol : String
  /-- Exact entry count. -/
  count : Nat

/-- The per-build contract supplied to `WellFormed`. It mirrors the Rust
verifier's `Contract`; the slot table lives in `IRFrag` because P0.4 models it
as part of the fragment. -/
structure IRContract where
  /-- Persistent root symbols. -/
  roots : List String
  /-- User callables. -/
  functions : List String
  /-- Module-local globals that bodies may access directly. -/
  localGlobals : List String
  /-- Required entry points and exact signatures. -/
  entryPoints : List EntryPointSpec
  /-- Required descriptor blob. -/
  descriptor : Option DescriptorSpec
  /-- Required function-address table. -/
  functionTable : Option FunctionTableSpec

/-- The host ABI 2 module entry points and signatures (GOT Artifacts §7.3).
Production modules require minor 2.1; the signatures are unchanged through the
current minor 2.3. -/
def moduleEntryPoints : List EntryPointSpec :=
  let entry := fun symbol returnType params => ⟨symbol, returnType, params⟩
  let i32 := IrType.int 32
  let i64 := IrType.int 64
  let ptrTy := IrType.ptr
  [ entry "stc_module_descriptor" ptrTy [ptrTy]
  , entry "stc_module_functions" ptrTy [ptrTy]
  , entry "stc_module_attach" i32 [ptrTy, ptrTy, i32, i64]
  , entry "stc_module_prepare" i32 [i32, i64, ptrTy, i32]
  , entry "stc_module_cycle" i32 [i64]
  , entry "stc_module_shutdown" i32 [i32]
  , entry "stc_module_detach" .void [i64] ]

/-! ## Well-formedness -/

/-- One entry point exists with the specified signature and external linkage. -/
def EntryPointMatches (frag : IRFrag) (spec : EntryPointSpec) : Prop :=
  ∃ fn ∈ frag.functions, fn.name = spec.symbol ∧ fn.linkage = .external ∧
    fn.returnType = spec.returnType ∧ fn.params.map (·.1) = spec.params

/-- Every required entry point matches the contract. -/
def EntryPointsMatch (contract : IRContract) (frag : IRFrag) : Prop :=
  ∀ spec ∈ contract.entryPoints, EntryPointMatches frag spec

/-- A function table permits the function symbol it names. -/
def FunctionTableAllows (contract : IRContract) (global : Global) : Prop :=
  match contract.functionTable with
  | some spec => spec.symbol = global.name
  | none => False

/-- The no-direct-root-or-callable rules. -/
structure NoDirectRootOrCallable (contract : IRContract) (frag : IRFrag) : Prop where
  /-- Persistent roots are not defined as module globals. -/
  rootsNotDefined : ∀ root ∈ contract.roots, ¬ frag.hasGlobal root
  /-- Bodies do not name roots or user callables directly. -/
  bodySymbols : ∀ fn ∈ frag.functions, ∀ symbol ∈ fn.facts.symbolRefs,
    symbol ∉ contract.roots ∧ symbol ∉ contract.functions
  /-- Bodies do not call user callables directly. -/
  directCalls : ∀ fn ∈ frag.functions, ∀ callee ∈ fn.facts.directCalls,
    callee ∉ contract.functions
  /-- Global initializers do not name roots or callables; only the function
  table names user callables. -/
  initializerSymbols : ∀ global ∈ frag.globals, ∀ symbol ∈ global.init.symbols,
    symbol ∉ contract.roots ∧
      (symbol ∈ contract.functions → FunctionTableAllows contract global)

/-- The pre-attach accessors do not use the slot table or make indirect calls. -/
def PreAttachClean (_contract : IRContract) (frag : IRFrag) : Prop :=
  ∀ name ∈ ["stc_module_descriptor", "stc_module_functions"],
    ∀ fn ∈ frag.functions, fn.name = name →
      (∀ entry ∈ fn.facts.origins,
        match entry.2 with
        | .cell _ _ => False
        | _ => True) ∧
      fn.facts.indirectCalls = []

/-- Direct global loads and stores target only the contract allowlist. -/
def LocalGlobalsAllowed (contract : IRContract) (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ entry ∈ fn.facts.memoryGlobals,
    entry.1 ∈ contract.localGlobals

/-- No function stores through a GOT cell address. -/
def CellsNotWritten (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, fn.facts.cellWrites = []

/-- Every slot reference is within the slot table. -/
def SlotRefsInRange (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ slot ∈ fn.facts.slotRefs, slot < frag.slots.length

/-- Every cell use matches the recorded kind. -/
def SlotRefsKindCorrect (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions,
    (∀ entry ∈ fn.facts.memoryAccesses,
      ∃ kind, frag.slots[entry.1]? = some kind ∧ kind.isStorage = true) ∧
    (∀ callee ∈ fn.facts.indirectCalls,
      match fn.facts.originOf? callee with
      | some (.cell slot true) =>
          ∃ kind, frag.slots[slot]? = some kind ∧ kind.isCallable = true
      | some .vtableCode => True
      | _ => False)

/-- The descriptor blob and function table have the contract's shapes and the
table entries match the fragment's function-table list. -/
def DescriptorAndTable (contract : IRContract) (frag : IRFrag) : Prop :=
  (∀ spec, contract.descriptor = some spec →
    ∃ global ∈ frag.globals, global.name = spec.symbol ∧ global.constant = true ∧
      global.linkage ≠ .external ∧ global.ty = .array (.int 8) spec.size) ∧
  (∀ spec, contract.functionTable = some spec →
    ∃ global ∈ frag.globals, global.name = spec.symbol ∧ global.constant = true ∧
      global.linkage ≠ .external ∧ global.ty = .array .ptr spec.count ∧
      frag.functionTable.length = spec.count ∧
      global.init = .array (frag.functionTable.map GlobalInit.symbol)) ∧
  (∀ symbol ∈ frag.functionTable, symbol ∈ contract.functions) ∧
  (∀ spec, contract.descriptor = some spec →
    ∀ fn ∈ frag.functions, fn.name ≠ "stc_module_descriptor" →
      spec.symbol ∉ fn.facts.symbolRefs)

/-- The P0.4 well-formedness predicate for a production module fragment. -/
structure WellFormed (contract : IRContract) (frag : IRFrag) : Prop where
  /-- Every block has a terminator. -/
  blocksHaveTerminators : ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks,
    block.terminator.isSome
  /-- Every `phi` precedes every non-`phi` instruction. -/
  phisFirst : ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, block.phisFirst
  /-- `phi` predecessor lists match the CFG. -/
  phiPredecessorsMatch : ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks,
    fn.phiPredecessorsMatch block
  /-- Every function is type-correct. -/
  typesCorrect : ∀ fn ∈ frag.functions, FunctionTypesCorrect frag.directCallEnv fn
  /-- Every function is SSA and dominance correct. -/
  usesDefined : ∀ fn ∈ frag.functions, FunctionUsesDefined fn
  /-- Module names and global/declaration types are well formed. -/
  moduleShapes : ModuleShapes frag
  /-- Every slot reference is in range. -/
  slotRefsInRange : SlotRefsInRange frag
  /-- Every cell use matches its kind. -/
  slotRefsKindCorrect : SlotRefsKindCorrect frag
  /-- The slot table is grouped in canonical kind order. -/
  slotTableCanonical : SlotTable.Canonical frag.slots
  /-- The required entry points match the contract. -/
  entryPointsMatchContract : EntryPointsMatch contract frag
  /-- No body or initializer names a root or user callable directly. -/
  noDirectRootOrCallable : NoDirectRootOrCallable contract frag
  /-- The pre-attach accessors do not use the slot table. -/
  preAttachClean : PreAttachClean contract frag
  /-- Direct global accesses target only allowed globals. -/
  localGlobalsAllowed : LocalGlobalsAllowed contract frag
  /-- No instruction writes a GOT cell. -/
  cellsNotWritten : CellsNotWritten frag
  /-- The descriptor blob and function table match the contract. -/
  descriptorAndTable : DescriptorAndTable contract frag

/-! ## Structural lemmas

Each lemma is a projection of `WellFormed`. They are the structural contract
P0.5 and Phase 5 consume. -/

/-- Every block in a well-formed function has a terminator. -/
theorem wellFormed_blocks_have_terminators {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, block.terminator.isSome :=
  h.blocksHaveTerminators

/-- Every `phi` precedes every non-`phi` instruction in its block. -/
theorem wellFormed_phis_first {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, block.phisFirst :=
  h.phisFirst

/-- `phi` predecessor lists match the block's CFG predecessors. -/
theorem wellFormed_phi_predecessors_match {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, fn.phiPredecessorsMatch block :=
  h.phiPredecessorsMatch

/-- Every function is type-correct. -/
theorem wellFormed_types_correct {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, FunctionTypesCorrect frag.directCallEnv fn :=
  h.typesCorrect

/-- Every function is SSA and dominance correct. -/
theorem wellFormed_uses_defined {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, FunctionUsesDefined fn :=
  h.usesDefined

/-- Module names and global/declaration types are well formed. -/
theorem wellFormed_module_shapes {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ModuleShapes frag :=
  h.moduleShapes

/-- Every slot reference is within the slot table. -/
theorem wellFormed_slot_refs_in_range {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    ∀ fn ∈ frag.functions, ∀ slot ∈ fn.facts.slotRefs, slot < frag.slots.length :=
  h.slotRefsInRange

/-- Every cell use matches the recorded kind. -/
theorem wellFormed_slot_refs_kind_correct {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    SlotRefsKindCorrect frag :=
  h.slotRefsKindCorrect

/-- The slot table's kinds are grouped in canonical order. -/
theorem wellFormed_slot_table_canonical {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    SlotTable.Canonical frag.slots :=
  h.slotTableCanonical

/-- The required entry points exist with the contract signatures and external
linkage. -/
theorem wellFormed_entry_points_match_contract {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    EntryPointsMatch contract frag :=
  h.entryPointsMatchContract

/-- No body or initializer names a persistent root or user callable directly. -/
theorem wellFormed_no_direct_root_or_callable {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    NoDirectRootOrCallable contract frag :=
  h.noDirectRootOrCallable

/-- The pre-attach accessors do not use the slot table. -/
theorem wellFormed_pre_attach_clean {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    PreAttachClean contract frag :=
  h.preAttachClean

/-- Direct global loads and stores target only the contract allowlist. -/
theorem wellFormed_local_globals_allowed {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    LocalGlobalsAllowed contract frag :=
  h.localGlobalsAllowed

/-- No instruction stores through a GOT cell address. -/
theorem wellFormed_cells_not_written {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    CellsNotWritten frag :=
  h.cellsNotWritten

/-- The descriptor blob and function table have the contract's shapes and the
table entries match the fragment's function-table list. -/
theorem wellFormed_descriptor_and_table {contract : IRContract} {frag : IRFrag}
    (h : WellFormed contract frag) :
    DescriptorAndTable contract frag :=
  h.descriptorAndTable

/-! ## A smoke inhabitant

The minimal fragment shows that `WellFormed` is satisfiable and that the
projections are not vacuous. It is a proof fixture, not a compiler conformance
case. -/

/-- The minimal contract: no roots, callables, local globals, or required
entries. -/
def minimalContract : IRContract :=
  { roots := [], functions := [], localGlobals := [], entryPoints := [],
    descriptor := none, functionTable := none }

/-- The minimal block: it returns `0`. -/
def minimalBlock : Block :=
  { label := "entry", instrs := [],
    terminator := some (.ret (some (.intLit (.int 32) 0))) }

/-- A one-block function returning `0`. -/
def minimalFunction : Function :=
  { name := "main", linkage := .external, returnType := .int 32, params := [],
    blocks := [minimalBlock],
    facts :=
      { origins := [], directCalls := [], indirectCalls := [], memoryAccesses := [],
        cellWrites := [], memoryGlobals := [], symbolRefs := [] } }

/-- The minimal emitted fragment. -/
def minimalFrag : IRFrag :=
  { typeDefs := [], globals := [], declarations := [], functions := [minimalFunction],
    slots := [], functionTable := [] }

/-- The minimal fragment is well formed. -/
example : WellFormed minimalContract minimalFrag where
  blocksHaveTerminators := by
    intro fn hfn block hblock
    simp only [minimalFrag] at hfn
    simp only [List.mem_singleton] at hfn
    subst hfn
    dsimp only [minimalFunction] at hblock
    simp only [List.mem_singleton] at hblock
    subst hblock
    simp [minimalBlock]
  phisFirst := by
    intro fn hfn block hblock
    simp only [minimalFrag] at hfn
    simp only [List.mem_singleton] at hfn
    subst hfn
    dsimp only [minimalFunction] at hblock
    simp only [List.mem_singleton] at hblock
    subst hblock
    exact ⟨[], [], rfl, by simp, by simp⟩
  phiPredecessorsMatch := by
    intro fn hfn block hblock
    simp only [minimalFrag] at hfn
    simp only [List.mem_singleton] at hfn
    subst hfn
    dsimp only [minimalFunction] at hblock
    simp only [List.mem_singleton] at hblock
    subst hblock
    simp [Function.phiPredecessorsMatch, Block.philist, minimalBlock]
  typesCorrect := by
    intro fn hfn
    simp only [minimalFrag] at hfn
    simp only [List.mem_singleton] at hfn
    subst hfn
    refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩
    · simp [minimalFunction]
    · decide
    · simp [minimalFunction]
    · simp [minimalFunction, Function.labels]
    · simp [minimalFunction, minimalBlock, Function.CfgClosed, Block.successors]
    · simp [minimalFunction, minimalBlock, Function.entryHasNoPhis, Block.philist]
    · simp [minimalFunction, minimalBlock]
    · intro block hblock terminator hterm
      dsimp only [minimalFunction] at hblock
      simp only [List.mem_singleton] at hblock
      subst hblock
      simp only [minimalBlock, Option.some.injEq] at hterm
      subst hterm
      simp [Terminator.WellTyped, operandHasType, operandTypeOf?, minimalFunction]
  usesDefined := by
    intro fn hfn
    simp only [minimalFrag] at hfn
    simp only [List.mem_singleton] at hfn
    subst hfn
    refine ⟨?_, ?_, ?_⟩
    · simp [minimalFunction, minimalBlock]
    · simp [minimalFunction, minimalBlock]
    · simp [minimalFunction, minimalBlock, Block.philist]
  moduleShapes := by
    refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩ <;> simp [minimalFrag]
  slotRefsInRange := by
    simp [minimalFrag, minimalFunction, SlotRefsInRange, FunctionFacts.slotRefs]
  slotRefsKindCorrect := by
    simp [minimalFrag, minimalFunction, SlotRefsKindCorrect]
  slotTableCanonical := by
    simp [minimalFrag, SlotTable.Canonical]
  entryPointsMatchContract := by
    simp [minimalContract, EntryPointsMatch]
  noDirectRootOrCallable := by
    refine ⟨?_, ?_, ?_, ?_⟩
    · simp [minimalContract, minimalFrag, IRFrag.hasGlobal]
    · simp [minimalFrag, minimalFunction]
    · simp [minimalFrag, minimalFunction]
    · simp [minimalFrag, minimalFunction]
  preAttachClean := by
    simp [minimalFrag, minimalFunction, PreAttachClean]
  localGlobalsAllowed := by
    simp [minimalContract, minimalFrag, minimalFunction, LocalGlobalsAllowed]
  cellsNotWritten := by
    simp [minimalFrag, minimalFunction, CellsNotWritten]
  descriptorAndTable := by
    simp [minimalContract, minimalFrag, DescriptorAndTable]

end Stc.Codegen
