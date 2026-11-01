/-
Operational semantics and target soundness for the core emitted-IR fragment
(P0.5).

`IRState` is an abstract target machine: a stack of frames, per-frame local
storage, GOT root values, the slot table, the function and import bindings, and
module globals. `IRStep` is the graph of the total `irStep` function; it steps,
halts, traps, reaches an undefined state, or reports a structural fault.
`IRStepStar` is the reflexive-transitive closure, and `irExecRec` is the
fuel-based recursive executor.

The covered fragment is the `Stc.Codegen.IRFrag`/`WellFormed` fragment from
`Stc.Codegen.Ir`. Stack cells are initialized by `alloca` so a well-formed
fragment never reads uninitialized storage. GOT cells are indices: a cell load
returns the root storage pointer (`Data`/`Direct`) or the callable
(`Function`/`Import`); the host writes cells, generated code never does.

The semantics is deliberately abstract: it models the emitted access pattern,
not general LLVM pointers, provenance, poison, or the full memory model. The
heap and raw pointer arithmetic are outside the fragment.

Proved in this module: `ir_deterministic`, `ir_got_sound`, `ir_trap_sound`,
`ir_checked_arith_refines_exact`, and `ir_recursive_model_equiv` (with the
executor soundness and completeness lemmas). `ir_progress`, `ir_memory_safe`,
and `ir_no_ub` are staged on the target state invariant: they quantify over
reachable well-formed states, which is the next P0.5 proof obligation.
-/
import Stc.Codegen.Ir
import Stc.Exact.Bits

namespace Stc.Codegen

/-- The trap set of the core target: overflow, division by zero, and range.
`Stc.Exact.Trap` has exactly these constructors. -/
abbrev IRTrap := Stc.Exact.Trap

/-- Running, halted, trapped, or undefined. -/
inductive IRStatus where
  /-- The machine may step. -/
  | running
  /-- The entry point returned. -/
  | halted
  /-- A specified trap terminated the run. -/
  | trapped (trap : IRTrap)
  /-- An undefined operation was reached. -/
  | ub
deriving Repr, DecidableEq

/-- A target address. Stack addresses carry the frame, allocation, and element
offset. `slotEntry` is a slot-table entry address, `cell` a host GOT cell
address, `root` a persistent-storage address, and `function`/`importCell` the
callable values of `Function`/`Import` cells. `slotTable` is the host
slot-table base parameter. -/
inductive IRAddr where
  | stack (frame : Nat) (slot : Nat) (offset : Nat)
  | slotEntry (slot : Nat)
  | cell (slot : Nat)
  | root (slot : Nat) (offset : Nat)
  | global (name : String) (offset : Nat)
  | slotTable
  | function (name : String)
  | importCell (slot : Nat)
deriving Repr, DecidableEq

/-- The identity of a persistent-storage region: a `Data`/`Direct` cell slot or
a module global. -/
inductive StoreId where
  | root (slot : Nat)
  | global (name : String)
deriving Repr, DecidableEq

/-- A target value. Integer payloads are unsigned bit patterns in
`[0, 2^bits)`; signed interpretations are computed at the operation. -/
inductive IRValue where
  /-- An integer of the given IR type. -/
  | int (ty : IrType) (value : Int)
  /-- A pointer value. -/
  | ptr (addr : IRAddr)
  /-- The `{ iN, i1 }` checked-operation result. -/
  | pair (ty : IrType) (value : IRValue) (overflow : Bool)
  /-- The null pointer. -/
  | null
deriving Repr, DecidableEq

namespace IRValue

/-- The value's IR type, when it has one. -/
def type? : IRValue → Option IrType
  | .int ty _ => some ty
  | .ptr _ => some .ptr
  | .pair ty _ _ => some ty
  | .null => some .ptr

/-- The default value of an element type. Arrays are initialized element by
element; a scalar default is used for the other cases. -/
def default : IrType → IRValue
  | .int bits => .int (.int bits) 0
  | .ptr => .null
  | .overflow bits => .pair (.overflow bits) (.int (.int bits) 0) false
  | .array elem _ => default elem
  | .void => .null

/-- The unsigned bit pattern of a value, for arithmetic. -/
def intPayload? : IRValue → Option Int
  | .int _ v => some v
  | _ => none

/-- The pointer address of a value, for memory operations. -/
def ptrAddr? : IRValue → Option IRAddr
  | .ptr addr => some addr
  | .null => none
  | _ => none

end IRValue

/-- The kind of a host import cell. The core fragment uses the trap service;
other host services are outside the interpreted cycle body. -/
inductive ImportKind where
  /-- The `stc_rt_trap` service. -/
  | trap
  /-- A host service outside the covered target semantics. -/
  | host
deriving Repr, DecidableEq

/-- The unsigned width and signed interpretation of a bit pattern. -/
def irModulus (bits : Nat) : Int := (2 : Int) ^ bits

/-- Reduce an integer to its unsigned bit pattern of `bits` bits. -/
def modBits (bits : Nat) (value : Int) : Int :=
  let m := irModulus bits
  let r := value % m
  if r < 0 then r + m else r

/-- Interpret an unsigned bit pattern as a signed value. -/
def signedBits (bits : Nat) (value : Int) : Int :=
  let m := irModulus bits
  let half := (2 : Int) ^ (bits - 1)
  if half ≤ value then value - m else value

/-- The scalar or array element count of a stack allocation. -/
def allocSize : IrType → Nat
  | .array _ count => count
  | _ => 1

/-- Whether a type has a representable default `IRValue`: integers, pointers,
and checked-overflow results. `void` and arrays do not. -/
def IrType.isRepresentable : IrType → Bool
  | .int _ => true
  | .ptr => true
  | .overflow _ => true
  | _ => false

/-! ## Frames and state -/

/-- A keyed list lookup. -/
def lookupList {α β : Type} [BEq α] (entries : List (α × β)) (key : α) : Option β :=
  (entries.find? (fun entry => entry.1 == key)).map (·.2)

/-- A keyed list update or insert. The updated entry is placed at the head and
all previous entries with the same key are removed; keyed lookup is unchanged
for unique keys and the latest value wins for duplicates. -/
def setList {α β : Type} [BEq α] (entries : List (α × β)) (key : α)
    (value : β) : List (α × β) :=
  (key, value) :: entries.filter (fun entry => !(entry.1 == key))

/-- One target frame. `locals` holds parameters and SSA definitions; `mem` holds
initialized stack cells; `allocs` records each allocation's element type. -/
structure IRFrame where
  /-- Unique frame id, used in stack addresses. -/
  id : Nat
  /-- The called function. -/
  fn : Function
  /-- The current block. -/
  block : Block
  /-- The instructions remaining in the current block. -/
  instrs : List Instr
  /-- The current block's terminator. -/
  terminator : Option Terminator
  /-- SSA locals and parameters. -/
  locals : List (String × IRValue)
  /-- Initialized stack cells. -/
  mem : List (IRAddr × IRValue)
  /-- Allocation id to element type. -/
  allocs : List (Nat × IrType)
  /-- The next free allocation id. -/
  nextAlloc : Nat
  /-- The predecessor block, for `phi` selection. -/
  prev : Option Label
  /-- Where the caller expects the return value, for a callee frame. -/
  retDest : Option Ssa

namespace IRFrame

/-- Look up a local or parameter. -/
def lookupLocal (f : IRFrame) (name : String) : Option IRValue :=
  (f.locals.find? (fun entry => entry.1 == name)).map (·.2)

/-- Update or insert a local. -/
def setLocal (f : IRFrame) (name : String) (value : IRValue) : IRFrame :=
  { f with locals := setList f.locals name value }

/-- Look up a stack cell. -/
def lookupMem (f : IRFrame) (addr : IRAddr) : Option IRValue :=
  (f.mem.find? (fun entry => entry.1 == addr)).map (·.2)

/-- Update or insert a stack cell. -/
def setMem (f : IRFrame) (addr : IRAddr) (value : IRValue) : IRFrame :=
  { f with mem := setList f.mem addr value }

/-- The allocation type for an allocation id. -/
def allocType? (f : IRFrame) (slot : Nat) : Option IrType :=
  (f.allocs.find? (fun entry => entry.1 == slot)).map (·.2)

end IRFrame

/-- The target machine state. -/
structure IRState where
  /-- The module being executed. -/
  module : IRFrag
  /-- The call stack; the head frame runs. -/
  frames : List IRFrame
  /-- GOT root values by slot. -/
  roots : List (Nat × IRValue)
  /-- Callable function names by slot. -/
  functions : List (Nat × String)
  /-- Import kinds by slot. -/
  imports : List (Nat × ImportKind)
  /-- Module globals by symbol. -/
  globals : List (String × IRValue)
  /-- Persistent storage: a value sequence per root slot or global. -/
  storage : List (StoreId × List IRValue)
  /-- Host function-cell signatures by slot. -/
  functionSignatures : List (Nat × Signature)
  /-- Completed cycles. -/
  cycles : Nat
  /-- Remaining cycles. -/
  cyclesLeft : Nat
  /-- The next frame id. -/
  nextFrame : Nat
  /-- The machine status. -/
  status : IRStatus

namespace IRState

/-- The running frame. -/
def head? (s : IRState) : Option IRFrame := s.frames.head?

/-- Look up a GOT root value. -/
def lookupRoot (s : IRState) (slot : Nat) : Option IRValue :=
  lookupList s.roots slot

/-- Update a GOT root value. -/
def setRoot (s : IRState) (slot : Nat) (value : IRValue) : IRState :=
  { s with roots := setList s.roots slot value }

/-- Look up a function cell binding. -/
def lookupFunction (s : IRState) (slot : Nat) : Option String :=
  lookupList s.functions slot

/-- Look up an import cell binding. -/
def lookupImport (s : IRState) (slot : Nat) : Option ImportKind :=
  lookupList s.imports slot

/-- Look up a module global. -/
def lookupGlobal (s : IRState) (name : String) : Option IRValue :=
  (s.globals.find? (fun entry => entry.1 == name)).map (·.2)

/-- Update a module global. -/
def setGlobal (s : IRState) (name : String) (value : IRValue) : IRState :=
  { s with globals := setList s.globals name value }

/-- The persistent-storage sequence for a store id. -/
def lookupStorage (s : IRState) (id : StoreId) : Option (List IRValue) :=
  lookupList s.storage id

/-- A persistent-storage read at an in-range offset. -/
def lookupStorageAt (s : IRState) (id : StoreId) (offset : Nat) : Option IRValue :=
  match s.lookupStorage id with
  | some values => if offset < values.length then values[offset]? else none
  | none => none

/-- A persistent-storage write at an in-range offset. -/
def writeStorage (s : IRState) (id : StoreId) (offset : Nat)
    (value : IRValue) : Option IRState :=
  match s.lookupStorage id with
  | some values =>
      if offset < values.length then
        some { s with
          storage := setList s.storage id
            (values.take offset ++ value :: values.drop (offset + 1)) }
      else none
  | none => none

/-- The host signature recorded for a function-cell slot. -/
def lookupFunctionSignature (s : IRState) (slot : Nat) : Option Signature :=
  lookupList s.functionSignatures slot

/-- The slot kind. -/
def slotKind? (s : IRState) (slot : Nat) : Option CellKind :=
  s.module.slots[slot]?

end IRState

/-- The result of evaluating one instruction or terminator. -/
inductive IROutcome where
  /-- The machine steps to a successor. -/
  | step (c : IRState)
  /-- The entry point returned. -/
  | halt (c : IRState)
  /-- A specified trap terminated the run. -/
  | trapped (trap : IRTrap) (c : IRState)
  /-- An undefined operation was reached. -/
  | ub (c : IRState)
  /-- A structural precondition failed. -/
  | stuck (c : IRState)

/-- The non-halting result of executing one instruction or terminator. Halting
is a property of the machine status, so the instruction-level functions cannot
produce it. -/
inductive IRStepResult where
  /-- The machine steps to a successor. -/
  | step (c : IRState)
  /-- A specified trap terminated the run. -/
  | trapped (trap : IRTrap) (c : IRState)
  /-- An undefined operation was reached. -/
  | ub (c : IRState)
  /-- A structural precondition failed. -/
  | stuck (c : IRState)

namespace IRStepResult

/-- Embed an instruction-level result in the machine outcome. -/
def toOutcome : IRStepResult → IROutcome
  | .step c => .step c
  | .trapped trap c => .trapped trap c
  | .ub c => .ub c
  | .stuck c => .stuck c

end IRStepResult

/-! ## Operand and arithmetic evaluation -/

/-- Evaluate an operand in a frame. -/
def evalOperand (f : IRFrame) : Operand → Option IRValue
  | .local name => f.lookupLocal name
  | .intLit ty value => some (.int ty value)
  | .nullPtr => some .null
  | .symbol name => some (.ptr (.global name 0))

/-- The integer width of a type. -/
def intBits? : IrType → Option Nat
  | .int bits => some bits
  | _ => none

/-- A checked integer binary operation with wrapping arithmetic for the plain
LLVM operators. Signed division and remainder trap on the two's-complement
minimum divided by `-1`; the covered fragment guards that case at the source. -/
def evalBinop (op : IntBinOp) (bits : Nat) (a b : Int) : Except IRTrap Int :=
  match op with
  | .add => .ok (modBits bits (a + b))
  | .sub => .ok (modBits bits (a - b))
  | .mul => .ok (modBits bits (a * b))
  | .sdiv =>
      if b == 0 then .error .divZero
      else if signedBits bits a == -((2 : Int) ^ (bits - 1)) &&
          signedBits bits b == -1 then .error .overflow
      else .ok (modBits bits (signedBits bits a / signedBits bits b))
  | .udiv => if b == 0 then .error .divZero else .ok (a / b)
  | .srem =>
      if b == 0 then .error .divZero
      else if signedBits bits a == -((2 : Int) ^ (bits - 1)) &&
          signedBits bits b == -1 then .error .overflow
      else .ok (modBits bits (signedBits bits a % signedBits bits b))
  | .urem => if b == 0 then .error .divZero else .ok (a % b)
  | .band => .ok (modBits bits (Int.ofNat (a.toNat &&& b.toNat)))
  | .bor => .ok (modBits bits (Int.ofNat (a.toNat ||| b.toNat)))
  | .bxor => .ok (modBits bits (Int.ofNat (a.toNat ^^^ b.toNat)))
  | .shl => .ok (modBits bits (a * (2 : Int) ^ (b.toNat % bits)))
  | .lshr => .ok (a / (2 : Int) ^ (b.toNat % bits))
  | .ashr => .ok (modBits bits (Int.ediv (signedBits bits a) ((2 : Int) ^ (b.toNat % bits))))

/-- An integer comparison. -/
def evalICmp (pred : ICmpPred) (bits : Nat) (a b : Int) : Bool :=
  match pred with
  | .eq => a == b
  | .ne => a != b
  | .sgt => signedBits bits a > signedBits bits b
  | .sge => signedBits bits a >= signedBits bits b
  | .slt => signedBits bits a < signedBits bits b
  | .sle => signedBits bits a <= signedBits bits b
  | .ugt => a > b
  | .uge => a >= b
  | .ult => a < b
  | .ule => a <= b

/-- The payload of a boolean. -/
def boolPayload (b : Bool) : Int := if b then 1 else 0

/-- An integer cast. -/
def evalCastOp (op : CastOp) (fromBits toBits : Nat) (a : Int) : Int :=
  match op with
  | .zext => modBits toBits a
  | .sext => modBits toBits (signedBits fromBits a)
  | .trunc => modBits toBits a

/-- Evaluate an integer operand. -/
def evalOperandInt? (f : IRFrame) (operand : Operand) : Option Int :=
  match evalOperand f operand with
  | some (.int _ value) => some value
  | _ => none

/-- Evaluate a list of operands. -/
def evalOperands (f : IRFrame) : List Operand → Option (List IRValue)
  | [] => some []
  | operand :: rest =>
      match evalOperand f operand, evalOperands f rest with
      | some value, some values => some (value :: values)
      | _, _ => none

/-- Evaluate a list of integer operands. -/
def evalOperandInts (f : IRFrame) : List Operand → Option (List Int)
  | [] => some []
  | operand :: rest =>
      match evalOperandInt? f operand, evalOperandInts f rest with
      | some value, some values => some (value :: values)
      | _, _ => none

/-- The checked-arithmetic intrinsic parsed from its emitted name. -/
structure CheckedInfo where
  /-- Whether the operands are signed. -/
  signed : Bool
  /-- The checked operation. -/
  op : IntBinOp
  /-- The integer width. -/
  bits : Nat

/-- The checked integer result and overflow flag. The value is the wrapped
two's-complement result; the flag is true exactly when the `Stc.Exact` model
reports overflow. -/
def checkedInt (info : CheckedInfo) (a b : Int) : IRValue × Bool :=
  let layout : Stc.Exact.IntType := ⟨info.bits, info.signed⟩
  let ia := if info.signed then signedBits info.bits a else a
  let ib := if info.signed then signedBits info.bits b else b
  match info.op with
  | .add =>
      match layout.add ia ib with
      | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
      | .error _ => (.int (.int info.bits) (modBits info.bits (ia + ib)), true)
  | .sub =>
      match layout.sub ia ib with
      | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
      | .error _ => (.int (.int info.bits) (modBits info.bits (ia - ib)), true)
  | .mul =>
      match layout.mul ia ib with
      | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
      | .error _ => (.int (.int info.bits) (modBits info.bits (ia * ib)), true)
  | _ => (.int (.int info.bits) 0, false)

/-- Parse `llvm.{s,u}{add,sub,mul}.with.overflow.iN`. -/
def parseChecked (name : String) : Option CheckedInfo :=
  let widths : List Nat := [8, 16, 32, 64]
  let ops : List (String × Bool × IntBinOp) :=
    [("sadd", true, .add), ("ssub", true, .sub), ("smul", true, .mul),
     ("uadd", false, .add), ("usub", false, .sub), ("umul", false, .mul)]
  let candidates := widths.flatMap fun width =>
    ops.map fun entry =>
      ({ signed := entry.2.1, op := entry.2.2, bits := width } : CheckedInfo)
  let names := widths.flatMap fun width =>
    ops.map fun entry => s!"llvm.{entry.1}.with.overflow.i{width}"
  (names.zip candidates).find? (fun entry => entry.1 == name) |>.map (·.2)

/-- The rotate intrinsic parsed from its emitted name. -/
structure RotateInfo where
  /-- `true` for `llvm.fshl`, `false` for `llvm.fshr`. -/
  left : Bool
  /-- The integer width. -/
  bits : Nat

/-- Parse `llvm.fshl.iN`/`llvm.fshr.iN`. -/
def parseRotate (name : String) : Option RotateInfo :=
  let widths : List Nat := [1, 8, 16, 32, 64]
  let names := widths.flatMap fun width =>
    [s!"llvm.fshl.i{width}", s!"llvm.fshr.i{width}"]
  let infos := widths.flatMap fun width =>
    [({ left := true, bits := width } : RotateInfo),
     ({ left := false, bits := width } : RotateInfo)]
  (names.zip infos).find? (fun entry => entry.1 == name) |>.map (·.2)

/-- A funnel shift on `bits`-wide bit patterns. -/
def evalRotate (info : RotateInfo) (a b count : Int) : IRValue :=
  let w := info.bits
  let r := count.toNat % w
  let m := irModulus w
  let x := a.toNat % m.toNat
  let y := b.toNat % m.toNat
  let result :=
    if info.left then
      ((x <<< r) ||| (y >>> ((w - r) % w))) % m.toNat
    else
      ((x >>> r) ||| (y <<< ((w - r) % w))) % m.toNat
  .int (.int w) (Int.ofNat result)

/-- A GEP address computation. The indices are summed as element offsets; the
covered fragment emits zero-based array GEPs, slot-table indices, and
persistent-storage offsets. -/
def evalGepAddr (base : IRAddr) (indices : List Int) : Option IRAddr :=
  let offset := indices.foldl (fun acc index => acc + index.toNat) 0
  match base with
  | .slotTable =>
      match indices with
      | index :: _ => some (.slotEntry index.toNat)
      | [] => none
  | .stack frame slot baseOffset => some (.stack frame slot (baseOffset + offset))
  | .root slot baseOffset => some (.root slot (baseOffset + offset))
  | .global name baseOffset => some (.global name (baseOffset + offset))
  | _ => none

/-! ## Stack initialization and memory -/

/-- Initialize a frame's parameters and SSA definitions. Every definition is
pre-seeded with its type's default, so a well-formed (dominating) use never
reads an uninitialized local. -/
def initLocals (fn : Function) (args : List IRValue) : List (String × IRValue) :=
  (fn.params.zip args).map (fun entry => (entry.1.2, entry.2)) ++
    fn.localDefs.map (fun entry => (entry.1, IRValue.default entry.2))

/-- Initialize every element of an `alloca`. -/
def initAllocMem (frame slot : Nat) (ty : IrType)
    (mem : List (IRAddr × IRValue)) : List (IRAddr × IRValue) :=
  match ty with
  | .array elem count =>
      (List.range count).foldl
        (fun mem offset => setList mem (.stack frame slot offset) (IRValue.default elem)) mem
  | _ => setList mem (.stack frame slot 0) (IRValue.default ty)

namespace IRState

/-- Replace the running frame. -/
def withHead (s : IRState) (f : IRFrame) : IRState :=
  match s.frames with
  | [] => s
  | _ :: rest => { s with frames := f :: rest }

/-- Replace the running frame's remaining instructions, preserving its locals
and memory (including a store just performed by the step). -/
def withHeadInstrs (s : IRState) (instrs : List Instr) : IRState :=
  match s.frames with
  | [] => s
  | f :: rest => { s with frames := { f with instrs := instrs } :: rest }

/-- Find a frame by id. -/
def frameById? (s : IRState) (id : Nat) : Option IRFrame :=
  s.frames.find? (fun frame => frame.id == id)

/-- Load a value from an address. A slot-table entry yields the cell address;
a cell yields its content; a root or global address reads persistent storage. -/
def loadValue (s : IRState) (addr : IRAddr) : Option IRValue :=
  match addr with
  | .stack frame slot offset =>
      match s.frameById? frame with
      | some f => f.lookupMem (.stack frame slot offset)
      | none => none
  | .slotEntry slot =>
      if slot < s.module.slots.length then some (.ptr (.cell slot)) else none
  | .cell slot =>
      match s.slotKind? slot with
      | some .data | some .direct => some (.ptr (.root slot 0))
      | some .function => (s.lookupFunction slot).map (fun name => .ptr (.function name))
      | some .importCell => some (.ptr (.importCell slot))
      | some .descriptor => none
      | none => none
  | .root slot offset => s.lookupStorageAt (.root slot) offset
  | .global name offset => s.lookupStorageAt (.global name) offset
  | .slotTable => none
  | .function _ => none
  | .importCell _ => none

/-- Store a value to an address. Stack and persistent-storage stores are
extent-checked; GOT cell and slot-table addresses are host-owned. -/
def storeValue (s : IRState) (addr : IRAddr) (value : IRValue) : Option IRState :=
  match addr with
  | .stack frame slot offset =>
      match s.frameById? frame with
      | some f =>
          match f.allocType? slot with
          | some ty =>
              if offset < allocSize ty then
                some { s with
                  frames := s.frames.map fun g =>
                    if g.id == frame then g.setMem (.stack frame slot offset) value
                    else g }
              else none
          | none => none
      | none => none
  | .root slot offset => s.writeStorage (.root slot) offset value
  | .global name offset => s.writeStorage (.global name) offset value
  | .slotEntry _ | .cell _ | .slotTable | .function _ | .importCell _ => none

end IRState

/-! ## Target step relation -/

/-- Jump to a block, recording the current block as the `phi` predecessor. -/
def irJump (s : IRState) (f : IRFrame) (target : Label) : IRStepResult :=
  match f.fn.block? target with
  | some block =>
      let f' : IRFrame :=
        { f with
          block := block, instrs := block.instrs,
          terminator := block.terminator, prev := some f.block.label }
      .step (s.withHead f')
  | none => .stuck s

/-- Return from a frame, delivering the value to the caller's destination. -/
def irStepRet (s : IRState) (f : IRFrame) (value : Option IRValue) : IRStepResult :=
  match s.frames with
  | [] => .stuck s
  | _ :: rest =>
      match rest with
      | [] => .step { s with frames := [], status := .halted }
      | caller :: below =>
          match f.retDest, value with
          | some dest, some v =>
              .step ({ s with frames := caller.setLocal dest v :: below })
          | some _, none => .stuck s
          | none, _ =>
              .step ({ s with frames := caller :: below })

/-- Execute the current block's terminator. -/
def stepTerminator (s : IRState) (f : IRFrame) : IRStepResult :=
  match f.terminator with
  | none => .stuck s
  | some (.ret value) =>
      match value with
      | none => irStepRet s f none
      | some operand =>
          match evalOperand f operand with
          | some v => irStepRet s f (some v)
          | none => .stuck s
  | some (.br target) => irJump s f target
  | some (.condBr cond thenTarget elseTarget) =>
      match evalOperand f cond with
      | some (.int _ value) =>
          irJump s f (if value == 0 then elseTarget else thenTarget)
      | _ => .stuck s
  | some .unreachable => .ub s

/-- Push a callee frame for a module function call. -/
def callFunction (s : IRState) (caller : IRFrame) (fn : Function)
    (args : List IRValue) (dest : Option Ssa) : IRStepResult :=
  match fn.blocks with
  | [] => .stuck s
  | block :: _ =>
      if args.length = fn.params.length then
        let callee : IRFrame :=
          { id := s.nextFrame, fn := fn, block := block,
            instrs := block.instrs, terminator := block.terminator,
            locals := initLocals fn args, mem := [], allocs := [],
            nextAlloc := 0, prev := none, retDest := dest }
        let s' : IRState :=
          { s with
            frames := callee :: caller :: s.frames.tail,
            nextFrame := s.nextFrame + 1 }
        .step s'
      else .stuck s

/-- Complete an instruction with an optional result. -/
def callResult (s : IRState) (f : IRFrame) (rest : List Instr) (dest : Option Ssa)
    (value : IRValue) : IRStepResult :=
  match dest with
  | some name =>
      .step (s.withHead ({ (f.setLocal name value) with instrs := rest }))
  | none => .step (s.withHead ({ f with instrs := rest }))

/-- The trap service: `call void %trap(i32 code, i32 span)`. -/
def trapOutcome (s : IRState) (values : List IRValue) : IRStepResult :=
  match values with
  | .int _ code :: _ =>
      if code == 1 then .trapped .overflow ({ s with status := .trapped .overflow })
      else if code == 2 then .trapped .divZero ({ s with status := .trapped .divZero })
      else if code == 3 then .trapped .range ({ s with status := .trapped .range })
      else .ub s
  | _ => .ub s

/-- Whether a store's value type matches the target stack allocation's element
type. -/
def stackStoreTypeOK (s : IRState) (addr : IRAddr) (ty : IrType) : Bool :=
  match addr with
  | .stack frame slot _ =>
      match s.frameById? frame with
      | some f =>
          match f.allocType? slot with
          | some (.array elem _) => ty == elem
          | some allocTy => ty == allocTy
          | none => false
      | none => false
  | _ => true

/-- Whether a GEP result addresses live storage within its extent. -/
def gepInBounds (s : IRState) (f : IRFrame) (addr : IRAddr) : Bool :=
  match addr with
  | .stack frame slot offset =>
      if frame == f.id then
        match f.allocType? slot with
        | some ty => decide (offset < allocSize ty)
        | none => false
      else false
  | .slotEntry slot => decide (slot < s.module.slots.length)
  | .root slot offset =>
      match s.lookupStorage (.root slot) with
      | some values => decide (offset < values.length)
      | none => false
  | .global name offset =>
      match s.lookupStorage (.global name) with
      | some values => decide (offset < values.length)
      | none => false
  | _ => true

/-- Execute one instruction. -/
def stepInstr (s : IRState) (f : IRFrame) (instr : Instr) (rest : List Instr) : IRStepResult :=
  match instr with
  | .alloca dest ty =>
      let addr := IRAddr.stack f.id f.nextAlloc 0
      let f' : IRFrame :=
        { (f.setLocal dest (.ptr addr)) with
          instrs := rest,
          mem := initAllocMem f.id f.nextAlloc ty f.mem,
          allocs := f.allocs ++ [(f.nextAlloc, ty)],
          nextAlloc := f.nextAlloc + 1 }
      .step (s.withHead f')
  | .load dest ty ptr _align =>
      match evalOperand f ptr with
      | some (.ptr addr) =>
          match s.loadValue addr with
          | some value =>
              if value.type? == some ty then
                .step (s.withHead ({ (f.setLocal dest value) with instrs := rest }))
              else .stuck s
          | none => .stuck s
      | _ => .stuck s
  | .store ty value ptr _align =>
      match evalOperand f value, evalOperand f ptr with
      | some v, some (.ptr addr) =>
          if v.type? == some ty && stackStoreTypeOK s addr ty then
            match s.storeValue addr v with
            | some s' => .step (s'.withHeadInstrs rest)
            | none => .stuck s
          else .stuck s
      | _, _ => .stuck s
  | .gep dest _pointee base indices _inbounds =>
      match evalOperand f base with
      | some (.ptr addr) =>
          match evalOperandInts f (indices.map (·.2)) with
          | some values =>
              match evalGepAddr addr values with
              | some addr' =>
                  if gepInBounds s f addr' then
                    .step (s.withHead
                      { (f.setLocal dest (.ptr addr')) with instrs := rest })
                  else .stuck s
              | none => .stuck s
          | none => .stuck s
      | _ => .stuck s
  | .binop dest op ty lhs rhs =>
      match evalOperandInt? f lhs, evalOperandInt? f rhs, intBits? ty with
      | some a, some b, some bits =>
          match evalBinop op bits a b with
          | .ok value =>
              .step (s.withHead
                { (f.setLocal dest (.int ty value)) with instrs := rest })
          | .error trap => .trapped trap ({ s with status := .trapped trap })
      | _, _, _ => .stuck s
  | .icmp dest pred ty lhs rhs =>
      match evalOperandInt? f lhs, evalOperandInt? f rhs, intBits? ty with
      | some a, some b, some bits =>
          .step (s.withHead
            { (f.setLocal dest (.int (.int 1) (boolPayload (evalICmp pred bits a b)))) with
              instrs := rest })
      | _, _, _ => .stuck s
  | .select dest _ty cond thenValue elseValue =>
      match evalOperand f cond with
      | some (.int _ condition) =>
          match evalOperand f (if condition == 0 then elseValue else thenValue) with
          | some value =>
              .step (s.withHead ({ (f.setLocal dest value) with instrs := rest }))
          | none => .stuck s
      | _ => .stuck s
  | .cast dest op value _fromTy toTy =>
      match evalOperandInt? f value, intBits? _fromTy, intBits? toTy with
      | some a, some fromBits, some toBits =>
          .step (s.withHead
            { (f.setLocal dest (.int toTy (evalCastOp op fromBits toBits a))) with
              instrs := rest })
      | _, _, _ => .stuck s
  | .phi dest _ty incoming =>
      match incoming.find? (fun inc => f.prev == some inc.predecessor) with
      | some inc =>
          match evalOperand f inc.value with
          | some value =>
              .step (s.withHead ({ (f.setLocal dest value) with instrs := rest }))
          | none => .stuck s
      | none => .stuck s
  | .call dest _ret callee args =>
      match evalOperands f args with
      | some values =>
          match parseChecked callee with
          | some info =>
              match values with
              | [.int _ a, .int _ b] =>
                  let result := checkedInt info a b
                  callResult s f rest dest (.pair (.overflow info.bits) result.1 result.2)
              | _ => .stuck s
          | none =>
              match parseRotate callee with
              | some info =>
                  match values with
                  | [.int _ a, .int _ b, .int _ c] =>
                      callResult s f rest dest (evalRotate info a b c)
                  | _ => .stuck s
              | none =>
                  match s.module.function? callee with
                  | some fn => callFunction s { f with instrs := rest } fn values dest
                  | none => .stuck s
      | none => .stuck s
  | .callPtr dest _ret callee args =>
      match evalOperands f args, evalOperand f callee with
      | some values, some (.ptr (.function name)) =>
          match s.module.function? name with
          | some fn => callFunction s { f with instrs := rest } fn values dest
          | none => .stuck s
      | some values, some (.ptr (.importCell slot)) =>
          match s.lookupImport slot with
          | some .trap => trapOutcome s values
          | some .host => .stuck s
          | none => .stuck s
      | _, _ => .stuck s
  | .extractValue dest aggregate index _ty =>
      match evalOperand f aggregate with
      | some (.pair _ value overflow) =>
          match index with
          | 0 => callResult s f rest (some dest) value
          | 1 => callResult s f rest (some dest) (.int (.int 1) (boolPayload overflow))
          | _ => .stuck s
      | _ => .stuck s

/-- One step of the target machine. -/
def irStep (c : IRState) : IROutcome :=
  match c.status with
  | .halted => .halt c
  | .trapped trap => .trapped trap c
  | .ub => .ub c
  | .running =>
      match c.frames with
      | [] => .step { c with status := .halted }
      | f :: _ =>
          match f.instrs with
          | [] => (stepTerminator c f).toOutcome
          | instr :: rest => (stepInstr c f instr rest).toOutcome

/-- A single target step. -/
def IRStep (c c' : IRState) : Prop := irStep c = .step c'

/-- The reflexive-transitive closure of `IRStep`. -/
inductive IRStepStar : IRState → IRState → Prop where
  /-- A zero-step run. -/
  | refl (c : IRState) : IRStepStar c c
  /-- A step followed by a run. -/
  | step {c c' c'' : IRState} : IRStep c c' → IRStepStar c' c'' → IRStepStar c c''

/-- The fuel-based recursive target executor. -/
def irExecRec : Nat → IRState → IROutcome
  | 0, c => .stuck c
  | fuel + 1, c =>
      match irStep c with
      | .step c' => irExecRec fuel c'
      | .halt c' => .halt c'
      | .trapped trap c' => .trapped trap c'
      | .ub c' => .ub c'
      | .stuck c' => .stuck c'

/-- The initial target state for an entry function. The host supplies the GOT
bindings and the initial arguments; the initialization boundary is the frame
construction itself. -/
def initState? (module : IRFrag) (entry : String) (args : List IRValue)
    (roots : List (Nat × IRValue)) (functions : List (Nat × String))
    (imports : List (Nat × ImportKind)) (globals : List (String × IRValue))
    (storage : List (StoreId × List IRValue))
    (functionSignatures : List (Nat × Signature))
    (cycles : Nat) : Option IRState :=
  match module.function? entry with
  | none => none
  | some fn =>
      match fn.blocks with
      | [] => none
      | block :: _ =>
          if args.length = fn.params.length then
            let state : IRState :=
              { module := module,
                frames := [{ id := 0, fn := fn, block := block,
                             instrs := block.instrs, terminator := block.terminator,
                             locals := initLocals fn args, mem := [], allocs := [],
                             nextAlloc := 0, prev := none, retDest := none }],
                roots := roots, functions := functions, imports := imports,
                globals := globals, storage := storage,
                functionSignatures := functionSignatures,
                cycles := 0, cyclesLeft := cycles,
                nextFrame := 1, status := .running }
            some state
          else none

/-- The target cycle entry configuration. -/
def IRSem (module : IRFrag) (entry : String) (args : List IRValue)
    (roots : List (Nat × IRValue)) (functions : List (Nat × String))
    (imports : List (Nat × ImportKind)) (globals : List (String × IRValue))
    (storage : List (StoreId × List IRValue))
    (functionSignatures : List (Nat × Signature))
    (cycles : Nat) : Option IRState :=
  initState? module entry args roots functions imports globals storage
    functionSignatures cycles

/-- Run the target machine with fuel. -/
def irRun (fuel : Nat) (module : IRFrag) (entry : String) (args : List IRValue)
    (roots : List (Nat × IRValue)) (functions : List (Nat × String))
    (imports : List (Nat × ImportKind)) (globals : List (String × IRValue))
    (storage : List (StoreId × List IRValue))
    (functionSignatures : List (Nat × Signature))
    (cycles : Nat) : Option IROutcome :=
  (IRSem module entry args roots functions imports globals storage
    functionSignatures cycles).map (irExecRec fuel)

/-- The P0.4 smoke fragment produces an initial target state, so `IRSem` is
not vacuous. -/
example :
    (IRSem minimalFrag "main" [] [] [] [] [] [] [] 0).isSome = true := by decide

/-- The default value has the declared type for representable types. -/
theorem default_type? {ty : IrType} (h : ty.isRepresentable = true) :
    (IRValue.default ty).type? = some ty := by
  cases ty <;> simp [IrType.isRepresentable, IRValue.default, IRValue.type?] at h ⊢

/-! ## Target invariant (approved design, Plan-Phase0 §22) -/

/-- The first instruction that defines an SSA name. -/
def defInstr? (fn : Function) (name : Ssa) : Option Instr :=
  match fn.defSite? name with
  | some (label, index) =>
      match fn.block? label with
      | some block => block.instrs[index]?
      | none => none
  | none => none

/-- Builder-recorded origins attach to pointer locals with the expected
defining instruction shape; vtable origins are outside the core fragment. -/
def OriginsTrusted (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions,
    (fn.facts.origins.map (·.1)).Nodup ∧
    ∀ entry ∈ fn.facts.origins,
      fn.localType? entry.1 = some .ptr ∧
        entry.1 ∉ fn.params.map (·.2) ∧
        match entry.2 with
      | .vtableCode => False
      | .cell _ false =>
          ∃ ty ptr align, defInstr? fn entry.1 = some (.load entry.1 ty ptr align)
      | .cell _ true =>
          ∃ ty ptr align, defInstr? fn entry.1 = some (.load entry.1 ty ptr align)
      | .global _ =>
          ∃ dest ty base indices inbounds,
            defInstr? fn entry.1 = some (.gep dest ty base indices inbounds)

/-- Pointer locals without a recorded origin are parameters, allocas, or GEP
results. -/
def StackOriginsTrusted (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ name,
    fn.facts.originOf? name = none → fn.localType? name = some .ptr →
      name ∈ fn.params.map (·.2) ∨
        (∃ ty, defInstr? fn name = some (.alloca name ty)) ∨
        (∃ dest ty base indices inbounds,
          defInstr? fn name = some (.gep dest ty base indices inbounds))

/-- Every indirect call is a local with a callable-cell origin. -/
def CallPtrOrigins (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs,
    ∀ dest ret callee args, instr = .callPtr dest ret callee args →
      ∃ name slot, callee = .local name ∧
        fn.facts.originOf? name = some (.cell slot true)

/-- Calls through import cells pass a literal trap code in `1..3`. -/
def TrapCodesSpecified (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs,
    ∀ dest ret callee args, instr = .callPtr dest ret callee args →
      (∃ name slot, callee = .local name ∧
          fn.facts.originOf? name = some (.cell slot true) ∧
          frag.slots[slot]? = some .importCell) →
        ∃ code span, args = [.intLit (.int 32) code, span] ∧
          (code = 1 ∨ code = 2 ∨ code = 3)

/-- Every `unreachable` block ends in a call through an import cell. -/
def UnreachableGuarded (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks,
    block.terminator = some .unreachable →
      ∃ instr, block.instrs.getLast? = some instr ∧
        ∃ dest ret callee args, instr = .callPtr dest ret callee args ∧
          (∃ name slot, callee = .local name ∧
            fn.facts.originOf? name = some (.cell slot true) ∧
            frag.slots[slot]? = some .importCell)

/-- Bases of GEPs whose results are loaded as `.cell slot false` origin
values. -/
def SlotBases (fn : Function) : List Ssa :=
  fn.facts.origins.filterMap fun entry =>
    match entry.2 with
    | .cell _ false =>
        match defInstr? fn entry.1 with
        | some (.load _ _ (.local gepName) _) =>
            match defInstr? fn gepName with
            | some (.gep _ _ (.local base) _ _) => some base
            | _ => none
        | _ => none
    | _ => none

/-- Slot-entry GEP result SSA names and their constant slot numbers. -/
def SlotEntries (fn : Function) : List (Ssa × Nat) :=
  fn.facts.origins.filterMap fun entry =>
    match entry.2 with
    | .cell slot false =>
        match defInstr? fn entry.1 with
        | some (.load _ _ (.local gepName) _) =>
            match defInstr? fn gepName with
            | some (.gep _ _ (.local _base) indices _) =>
                if indices = [(.int 64, .intLit (.int 64) (Int.ofNat slot))] then
                  some (gepName, slot)
                else none
            | _ => none
        | _ => none
    | _ => none

/-- The slots-global storage entry for a base defined by a load is the
slot-table pointer. -/
def SlotsGlobalStored (c : IRState) (f : IRFrame) : Prop :=
  ∀ base ∈ SlotBases f.fn, ∀ g ty ptr align,
    defInstr? f.fn base = some (.load base ty ptr align) → ptr = .symbol g →
      c.lookupStorageAt (.global g) 0 = some (.ptr .slotTable)

/-- A slot-table base local holds the slot-table pointer, or is still the
initialization default before its defining load executes; slot-entry GEP
results hold the corresponding entry address (or the default); and a base
initialized from the slots global has the slot-table pointer in storage. -/
def SlotsBase (c : IRState) (f : IRFrame) : Prop :=
  (∀ base ∈ SlotBases f.fn,
    f.lookupLocal base = some (.ptr .slotTable) ∨ f.lookupLocal base = some .null) ∧
  (∀ gepName slot, (gepName, slot) ∈ SlotEntries f.fn →
    f.lookupLocal gepName = some (.ptr (.slotEntry slot)) ∨
      f.lookupLocal gepName = some .null) ∧
  SlotsGlobalStored c f


/-- Globals loaded to define a slot-table base. -/
def SlotGlobalNames (fn : Function) : List String :=
  (SlotBases fn).filterMap fun base =>
    match defInstr? fn base with
    | some (.load _ _ (.symbol g) _) => some g
    | _ => none

/-- Slot bases are non-parameter locals. -/
def SlotBasesNonParam (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ base ∈ SlotBases fn, base ∉ fn.params.map (·.2)

/-- The slot-table storage value for every module function's slot-base globals. -/
def SlotsStorage (c : IRState) : Prop :=
  ∀ fn ∈ c.module.functions, ∀ base ∈ SlotBases fn, ∀ g ty ptr align,
    defInstr? fn base = some (.load base ty ptr align) → ptr = .symbol g →
      c.lookupStorageAt (.global g) 0 = some (.ptr .slotTable)

/-- No store targets the slots global. -/
def SlotsGlobalReadOnly (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ g ∈ SlotGlobalNames fn,
    (g, true) ∉ fn.facts.memoryGlobals

/-- The `@symbol` names referenced by an operand. -/
def operandSymbols : Operand → List String
  | .symbol g => [g]
  | _ => []

/-- The operand positions that can become a state value. A `load`'s pointer
operand is excluded: a load reads storage instead of producing the symbol's
address. -/
def instrValueSymbols : Instr → List String
  | .alloca _ _ => []
  | .load _ _ _ _ => []
  | .store _ value ptr _ => operandSymbols value ++ operandSymbols ptr
  | .gep _ _ base indices _ =>
      operandSymbols base ++ indices.flatMap (fun index => operandSymbols index.2)
  | .binop _ _ _ lhs rhs => operandSymbols lhs ++ operandSymbols rhs
  | .icmp _ _ _ lhs rhs => operandSymbols lhs ++ operandSymbols rhs
  | .select _ _ cond thenValue elseValue =>
      operandSymbols cond ++ operandSymbols thenValue ++ operandSymbols elseValue
  | .cast _ _ value _ _ => operandSymbols value
  | .phi _ _ incoming => incoming.flatMap (fun inc => operandSymbols inc.value)
  | .call _ _ _ args => args.flatMap operandSymbols
  | .callPtr _ _ callee args => operandSymbols callee ++ args.flatMap operandSymbols
  | .extractValue _ aggregate _ _ => operandSymbols aggregate

/-- The `@symbol` names in value positions of a terminator. -/
def terminatorValueSymbols : Terminator → List String
  | .ret none => []
  | .ret (some operand) => operandSymbols operand
  | .br _ => []
  | .condBr cond _ _ => operandSymbols cond
  | .unreachable => []

/-- The slots globals of a fragment. -/
def SlotsGlobals (frag : IRFrag) : List String :=
  frag.functions.flatMap SlotGlobalNames

/-- Every slot-base global of a module function is a slots global. -/
theorem mem_slotsGlobals_of_slotBase {module : IRFrag} {fn : Function} {base : Ssa}
    {g : String} {ty : IrType} {ptr : Operand} {align : Option Nat}
    (hfn : fn ∈ module.functions) (hbase : base ∈ SlotBases fn)
    (hdef : defInstr? fn base = some (.load base ty ptr align)) (hsym : ptr = .symbol g) :
    g ∈ SlotsGlobals module := by
  unfold SlotsGlobals
  rw [List.mem_flatMap]
  refine ⟨fn, hfn, ?_⟩
  unfold SlotGlobalNames
  rw [List.mem_filterMap]
  exact ⟨base, hbase, by rw [hdef, hsym]⟩

/-- The slots globals never appear in value positions; they are only loaded. -/
def SlotsSymbolsLoadOnly (frag : IRFrag) : Prop :=
  (∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs,
    ∀ g ∈ instrValueSymbols instr, g ∉ SlotsGlobals frag) ∧
  (∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ term,
    block.terminator = some term →
      ∀ g ∈ terminatorValueSymbols term, g ∉ SlotsGlobals frag)

/-- Every `alloca`'s initialized element type is representable. -/
def AllocaElementRepresentable (ty : IrType) : Prop :=
  match ty with
  | .array elem _ => elem.isRepresentable = true
  | _ => ty.isRepresentable = true

/-- Origin-recorded global pointers have a symbol or same-global base, and the
global is a contract-local global. -/
def OriginGlobalsLink (contract : IRContract) (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ entry ∈ fn.facts.origins, ∀ g, entry.2 = .global g →
    g ∈ contract.localGlobals ∧
      ∃ dest ty base indices inbounds,
        defInstr? fn entry.1 = some (.gep dest ty base indices inbounds) ∧
        (base = .symbol g ∨
          ∃ baseName, base = .local baseName ∧
            fn.facts.originOf? baseName = some (.global g))

/-- `.cell slot false` origin slots are storage or callable cells, never
descriptor cells. -/
def OriginSlotKinds (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ entry ∈ fn.facts.origins, ∀ slot,
    entry.2 = .cell slot false →
      ∃ kind, frag.slots[slot]? = some kind ∧
        (kind.isStorage = true ∨ kind.isCallable = true)

/-- Every call forwards the caller's slot-table value to the callee's appended
slots parameter (the caller's last parameter, or the loaded slots global). -/
def CallSlots (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs,
    ∀ dest ret callee args, instr = .call dest ret callee args →
      ∃ name, args.getLast? = some (.local name) ∧
        (fn.params.getLast? = some (IrType.ptr, name) ∨
          fn.facts.originOf? name ≠ none)

/-- Syntactic stack provenance: a local defined by an alloca or by a GEP from a
stack-defined base. -/
inductive StackDefined (fn : Function) : Ssa → Prop where
  | alloca {dest : Ssa} {ty : IrType} :
      defInstr? fn dest = some (.alloca dest ty) → StackDefined fn dest
  | gep {dest base : Ssa} {ty : IrType} {indices : List (IrType × Operand)}
      {inbounds : Bool} :
      defInstr? fn dest = some (.gep dest ty (.local base) indices inbounds) →
      StackDefined fn base → StackDefined fn dest

/-- Return operands come from parameters, origin-recorded pointers, or
integer-producing instructions, so they cannot be a stack pointer to the
returning frame. -/
inductive SafeReturnValue (fn : Function) : Ssa → Prop where
  | param {name : Ssa} : name ∈ fn.params.map (·.2) → SafeReturnValue fn name
  | origin {name : Ssa} {origin : PointerOrigin} :
      fn.facts.originOf? name = some origin → SafeReturnValue fn name
  | binop {dest : Ssa} {op : IntBinOp} {ty : IrType} {lhs rhs : Operand} :
      defInstr? fn dest = some (.binop dest op ty lhs rhs) → SafeReturnValue fn dest
  | icmp {dest : Ssa} {pred : ICmpPred} {ty : IrType} {lhs rhs : Operand} :
      defInstr? fn dest = some (.icmp dest pred ty lhs rhs) → SafeReturnValue fn dest
  | cast {dest : Ssa} {op : CastOp} {value : Operand} {fromTy toTy : IrType} :
      defInstr? fn dest = some (.cast dest op value fromTy toTy) → SafeReturnValue fn dest
  | extractValue {dest : Ssa} {aggregate : Operand} {index : Nat} {ty : IrType} :
      defInstr? fn dest = some (.extractValue dest aggregate index ty) →
        SafeReturnValue fn dest

/-- Return operands are safe values. -/
def ReturnNonStack (frag : IRFrag) : Prop :=
  ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks, ∀ term, block.terminator = some term →
    ∀ operand, term = .ret (some operand) →
      ∀ name ∈ operand.localNames, SafeReturnValue fn name

/-- Module-level facts for the target invariant. -/
structure WFProgram (contract : IRContract) (frag : IRFrag) : Prop where
  wellFormed : WellFormed contract frag
  allocsNonEmpty : ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks,
    ∀ instr ∈ block.instrs, ∀ dest ty, instr = .alloca dest ty → 0 < allocSize ty
  localTypesRepresentable : ∀ fn ∈ frag.functions, ∀ name ty,
    fn.localType? name = some ty → ty.isRepresentable = true
  originsTrusted : OriginsTrusted frag
  stackOriginsTrusted : StackOriginsTrusted frag
  callPtrOrigins : CallPtrOrigins frag
  trapCodesSpecified : TrapCodesSpecified frag
  unreachableGuarded : UnreachableGuarded frag
  callSlots : CallSlots frag
  returnNonStack : ReturnNonStack frag
  slotsGlobalReadOnly : SlotsGlobalReadOnly frag
  slotBasesNonParam : SlotBasesNonParam frag
  allocaRepresentable : ∀ fn ∈ frag.functions, ∀ block ∈ fn.blocks,
    ∀ instr ∈ block.instrs, ∀ dest ty, instr = .alloca dest ty →
      AllocaElementRepresentable ty
  originGlobals : OriginGlobalsLink contract frag
  originSlotKinds : OriginSlotKinds frag
  slotsSymbolsLoadOnly : SlotsSymbolsLoadOnly frag

/-- A local value realizes the builder-recorded pointer origin, or is still
the initialization default (`.null`) before its defining instruction runs. -/
def OriginHolds (contract : IRContract) (c : IRState) (f : IRFrame)
    (name : String) (origin : PointerOrigin) : Prop :=
  ∃ v, f.lookupLocal name = some v ∧ f.fn.localType? name = some .ptr ∧
    (v = .null ∨
    match origin with
    | .cell slot false =>
        v = .ptr (.cell slot) ∧
          ∃ kind, c.slotKind? slot = some kind ∧
            (kind.isStorage = true ∨ kind.isCallable = true)
    | .cell slot true =>
        ∃ kind, c.slotKind? slot = some kind ∧
          ((kind.isStorage = true ∧ v = .ptr (.root slot 0)) ∨
            (kind = .function ∧
              ∃ fname, c.lookupFunction slot = some fname ∧
                v = .ptr (.function fname) ∧
                ∃ fn ∈ c.module.functions, fn.name = fname) ∨
            (kind = .importCell ∧ v = .ptr (.importCell slot)))
    | .global g =>
        (∃ off, v = .ptr (.global g off)) ∧ g ∈ contract.localGlobals
    | .vtableCode => False)

/-- A stored stack pointer is inside an allocated extent. -/
def MemValuePtrBounded (c : IRState) (v : IRValue) : Prop :=
  match v with
  | .ptr (.stack frame slot off) =>
      ∃ g, c.frameById? frame = some g ∧
        ∃ ty, g.allocType? slot = some ty ∧ off < allocSize ty
  | _ => True

/-- Every storage-pointer value in persistent storage addresses a live entry. -/
def StorageValuesLive (c : IRState) : Prop :=
  ∀ id off v, c.lookupStorageAt id off = some v →
    (∀ slot off', v = .ptr (.root slot off') →
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off' < values.length) ∧
    (∀ g off', v = .ptr (.global g off') →
      ∃ values, c.lookupStorage (.global g) = some values ∧ off' < values.length)

/-- Every `@symbol` in a value position of the fragment. -/
def fragValueSymbols (frag : IRFrag) : List String :=
  frag.functions.flatMap fun fn =>
    fn.blocks.flatMap fun block =>
      block.instrs.flatMap instrValueSymbols ++
        match block.terminator with
        | some term => terminatorValueSymbols term
        | none => []

/-- The host storage domain: every `Data`/`Direct` root slot and every global
that the code can produce as a value has a non-empty storage entry. -/
def StorageDomain (c : IRState) : Prop :=
  (∀ slot kind, c.slotKind? slot = some kind → kind.isStorage = true →
    ∃ values, c.lookupStorage (.root slot) = some values ∧ 0 < values.length) ∧
  (∀ g, g ∈ fragValueSymbols c.module →
    ∃ values, c.lookupStorage (.global g) = some values ∧ 0 < values.length)

/-- No local, stack cell, or persistent-storage value is a pointer to a slots
global. Stack memory is included because a stack load returns its value to a
local. -/
def NoSlotsGlobalPointers (c : IRState) : Prop :=
  (∀ f ∈ c.frames, ∀ name v g off, f.lookupLocal name = some v →
    v = .ptr (.global g off) → g ∉ SlotsGlobals c.module) ∧
  (∀ f ∈ c.frames, ∀ addr v g off, f.lookupMem addr = some v →
    v = .ptr (.global g off) → g ∉ SlotsGlobals c.module) ∧
  (∀ id off v g off', c.lookupStorageAt id off = some v →
    v = .ptr (.global g off') → g ∉ SlotsGlobals c.module)

/-- The recorded predecessor is a real CFG edge, or the frame is at the entry
block. -/
def PrevEdge (c : IRState) (f : IRFrame) : Prop :=
  (f.prev = none ∧ ∃ rest, f.fn.blocks = f.block :: rest) ∨
  (∃ label block, f.prev = some label ∧ f.fn.block? label = some block ∧
    f.block.label ∈ block.successors)

/-- Allocation slots are unique, exactly the range `[0, nextAlloc)`, and every
allocation has a non-empty element extent. -/
def AllocsWellFormed (f : IRFrame) : Prop :=
  (f.allocs.map (·.1)).Nodup ∧
  (f.allocs.map (·.1)).Perm (List.range f.nextAlloc) ∧
  ∀ slot ty, f.allocType? slot = some ty → 0 < allocSize ty

/-- Every stack pointer held in a local is inside an allocated slot. -/
def StackPtrsBounded (c : IRState) (f : IRFrame) : Prop :=
  ∀ name v addr, f.lookupLocal name = some v → v = .ptr addr →
    match addr with
    | .stack frame slot off =>
        ∃ g, c.frameById? frame = some g ∧
          ∃ ty, g.allocType? slot = some ty ∧ off < allocSize ty
    | _ => True

/-- A stack cell's stored value matches the allocation element type. -/
def MemValueTyped (ty : IrType) (v : IRValue) : Prop :=
  match ty with
  | .array elem _ => v.type? = some elem
  | _ => v.type? = some ty

def MemTyped (c : IRState) (f : IRFrame) : Prop :=
  ∀ addr v, f.lookupMem addr = some v →
    ∃ slot off ty, addr = .stack f.id slot off ∧
      f.allocType? slot = some ty ∧ off < allocSize ty ∧
      MemValueTyped ty v ∧ MemValuePtrBounded c v

/-- Every persistent-storage pointer held in a local addresses a live storage
entry. -/
def StoragePtrsLive (c : IRState) (f : IRFrame) : Prop :=
  (∀ name v slot off, f.lookupLocal name = some v → v = .ptr (.root slot off) →
    ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length) ∧
  (∀ name v g off, f.lookupLocal name = some v → v = .ptr (.global g off) →
    ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length)

/-- The host's function-cell binding names a module function whose signature is
the one recorded for the cell. -/
def FunctionBindingsTyped (c : IRState) : Prop :=
  ∀ slot fname, c.lookupFunction slot = some fname →
    ∃ fn ∈ c.module.functions, fn.name = fname ∧
      c.lookupFunctionSignature slot = some fn.signature

/-- Every indirect call agrees with the signature of the callable cell it
targets. -/
def CallTargetsTyped (c : IRState) : Prop :=
  ∀ fn ∈ c.module.functions, ∀ block ∈ fn.blocks, ∀ instr ∈ block.instrs,
    ∀ dest ret callee args, instr = .callPtr dest ret callee args →
      ∃ name slot, callee = .local name ∧
        fn.facts.originOf? name = some (.cell slot true) →
        (∃ sig, c.lookupFunctionSignature slot = some sig ∧
          sig.returnType = ret ∧
          listForall₂ (fun arg ty => operandHasType fn ty arg) args sig.params) ∨
        (c.slotKind? slot = some .importCell ∧
          c.lookupImport slot = some .trap ∧ ret = .void ∧ dest = none)

/-- The callee's return destination agrees with the caller's local type, for
every adjacent frame pair in the stack. -/
def ReturnLink (c : IRState) : Prop :=
  ∀ pre callee caller suffix,
    c.frames = pre ++ callee :: caller :: suffix →
    match callee.retDest with
    | none => True
    | some dest =>
        callee.fn.returnType ≠ .void ∧
          caller.fn.localType? dest = some callee.fn.returnType

/-- The appended slots parameter of every frame holds the slot-table pointer. -/
def SlotsParamsSafe (c : IRState) : Prop :=
  ∀ f ∈ c.frames, ∀ p : IrType × String,
    f.fn.params.getLast? = some p → p.2 = "%slots" →
      f.lookupLocal p.2 = some (.ptr .slotTable)

/-- A return value does not point at the returning frame. -/
def StackReturnsSafe (c : IRState) : Prop :=
  ∀ f ∈ c.frames, ∀ operand v,
    f.terminator = some (.ret (some operand)) →
    evalOperand f operand = some v →
    ∀ frame slot off, v = .ptr (.stack frame slot off) → frame ≠ f.id

/-- The state-level target invariant. -/
structure WFState (contract : IRContract) (c : IRState) : Prop where
  program : WFProgram contract c.module
  frameIdsNodup : (c.frames.map (·.id)).Nodup
  frameIdsBound : ∀ f ∈ c.frames, f.id < c.nextFrame
  frameFunction : ∀ f ∈ c.frames, f.fn ∈ c.module.functions
  frameBlock : ∀ f ∈ c.frames, ∃ block ∈ f.fn.blocks,
    f.block = block ∧ f.terminator = block.terminator ∧
      ∃ done, block.instrs = done ++ f.instrs
  framePrev : ∀ f ∈ c.frames, PrevEdge c f
  noBareUnreachable : ∀ f rest, c.frames = f :: rest → f.instrs = [] →
    f.terminator ≠ some .unreachable
  localsInitialized : ∀ f ∈ c.frames, ∀ name ty,
    f.fn.localType? name = some ty → ∃ v, f.lookupLocal name = some v
  localsTyped : ∀ f ∈ c.frames, ∀ name v,
    f.lookupLocal name = some v → ∀ ty, f.fn.localType? name = some ty →
      v.type? = some ty
  originsMatch : ∀ f ∈ c.frames, ∀ entry ∈ f.fn.facts.origins,
    OriginHolds contract c f entry.1 entry.2
  allocsWellFormed : ∀ f ∈ c.frames, AllocsWellFormed f
  stackPtrsBounded : ∀ f ∈ c.frames, StackPtrsBounded c f
  memTyped : ∀ f ∈ c.frames, MemTyped c f
  storagePtrsLive : ∀ f ∈ c.frames, StoragePtrsLive c f
  slotsBase : ∀ f ∈ c.frames, SlotsBase c f
  slotsStorage : SlotsStorage c
  storageValuesLive : StorageValuesLive c
  storageDomain : StorageDomain c
  noSlotsGlobalPointers : NoSlotsGlobalPointers c
  functionBindings : FunctionBindingsTyped c
  callTargets : CallTargetsTyped c
  returnLink : ReturnLink c
  bottomRetDest : ∀ last, c.frames.getLast? = some last → last.retDest = none
  stackReturnsSafe : StackReturnsSafe c
  slotsParamsSafe : SlotsParamsSafe c

/-- The running restriction used by the three soundness theorems. -/
def WFRunning (contract : IRContract) (c : IRState) : Prop :=
  WFState contract c ∧ c.status = .running ∧ c.frames ≠ []

/-- A successful target access is to valid stack, GOT, or persistent storage. -/
def AccessValid (_contract : IRContract) (c : IRState) (addr : IRAddr)
    (store : Bool) (ty : IrType) : Prop :=
  match addr with
  | .stack frame slot off =>
      ∃ f, c.frameById? frame = some f ∧
        ((∃ elem count, f.allocType? slot = some (.array elem count) ∧
            ty = elem ∧ off < count) ∨
          (f.allocType? slot = some ty ∧ off = 0))
  | .slotEntry slot => store = false ∧ slot < c.module.slots.length
  | .cell slot =>
      store = false ∧ ∃ kind, c.slotKind? slot = some kind ∧
        (kind.isStorage = true ∨
          (kind = .function ∧ ∃ fname, c.lookupFunction slot = some fname) ∨
          kind = .importCell)
  | .root slot off =>
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length
  | .global g off =>
      ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length
  | _ => False

/-- The current configuration performs this successful load. -/
def StepLoad (c : IRState) (addr : IRAddr) (ty : IrType) (v : IRValue) : Prop :=
  ∃ f restFrames dest ptr align restInstrs,
    c.frames = f :: restFrames ∧ f.instrs = .load dest ty ptr align :: restInstrs ∧
      evalOperand f ptr = some (.ptr addr) ∧ c.loadValue addr = some v ∧
      v.type? = some ty

/-- The current configuration performs this successful store. -/
def StepStore (c : IRState) (addr : IRAddr) (ty : IrType) (v : IRValue)
    (c' : IRState) : Prop :=
  ∃ f restFrames value ptr align restInstrs,
    c.frames = f :: restFrames ∧ f.instrs = .store ty value ptr align :: restInstrs ∧
      evalOperand f value = some v ∧ v.type? = some ty ∧
      evalOperand f ptr = some (.ptr addr) ∧
      stackStoreTypeOK c addr ty = true ∧ c.storeValue addr v = some c'

/-- The per-instruction stuck-freedom precondition. -/
def AccessBoundedInstr (_contract : IRContract) (c : IRState) (f : IRFrame) :
    Instr → Prop
  | .alloca _ _ => True
  | .load _ ty ptr _ =>
      ∃ addr v, evalOperand f ptr = some (.ptr addr) ∧ c.loadValue addr = some v ∧
        v.type? = some ty
  | .store ty value ptr _ =>
      ∃ addr v s', evalOperand f value = some v ∧ v.type? = some ty ∧
        evalOperand f ptr = some (.ptr addr) ∧ stackStoreTypeOK c addr ty = true ∧
        c.storeValue addr v = some s'
  | .gep _ _ base indices _ =>
      ∃ addr values addr', evalOperand f base = some (.ptr addr) ∧
        evalOperandInts f (indices.map (·.2)) = some values ∧
        evalGepAddr addr values = some addr' ∧ gepInBounds c f addr' = true
  | .binop _ _ ty lhs rhs =>
      ∃ a b bits, evalOperandInt? f lhs = some a ∧ evalOperandInt? f rhs = some b ∧
        intBits? ty = some bits
  | .icmp _ _ ty lhs rhs =>
      ∃ a b bits, evalOperandInt? f lhs = some a ∧ evalOperandInt? f rhs = some b ∧
        intBits? ty = some bits
  | .select _ _ cond thenValue elseValue =>
      ∃ condTy condition, evalOperand f cond = some (.int condTy condition) ∧
        ∃ value, evalOperand f (if condition == 0 then elseValue else thenValue) = some value
  | .cast _ _ value fromTy toTy =>
      ∃ a fromBits toBits, evalOperandInt? f value = some a ∧
        intBits? fromTy = some fromBits ∧ intBits? toTy = some toBits
  | .phi _ _ incoming =>
      ∃ inc value,
        incoming.find? (fun inc => f.prev == some inc.predecessor) = some inc ∧
          evalOperand f inc.value = some value
  | .call _ _ callee args =>
      ∃ values, evalOperands f args = some values ∧
        ((∃ info ty1 ty2 a b, parseChecked callee = some info ∧
            values = [.int ty1 a, .int ty2 b]) ∨
         (parseChecked callee = none ∧
            ∃ info ty1 ty2 ty3 a b c, parseRotate callee = some info ∧
              values = [.int ty1 a, .int ty2 b, .int ty3 c]) ∨
         (parseChecked callee = none ∧ parseRotate callee = none ∧
            ∃ fn, c.module.function? callee = some fn ∧
              fn.blocks ≠ [] ∧ values.length = fn.params.length))
  | .callPtr _ _ callee args =>
      ∃ values calleeValue, evalOperands f args = some values ∧
        evalOperand f callee = some calleeValue ∧
        ((∃ name fn, calleeValue = .ptr (.function name) ∧
            c.module.function? name = some fn ∧ fn.blocks ≠ [] ∧
            values.length = fn.params.length) ∨
         (∃ slot a b, calleeValue = .ptr (.importCell slot) ∧
            c.lookupImport slot = some .trap ∧
            values = [.int (.int 32) a, b] ∧ (a = 1 ∨ a = 2 ∨ a = 3)))
  | .extractValue _ aggregate index _ =>
      ∃ pairTy value overflow, evalOperand f aggregate = some (.pair pairTy value overflow) ∧
        (index = 0 ∨ index = 1)

/-- The terminator stuck-freedom precondition. -/
def AccessBoundedTerminator (_contract : IRContract) (c : IRState) (f : IRFrame) :
    Terminator → Prop
  | .ret none => f.retDest = none
  | .ret (some operand) => ∃ v, evalOperand f operand = some v
  | .br target => ∃ block, f.fn.block? target = some block
  | .condBr cond thenTarget elseTarget =>
      ∃ condTy value, evalOperand f cond = some (.int condTy value) ∧
        ∃ block, f.fn.block? (if value == 0 then elseTarget else thenTarget) = some block
  | .unreachable => False

/-- The current instruction or terminator is ready to execute. -/
def AccessBounded (contract : IRContract) (c : IRState) : Prop :=
  match c.frames with
  | [] => True
  | f :: _ =>
      match f.instrs with
      | [] =>
          match f.terminator with
          | some terminator => AccessBoundedTerminator contract c f terminator
          | none => False
      | instr :: _ => AccessBoundedInstr contract c f instr

/-! ## Target soundness -/

/-- The target step is deterministic. -/
theorem ir_deterministic {c c₁ c₂ : IRState} (h₁ : IRStep c c₁) (h₂ : IRStep c c₂) :
    c₁ = c₂ := by
  unfold IRStep at h₁ h₂
  rw [h₁] at h₂
  exact IROutcome.step.inj h₂

/-- Every trap the target machine can report is one of the specified target
traps. The trap set is `Stc.Exact.Trap`, so this also fixes the target/spec
code correspondence. -/
theorem ir_trap_sound {c c' : IRState} {trap : IRTrap}
    (_h : irStep c = .trapped trap c') :
    trap = .overflow ∨ trap = .divZero ∨ trap = .range := by
  cases trap with
  | overflow => exact Or.inl rfl
  | divZero => exact Or.inr (Or.inl rfl)
  | range => exact Or.inr (Or.inr rfl)

/-- A successful GOT cell load returns exactly what the slot table records: the
root pointer for `Data`/`Direct` storage, the callable for `Function` cells, the
import cell identity for `Import` cells, and nothing for `Descriptor` cells. -/
theorem ir_got_sound (s : IRState) (slot : Nat) (kind : CellKind)
    (h : s.slotKind? slot = some kind) :
    match kind with
    | .data => s.loadValue (.cell slot) = some (.ptr (.root slot 0))
    | .direct => s.loadValue (.cell slot) = some (.ptr (.root slot 0))
    | .function =>
        s.loadValue (.cell slot) =
          (s.lookupFunction slot).map (fun name => .ptr (.function name))
    | .importCell => s.loadValue (.cell slot) = some (.ptr (.importCell slot))
    | .descriptor => s.loadValue (.cell slot) = none := by
  cases kind <;> simp [IRState.loadValue, h]

/-- The checked intrinsic result is the exact model's result; on overflow it is
the wrapped value with the overflow flag set. -/
theorem ir_checked_arith_refines_exact (info : CheckedInfo) (a b : Int) :
    checkedInt info a b =
      (let layout : Stc.Exact.IntType := ⟨info.bits, info.signed⟩
       let ia := if info.signed then signedBits info.bits a else a
       let ib := if info.signed then signedBits info.bits b else b
       match info.op with
       | .add =>
           match layout.add ia ib with
           | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
           | .error _ => (.int (.int info.bits) (modBits info.bits (ia + ib)), true)
       | .sub =>
           match layout.sub ia ib with
           | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
           | .error _ => (.int (.int info.bits) (modBits info.bits (ia - ib)), true)
       | .mul =>
           match layout.mul ia ib with
           | .ok v => (.int (.int info.bits) (modBits info.bits v), false)
           | .error _ => (.int (.int info.bits) (modBits info.bits (ia * ib)), true)
       | _ => (.int (.int info.bits) 0, false)) := by
  rfl

/-- A `.halt` outcome is produced only when the machine is already halted, so
the reported state is the input state. -/
theorem irStep_halt_eq {c c' : IRState} (h : irStep c = .halt c') : c = c' := by
  unfold irStep at h
  cases hs : c.status with
  | halted =>
      simp [hs] at h
      exact h
  | trapped trap => simp [hs] at h
  | ub => simp [hs] at h
  | running =>
      cases hf : c.frames with
      | nil => simp [hs, hf] at h
      | cons f rest =>
          cases hi : f.instrs with
          | nil =>
              simp [hs, hf, hi, IRStepResult.toOutcome] at h
              split at h <;> simp_all
          | cons instr rest' =>
              simp [hs, hf, hi, IRStepResult.toOutcome] at h
              split at h <;> simp_all

/-- Soundness of the recursive executor: a halting result is a finite halting
run in the step relation. -/
theorem irExecRec_halt_sound {fuel : Nat} {c r : IRState}
    (h : irExecRec fuel c = .halt r) :
    IRStepStar c r ∧ irStep r = .halt r := by
  induction fuel generalizing c with
  | zero => simp [irExecRec] at h
  | succ fuel ih =>
      cases hstep : irStep c with
      | step c' =>
          simp only [irExecRec, hstep] at h
          rcases ih h with ⟨hstar, ht⟩
          exact ⟨IRStepStar.step hstep hstar, ht⟩
      | halt c' =>
          simp only [irExecRec, hstep] at h
          injection h with hcr
          subst hcr
          have hc : c = c' := irStep_halt_eq hstep
          subst hc
          exact ⟨IRStepStar.refl _, hstep⟩
      | trapped trap c' => simp only [irExecRec, hstep] at h; cases h
      | ub c' => simp only [irExecRec, hstep] at h; cases h
      | stuck c' => simp only [irExecRec, hstep] at h; cases h

/-- Completeness of the recursive executor for a finite halting run: enough
fuel reproduces the halt. -/
theorem irExecRec_halt_complete {c r : IRState} (h : IRStepStar c r)
    (ht : irStep r = .halt r) :
    ∃ fuel, irExecRec fuel c = .halt r := by
  induction h with
  | refl => exact ⟨1, by simp [irExecRec, ht]⟩
  | step hstep hrest ih =>
      rcases ih ht with ⟨fuel, hfuel⟩
      refine ⟨fuel + 1, ?_⟩
      simp only [IRStep] at hstep
      simp [irExecRec, hstep, hfuel]

/-- The recursive target model agrees with the step relation on halting runs. -/
theorem ir_recursive_model_equiv {c r : IRState} :
    (∃ fuel, irExecRec fuel c = .halt r) ↔
      IRStepStar c r ∧ irStep r = .halt r := by
  constructor
  · rintro ⟨fuel, h⟩
    exact irExecRec_halt_sound h
  · rintro ⟨hstar, ht⟩
    exact irExecRec_halt_complete hstar ht

theorem mem_of_frameById? {c : IRState} {id : Nat} {f : IRFrame}
    (h : c.frameById? id = some f) : f ∈ c.frames := by
  unfold IRState.frameById? at h
  exact List.mem_of_find?_eq_some h

theorem ir_memory_safe {contract : IRContract} {c : IRState}
    (hwf : WFRunning contract c) :
    (∀ addr ty v, StepLoad c addr ty v → AccessValid contract c addr false ty) ∧
    (∀ addr ty v c', StepStore c addr ty v c' → AccessValid contract c addr true ty) := by
  constructor
  · intro addr ty v hstep
    rcases hstep with
      ⟨f, restFrames, dest, ptr, align, restInstrs, hframes, hinstrs, hptr, hload, hty⟩
    rcases hwf with ⟨hwf, hstatus, hne⟩
    cases addr with
    | stack frame slot off =>
        cases hfr : c.frameById? frame with
        | none => simp [IRState.loadValue, hfr] at hload
        | some g =>
            simp only [IRState.loadValue, hfr] at hload
            have hmem : g ∈ c.frames := mem_of_frameById? hfr
            have hm := hwf.memTyped g hmem
            unfold MemTyped at hm
            rcases hm (.stack frame slot off) v hload with
              ⟨slot', off', allocTy, haddr, halloc, hbound, hmemty, _⟩
            injection haddr with hframe hslot hoff
            subst hframe
            subst hslot
            subst hoff
            refine ⟨g, hfr, ?_⟩
            cases allocTy with
            | array elem count =>
                simp [MemValueTyped] at hmemty
                exact Or.inl ⟨elem, count, halloc,
                  Option.some.inj (hty.symm.trans hmemty),
                  by simpa [allocSize] using hbound⟩
            | void =>
                simp [MemValueTyped] at hmemty
                exact Or.inr ⟨by simpa [Option.some.inj (hty.symm.trans hmemty)] using halloc,
                  Nat.lt_one_iff.mp (by simpa [allocSize] using hbound)⟩
            | int bits =>
                simp [MemValueTyped] at hmemty
                exact Or.inr ⟨by simpa [Option.some.inj (hty.symm.trans hmemty)] using halloc,
                  Nat.lt_one_iff.mp (by simpa [allocSize] using hbound)⟩
            | ptr =>
                simp [MemValueTyped] at hmemty
                exact Or.inr ⟨by simpa [Option.some.inj (hty.symm.trans hmemty)] using halloc,
                  Nat.lt_one_iff.mp (by simpa [allocSize] using hbound)⟩
            | overflow bits =>
                simp [MemValueTyped] at hmemty
                exact Or.inr ⟨by simpa [Option.some.inj (hty.symm.trans hmemty)] using halloc,
                  Nat.lt_one_iff.mp (by simpa [allocSize] using hbound)⟩
    | slotEntry slot =>
        by_cases hs : slot < c.module.slots.length
        · simp [IRState.loadValue, if_pos hs] at hload
          exact ⟨rfl, hs⟩
        · simp [IRState.loadValue, if_neg hs] at hload
    | cell slot =>
        cases hk : c.slotKind? slot with
        | none => simp [IRState.loadValue, hk] at hload
        | some kind =>
            cases kind with
            | data =>
                simp only [IRState.loadValue, hk] at hload
                exact ⟨rfl, CellKind.data, hk, Or.inl rfl⟩
            | direct =>
                simp only [IRState.loadValue, hk] at hload
                exact ⟨rfl, CellKind.direct, hk, Or.inl rfl⟩
            | function =>
                cases hfn : c.lookupFunction slot with
                | none => simp [IRState.loadValue, hk, hfn] at hload
                | some fname =>
                    simp only [IRState.loadValue, hk, hfn] at hload
                    exact ⟨rfl, CellKind.function, hk,
                      Or.inr (Or.inl ⟨rfl, fname, hfn⟩)⟩
            | descriptor => simp [IRState.loadValue, hk] at hload
            | importCell =>
                simp only [IRState.loadValue, hk] at hload
                exact ⟨rfl, CellKind.importCell, hk, Or.inr (Or.inr rfl)⟩
    | root slot off =>
        cases hs : c.lookupStorage (.root slot) with
        | none => simp [IRState.loadValue, IRState.lookupStorageAt, hs] at hload
        | some values =>
            by_cases hb : off < values.length
            · simp only [IRState.loadValue, IRState.lookupStorageAt, hs, if_pos hb] at hload
              exact ⟨values, hs, hb⟩
            · simp [IRState.loadValue, IRState.lookupStorageAt, hs, if_neg hb] at hload
    | global name off =>
        cases hs : c.lookupStorage (.global name) with
        | none => simp [IRState.loadValue, IRState.lookupStorageAt, hs] at hload
        | some values =>
            by_cases hb : off < values.length
            · simp only [IRState.loadValue, IRState.lookupStorageAt, hs, if_pos hb] at hload
              exact ⟨values, hs, hb⟩
            · simp [IRState.loadValue, IRState.lookupStorageAt, hs, if_neg hb] at hload
    | slotTable => simp [IRState.loadValue] at hload
    | function name => simp [IRState.loadValue] at hload
    | importCell slot => simp [IRState.loadValue] at hload
  · intro addr ty v c' hstep
    rcases hstep with
      ⟨f, restFrames, value, ptr, align, restInstrs, hframes, hinstrs,
        hvalue, hty, hptr, hss, hstore⟩
    rcases hwf with ⟨hwf, hstatus, hne⟩
    cases addr with
    | stack frame slot off =>
        cases hfr : c.frameById? frame with
        | none => simp [IRState.storeValue, hfr] at hstore
        | some g =>
            simp only [IRState.storeValue, hfr] at hstore
            cases halloc : g.allocType? slot with
            | none => simp [halloc] at hstore
            | some allocTy =>
                simp only [halloc] at hstore
                by_cases hb : off < allocSize allocTy
                ·
                    unfold stackStoreTypeOK at hss
                    simp only [hfr, halloc] at hss
                    refine ⟨g, hfr, ?_⟩
                    cases allocTy with
                    | array elem count =>
                        exact Or.inl ⟨elem, count, halloc, beq_iff_eq.mp hss,
                          by simpa [allocSize] using hb⟩
                    | void =>
                        exact Or.inr ⟨by rw [beq_iff_eq.mp hss]; exact halloc,
                          Nat.lt_one_iff.mp (by simpa [allocSize] using hb)⟩
                    | int bits =>
                        exact Or.inr ⟨by rw [beq_iff_eq.mp hss]; exact halloc,
                          Nat.lt_one_iff.mp (by simpa [allocSize] using hb)⟩
                    | ptr =>
                        exact Or.inr ⟨by rw [beq_iff_eq.mp hss]; exact halloc,
                          Nat.lt_one_iff.mp (by simpa [allocSize] using hb)⟩
                    | overflow bits =>
                        exact Or.inr ⟨by rw [beq_iff_eq.mp hss]; exact halloc,
                          Nat.lt_one_iff.mp (by simpa [allocSize] using hb)⟩
                · simp [hb] at hstore
    | slotEntry slot => simp [IRState.storeValue] at hstore
    | cell slot => simp [IRState.storeValue] at hstore
    | root slot off =>
        cases hs : c.lookupStorage (.root slot) with
        | none => simp [IRState.storeValue, IRState.writeStorage, hs] at hstore
        | some values =>
            by_cases hb : off < values.length
            · simp only [IRState.storeValue, IRState.writeStorage, hs, if_pos hb] at hstore
              exact ⟨values, hs, hb⟩
            · simp [IRState.storeValue, IRState.writeStorage, hs, if_neg hb] at hstore
    | global name off =>
        cases hs : c.lookupStorage (.global name) with
        | none => simp [IRState.storeValue, IRState.writeStorage, hs] at hstore
        | some values =>
            by_cases hb : off < values.length
            · simp only [IRState.storeValue, IRState.writeStorage, hs, if_pos hb] at hstore
              exact ⟨values, hs, hb⟩
            · simp [IRState.storeValue, IRState.writeStorage, hs, if_neg hb] at hstore
    | slotTable => simp [IRState.storeValue] at hstore
    | function name => simp [IRState.storeValue] at hstore
    | importCell slot => simp [IRState.storeValue] at hstore

theorem stepInstr_progress {contract : IRContract} {c : IRState} {f : IRFrame}
    {instr : Instr} {rest : List Instr}
    (h : AccessBoundedInstr contract c f instr) :
    (∃ c', stepInstr c f instr rest = .step c') ∨
      (∃ trap c', stepInstr c f instr rest = .trapped trap c') := by
  cases instr with
  | alloca dest ty =>
      exact Or.inl ⟨_, rfl⟩
  | load dest ty ptr align =>
      rcases h with ⟨addr, v, hptr, hload, hty⟩
      have hb : (v.type? == some ty) = true := by simp [hty]
      simp only [stepInstr, hptr, hload, if_pos hb]
      exact Or.inl ⟨_, rfl⟩
  | store ty value ptr align =>
      rcases h with ⟨addr, v, s', hvalue, hty, hptr, hss, hstore⟩
      have hb : (v.type? == some ty && stackStoreTypeOK c addr ty) = true := by
        simp [hty, hss]
      simp only [stepInstr, hvalue, hptr, if_pos hb, hstore]
      exact Or.inl ⟨_, rfl⟩
  | gep dest pointee base indices inbounds =>
      rcases h with ⟨addr, values, addr', hbase, hindices, hgep, hbounds⟩
      simp only [stepInstr, hbase, hindices, hgep, if_pos hbounds]
      exact Or.inl ⟨_, rfl⟩
  | binop dest op ty lhs rhs =>
      rcases h with ⟨a, b, bits, hlhs, hrhs, hbits⟩
      simp only [stepInstr, hlhs, hrhs, hbits]
      split
      · exact Or.inl ⟨_, rfl⟩
      · exact Or.inr ⟨_, _, rfl⟩
  | icmp dest pred ty lhs rhs =>
      rcases h with ⟨a, b, bits, hlhs, hrhs, hbits⟩
      simp only [stepInstr, hlhs, hrhs, hbits]
      exact Or.inl ⟨_, rfl⟩
  | select dest ty cond thenValue elseValue =>
      rcases h with ⟨condTy, condition, hcond, value, hvalue⟩
      simp only [stepInstr, hcond, hvalue]
      exact Or.inl ⟨_, rfl⟩
  | cast dest op value fromTy toTy =>
      rcases h with ⟨a, fromBits, toBits, hvalue, hfrom, hto⟩
      simp only [stepInstr, hvalue, hfrom, hto]
      exact Or.inl ⟨_, rfl⟩
  | phi dest ty incoming =>
      rcases h with ⟨inc, value, hfind, hvalue⟩
      simp only [stepInstr, hfind, hvalue]
      exact Or.inl ⟨_, rfl⟩
  | call dest ret callee args =>
      rcases h with ⟨values, hargs, hcase⟩
      rcases hcase with ⟨info, ty1, ty2, a, b, hparse, hvalues⟩
        | ⟨hparse, info, ty1, ty2, ty3, a, b, c, hrotate, hvalues⟩
        | ⟨hparse, hrotate, fn, hfn, hblocks, hlen⟩
      · simp only [stepInstr, hargs, hparse, hvalues, callResult]
        split <;> exact Or.inl ⟨_, rfl⟩
      · simp only [stepInstr, hargs, hparse, hrotate, hvalues, callResult]
        split <;> exact Or.inl ⟨_, rfl⟩
      · cases hb : fn.blocks with
        | nil => exact absurd hb hblocks
        | cons block restBlocks =>
            simp only [stepInstr, hargs, hparse, hrotate, hfn, hb, hlen, callFunction]
            exact Or.inl ⟨_, rfl⟩
  | callPtr dest ret callee args =>
      rcases h with ⟨values, calleeValue, hargs, hcallee, hcase⟩
      rcases hcase with ⟨name, fn, hcv, hfn, hblocks, hlen⟩
        | ⟨slot, a, b, hcv, himp, hvalues, hcode⟩
      · cases hb : fn.blocks with
        | nil => exact absurd hb hblocks
        | cons block restBlocks =>
            simp only [stepInstr, hargs, hcallee, hcv, hfn, hb, hlen, callFunction]
            exact Or.inl ⟨_, rfl⟩
      · simp only [stepInstr, hargs, hcallee, hcv, himp, hvalues, trapOutcome]
        rcases hcode with h1 | h2 | h3
        · simp only [h1]
          exact Or.inr ⟨_, _, rfl⟩
        · simp only [h2]
          exact Or.inr ⟨_, _, rfl⟩
        · simp only [h3]
          exact Or.inr ⟨_, _, rfl⟩
  | extractValue dest aggregate index ty =>
      rcases h with ⟨pairTy, value, overflow, hagg, hidx⟩
      rcases hidx with hidx | hidx
      · simp only [stepInstr, hagg, hidx, callResult]
        exact Or.inl ⟨_, rfl⟩
      · simp only [stepInstr, hagg, hidx, callResult]
        exact Or.inl ⟨_, rfl⟩

theorem stepTerminator_progress {contract : IRContract} {c : IRState} {f : IRFrame}
    {restFrames : List IRFrame} {term : Terminator}
    (h : AccessBoundedTerminator contract c f term)
    (hframes : c.frames = f :: restFrames)
    (hterm : f.terminator = some term) :
    (∃ c', stepTerminator c f = .step c') ∨
      (∃ trap c', stepTerminator c f = .trapped trap c') := by
  cases term with
  | ret value =>
      cases restFrames with
      | nil =>
          cases value with
          | none =>
              simp only [AccessBoundedTerminator] at h
              cases hd : f.retDest
              · simp only [stepTerminator, hterm, irStepRet, hframes, hd]
                exact Or.inl ⟨_, rfl⟩
              · simp only [hd] at h
                cases h
          | some operand =>
              rcases h with ⟨v, hv⟩
              cases hd : f.retDest
              · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                exact Or.inl ⟨_, rfl⟩
              · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                exact Or.inl ⟨_, rfl⟩
      | cons caller below =>
          cases below with
          | nil =>
              cases value with
              | none =>
                  simp only [AccessBoundedTerminator] at h
                  cases hd : f.retDest
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd]
                    exact Or.inl ⟨_, rfl⟩
                  · simp only [hd] at h
                    cases h
              | some operand =>
                  rcases h with ⟨v, hv⟩
                  cases hd : f.retDest
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                    exact Or.inl ⟨_, rfl⟩
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                    exact Or.inl ⟨_, rfl⟩
          | cons caller2 below2 =>
              cases value with
              | none =>
                  simp only [AccessBoundedTerminator] at h
                  cases hd : f.retDest
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd]
                    exact Or.inl ⟨_, rfl⟩
                  · simp only [hd] at h
                    cases h
              | some operand =>
                  rcases h with ⟨v, hv⟩
                  cases hd : f.retDest
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                    exact Or.inl ⟨_, rfl⟩
                  · simp only [stepTerminator, hterm, irStepRet, hframes, hd, hv]
                    exact Or.inl ⟨_, rfl⟩
  | br target =>
      rcases h with ⟨block, hb⟩
      simp only [stepTerminator, hterm, irJump, hb]
      exact Or.inl ⟨_, rfl⟩
  | condBr cond thenTarget elseTarget =>
      rcases h with ⟨condTy, value, hcond, block, hb⟩
      simp only [stepTerminator, hterm, irJump, hcond, hb]
      exact Or.inl ⟨_, rfl⟩
  | unreachable =>
      exact absurd h (by simp [AccessBoundedTerminator])

theorem ir_progress {contract : IRContract} {c : IRState}
    (hwf : WFRunning contract c) (haccess : AccessBounded contract c) :
    (∃ c', irStep c = .step c') ∨
      (∃ trap c', irStep c = .trapped trap c') := by
  rcases hwf with ⟨hwf, hstatus, hne⟩
  cases hframes : c.frames with
  | nil => exact absurd hframes hne
  | cons f restFrames =>
      cases hinstrs : f.instrs with
      | nil =>
          cases hterm : f.terminator with
          | none =>
              simp [AccessBounded, hframes, hinstrs, hterm] at haccess
          | some term =>
              simp only [AccessBounded, hframes, hinstrs, hterm] at haccess
              have hp := stepTerminator_progress (contract := contract) (c := c)
                (f := f) (restFrames := restFrames) (term := term) haccess hframes hterm
              rcases hp with ⟨c', hc⟩ | ⟨trap, c', hc⟩
              · exact Or.inl ⟨c', by
                  simp only [irStep, hstatus, hframes, hinstrs, hterm,
                    IRStepResult.toOutcome, hc]⟩
              · exact Or.inr ⟨trap, c', by
                  simp only [irStep, hstatus, hframes, hinstrs, hterm,
                    IRStepResult.toOutcome, hc]⟩
      | cons instr restInstrs =>
          simp only [AccessBounded, hframes, hinstrs] at haccess
          have hp := stepInstr_progress (contract := contract) (c := c)
            (f := f) (instr := instr) (rest := restInstrs) haccess
          rcases hp with ⟨c', hc⟩ | ⟨trap, c', hc⟩
          · exact Or.inl ⟨c', by
              simp only [irStep, hstatus, hframes, hinstrs, IRStepResult.toOutcome, hc]⟩
          · exact Or.inr ⟨trap, c', by
              simp only [irStep, hstatus, hframes, hinstrs, IRStepResult.toOutcome, hc]⟩

theorem toOutcome_ub {r : IRStepResult} {c' : IRState}
    (h : r.toOutcome = .ub c') : r = .ub c' := by
  cases r <;> simp_all [IRStepResult.toOutcome]

theorem stepInstr_ub_imp {c : IRState} {f : IRFrame} {instr : Instr}
    {rest : List Instr} {c' : IRState}
    (h : stepInstr c f instr rest = .ub c') :
    ∃ dest ret callee args values slot,
      instr = .callPtr dest ret callee args ∧
        evalOperands f args = some values ∧
        evalOperand f callee = some (.ptr (.importCell slot)) ∧
        c.lookupImport slot = some .trap ∧
        trapOutcome c values = .ub c' := by
  cases instr with
  | alloca dest ty =>
      simp_all [stepInstr]
  | load dest ty ptr align =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | store ty value ptr align =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | gep dest pointee base indices inbounds =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | binop dest op ty lhs rhs =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | icmp dest pred ty lhs rhs =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | select dest ty cond thenValue elseValue =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | cast dest op value fromTy toTy =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | phi dest ty incoming =>
      simp only [stepInstr] at h
      repeat' split at h
      all_goals simp_all
  | call dest ret callee args =>
      simp only [stepInstr, callResult, callFunction] at h
      repeat' split at h
      all_goals simp_all
  | callPtr dest ret callee args =>
      cases hargs : evalOperands f args with
      | none => simp [stepInstr, hargs] at h
      | some values =>
          cases hcallee : evalOperand f callee with
          | none => simp [stepInstr, hargs, hcallee] at h
          | some cv =>
              cases cv with
              | int ty v => simp [stepInstr, hargs, hcallee] at h
              | pair ty v b => simp [stepInstr, hargs, hcallee] at h
              | null => simp [stepInstr, hargs, hcallee] at h
              | ptr addr =>
                  cases addr with
                  | function fname =>
                      cases hfn : c.module.function? fname with
                      | none => simp [stepInstr, hargs, hcallee, hfn] at h
                      | some fn =>
                          simp only [stepInstr, hargs, hcallee, hfn, callFunction] at h
                          repeat' split at h
                          all_goals simp_all
                  | importCell slot =>
                      cases himp : c.lookupImport slot with
                      | none => simp [stepInstr, hargs, hcallee, himp] at h
                      | some kind =>
                          cases kind with
                          | trap =>
                              simp only [stepInstr, hargs, hcallee, himp] at h
                              exact ⟨dest, ret, callee, args, values, slot, rfl, hargs,
                                hcallee, himp, h⟩
                          | host => simp [stepInstr, hargs, hcallee, himp] at h
                  | stack frame slot off => simp [stepInstr, hargs, hcallee] at h
                  | slotEntry slot => simp [stepInstr, hargs, hcallee] at h
                  | cell slot => simp [stepInstr, hargs, hcallee] at h
                  | root slot off => simp [stepInstr, hargs, hcallee] at h
                  | global name off => simp [stepInstr, hargs, hcallee] at h
                  | slotTable => simp [stepInstr, hargs, hcallee] at h
  | extractValue dest aggregate index ty =>
      simp only [stepInstr, callResult] at h
      repeat' split at h
      all_goals simp_all

theorem trapOutcome_trapped {c : IRState} {values : List IRValue} {ty : IrType}
    {code : Int} {rest : List IRValue}
    (hv : values = .int ty code :: rest)
    (hc : code = 1 ∨ code = 2 ∨ code = 3) :
    ∃ trap c', trapOutcome c values = .trapped trap c' := by
  subst hv
  rcases hc with h1 | h2 | h3
  · subst h1
    exact ⟨.overflow, { c with status := .trapped .overflow }, by simp [trapOutcome]⟩
  · subst h2
    exact ⟨.divZero, { c with status := .trapped .divZero }, by simp [trapOutcome]⟩
  · subst h3
    exact ⟨.range, { c with status := .trapped .range }, by simp [trapOutcome]⟩

theorem mem_of_originOf? {facts : FunctionFacts} {name : String}
    {origin : PointerOrigin}
    (h : facts.originOf? name = some origin) : (name, origin) ∈ facts.origins := by
  unfold FunctionFacts.originOf? at h
  cases hfind : facts.origins.find? (fun entry => entry.1 == name) with
  | none => simp [hfind] at h
  | some entry =>
      simp only [hfind, Option.map_some] at h
      have hmem := List.mem_of_find?_eq_some hfind
      have hp := List.find?_some hfind
      rcases entry with ⟨entryName, entryOrigin⟩
      simp only at hp h hmem ⊢
      have heq : entryName = name := beq_iff_eq.mp hp
      have hEq : entryOrigin = origin := Option.some.inj h
      subst heq
      subst hEq
      exact hmem

theorem evalOperands_intLit_head {f : IRFrame} {ty : IrType} {code : Int}
    {span : Operand} {values : List IRValue}
    (h : evalOperands f [.intLit ty code, span] = some values) :
    ∃ rest, values = .int ty code :: rest := by
  rw [evalOperands.eq_2] at h
  simp only [evalOperand] at h
  split at h
  · rename_i v rest hv hrest
    have hv' : v = .int ty code := (Option.some.inj hv).symm
    have hlist : values = v :: rest := (Option.some.inj h).symm
    exact ⟨rest, by rw [hlist, hv']⟩
  · cases h

theorem ir_no_ub {contract : IRContract} {c c' : IRState}
    (hwf : WFRunning contract c) : irStep c ≠ .ub c' := by
  rintro h
  rcases hwf with ⟨hwf, hstatus, hne⟩
  unfold irStep at h
  rw [hstatus] at h
  cases hframes : c.frames with
  | nil => simp [hframes] at h
  | cons f restFrames =>
      simp only [hframes] at h
      cases hinstrs : f.instrs with
      | nil =>
          cases hterm : f.terminator with
          | none => simp [hinstrs, hterm, stepTerminator, IRStepResult.toOutcome] at h
          | some term =>
              cases term with
              | unreachable =>
                  exact hwf.noBareUnreachable f restFrames hframes hinstrs hterm
              | ret v =>
                  cases v with
                  | none =>
                      simp only [stepTerminator, irStepRet, hinstrs, hterm, hframes] at h
                      repeat' split at h
                      all_goals simp_all [IRStepResult.toOutcome]
                  | some operand =>
                      simp only [stepTerminator, irStepRet, hinstrs, hterm, hframes] at h
                      repeat' split at h
                      all_goals simp_all [IRStepResult.toOutcome]
              | br target =>
                  simp only [stepTerminator, irJump, hinstrs, hterm] at h
                  repeat' split at h
                  all_goals simp_all [IRStepResult.toOutcome]
              | condBr cond thenTarget elseTarget =>
                  simp only [stepTerminator, irJump, hinstrs, hterm] at h
                  repeat' split at h
                  all_goals simp_all [IRStepResult.toOutcome]
      | cons instr restInstrs =>
          simp only [hinstrs] at h
          have hub := toOutcome_ub h
          rcases stepInstr_ub_imp hub with
            ⟨dest, ret, callee, args, values, slot, hinstr, hargs, hcallee, himp, htub⟩
          have hfmem : f ∈ c.frames := by
            rw [hframes]
            exact List.mem_cons_self ..
          rcases hwf.frameBlock f hfmem with
            ⟨block, hblockmem, hblockeq, htermEq, done, hblockinstrs⟩
          have hinstr_mem : instr ∈ block.instrs := by
            rw [hblockinstrs, hinstrs]
            simp
          rcases hwf.program.callPtrOrigins f.fn (hwf.frameFunction f hfmem)
            block hblockmem instr hinstr_mem dest ret callee args hinstr with
            ⟨name, originSlot, hcalleeEq, horigin⟩
          have hmemorig := mem_of_originOf? horigin
          rcases hwf.originsMatch f hfmem (name, .cell originSlot true) hmemorig with
            ⟨v, hlookup, hlocType, hkindcase⟩
          rw [hcalleeEq] at hcallee
          simp only [evalOperand] at hcallee
          have hvEq : v = .ptr (.importCell slot) := by
            rw [hcallee] at hlookup
            exact (Option.some.inj hlookup).symm
          rcases hkindcase with hdefault | hkindcase
          · rw [hvEq] at hdefault
            cases hdefault
          · rcases hkindcase with ⟨kind, hkind, hcase⟩
            rcases hcase with ⟨hstorage, hroot⟩
              | ⟨hkindFn, hfname, hfnlook, hfnval, hfnmem⟩
              | ⟨hkindImp, hval⟩
            · rw [hvEq] at hroot
              cases hroot
            · rw [hvEq] at hfnval
              cases hfnval
            · cases hval
              subst hkindImp
              have htrap := hwf.program.trapCodesSpecified f.fn (hwf.frameFunction f hfmem)
                block hblockmem instr hinstr_mem dest ret callee args hinstr
                ⟨name, originSlot, hcalleeEq, horigin,
                  by simpa [IRState.slotKind?] using hkind⟩
              rcases htrap with ⟨code, span, hargsEq, hcode⟩
              rw [hargsEq] at hargs
              rcases evalOperands_intLit_head hargs with ⟨restValues, hvalues⟩
              rcases trapOutcome_trapped hvalues hcode with ⟨trap, c'', htr⟩
              rw [htr] at htub
              cases htub

theorem filterMap_sublist_of_imp {α β : Type} {f g : α → Option β} :
    ∀ {l : List α}, (∀ a b, f a = some b → g a = some b) →
      (l.filterMap f).Sublist (l.filterMap g)
  | [], _ => by simp
  | a :: as, h => by
      simp only [List.filterMap_cons]
      cases hf : f a with
      | none =>
          simp only [hf]
          cases hg : g a with
          | none =>
              simp only [hg]
              exact filterMap_sublist_of_imp h (l := as)
          | some b =>
              simp only [hg]
              exact List.Sublist.cons b (filterMap_sublist_of_imp h (l := as))
      | some b =>
          have hga : g a = some b := h a b hf
          simp only [hf, hga]
          exact List.Sublist.cons₂ b (filterMap_sublist_of_imp h (l := as))

theorem flatMap_sublist_of_forall {α β : Type} {f g : α → List β} :
    ∀ {l : List α}, (∀ a ∈ l, (f a).Sublist (g a)) →
      (l.flatMap f).Sublist (l.flatMap g)
  | [], _ => by simp
  | a :: as, h => by
      rw [List.flatMap_cons, List.flatMap_cons]
      exact List.Sublist.append (h a (by simp))
        (flatMap_sublist_of_forall (fun b hb => h b (by simp [hb])))

theorem localDefs_names_sublist (fn : Function) :
    (fn.localDefs.map (·.1)).Sublist
      (fn.blocks.flatMap fun block => block.instrs.filterMap Instr.dest?) := by
  unfold Function.localDefs
  rw [List.map_flatMap]
  apply flatMap_sublist_of_forall
  intro block _
  rw [List.map_filterMap]
  apply filterMap_sublist_of_imp
  intro instr dest hdest
  split at hdest
  · rename_i d t hd ht
    simp only [Option.map_some, Option.some.injEq] at hdest
    subst hdest
    exact hd
  · simp at hdest

theorem localDefs_names_nodup {fn : Function} (h : FunctionUsesDefined fn) :
    (fn.localDefs.map (·.1)).Nodup := by
  have hall := h.allNodup
  rw [List.nodup_append] at hall
  exact (localDefs_names_sublist fn).nodup hall.2.1

theorem find?_localDefs_eq {fn : Function}
    (hnd : (fn.localDefs.map (·.1)).Nodup)
    {name : String} {ty : IrType} (hmem : (name, ty) ∈ fn.localDefs) :
    fn.localDefs.find? (fun e => e.1 == name) = some (name, ty) := by
  revert hnd hmem
  induction fn.localDefs with
  | nil => intro hnd hmem; simp at hmem
  | cons hd tl ih =>
      intro hnd hmem
      simp only [List.map_cons] at hnd
      rw [List.nodup_cons] at hnd
      rcases hd with ⟨hdName, hdTy⟩
      by_cases hh : hdName = name
      · subst hdName
        have hty : hdTy = ty := by
          rcases List.mem_cons.mp hmem with hhead | htail
          · have h' : ty = hdTy := by simpa using hhead
            exact h'.symm
          · exfalso
            exact hnd.1 (List.mem_map.mpr ⟨(name, ty), htail, rfl⟩)
        subst hty
        exact List.find?_cons_of_pos (by simp)
      · have hmem' : (name, ty) ∈ tl := by
          rcases List.mem_cons.mp hmem with hhead | htail
          · exfalso
            apply hh
            simpa using (congrArg Prod.fst hhead).symm
          · exact htail
        rw [List.find?_cons_of_neg (by simp [beq_iff_eq, hh])]
        exact ih hnd.2 hmem'

theorem find?_localDefs_eq' {fn : Function} (hud : FunctionUsesDefined fn)
    {name : String} {ty : IrType} (hmem : (name, ty) ∈ fn.localDefs) :
    fn.localDefs.find? (fun e => e.1 == name) = some (name, ty) :=
  find?_localDefs_eq (localDefs_names_nodup hud) hmem

theorem mem_dests_of_block {fn : Function} {block : Block} {instr : Instr}
    {dest : Ssa} (hblock : block ∈ fn.blocks) (hinstr : instr ∈ block.instrs)
    (hdest : instr.dest? = some dest) :
    dest ∈ fn.blocks.flatMap (fun b => b.instrs.filterMap Instr.dest?) :=
  List.mem_flatMap.mpr ⟨block, hblock,
    List.mem_filterMap.mpr ⟨instr, hinstr, hdest⟩⟩

theorem mem_localDefs_of_block {fn : Function} {block : Block} {instr : Instr}
    {dest : Ssa} {ty : IrType} (hblock : block ∈ fn.blocks)
    (hinstr : instr ∈ block.instrs) (hdest : instr.dest? = some dest)
    (hty : instr.resultType? = some ty) :
    (dest, ty) ∈ fn.localDefs := by
  unfold Function.localDefs
  exact List.mem_flatMap.mpr ⟨block, hblock,
    List.mem_filterMap.mpr ⟨instr, hinstr, by simp [hdest, hty]⟩⟩

theorem localType?_eq_resultType {fn : Function} (hud : FunctionUsesDefined fn)
    {label : Label} {index : Nat} {block : Block} {instr : Instr}
    {dest : Ssa} {ty : IrType}
    (hblock : fn.block? label = some block)
    (hinstr : block.instrs[index]? = some instr)
    (hdest : instr.dest? = some dest)
    (hty : instr.resultType? = some ty) :
    fn.localType? dest = some ty := by
  have hblockMem : block ∈ fn.blocks := by
    unfold Function.block? at hblock
    exact List.mem_of_find?_eq_some hblock
  have hinstrMem : instr ∈ block.instrs := List.mem_iff_getElem?.mpr ⟨index, hinstr⟩
  have hdestsMem : dest ∈ fn.blocks.flatMap (fun b => b.instrs.filterMap Instr.dest?) :=
    mem_dests_of_block hblockMem hinstrMem hdest
  have hlocalMem : (dest, ty) ∈ fn.localDefs :=
    mem_localDefs_of_block hblockMem hinstrMem hdest hty
  have hall := hud.allNodup
  rw [List.nodup_append] at hall
  have hnotParam : dest ∉ fn.params.map (·.2) := by
    intro hmem
    exact hall.2.2 dest hmem dest hdestsMem rfl
  have hfindNone : fn.params.find? (fun p => p.2 == dest) = none := by
    rw [List.find?_eq_none]
    intro p hp hbeq
    apply hnotParam
    exact List.mem_map.mpr ⟨p, hp, beq_iff_eq.mp hbeq⟩
  have hlocalFind : fn.localDefs.find? (fun e => e.1 == dest) = some (dest, ty) :=
    find?_localDefs_eq' hud hlocalMem
  unfold Function.localType?
  rw [hfindNone]
  simp only [hlocalFind, Option.map_some]

theorem find?_params_eq {params : List (IrType × String)}
    (hnd : (params.map (·.2)).Nodup) {p : IrType × String} (hp : p ∈ params) :
    params.find? (fun q => q.2 == p.2) = some p := by
  revert hnd hp
  induction params with
  | nil => intro hnd hp; simp at hp
  | cons hd tl ih =>
      intro hnd hp
      simp only [List.map_cons] at hnd
      rw [List.nodup_cons] at hnd
      by_cases hh : hd.2 = p.2
      · have hp' : p = hd := by
          rcases List.mem_cons.mp hp with hhead | htail
          · exact hhead
          · exfalso
            exact hnd.1 (List.mem_map.mpr ⟨p, htail, by simp [hh]⟩)
        subst hp'
        exact List.find?_cons_of_pos (by simp)
      · rcases List.mem_cons.mp hp with hhead | htail
        · subst hhead
          exact absurd rfl hh
        · rw [List.find?_cons_of_neg (by simp [hh])]
          exact ih hnd.2 htail

theorem localType?_of_param {fn : Function} (hud : FunctionUsesDefined fn)
    {p : IrType × String} (hp : p ∈ fn.params) :
    fn.localType? p.2 = some p.1 := by
  have hnd : (fn.params.map (·.2)).Nodup := by
    have hall := hud.allNodup
    rw [List.nodup_append] at hall
    exact hall.1
  have hfind := find?_params_eq hnd hp
  unfold Function.localType?
  rw [hfind]

theorem mem_left_of_zip {α β : Type} {l1 : List α} {l2 : List β}
    {a : α} {b : β} (h : (a, b) ∈ l1.zip l2) : a ∈ l1 := by
  induction l1 generalizing l2 with
  | nil => simp at h
  | cons x xs ih =>
      cases l2 with
      | nil => simp at h
      | cons y ys =>
          simp only [List.zip_cons_cons, List.mem_cons] at h
          rcases h with h | h
          · cases h
            exact List.mem_cons_self ..
          · exact List.mem_cons_of_mem _ (ih h)

theorem mem_zip_of_mem_left {α β : Type} :
    ∀ {l1 : List α} {l2 : List β}, l1.length = l2.length →
      ∀ a ∈ l1, ∃ b, (a, b) ∈ l1.zip l2
  | [], l2, hlen, a, ha => by simp at ha
  | _ :: _, [], hlen, a, ha => by simp at hlen
  | a1 :: as, b1 :: bs, hlen, a, ha => by
      rcases List.mem_cons.mp ha with h | h
      · subst h
        exact ⟨b1, by simp⟩
      · have hlen' : as.length = bs.length := by simpa using hlen
        rcases mem_zip_of_mem_left hlen' a h with ⟨b, hb⟩
        exact ⟨b, by simp [hb]⟩

theorem find?_zip_map_eq {params : List (IrType × String)} {args : List IRValue}
    (hnd : (params.map (·.2)).Nodup) (hlen : params.length = args.length)
    {p : IrType × String} (hp : p ∈ params) {a : IRValue}
    (ha : (p, a) ∈ params.zip args) :
    (params.zip args).find? (fun e => e.1.2 == p.2) = some (p, a) := by
  revert hnd hlen hp ha
  induction params generalizing args with
  | nil => intro hnd hlen hp ha; simp at hp
  | cons hd tl ih =>
      intro hnd hlen hp ha
      cases args with
      | nil => simp at hlen
      | cons a0 as =>
          simp only [List.map_cons] at hnd
          rw [List.nodup_cons] at hnd
          simp only [List.zip_cons_cons, List.mem_cons] at ha
          by_cases hh : hd.2 = p.2
          · have hp' : p = hd := by
              rcases List.mem_cons.mp hp with hhead | htail
              · exact hhead
              · exfalso
                exact hnd.1 (List.mem_map.mpr ⟨p, htail, by simp [hh]⟩)
            subst p
            simp only [List.zip_cons_cons]
            have ha' : a = a0 := by
              rcases ha with hhead | htail
              · cases hhead; rfl
              · exfalso
                exact hnd.1 (List.mem_map.mpr ⟨hd, mem_left_of_zip htail, rfl⟩)
            cases ha'
            exact List.find?_cons_of_pos (by simp)
          · have hpTail : p ∈ tl := by
              rcases List.mem_cons.mp hp with hhead | htail
              · exfalso
                apply hh
                rw [hhead]
              · exact htail
            have haTail : (p, a) ∈ tl.zip as := by
              rcases ha with hhead | htail
              · exfalso
                apply hh
                have hp_eq : p = hd := congrArg Prod.fst hhead
                rw [hp_eq]
              · exact htail
            have hlen' : tl.length = as.length := by simpa using hlen
            simp only [List.zip_cons_cons]
            rw [List.find?_cons_of_neg (by simp [beq_iff_eq, hh])]
            exact ih hnd.2 hlen' hpTail haTail

theorem listForall2_zip {α β : Type} {rel : α → β → Prop} :
    ∀ {l1 : List α} {l2 : List β}, listForall₂ rel l1 l2 →
      ∀ a b, (a, b) ∈ l1.zip l2 → rel a b
  | [], [], _, a, b, hmem => by simp at hmem
  | [], _ :: _, h, a, b, hmem => by simp [listForall₂] at h
  | _ :: _, [], h, a, b, hmem => by simp [listForall₂] at h
  | a1 :: as, b1 :: bs, h, a, b, hmem => by
      simp only [listForall₂] at h
      simp only [List.zip_cons_cons, List.mem_cons] at hmem
      rcases h with ⟨hr, hrest⟩
      rcases hmem with hhead | htail
      · cases hhead
        exact hr
      · exact listForall2_zip hrest a b htail

def ArgsTyped (fn : Function) (args : List IRValue) : Prop :=
  listForall₂ (fun param v => v.type? = some param.1) fn.params args

theorem ArgsTyped.of_zip_mem {fn : Function} {args : List IRValue}
    (h : ArgsTyped fn args) {p : IrType × String} {a : IRValue}
    (hmem : (p, a) ∈ fn.params.zip args) : a.type? = some p.1 :=
  listForall2_zip h p a hmem

theorem mem_initLocals_cases {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue}
    (hmem : (name, v) ∈ initLocals fn args) :
    (∃ (p : IrType × String) (a : IRValue),
        p ∈ fn.params ∧ p.2 = name ∧ (p, a) ∈ fn.params.zip args ∧ a = v) ∨
      (∃ ty, (name, ty) ∈ fn.localDefs ∧ v = IRValue.default ty) := by
  unfold initLocals at hmem
  rcases List.mem_append.mp hmem with hleft | hright
  · rcases List.mem_map.mp hleft with ⟨entry, hzip, hentry⟩
    rcases entry with ⟨p, a⟩
    injection hentry with hname hv
    exact Or.inl ⟨p, a, mem_left_of_zip hzip, hname, hzip, hv⟩
  · rcases List.mem_map.mp hright with ⟨entry, hdef, hentry⟩
    rcases entry with ⟨d, ty⟩
    injection hentry with hname hv
    exact Or.inr ⟨ty, by simpa [← hname] using hdef, hv.symm⟩

theorem params_names_nodup {fn : Function} (hud : FunctionUsesDefined fn) :
    (fn.params.map (·.2)).Nodup := by
  have hall := hud.allNodup
  rw [List.nodup_append] at hall
  exact hall.1

theorem find?_initLocals_param {fn : Function} {args : List IRValue}
    (hud : FunctionUsesDefined fn) (hlen : args.length = fn.params.length)
    {p : IrType × String} {a : IRValue}
    (hp : p ∈ fn.params) (ha : (p, a) ∈ fn.params.zip args) :
    (initLocals fn args).find? (fun e => e.1 == p.2) = some (p.2, a) := by
  have hzip : (fn.params.zip args).find? (fun e => e.1.2 == p.2) = some (p, a) :=
    find?_zip_map_eq (params_names_nodup hud) hlen.symm hp ha
  have hmap : ((fn.params.zip args).map (fun e => (e.1.2, e.2))).find?
      (fun e => e.1 == p.2) = some (p.2, a) := by
    rw [List.find?_map]
    change Option.map (fun e : (IrType × String) × IRValue => (e.1.2, e.2))
      (List.find? (fun e : (IrType × String) × IRValue => e.1.2 == p.2)
        (fn.params.zip args)) = some (p.2, a)
    rw [hzip]
    rfl
  unfold initLocals
  rw [List.find?_append, hmap]
  simp

theorem find?_initLocals_localDef {fn : Function} {args : List IRValue}
    (hud : FunctionUsesDefined fn) {name : String} {ty : IrType}
    (hmem : (name, ty) ∈ fn.localDefs) :
    (initLocals fn args).find? (fun e => e.1 == name) =
      some (name, IRValue.default ty) := by
  have hall := hud.allNodup
  rw [List.nodup_append] at hall
  have hnameDests : name ∈ fn.blocks.flatMap (fun b => b.instrs.filterMap Instr.dest?) :=
    (localDefs_names_sublist fn).subset (List.mem_map.mpr ⟨(name, ty), hmem, rfl⟩)
  have hnameNotParams : name ∉ fn.params.map (·.2) := by
    intro hmemP
    exact hall.2.2 name hmemP name hnameDests rfl
  have hparamsNone : ((fn.params.zip args).map (fun e => (e.1.2, e.2))).find?
      (fun e => e.1 == name) = none := by
    rw [List.find?_map]
    have hinner : List.find?
        (fun e : (IrType × String) × IRValue => e.1.2 == name)
        (fn.params.zip args) = none := by
      rw [List.find?_eq_none]
      intro entry hentry hpred
      have hp : entry.1 ∈ fn.params := mem_left_of_zip hentry
      exact hnameNotParams (List.mem_map.mpr ⟨entry.1, hp, beq_iff_eq.mp hpred⟩)
    change Option.map (fun e : (IrType × String) × IRValue => (e.1.2, e.2))
      (List.find? (fun e : (IrType × String) × IRValue => e.1.2 == name)
        (fn.params.zip args)) = none
    rw [hinner]
    rfl
  have hdefMap : (fn.localDefs.map (fun e => (e.1, IRValue.default e.2))).find?
      (fun e => e.1 == name) = some (name, IRValue.default ty) := by
    rw [List.find?_map]
    change Option.map (fun e : String × IrType => (e.1, IRValue.default e.2))
      (List.find? (fun e : String × IrType => e.1 == name) fn.localDefs) =
      some (name, IRValue.default ty)
    rw [find?_localDefs_eq' hud hmem]
    rfl
  unfold initLocals
  rw [List.find?_append, hparamsNone]
  simp only [Option.none_or]
  exact hdefMap

theorem lookupLocal_initLocals_of_localType {fn : Function} {args : List IRValue}
    {name : String} {ty : IrType}
    (hud : FunctionUsesDefined fn) (hlen : args.length = fn.params.length)
    (hargs : ArgsTyped fn args)
    (hrep : ∀ name ty, fn.localType? name = some ty → ty.isRepresentable = true)
    (hty : fn.localType? name = some ty) :
    ∃ v, (initLocals fn args).find? (fun e => e.1 == name) = some (name, v) ∧
      v.type? = some ty := by
  have hty0 := hty
  unfold Function.localType? at hty
  cases hfind : fn.params.find? (fun p => p.2 == name) with
  | some p =>
      simp only [hfind, Option.some.injEq] at hty
      have hp : p ∈ fn.params := List.mem_of_find?_eq_some hfind
      have hpbeq : (p.2 == name) = true :=
        List.find?_some (p := fun q : IrType × String => q.2 == name) hfind
      have hpname : p.2 = name := beq_iff_eq.mp hpbeq
      rcases mem_zip_of_mem_left hlen.symm p hp with ⟨a, ha⟩
      have hargty : a.type? = some p.1 := ArgsTyped.of_zip_mem hargs ha
      have hfindInit := find?_initLocals_param hud hlen hp ha
      refine ⟨a, ?_, ?_⟩
      · simpa [hpname] using hfindInit
      · rw [hargty, hty]
  | none =>
      simp only [hfind] at hty
      cases hdefFind : fn.localDefs.find? (fun e => e.1 == name) with
      | none => simp [hdefFind] at hty
      | some entry =>
          rcases entry with ⟨ename, ety⟩
          simp only [hdefFind, Option.map_some, Option.some.injEq] at hty
          have hmemFind := List.mem_of_find?_eq_some hdefFind
          have hebeq : (ename == name) = true :=
            List.find?_some (p := fun e : String × IrType => e.1 == name) hdefFind
          have hename : ename = name := beq_iff_eq.mp hebeq
          have hentryMem : (name, ty) ∈ fn.localDefs := by
            have hpair : (ename, ety) = (name, ty) := by
              rw [hename, hty]
            simpa [hpair] using hmemFind
          exact ⟨IRValue.default ty, find?_initLocals_localDef hud hentryMem,
            default_type? (hrep name ty hty0)⟩

theorem mem_of_function? {frag : IRFrag} {name : String} {fn : Function}
    (h : frag.function? name = some fn) : fn ∈ frag.functions := by
  unfold IRFrag.function? at h
  exact List.mem_of_find?_eq_some h

theorem default_ne_root (ty : IrType) (slot off : Nat) :
    IRValue.default ty ≠ .ptr (.root slot off) := by
  induction ty with
  | array elem count ih => simpa [IRValue.default] using ih
  | _ => simp [IRValue.default]

theorem default_ne_global (ty : IrType) (g : String) (off : Nat) :
    IRValue.default ty ≠ .ptr (.global g off) := by
  induction ty with
  | array elem count ih => simpa [IRValue.default] using ih
  | _ => simp [IRValue.default]

theorem default_ne_stack (ty : IrType) (frame slot off : Nat) :
    IRValue.default ty ≠ .ptr (.stack frame slot off) := by
  induction ty with
  | array elem count ih => simpa [IRValue.default] using ih
  | _ => simp [IRValue.default]

theorem lookupLocal_find?_eq {f : IRFrame} {name : String} {v : IRValue}
    (h : f.lookupLocal name = some v) :
    f.locals.find? (fun e => e.1 == name) = some (name, v) := by
  unfold IRFrame.lookupLocal at h
  cases hf : f.locals.find? (fun e => e.1 == name) with
  | none => simp [hf] at h
  | some entry =>
      rcases entry with ⟨n, w⟩
      simp only [hf, Option.map_some, Option.some.injEq] at h
      have hbeq : (n == name) = true :=
        List.find?_some (p := fun e : String × IRValue => e.1 == name) hf
      have hn : n = name := beq_iff_eq.mp hbeq
      subst hn
      subst h
      rfl

theorem params_find?_none {fn : Function} {name : String}
    (h : name ∉ fn.params.map (·.2)) :
    fn.params.find? (fun p => p.2 == name) = none := by
  rw [List.find?_eq_none]
  intro p hp hbeq
  exact h (List.mem_map.mpr ⟨p, hp, beq_iff_eq.mp hbeq⟩)

theorem initLocals_lookup_default {fn : Function} {args : List IRValue}
    {name : String} {ty : IrType}
    (hud : FunctionUsesDefined fn)
    (hnotParam : name ∉ fn.params.map (·.2))
    (hty : fn.localType? name = some ty) :
    (initLocals fn args).find? (fun e => e.1 == name) =
      some (name, IRValue.default ty) := by
  have hfindNone := params_find?_none hnotParam
  have hty0 := hty
  unfold Function.localType? at hty
  rw [hfindNone] at hty
  cases hdefFind : fn.localDefs.find? (fun e => e.1 == name) with
  | none => simp [hdefFind] at hty
  | some entry =>
      rcases entry with ⟨ename, ety⟩
      simp only [hdefFind, Option.map_some, Option.some.injEq] at hty
      have hmemFind := List.mem_of_find?_eq_some hdefFind
      have hebeq : (ename == name) = true :=
        List.find?_some (p := fun e : String × IrType => e.1 == name) hdefFind
      have hename : ename = name := beq_iff_eq.mp hebeq
      have hentry : (name, ty) ∈ fn.localDefs := by
        have hpair : (ename, ety) = (name, ty) := by
          rw [hename, hty]
        simpa [hpair] using hmemFind
      exact find?_initLocals_localDef hud hentry

def HostArgsLive (c : IRState) (fn : Function) (args : List IRValue) : Prop :=
  ∀ p a, (p, a) ∈ fn.params.zip args →
    (∀ slot off, a = .ptr (.root slot off) →
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length) ∧
    (∀ g off, a = .ptr (.global g off) →
      ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length) ∧
    (∀ frame slot off, a = .ptr (.stack frame slot off) → False)

theorem initLocals_find_cases {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue}
    (h : (initLocals fn args).find? (fun e => e.1 == name) = some (name, v)) :
    (∃ (p : IrType × String) (a : IRValue),
        p ∈ fn.params ∧ p.2 = name ∧ (p, a) ∈ fn.params.zip args ∧ a = v) ∨
      (∃ ty, (name, ty) ∈ fn.localDefs ∧ v = IRValue.default ty) :=
  mem_initLocals_cases (List.mem_of_find?_eq_some h)

theorem initLocals_no_stack {c : IRState} {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue} {frame slot off : Nat}
    (hlive : HostArgsLive c fn args)
    (h : (initLocals fn args).find? (fun e => e.1 == name) = some (name, v))
    (hv : v = .ptr (.stack frame slot off)) : False := by
  rcases initLocals_find_cases h with ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
  · exact (hlive p a hzip).2.2 frame slot off (by rw [ha, hv])
  · exact default_ne_stack ty frame slot off (by rw [← hvdef]; exact hv)

theorem initLocals_root_live {c : IRState} {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue} {slot off : Nat}
    (hlive : HostArgsLive c fn args)
    (h : (initLocals fn args).find? (fun e => e.1 == name) = some (name, v))
    (hv : v = .ptr (.root slot off)) :
    ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length := by
  rcases initLocals_find_cases h with ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
  · exact (hlive p a hzip).1 slot off (by rw [ha, hv])
  · exact absurd (by rw [← hvdef]; exact hv) (default_ne_root ty slot off)

theorem initLocals_global_live {c : IRState} {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue} {g : String} {off : Nat}
    (hlive : HostArgsLive c fn args)
    (h : (initLocals fn args).find? (fun e => e.1 == name) = some (name, v))
    (hv : v = .ptr (.global g off)) :
    ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length := by
  rcases initLocals_find_cases h with ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
  · exact (hlive p a hzip).2.1 g off (by rw [ha, hv])
  · exact absurd (by rw [← hvdef]; exact hv) (default_ne_global ty g off)

theorem initLocals_value_typed {fn : Function} {args : List IRValue}
    {name : String} {v : IRValue} {ty : IrType}
    (hud : FunctionUsesDefined fn) (hlen : args.length = fn.params.length)
    (hargs : ArgsTyped fn args)
    (hrep : ∀ name ty, fn.localType? name = some ty → ty.isRepresentable = true)
    (hv : (initLocals fn args).find? (fun e => e.1 == name) = some (name, v))
    (hty : fn.localType? name = some ty) : v.type? = some ty := by
  rcases lookupLocal_initLocals_of_localType hud hlen hargs hrep hty with
    ⟨v', hfind', htype'⟩
  rw [hv] at hfind'
  have hpair := Option.some.inj hfind'
  have hvv : v = v' := (Prod.mk.inj hpair).2
  rw [hvv]
  exact htype'

theorem initState?_wf {contract : IRContract} {module : IRFrag} {entry : String}
    {args : List IRValue} {roots : List (Nat × IRValue)}
    {functions : List (Nat × String)} {imports : List (Nat × ImportKind)}
    {globals : List (String × IRValue)} {storage : List (StoreId × List IRValue)}
    {functionSignatures : List (Nat × Signature)} {cycles : Nat} {c : IRState}
    (hprog : WFProgram contract module)
    (hinit : initState? module entry args roots functions imports globals storage
      functionSignatures cycles = some c)
    (hargsTyped : ∀ fn, module.function? entry = some fn → ArgsTyped fn args)
    (hargsLive : ∀ fn, module.function? entry = some fn → HostArgsLive c fn args)
    (hslots : ∀ f ∈ c.frames, SlotsBase c f)
    (hslotsStorage : SlotsStorage c)
    (hstorage : StorageValuesLive c)
    (hdomain : StorageDomain c)
    (hnoSlotsArgs : ∀ fn, module.function? entry = some fn → ∀ p a,
      (p, a) ∈ fn.params.zip args → ∀ g off, a = .ptr (.global g off) →
        g ∉ SlotsGlobals module)
    (hnoSlotsStorage : ∀ id off v g off', c.lookupStorageAt id off = some v →
      v = .ptr (.global g off') → g ∉ SlotsGlobals module)
    (hbindings : FunctionBindingsTyped c)
    (hcallTargets : CallTargetsTyped c)
    (hstackReturns : StackReturnsSafe c)
    (hslotsParams : SlotsParamsSafe c) :
    WFRunning contract c := by
  unfold initState? at hinit
  split at hinit
  · simp at hinit
  · rename_i fn hfn
    split at hinit
    · simp at hinit
    · rename_i block restBlocks hblocks
      split at hinit
      · rename_i hlen
        cases hinit
        have hfunMem : fn ∈ module.functions := mem_of_function? hfn
        have hblockMem : block ∈ fn.blocks := by
          rw [hblocks]
          exact List.mem_cons_self ..
        have hud : FunctionUsesDefined fn := hprog.wellFormed.usesDefined fn hfunMem
        refine ⟨?_, rfl, ?_⟩
        · constructor
          · simpa using hprog
          · simp
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            simp
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            exact hfunMem
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            exact ⟨block, hblockMem, rfl, rfl, [], by simp⟩
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            exact Or.inl ⟨rfl, ⟨restBlocks, hblocks⟩⟩
          · intro f rest hframes hinstrs hterm
            simp only [List.mem_singleton, List.cons.injEq] at hframes
            rcases hframes with ⟨rfl, rfl⟩
            have hguard := hprog.unreachableGuarded fn hfunMem block hblockMem hterm
            rcases hguard with ⟨instr, hlast, _⟩
            have hinstrs' : block.instrs = [] := by simpa using hinstrs
            rw [hinstrs'] at hlast
            simp at hlast
          · intro f hf name ty hty
            simp only [List.mem_singleton] at hf
            subst hf
            rcases lookupLocal_initLocals_of_localType hud hlen (hargsTyped fn hfn)
              (hprog.localTypesRepresentable fn hfunMem) hty with ⟨v, hv, _⟩
            exact ⟨v, by simpa [IRFrame.lookupLocal, hv]⟩
          · intro f hf name v hv ty hty
            simp only [List.mem_singleton] at hf
            subst hf
            have hfind := lookupLocal_find?_eq hv
            exact initLocals_value_typed hud hlen (hargsTyped fn hfn)
              (hprog.localTypesRepresentable fn hfunMem) hfind hty
          · intro f hf entry he
            simp only [List.mem_singleton] at hf
            subst hf
            rcases hprog.originsTrusted fn hfunMem with ⟨_, htrusted⟩
            rcases htrusted entry he with ⟨hlocTy, hnotParam, _⟩
            have hfind := initLocals_lookup_default (args := args) hud hnotParam hlocTy
            refine ⟨.null, ?_, hlocTy, Or.inl rfl⟩
            simpa [IRFrame.lookupLocal, IRValue.default] using congrArg (Option.map (·.2)) hfind
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            simp [AllocsWellFormed, IRFrame.allocType?]
          · intro f hf name v addr hv haddr
            simp only [List.mem_singleton] at hf
            subst hf
            have hfind := lookupLocal_find?_eq hv
            cases addr with
            | stack frame slot off =>
                exact False.elim (initLocals_no_stack (hargsLive fn hfn) hfind haddr)
            | slotEntry slot => trivial
            | cell slot => trivial
            | root slot off => trivial
            | global name off => trivial
            | slotTable => trivial
            | function name => trivial
            | importCell slot => trivial
          · intro f hf addr v hmem
            simp only [List.mem_singleton] at hf
            subst hf
            simp [IRFrame.lookupMem] at hmem
          · intro f hf
            simp only [List.mem_singleton] at hf
            subst hf
            constructor
            · intro name v slot off hv hval
              exact initLocals_root_live (hargsLive fn hfn)
                (lookupLocal_find?_eq hv) hval
            · intro name v g off hv hval
              exact initLocals_global_live (hargsLive fn hfn)
                (lookupLocal_find?_eq hv) hval
          · exact hslots
          · exact hslotsStorage
          · exact hstorage
          · exact hdomain
          · constructor
            · intro f hf name v gname off hv hval
              simp only [List.mem_singleton] at hf
              subst hf
              rcases initLocals_find_cases (lookupLocal_find?_eq hv) with
                ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
              · exact hnoSlotsArgs fn hfn p a hzip gname off (by rw [ha, hval])
              · exact absurd (by rw [← hvdef]; exact hval) (default_ne_global ty gname off)
            · constructor
              · intro f hf addr v gname off hmem hval
                simp only [List.mem_singleton] at hf
                subst hf
                simp [IRFrame.lookupMem] at hmem
              · exact hnoSlotsStorage
          · exact hbindings
          · exact hcallTargets
          · intro pre callee caller suffix hframes
            have hlen : (pre ++ callee :: caller :: suffix).length = 1 := by
              rw [← hframes]
              rfl
            simp only [List.length_append, List.length_cons] at hlen
            omega
          · intro last hlast
            simp at hlast
            cases hlast
            rfl
          · exact hstackReturns
          · exact hslotsParams
        · simp

      · simp at hinit

theorem find?_filter_of_imp {α : Type} (l : List α) (p q : α → Bool)
    (h : ∀ a, q a = true → p a = true) :
    (l.filter p).find? q = l.find? q := by
  induction l with
  | nil => rfl
  | cons a as ih =>
      simp only [List.filter_cons]
      by_cases hp : p a = true
      · rw [if_pos hp]
        by_cases hq : q a = true
        · rw [List.find?_cons_of_pos hq, List.find?_cons_of_pos hq]
        · rw [List.find?_cons_of_neg hq, List.find?_cons_of_neg hq]
          exact ih
      · rw [if_neg hp]
        by_cases hq : q a = true
        · exact absurd (h a hq) (by simp [hp])
        · rw [List.find?_cons_of_neg hq]
          exact ih

theorem find?_setList_eq {α β : Type} [BEq α] [LawfulBEq α]
    (entries : List (α × β)) (key : α) (value : β) :
    (setList entries key value).find? (fun e => e.1 == key) = some (key, value) := by
  unfold setList
  rw [List.find?_cons_of_pos]
  simp [beq_iff_eq]

theorem find?_setList_ne {α β : Type} [BEq α] [LawfulBEq α]
    (entries : List (α × β)) {key key' : α} (value : β)
    (hne : (key' == key) = false) :
    (setList entries key value).find? (fun e => e.1 == key') =
      entries.find? (fun e => e.1 == key') := by
  unfold setList
  have hk : (key == key') = false := by
    rw [beq_eq_false_iff_ne] at hne ⊢
    intro h
    exact hne h.symm
  rw [List.find?_cons_of_neg (by simp [hk])]
  apply find?_filter_of_imp
  intro a hq
  have ha : a.1 = key' := beq_iff_eq.mp hq
  have hak : (a.1 == key) = false := by
    rw [beq_eq_false_iff_ne]
    intro h
    exact (beq_eq_false_iff_ne.mp hne) (by rw [← ha]; exact h)
  simp [hak]

theorem lookupList_setList_eq {α β : Type} [BEq α] [LawfulBEq α]
    (entries : List (α × β)) (key : α) (value : β) :
    lookupList (setList entries key value) key = some value := by
  unfold lookupList
  rw [find?_setList_eq]
  rfl

theorem lookupList_setList_ne {α β : Type} [BEq α] [LawfulBEq α]
    (entries : List (α × β)) {key key' : α} (value : β)
    (hne : (key' == key) = false) :
    lookupList (setList entries key value) key' = lookupList entries key' := by
  unfold lookupList
  rw [find?_setList_ne entries value hne]

theorem lookupLocal_setLocal_eq (f : IRFrame) (name : String) (v : IRValue) :
    (f.setLocal name v).lookupLocal name = some v := by
  unfold IRFrame.setLocal IRFrame.lookupLocal
  rw [find?_setList_eq]
  rfl

theorem lookupLocal_setLocal_ne (f : IRFrame) {name other : String}
    (h : (other == name) = false) (v : IRValue) :
    (f.setLocal name v).lookupLocal other = f.lookupLocal other := by
  unfold IRFrame.setLocal IRFrame.lookupLocal
  rw [find?_setList_ne _ v h]

theorem lookupMem_setMem_eq (f : IRFrame) (addr : IRAddr) (v : IRValue) :
    (f.setMem addr v).lookupMem addr = some v := by
  unfold IRFrame.setMem IRFrame.lookupMem
  rw [find?_setList_eq]
  rfl

theorem lookupMem_setMem_ne (f : IRFrame) {addr other : IRAddr}
    (h : (other == addr) = false) (v : IRValue) :
    (f.setMem addr v).lookupMem other = f.lookupMem other := by
  unfold IRFrame.setMem IRFrame.lookupMem
  rw [find?_setList_ne _ v h]

theorem localsInitialized_setLocal {f : IRFrame} {dest : String} {value : IRValue}
    (h : ∀ name ty, f.fn.localType? name = some ty → ∃ v, f.lookupLocal name = some v) :
    ∀ name ty, f.fn.localType? name = some ty →
      ∃ v, (f.setLocal dest value).lookupLocal name = some v := by
  intro name ty hty
  by_cases hname : name = dest
  · subst hname
    exact ⟨value, lookupLocal_setLocal_eq f name value⟩
  · rcases h name ty hty with ⟨v, hv⟩
    exact ⟨v, by
      rw [lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
      exact hv⟩

theorem localsTyped_setLocal {f : IRFrame} {dest : String} {value : IRValue}
    (h : ∀ name v, f.lookupLocal name = some v → ∀ ty,
      f.fn.localType? name = some ty → v.type? = some ty)
    (hdest : ∀ ty, f.fn.localType? dest = some ty → value.type? = some ty) :
    ∀ name v, (f.setLocal dest value).lookupLocal name = some v → ∀ ty,
      f.fn.localType? name = some ty → v.type? = some ty := by
  intro name v hv ty hty
  by_cases hname : name = dest
  · subst hname
    rw [lookupLocal_setLocal_eq] at hv
    have hvEq : v = value := (Option.some.inj hv).symm
    rw [hvEq]
    exact hdest ty hty
  · have hvOld : f.lookupLocal name = some v := by
      rw [← lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
      exact hv
    exact h name v hvOld ty hty

theorem originOf?_of_mem {facts : FunctionFacts}
    (hnd : (facts.origins.map (·.1)).Nodup)
    {name : String} {origin : PointerOrigin}
    (hmem : (name, origin) ∈ facts.origins) :
    facts.originOf? name = some origin := by
  unfold FunctionFacts.originOf?
  revert hnd hmem
  induction facts.origins with
  | nil => intro hnd hmem; simp at hmem
  | cons hd tl ih =>
      intro hnd hmem
      rw [List.map_cons, List.nodup_cons] at hnd
      rcases hd with ⟨a, b⟩
      rcases List.mem_cons.mp hmem with hhead | htail
      · cases hhead
        rw [List.find?_cons_of_pos
          (p := fun entry : String × PointerOrigin => entry.1 == name)
          (a := (name, origin)) (beq_iff_eq.mpr rfl)]
        rfl
      · have hneq : (a == name) = false := by
          rw [beq_eq_false_iff_ne]
          intro h
          exact hnd.1 (List.mem_map.mpr ⟨(name, origin), htail, by rw [h]⟩)
        have hneg : ¬(a == name) = true := by
          rw [hneq]
          intro h
          cases h
        rw [List.find?_cons_of_neg
          (p := fun entry : String × PointerOrigin => entry.1 == name)
          (a := (a, b)) hneg]
        exact ih hnd.2 htail

theorem originsMatch_setLocal {contract : IRContract} {c : IRState}
    {f : IRFrame} {dest : String} {value : IRValue}
    (hnd : (f.fn.facts.origins.map (·.1)).Nodup)
    (h : ∀ entry ∈ f.fn.facts.origins,
      OriginHolds contract c f entry.1 entry.2)
    (hnew : ∀ origin, f.fn.facts.originOf? dest = some origin →
      OriginHolds contract c (f.setLocal dest value) dest origin) :
    ∀ entry ∈ f.fn.facts.origins,
      OriginHolds contract c (f.setLocal dest value) entry.1 entry.2 := by
  intro entry he
  by_cases hname : entry.1 = dest
  · subst hname
    exact hnew entry.2 (originOf?_of_mem hnd he)
  · rcases h entry he with ⟨v, hv, hty, hshape⟩
    refine ⟨v, ?_, hty, hshape⟩
    rw [lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
    exact hv

theorem stackPtrsBounded_setLocal {c : IRState} {f : IRFrame}
    {dest : String} {value : IRValue}
    (h : StackPtrsBounded c f)
    (hval : ∀ frame slot off, value = .ptr (.stack frame slot off) →
      ∃ g, c.frameById? frame = some g ∧
        ∃ ty, g.allocType? slot = some ty ∧ off < allocSize ty) :
    StackPtrsBounded c (f.setLocal dest value) := by
  intro name v addr hv haddr
  by_cases hname : name = dest
  · subst hname
    rw [lookupLocal_setLocal_eq] at hv
    have hvEq : v = value := (Option.some.inj hv).symm
    subst hvEq
    cases addr with
    | stack frame slot off =>
        exact hval frame slot off haddr
    | slotEntry slot => trivial
    | cell slot => trivial
    | root slot off => trivial
    | global name off => trivial
    | slotTable => trivial
    | function name => trivial
    | importCell slot => trivial
  · have hvOld : f.lookupLocal name = some v := by
      rw [← lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
      exact hv
    exact h name v addr hvOld haddr

theorem storagePtrsLive_setLocal {c : IRState} {f : IRFrame}
    {dest : String} {value : IRValue}
    (h : StoragePtrsLive c f)
    (hroot : ∀ slot off, value = .ptr (.root slot off) →
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length)
    (hglobal : ∀ g off, value = .ptr (.global g off) →
      ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length) :
    StoragePtrsLive c (f.setLocal dest value) := by
  constructor
  · intro name v slot off hv hval
    by_cases hname : name = dest
    · subst hname
      rw [lookupLocal_setLocal_eq] at hv
      have hvEq : v = value := (Option.some.inj hv).symm
      subst hvEq
      exact hroot slot off hval
    · have hvOld : f.lookupLocal name = some v := by
        rw [← lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
        exact hv
      exact h.1 name v slot off hvOld hval
  · intro name v g off hv hval
    by_cases hname : name = dest
    · subst hname
      rw [lookupLocal_setLocal_eq] at hv
      have hvEq : v = value := (Option.some.inj hv).symm
      subst hvEq
      exact hglobal g off hval
    · have hvOld : f.lookupLocal name = some v := by
        rw [← lookupLocal_setLocal_ne f (beq_eq_false_iff_ne.mpr hname) value]
        exact hv
      exact h.2 name v g off hvOld hval

theorem allocType?_setLocal (f : IRFrame) (name : String) (value : IRValue) (slot : Nat) :
    (f.setLocal name value).allocType? slot = f.allocType? slot := rfl

theorem allocType?_setMem (f : IRFrame) (addr : IRAddr) (value : IRValue) (slot : Nat) :
    (f.setMem addr value).allocType? slot = f.allocType? slot := rfl

theorem allocType?_withInstrs (f : IRFrame) (instrs : List Instr) (slot : Nat) :
    ({ f with instrs := instrs } : IRFrame).allocType? slot = f.allocType? slot := rfl

theorem allocType?_alloca_old {f : IRFrame} {ty : IrType} {slot : Nat} {oldTy : IrType}
    (h : f.allocType? slot = some oldTy) :
    ({ f with
       allocs := f.allocs ++ [(f.nextAlloc, ty)],
       nextAlloc := f.nextAlloc + 1 } : IRFrame).allocType? slot = some oldTy := by
  unfold IRFrame.allocType? at h ⊢
  rw [List.find?_append]
  cases hf : f.allocs.find? (fun entry => entry.1 == slot) with
  | none => simp [hf] at h
  | some entry => simpa [hf] using h

theorem memValuePtrBounded_default (c : IRState) : ∀ ty : IrType,
    MemValuePtrBounded c (IRValue.default ty)
  | .int _ => trivial
  | .ptr => trivial
  | .overflow _ => trivial
  | .array elem _ => memValuePtrBounded_default c elem
  | .void => trivial

/-- Proof helper: every frame lookup in the old state is found in the new state
with the same allocation types. This is the frame view preserved by `MemTyped`,
`MemValuePtrBounded`, and `StackPtrsBounded` across the state updates of `irStep`. -/
def FrameLookupsPreserved (s s' : IRState) : Prop :=
  ∀ id g, s.frameById? id = some g →
    ∃ g', s'.frameById? id = some g' ∧
      ∀ slot ty, g.allocType? slot = some ty → g'.allocType? slot = some ty

theorem memValuePtrBounded_of_frameLookups {s s' : IRState}
    (h : FrameLookupsPreserved s s') {v : IRValue}
    (hv : MemValuePtrBounded s v) : MemValuePtrBounded s' v := by
  cases v with
  | int ty payload => trivial
  | pair ty value overflow => trivial
  | null => trivial
  | ptr addr =>
      cases addr with
      | stack frame slot off =>
          simp only [MemValuePtrBounded] at hv ⊢
          rcases hv with ⟨g, hfind, ty, hty, hoff⟩
          rcases h frame g hfind with ⟨g', hfind', halloc⟩
          exact ⟨g', hfind', ty, halloc slot ty hty, hoff⟩
      | slotEntry slot => trivial
      | cell slot => trivial
      | root slot off => trivial
      | global name off => trivial
      | slotTable => trivial
      | function name => trivial
      | importCell slot => trivial

theorem memTyped_of_frameLookups {s s' : IRState} {f : IRFrame}
    (h : FrameLookupsPreserved s s') (hm : MemTyped s f) : MemTyped s' f := by
  intro addr v hv
  rcases hm addr v hv with ⟨slot, off, ty, haddr, hty, hoff, hvt, hvb⟩
  exact ⟨slot, off, ty, haddr, hty, hoff, hvt,
    memValuePtrBounded_of_frameLookups h hvb⟩

theorem stackPtrsBounded_of_frameLookups {s s' : IRState} {f : IRFrame}
    (h : FrameLookupsPreserved s s') (hb : StackPtrsBounded s f) :
    StackPtrsBounded s' f := by
  intro name v addr hv haddr
  have h' := hb name v addr hv haddr
  cases addr with
  | stack frame slot off =>
      rcases h' with ⟨g, hfind, ty, hty, hoff⟩
      rcases h frame g hfind with ⟨g', hfind', halloc⟩
      exact ⟨g', hfind', ty, halloc slot ty hty, hoff⟩
  | slotEntry slot => trivial
  | cell slot => trivial
  | root slot off => trivial
  | global name off => trivial
  | slotTable => trivial
  | function name => trivial
  | importCell slot => trivial

theorem frameLookups_withHeadInstrs (s : IRState) (instrs : List Instr) :
    FrameLookupsPreserved s (s.withHeadInstrs instrs) := by
  intro id g hfind
  cases hframes : s.frames with
  | nil =>
      exfalso
      have hnone : s.frameById? id = none := by
        unfold IRState.frameById?
        rw [hframes]
        rfl
      rw [hnone] at hfind
      cases hfind
  | cons g0 rest =>
      by_cases hgt : (g0.id == id) = true
      · have hsome : s.frameById? id = some g0 := by
          unfold IRState.frameById?
          rw [hframes]
          exact List.find?_cons_of_pos hgt
        rw [hsome] at hfind
        have hg : g = g0 := (Option.some.inj hfind).symm
        subst g
        refine ⟨{ g0 with instrs := instrs }, ?_, ?_⟩
        · unfold IRState.withHeadInstrs IRState.frameById?
          rw [hframes]
          exact List.find?_cons_of_pos hgt
        · intro slot ty hty
          exact hty
      · have hrest : rest.find? (fun f : IRFrame => f.id == id) = some g := by
          unfold IRState.frameById? at hfind
          rw [hframes, List.find?_cons_of_neg (p := fun f : IRFrame => f.id == id)
            (a := g0) hgt] at hfind
          exact hfind
        refine ⟨g, ?_, ?_⟩
        · unfold IRState.withHeadInstrs IRState.frameById?
          rw [hframes]
          rw [List.find?_cons_of_neg (p := fun f : IRFrame => f.id == id)
            (a := { g0 with instrs := instrs }) ?_]
          · exact hrest
          · simpa using hgt
        · intro slot ty hty
          exact hty

theorem frameLookups_withHead {s : IRState} {h : IRFrame}
    (hid : ∀ g, s.frames.head? = some g → h.id = g.id)
    (halloc : ∀ g, s.frames.head? = some g →
      ∀ slot ty, g.allocType? slot = some ty → h.allocType? slot = some ty) :
    FrameLookupsPreserved s (s.withHead h) := by
  intro id g hfind
  cases hframes : s.frames with
  | nil =>
      exfalso
      have hnone : s.frameById? id = none := by
        unfold IRState.frameById?
        rw [hframes]
        rfl
      rw [hnone] at hfind
      cases hfind
  | cons g0 rest =>
      have hhead : s.frames.head? = some g0 := by
        rw [hframes]
        rfl
      have hidEq : h.id = g0.id := hid g0 hhead
      by_cases hgt : (g0.id == id) = true
      · have hsome : s.frameById? id = some g0 := by
          unfold IRState.frameById?
          rw [hframes]
          exact List.find?_cons_of_pos hgt
        rw [hsome] at hfind
        have hg : g = g0 := (Option.some.inj hfind).symm
        subst g
        refine ⟨h, ?_, ?_⟩
        · unfold IRState.withHead IRState.frameById?
          rw [hframes]
          exact List.find?_cons_of_pos (a := h) (p := fun f : IRFrame => f.id == id)
            (by rw [hidEq]; exact hgt)
        · intro slot ty hty
          exact halloc g0 hhead slot ty hty
      · have hrest : rest.find? (fun f : IRFrame => f.id == id) = some g := by
          unfold IRState.frameById? at hfind
          rw [hframes, List.find?_cons_of_neg (p := fun f : IRFrame => f.id == id)
            (a := g0) hgt] at hfind
          exact hfind
        refine ⟨g, ?_, ?_⟩
        · unfold IRState.withHead IRState.frameById?
          rw [hframes]
          rw [List.find?_cons_of_neg (p := fun f : IRFrame => f.id == id) (a := h) ?_]
          · exact hrest
          · rw [hidEq]
            exact hgt
        · intro slot ty hty
          exact hty

theorem frameLookups_storeFrames (s : IRState) (frame slot off : Nat) (value : IRValue) :
    FrameLookupsPreserved s
      { s with frames := s.frames.map fun g =>
          if g.id == frame then g.setMem (.stack frame slot off) value else g } := by
  intro id g hfind
  unfold IRState.frameById? at hfind
  let F : IRFrame → IRFrame :=
    fun g => if g.id == frame then g.setMem (.stack frame slot off) value else g
  have hFid : ∀ g, (F g).id = g.id := by
    intro g
    unfold F
    split <;> rfl
  refine ⟨F g, ?_, ?_⟩
  · show ({ s with frames := s.frames.map F } : IRState).frameById? id = some (F g)
    simp only [IRState.frameById?]
    rw [List.find?_map]
    have hpred : ((fun frame : IRFrame => frame.id == id) ∘ F) =
        (fun a : IRFrame => a.id == id) := by
      funext a
      simp only [Function.comp_apply]
      rw [hFid a]
    rw [hpred, hfind]
    rfl
  · intro slot' ty hty
    unfold F
    by_cases hgt : (g.id == frame) = true
    · rw [ite_eq_left hgt, allocType?_setMem]
      exact hty
    · rw [ite_eq_right hgt]
      exact hty

theorem frame_eq_of_mem_of_frameById? {l : List IRFrame} (hnd : (l.map (·.id)).Nodup)
    {id : Nat} {g0 : IRFrame} (h0 : l.find? (fun f => f.id == id) = some g0)
    {g : IRFrame} (hmem : g ∈ l) (hid : (g.id == id) = true) : g = g0 := by
  have h0mem : g0 ∈ l :=
    List.mem_of_find?_eq_some (p := fun f : IRFrame => f.id == id) (a := g0) h0
  have h0id : g0.id = id := beq_iff_eq.mp
    (List.find?_some (p := fun f : IRFrame => f.id == id) (a := g0) h0)
  have hgid : g.id = id := beq_iff_eq.mp hid
  obtain ⟨i, hi⟩ := List.getElem?_of_mem hmem
  obtain ⟨j, hj⟩ := List.getElem?_of_mem h0mem
  have hiLt : i < l.length := (List.getElem?_eq_some_iff.mp hi).choose
  have hiMap : (l.map (·.id))[i]? = some g.id := by
    rw [List.getElem?_map, hi]
    rfl
  have hjMap : (l.map (·.id))[j]? = some g0.id := by
    rw [List.getElem?_map, hj]
    rfl
  have hEq : (l.map (·.id))[i]? = (l.map (·.id))[j]? := by
    rw [hiMap, hjMap, hgid, h0id]
  have hij : i = j := (hnd.getElem?_inj (by simpa using hiLt)).mp hEq
  rw [hij] at hi
  rw [hi] at hj
  exact Option.some.inj hj

theorem memTyped_setMem_stack {c : IRState} {f : IRFrame} {slot off : Nat}
    {ty : IrType} {value : IRValue}
    (h : MemTyped c f) (hty : f.allocType? slot = some ty)
    (hoff : off < allocSize ty) (hvt : MemValueTyped ty value)
    (hvb : MemValuePtrBounded c value) :
    MemTyped c (f.setMem (.stack f.id slot off) value) := by
  intro addr v hv
  by_cases hsame : addr = .stack f.id slot off
  · rw [hsame] at hv ⊢
    rw [lookupMem_setMem_eq] at hv
    have hvEq : v = value := (Option.some.inj hv).symm
    subst hvEq
    exact ⟨slot, off, ty, rfl, hty, hoff, hvt, hvb⟩
  · have hvOld : f.lookupMem addr = some v := by
      rw [← lookupMem_setMem_ne f (beq_eq_false_iff_ne.mpr hsame) value]
      exact hv
    exact h addr v hvOld

theorem storeValue_stack_eq {s : IRState} {frame slot offset : Nat} {value : IRValue}
    {g0 : IRFrame} {ty : IrType}
    (hfind : s.frameById? frame = some g0)
    (halloc : g0.allocType? slot = some ty)
    (hoff : offset < allocSize ty) :
    s.storeValue (.stack frame slot offset) value =
      some { s with frames := s.frames.map fun g =>
        if g.id == frame then g.setMem (.stack frame slot offset) value else g } := by
  unfold IRState.storeValue
  simp only [hfind, halloc]
  simp [hoff]

theorem memTyped_storeFrames {s : IRState} {frame slot off : Nat} {value : IRValue}
    {ty : IrType} {g0 : IRFrame}
    (hnd : (s.frames.map (·.id)).Nodup)
    (hmem : ∀ g ∈ s.frames, MemTyped s g)
    (hfind : s.frameById? frame = some g0)
    (halloc : g0.allocType? slot = some ty)
    (hoff : off < allocSize ty)
    (hvt : MemValueTyped ty value)
    (hvb : MemValuePtrBounded s value) :
    ∀ g ∈ s.frames.map (fun g =>
        if g.id == frame then g.setMem (.stack frame slot off) value else g),
      MemTyped
        { s with frames := s.frames.map fun g =>
            if g.id == frame then g.setMem (.stack frame slot off) value else g } g := by
  intro g' hg'
  let F : IRFrame → IRFrame :=
    fun g => if g.id == frame then g.setMem (.stack frame slot off) value else g
  let s' : IRState := { s with frames := s.frames.map F }
  have hlook : FrameLookupsPreserved s s' := frameLookups_storeFrames s frame slot off value
  have hvb' : MemValuePtrBounded s' value := memValuePtrBounded_of_frameLookups hlook hvb
  rcases List.mem_map.mp hg' with ⟨g, hgMem, hgEq⟩
  rw [← hgEq]
  show MemTyped s' (F g)
  by_cases hgt : (g.id == frame) = true
  · have hF : F g = g.setMem (.stack frame slot off) value := by
      unfold F
      rw [ite_eq_left hgt]
    rw [hF]
    have hgEq0 : g = g0 := frame_eq_of_mem_of_frameById? hnd hfind hgMem hgt
    subst g
    have hframeEq : frame = g0.id := (beq_iff_eq.mp hgt).symm
    rw [hframeEq]
    exact memTyped_setMem_stack (memTyped_of_frameLookups hlook (hmem g0 hgMem))
      halloc hoff hvt hvb'
  · have hF : F g = g := by
      unfold F
      rw [ite_eq_right hgt]
    rw [hF]
    exact memTyped_of_frameLookups hlook (hmem g hgMem)

theorem memTyped_withHeadInstrs {s : IRState} {instrs : List Instr}
    (h : ∀ g ∈ s.frames, MemTyped s g) :
    ∀ g ∈ (s.withHeadInstrs instrs).frames, MemTyped (s.withHeadInstrs instrs) g := by
  intro g' hg'
  have hlook : FrameLookupsPreserved s (s.withHeadInstrs instrs) :=
    frameLookups_withHeadInstrs s instrs
  cases hframes : s.frames with
  | nil =>
      have hsame : s.withHeadInstrs instrs = s := by
        simp [IRState.withHeadInstrs, hframes]
      rw [hsame] at hg' ⊢
      exact h g' hg'
  | cons g0 rest =>
      have hframes' : (s.withHeadInstrs instrs).frames =
          { g0 with instrs := instrs } :: rest := by
        simp [IRState.withHeadInstrs, hframes]
      rw [hframes'] at hg'
      rcases List.mem_cons.mp hg' with hhead | htail
      · subst hhead
        have hm : MemTyped s g0 := h g0 (by rw [hframes]; exact List.mem_cons_self ..)
        exact memTyped_of_frameLookups hlook hm
      · have hm : MemTyped s g' :=
          h g' (by rw [hframes]; exact List.mem_cons_of_mem g0 htail)
        exact memTyped_of_frameLookups hlook hm



theorem memValuePtrBounded_of_evalOperand {c : IRState} {f : IRFrame}
    (hstack : StackPtrsBounded c f) {operand : Operand} {v : IRValue}
    (h : evalOperand f operand = some v) : MemValuePtrBounded c v := by
  cases operand with
  | «local» name =>
      simp only [evalOperand] at h
      cases v with
      | int ty payload => trivial
      | pair ty value overflow => trivial
      | null => trivial
      | ptr addr =>
          cases addr with
          | stack frame slot off =>
              simp only [MemValuePtrBounded]
              exact hstack name _ _ h rfl
          | slotEntry slot => trivial
          | cell slot => trivial
          | root slot off => trivial
          | global name off => trivial
          | slotTable => trivial
          | function name => trivial
          | importCell slot => trivial
  | intLit ty value =>
      simp only [evalOperand] at h
      cases h
      trivial
  | nullPtr =>
      simp only [evalOperand] at h
      cases h
      trivial
  | symbol name =>
      simp only [evalOperand] at h
      cases h
      trivial

/-- The empty-frame halt form: changing only the machine status preserves the
state invariant. The halt is the only `.step` outcome that changes the status. -/
theorem wfState_status {contract : IRContract} {c : IRState} (st : IRStatus)
    (hwf : WFState contract c) : WFState contract { c with status := st } := by
  obtain ⟨h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, h13, h14, h15, h16,
    h17, h18, h19, h20, h21, h22, h23, h24, h25⟩ := hwf
  exact ⟨h1, h2, h3, h4, h5, h6, h7, h8, h9, h10, h11, h12, h13, h14, h15, h16,
    h17, h18, h19, h20, h21, h22, h23, h24, h25⟩

theorem wfState_withHead {contract : IRContract} {c : IRState} {f g : IRFrame}
    {rest : List IRFrame}
    (hwf : WFState contract c) (hframes : c.frames = f :: rest)
    (hid : g.id = f.id) (hfn : g.fn = f.fn) (hblock : g.block = f.block)
    (hterm : g.terminator = f.terminator) (hprev : g.prev = f.prev)
    (hretDest : g.retDest = f.retDest)
    (halloc : ∀ slot ty, f.allocType? slot = some ty → g.allocType? slot = some ty)
    (hblockMem : ∃ block ∈ g.fn.blocks, g.block = block ∧
      g.terminator = block.terminator ∧ ∃ done, block.instrs = done ++ g.instrs)
    (hprevEdge : PrevEdge c g)
    (hnoBare : g.instrs = [] → g.terminator ≠ some .unreachable)
    (hlocalsInit : ∀ name ty, g.fn.localType? name = some ty →
      ∃ v, g.lookupLocal name = some v)
    (hlocalsTyped : ∀ name v, g.lookupLocal name = some v → ∀ ty,
      g.fn.localType? name = some ty → v.type? = some ty)
    (horigins : ∀ entry ∈ g.fn.facts.origins,
      OriginHolds contract c g entry.1 entry.2)
    (hallocs : AllocsWellFormed g)
    (hstackPtrs : StackPtrsBounded c g)
    (hmem : MemTyped c g)
    (hstoragePtrs : StoragePtrsLive c g)
    (hslotsBase : SlotsBase c g)
    (hnoSlotsLocals : ∀ name v gname off, g.lookupLocal name = some v →
      v = .ptr (.global gname off) → gname ∉ SlotsGlobals c.module)
    (hnoSlotsMem : ∀ addr v gname off, g.lookupMem addr = some v →
      v = .ptr (.global gname off) → gname ∉ SlotsGlobals c.module)
    (hstackReturns : ∀ operand v, g.terminator = some (.ret (some operand)) →
      evalOperand g operand = some v →
      ∀ frame slot off, v = .ptr (.stack frame slot off) → frame ≠ g.id)
    (hslotsParams : ∀ p : IrType × String, g.fn.params.getLast? = some p →
      p.2 = "%slots" → g.lookupLocal p.2 = some (.ptr .slotTable)) :
    WFState contract (c.withHead g) := by
  have hhead : c.frames.head? = some f := by rw [hframes]; rfl
  have hwithHead : c.withHead g = { c with frames := g :: rest } := by
    simp [IRState.withHead, hframes]
  rw [hwithHead]
  have hlook : FrameLookupsPreserved c { c with frames := g :: rest } := by
    rw [← hwithHead]
    apply frameLookups_withHead
    · intro g0 hg0
      rw [hhead] at hg0
      have hfg : g0 = f := (Option.some.inj hg0).symm
      rw [hfg]
      exact hid
    · intro g0 hg0 slot ty hty
      rw [hhead] at hg0
      have hfg : g0 = f := (Option.some.inj hg0).symm
      rw [hfg] at hty
      exact halloc slot ty hty
  constructor
  · exact hwf.program
  · have hmap : (g :: rest).map (·.id) = (f :: rest).map (·.id) := by
      simp [hid]
    rw [hmap, ← hframes]
    exact hwf.frameIdsNodup
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · rw [hid]
      exact hwf.frameIdsBound f (by rw [hframes]; exact List.mem_cons_self ..)
    · exact hwf.frameIdsBound f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · rw [hfn]
      exact hwf.frameFunction f (by rw [hframes]; exact List.mem_cons_self ..)
    · exact hwf.frameFunction f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact hblockMem
    · exact hwf.frameBlock f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact hprevEdge
    · exact hwf.framePrev f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' rest' heq hinst
    injection heq with hgf _
    have hginst : g.instrs = [] := by rw [hgf]; exact hinst
    have hgterm : g.terminator ≠ some .unreachable := hnoBare hginst
    intro hterm
    exact hgterm (by rw [hgf]; exact hterm)
  · intro f' hf' name ty hty
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact hlocalsInit name ty hty
    · exact hwf.localsInitialized f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
        name ty hty
  · intro f' hf' name v hv ty hty
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact hlocalsTyped name v hv ty hty
    · exact hwf.localsTyped f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
        name v hv ty hty
  · intro f' hf' entry hentry
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact horigins entry hentry
    · exact hwf.originsMatch f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
        entry hentry
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact hallocs
    · exact hwf.allocsWellFormed f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact stackPtrsBounded_of_frameLookups hlook hstackPtrs
    · exact stackPtrsBounded_of_frameLookups hlook
        (hwf.stackPtrsBounded f' (by rw [hframes]; exact List.mem_cons_of_mem f hf'))
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact memTyped_of_frameLookups hlook hmem
    · exact memTyped_of_frameLookups hlook
        (hwf.memTyped f' (by rw [hframes]; exact List.mem_cons_of_mem f hf'))
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact hstoragePtrs
    · exact hwf.storagePtrsLive f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · intro f' hf'
    have hf'' : f' ∈ g :: rest := by simpa using hf'
    rcases List.mem_cons.mp hf'' with rfl | hf'
    · exact hslotsBase
    · exact hwf.slotsBase f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
  · exact hwf.slotsStorage
  · exact hwf.storageValuesLive
  · exact hwf.storageDomain
  · constructor
    · intro f' hf' name v gname off hv hval
      have hf'' : f' ∈ g :: rest := by simpa using hf'
      rcases List.mem_cons.mp hf'' with rfl | hf'
      · exact hnoSlotsLocals name v gname off hv hval
      · exact hwf.noSlotsGlobalPointers.1 f'
          (by rw [hframes]; exact List.mem_cons_of_mem f hf') name v gname off hv hval
    · constructor
      · intro f' hf' addr v gname off hv hval
        have hf'' : f' ∈ g :: rest := by simpa using hf'
        rcases List.mem_cons.mp hf'' with rfl | hf'
        · exact hnoSlotsMem addr v gname off hv hval
        · exact hwf.noSlotsGlobalPointers.2.1 f'
            (by rw [hframes]; exact List.mem_cons_of_mem f hf') addr v gname off hv hval
      · exact hwf.noSlotsGlobalPointers.2.2
  · exact hwf.functionBindings
  · exact hwf.callTargets
  · intro pre callee caller suffix heq
    cases pre with
    | nil =>
        simp only [List.nil_append] at heq
        injection heq with hgc hrest
        subst hgc
        rw [hretDest, hfn]
        exact hwf.returnLink [] f caller suffix (by rw [hframes, hrest]; rfl)
    | cons a as =>
        simp only [List.cons_append] at heq
        injection heq with _ hrest
        exact hwf.returnLink (f :: as) callee caller suffix (by
          rw [hframes, hrest]; rfl)
  · intro last hlast
    cases rest with
    | nil =>
        have hcf : c.frames.getLast? = some f := by rw [hframes]; rfl
        have hfret : f.retDest = none := hwf.bottomRetDest f hcf
        have hgl : last = g := by
          have hg : (g :: ([] : List IRFrame)).getLast? = some g := rfl
          rw [hg] at hlast
          exact (Option.some.inj hlast).symm
        rw [hgl, hretDest, hfret]
    | cons h t =>
        have hcf : c.frames.getLast? = some last := by
          rw [hframes]
          simpa using hlast
        exact hwf.bottomRetDest last hcf
  · intro f' hf' operand v htermv heval frame slot off hv
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact hstackReturns operand v htermv heval frame slot off hv
    · exact hwf.stackReturnsSafe f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
        operand v htermv heval frame slot off hv
  · intro f' hf' p hp hname
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact hslotsParams p hp hname
    · exact hwf.slotsParamsSafe f' (by rw [hframes]; exact List.mem_cons_of_mem f hf')
        p hp hname

theorem wfState_withHeadInstrs {contract : IRContract} {c : IRState} {instrs : List Instr}
    (hwf : WFState contract c)
    (hblockNew : ∀ f rest, c.frames = f :: rest → ∃ block ∈ f.fn.blocks,
      f.block = block ∧ f.terminator = block.terminator ∧
      ∃ done, block.instrs = done ++ instrs)
    (hnoBareNew : ∀ f rest, c.frames = f :: rest → instrs = [] →
      f.terminator ≠ some .unreachable) :
    WFState contract (c.withHeadInstrs instrs) := by
  cases hframes : c.frames with
  | nil =>
      have hsame : c.withHeadInstrs instrs = c := by
        simp [IRState.withHeadInstrs, hframes]
      rw [hsame]
      exact hwf
  | cons f rest =>
      have hsame : c.withHeadInstrs instrs = c.withHead { f with instrs := instrs } := by
        simp [IRState.withHeadInstrs, IRState.withHead, hframes]
      rw [hsame]
      have hmem : f ∈ c.frames := by rw [hframes]; exact List.mem_cons_self ..
      exact wfState_withHead hwf hframes rfl rfl rfl rfl rfl rfl
        (fun slot ty hty => hty)
        (hblockNew f rest hframes)
        (hwf.framePrev f hmem)
        (hnoBareNew f rest hframes)
        (hwf.localsInitialized f hmem)
        (hwf.localsTyped f hmem)
        (hwf.originsMatch f hmem)
        (hwf.allocsWellFormed f hmem)
        (hwf.stackPtrsBounded f hmem)
        (hwf.memTyped f hmem)
        (hwf.storagePtrsLive f hmem)
        (hwf.slotsBase f hmem)
        (hwf.noSlotsGlobalPointers.1 f hmem)
        (hwf.noSlotsGlobalPointers.2.1 f hmem)
        (hwf.stackReturnsSafe f hmem)
        (hwf.slotsParamsSafe f hmem)

theorem lookupStorage_setStorage_eq (s : IRState) (id : StoreId) (values : List IRValue) :
    ({ s with storage := setList s.storage id values }).lookupStorage id = some values := by
  unfold IRState.lookupStorage
  rw [lookupList_setList_eq]

theorem lookupStorage_setStorage_ne {s : IRState} {id id' : StoreId} {values : List IRValue}
    (h : (id' == id) = false) :
    ({ s with storage := setList s.storage id values }).lookupStorage id' =
      s.lookupStorage id' := by
  unfold IRState.lookupStorage
  rw [lookupList_setList_ne _ values h]

theorem length_take_cons_drop {values : List IRValue} {off : Nat} (h : off < values.length)
    {v : IRValue} :
    (values.take off ++ v :: values.drop (off + 1)).length = values.length := by
  simp only [List.length_append, List.length_cons, List.length_take, List.length_drop]
  have hmin : min off values.length = off := Nat.min_eq_left (Nat.le_of_lt h)
  rw [hmin]
  omega

theorem storageDomain_writeStorage {s s' : IRState} {id : StoreId} {off : Nat}
    {value : IRValue} (h : StorageDomain s)
    (hw : s.writeStorage id off value = some s') : StorageDomain s' := by
  unfold IRState.writeStorage at hw
  cases hlook : s.lookupStorage id with
  | none => simp [hlook] at hw
  | some values =>
      simp only [hlook] at hw
      by_cases hoff : off < values.length
      · rw [ite_eq_left hoff] at hw
        cases hw
        constructor
        · intro slot kind hkind hstorage
          by_cases hid : (StoreId.root slot) = id
          · subst hid
            rcases h.1 slot kind hkind hstorage with ⟨vs, hvs, hpos⟩
            have hvs' : vs = values := by
              rw [hvs] at hlook
              exact Option.some.inj hlook
            rw [hvs'] at hpos
            refine ⟨values.take off ++ value :: values.drop (off + 1), ?_, ?_⟩
            · exact lookupStorage_setStorage_eq s (.root slot) _
            · rw [length_take_cons_drop hoff]
              exact hpos
          · rcases h.1 slot kind hkind hstorage with ⟨vs, hvs, hpos⟩
            refine ⟨vs, ?_, hpos⟩
            rw [lookupStorage_setStorage_ne (beq_eq_false_iff_ne.mpr hid)]
            exact hvs
        · intro g hg
          by_cases hid : (StoreId.global g) = id
          · subst hid
            rcases h.2 g hg with ⟨vs, hvs, hpos⟩
            have hvs' : vs = values := by
              rw [hvs] at hlook
              exact Option.some.inj hlook
            rw [hvs'] at hpos
            refine ⟨values.take off ++ value :: values.drop (off + 1), ?_, ?_⟩
            · exact lookupStorage_setStorage_eq s (.global g) _
            · rw [length_take_cons_drop hoff]
              exact hpos
          · rcases h.2 g hg with ⟨vs, hvs, hpos⟩
            refine ⟨vs, ?_, hpos⟩
            rw [lookupStorage_setStorage_ne (beq_eq_false_iff_ne.mpr hid)]
            exact hvs
      · rw [ite_eq_right hoff] at hw
        simp at hw

/-- State projections unaffected by a frame-list update. -/
theorem lookupStorage_updateFrames (c : IRState) (fs : List IRFrame) (id : StoreId) :
    ({ c with frames := fs } : IRState).lookupStorage id = c.lookupStorage id := rfl

theorem lookupStorageAt_updateFrames (c : IRState) (fs : List IRFrame) (id : StoreId)
    (off : Nat) :
    ({ c with frames := fs } : IRState).lookupStorageAt id off = c.lookupStorageAt id off := rfl

theorem lookupFunction_updateFrames (c : IRState) (fs : List IRFrame) (slot : Nat) :
    ({ c with frames := fs } : IRState).lookupFunction slot = c.lookupFunction slot := rfl

theorem slotKind?_updateFrames (c : IRState) (fs : List IRFrame) (slot : Nat) :
    ({ c with frames := fs } : IRState).slotKind? slot = c.slotKind? slot := rfl

theorem module_updateFrames (c : IRState) (fs : List IRFrame) :
    ({ c with frames := fs } : IRState).module = c.module := rfl

/-- The per-frame update of a successful stack store. -/
def storeFrame (frame slot off : Nat) (v : IRValue) (g : IRFrame) : IRFrame :=
  if g.id == frame then g.setMem (.stack frame slot off) v else g

theorem wfState_storeFrames {contract : IRContract} {c : IRState}
    {frame slot off : Nat} {v : IRValue} {ty : IrType} {g0 : IRFrame}
    (hwf : WFState contract c)
    (hfind : c.frameById? frame = some g0)
    (halloc : g0.allocType? slot = some ty)
    (hoff : off < allocSize ty)
    (hvt : MemValueTyped ty v)
    (hvb : MemValuePtrBounded c v)
    (hnoSlotsV : ∀ gname off', v = .ptr (.global gname off') →
      gname ∉ SlotsGlobals c.module) :
    WFState contract
      { c with frames := c.frames.map (storeFrame frame slot off v) } := by
  have hFid : ∀ g, (storeFrame frame slot off v g).id = g.id := by
    intro g; unfold storeFrame; split <;> rfl
  have hFfn : ∀ g, (storeFrame frame slot off v g).fn = g.fn := by
    intro g; unfold storeFrame; split <;> rfl
  have hFblock : ∀ g, (storeFrame frame slot off v g).block = g.block := by
    intro g; unfold storeFrame; split <;> rfl
  have hFterm : ∀ g, (storeFrame frame slot off v g).terminator = g.terminator := by
    intro g; unfold storeFrame; split <;> rfl
  have hFinstrs : ∀ g, (storeFrame frame slot off v g).instrs = g.instrs := by
    intro g; unfold storeFrame; split <;> rfl
  have hFprev : ∀ g, (storeFrame frame slot off v g).prev = g.prev := by
    intro g; unfold storeFrame; split <;> rfl
  have hFret : ∀ g, (storeFrame frame slot off v g).retDest = g.retDest := by
    intro g; unfold storeFrame; split <;> rfl
  have hFlocals : ∀ g, (storeFrame frame slot off v g).locals = g.locals := by
    intro g; unfold storeFrame; split <;> rfl
  have hFallocs : ∀ g, (storeFrame frame slot off v g).allocs = g.allocs := by
    intro g; unfold storeFrame; split <;> rfl
  have hFnextAlloc : ∀ g, (storeFrame frame slot off v g).nextAlloc = g.nextAlloc := by
    intro g; unfold storeFrame; split <;> rfl
  have hFallocType : ∀ g slot' ty',
      g.allocType? slot' = some ty' → (storeFrame frame slot off v g).allocType? slot' = some ty' := by
    intro g slot' ty' hty
    rw [show (storeFrame frame slot off v g).allocType? slot' = g.allocType? slot' from by
      unfold storeFrame; split <;> rfl]
    exact hty
  have hlook : FrameLookupsPreserved c
      { c with frames := c.frames.map (storeFrame frame slot off v) } :=
    frameLookups_storeFrames c frame slot off v
  constructor
  · exact hwf.program
  · have hmapid : (c.frames.map (storeFrame frame slot off v)).map (·.id) = c.frames.map (·.id) := by
      rw [List.map_map]
      exact List.map_congr_left (fun g _ => hFid g)
    rw [hmapid]
    exact hwf.frameIdsNodup
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFid]
    exact hwf.frameIdsBound g hg
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFfn]
    exact hwf.frameFunction g hg
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rcases hwf.frameBlock g hg with ⟨block, hb, hblock, hterm, done, hdone⟩
    refine ⟨block, ?_, ?_, ?_, done, ?_⟩
    · simpa [hFfn g] using hb
    · rw [hFblock g]; exact hblock
    · rw [hFterm g]; exact hterm
    · rw [hFinstrs g]; exact hdone
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have h := hwf.framePrev g hg
    simpa [PrevEdge, hFprev g, hFfn g, hFblock g] using h
  · cases hframes : c.frames with
    | nil =>
        intro f' rest' heq
        simp [hframes] at heq
    | cons f0 rest0 =>
        intro f' rest' heq hinst
        simp only [hframes, List.map_cons, List.cons.injEq] at heq
        rcases heq with ⟨hf', hrest'⟩
        rw [← hf'] at hinst ⊢
        rw [hFinstrs f0] at hinst
        rw [hFterm f0]
        exact hwf.noBareUnreachable f0 rest0 hframes hinst
  · intro f' hf' name ty' hty
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFfn g] at hty
    rcases hwf.localsInitialized g hg name ty' hty with ⟨val, hval⟩
    exact ⟨val, by simpa [IRFrame.lookupLocal, hFlocals g] using hval⟩
  · intro f' hf' name val hv ty' hty
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFfn g] at hty
    have hv' : g.lookupLocal name = some val := by
      simpa [IRFrame.lookupLocal, hFlocals g] using hv
    exact hwf.localsTyped g hg name val hv' ty' hty
  · intro f' hf' entry hentry
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have h := hwf.originsMatch g hg entry (by simpa [hFfn g] using hentry)
    simpa [OriginHolds, IRFrame.lookupLocal, hFlocals g, hFfn g, lookupFunction_updateFrames,
      slotKind?_updateFrames, module_updateFrames] using h
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have h := hwf.allocsWellFormed g hg
    refine ⟨?_, ?_, ?_⟩
    · simpa [hFallocs g] using h.1
    · simpa [hFallocs g, hFnextAlloc g] using h.2.1
    · intro slot' ty' hty
      rw [show (storeFrame frame slot off v g).allocType? slot' = g.allocType? slot' from by
        unfold storeFrame; split <;> rfl] at hty
      exact h.2.2 slot' ty' hty
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have hspg : StackPtrsBounded c (storeFrame frame slot off v g) := by
      intro name val addr hv haddr
      exact hwf.stackPtrsBounded g hg name val addr
        (by simpa [IRFrame.lookupLocal, hFlocals g] using hv) haddr
    exact stackPtrsBounded_of_frameLookups hlook hspg
  · intro f' hf'
    exact memTyped_storeFrames hwf.frameIdsNodup hwf.memTyped hfind halloc hoff hvt hvb f' hf'
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have h := hwf.storagePtrsLive g hg
    simpa [StoragePtrsLive, IRFrame.lookupLocal, hFlocals g, lookupStorage_updateFrames] using h
  · intro f' hf'
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    have h := hwf.slotsBase g hg
    simpa [SlotsBase, SlotsGlobalStored, IRFrame.lookupLocal, hFlocals g, hFfn g,
      lookupStorageAt_updateFrames] using h
  · exact hwf.slotsStorage
  · exact hwf.storageValuesLive
  · exact hwf.storageDomain
  · constructor
    · intro f' hf' name val gname off' hv hval
      rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
      exact hwf.noSlotsGlobalPointers.1 g hg name val gname off'
        (by simpa [IRFrame.lookupLocal, hFlocals g] using hv) hval
    · constructor
      · intro f' hf' addr val gname off' hv hval
        rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
        unfold storeFrame at hv
        split at hv
        · rename_i hgt
          by_cases hsame : addr = .stack frame slot off
          · rw [hsame] at hv
            rw [lookupMem_setMem_eq] at hv
            have hvalEq : val = v := (Option.some.inj hv).symm
            subst hvalEq
            exact hnoSlotsV gname off' hval
          · exact hwf.noSlotsGlobalPointers.2.1 g hg addr val gname off'
              (by
                rw [← lookupMem_setMem_ne g (beq_eq_false_iff_ne.mpr hsame) v]
                exact hv) hval
        · rename_i hgt
          exact hwf.noSlotsGlobalPointers.2.1 g hg addr val gname off' hv hval
      · simpa [lookupStorageAt_updateFrames] using hwf.noSlotsGlobalPointers.2.2
  · exact hwf.functionBindings
  · exact hwf.callTargets
  · intro pre callee caller suffix heq
    rw [List.map_eq_append_iff] at heq
    rcases heq with ⟨l1, l2, hl, hpre, hrest⟩
    rw [List.map_eq_cons_iff] at hrest
    rcases hrest with ⟨a, l2', hl2, hca, hrest2⟩
    rw [List.map_eq_cons_iff] at hrest2
    rcases hrest2 with ⟨b, l2'', hl2', hcb, hsuf⟩
    have hf : c.frames = l1 ++ a :: b :: l2'' := by
      rw [hl, hl2, hl2']
    have h := hwf.returnLink l1 a b l2'' hf
    rw [← hca, ← hcb, hFret a, hFfn a, hFfn b]
    exact h
  · intro last hlast
    rw [List.getLast?_map] at hlast
    cases hlast' : c.frames.getLast? with
    | none => rw [hlast'] at hlast; simp at hlast
    | some g =>
        rw [hlast'] at hlast
        have hgl : storeFrame frame slot off v g = last := by
          have hmap : some (storeFrame frame slot off v g) = some last := by
            simpa using hlast
          exact Option.some.inj hmap
        rw [← hgl, hFret g]
        exact hwf.bottomRetDest g hlast'
  · intro f' hf' operand val htermv heval frame slot' off' hv
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFterm g] at htermv
    have heval' : evalOperand g operand = some val := by
      simpa [evalOperand, IRFrame.lookupLocal, hFlocals g] using heval
    have h := hwf.stackReturnsSafe g hg operand val htermv heval' frame slot' off' hv
    rw [hFid g]
    exact h

  · intro f' hf' p hp hname
    rcases List.mem_map.mp hf' with ⟨g, hg, rfl⟩
    rw [hFfn g] at hp
    have h := hwf.slotsParamsSafe g hg p hp hname
    simpa [IRFrame.lookupLocal, hFlocals g] using h

theorem lookupStorageAt_setStorage_eq (s : IRState) (sid : StoreId)
    (values : List IRValue) (off : Nat) :
    ({ s with storage := setList s.storage sid values }).lookupStorageAt sid off =
      if off < values.length then values[off]? else none := by
  unfold IRState.lookupStorageAt
  rw [lookupStorage_setStorage_eq]

theorem lookupStorageAt_setStorage_ne {s : IRState} {sid sid' : StoreId}
    {values : List IRValue} (h : (sid' == sid) = false) (off : Nat) :
    ({ s with storage := setList s.storage sid values }).lookupStorageAt sid' off =
      s.lookupStorageAt sid' off := by
  unfold IRState.lookupStorageAt
  rw [lookupStorage_setStorage_ne h]

theorem writeStorage_eq {s s' : IRState} {sid : StoreId} {off : Nat} {v : IRValue}
    {values : List IRValue} (hlook : s.lookupStorage sid = some values)
    (hoff : off < values.length) (hw : s.writeStorage sid off v = some s') :
    s' = { s with
      storage := setList s.storage sid
        (values.take off ++ v :: values.drop (off + 1)) } := by
  unfold IRState.writeStorage at hw
  simp only [hlook] at hw
  rw [ite_eq_left hoff] at hw
  cases hw
  rfl

theorem mem_take_cons_drop {values : List IRValue} {off : Nat} {v v0 : IRValue} :
    v0 ∈ values.take off ++ v :: values.drop (off + 1) → v0 = v ∨ v0 ∈ values := by
  intro h
  rcases List.mem_append.mp h with h | h
  · exact Or.inr (List.mem_of_mem_take h)
  · rcases List.mem_cons.mp h with h | h
    · exact Or.inl h
    · exact Or.inr (List.mem_of_mem_drop h)

theorem lookupStorageAt_setStorage_some_mem {s : IRState} {sid : StoreId}
    {values : List IRValue} {sid' : StoreId} {off : Nat} {v0 : IRValue}
    (h : ({ s with storage := setList s.storage sid values }).lookupStorageAt sid' off =
      some v0) :
    v0 ∈ values ∨ s.lookupStorageAt sid' off = some v0 := by
  by_cases hid : (sid' == sid) = true
  · have hEq : sid' = sid := beq_iff_eq.mp hid
    rw [hEq] at h
    rw [lookupStorageAt_setStorage_eq] at h
    left
    by_cases hoff : off < values.length
    · rw [ite_eq_left hoff] at h
      exact List.mem_of_getElem? h
    · rw [ite_eq_right hoff] at h
      cases h
  · have hne : (sid' == sid) = false := by
      rw [Bool.not_eq_true] at hid
      exact hid
    right
    rwa [lookupStorageAt_setStorage_ne hne] at h

theorem getElem?_take_cons_drop_self {values : List IRValue} {off : Nat}
    (h : off < values.length) {v : IRValue} :
    (values.take off ++ v :: values.drop (off + 1))[off]? = some v := by
  rw [List.getElem?_append]
  have hnot : ¬ off < (values.take off).length := by
    rw [List.length_take]
    omega
  rw [ite_eq_right hnot]
  have hsub : off - (values.take off).length = 0 := by
    rw [List.length_take]
    omega
  rw [hsub, List.getElem?_cons_zero]

theorem lookupStorage_setStorage_live {s : IRState} {sid : StoreId}
    {newValues : List IRValue} {sid' : StoreId} {off' : Nat}
    (h : ∃ vs, s.lookupStorage sid' = some vs ∧ off' < vs.length)
    (hvalues : ∀ vs, s.lookupStorage sid = some vs → newValues.length = vs.length) :
    ∃ vs, ({ s with storage := setList s.storage sid newValues }).lookupStorage sid' =
      some vs ∧ off' < vs.length := by
  rcases h with ⟨vs, hvs, hlt⟩
  by_cases hid : (sid' == sid) = true
  · have hEq : sid' = sid := beq_iff_eq.mp hid
    rw [hEq] at hvs
    have hlen : newValues.length = vs.length := hvalues vs hvs
    refine ⟨newValues, ?_, ?_⟩
    · rw [hEq]
      exact lookupStorage_setStorage_eq s sid newValues
    · rw [hlen]
      exact hlt
  · have hne : (sid' == sid) = false := by
      rw [Bool.not_eq_true] at hid
      exact hid
    exact ⟨vs, by rwa [lookupStorage_setStorage_ne hne], hlt⟩

theorem wfState_storeStorage {contract : IRContract} {c s' : IRState}
    {sid : StoreId} {off : Nat} {v : IRValue}
    (hwf : WFState contract c)
    (hw : c.writeStorage sid off v = some s')
    (hvb : ∀ slot off', v = .ptr (.root slot off') →
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off' < values.length)
    (hvg : ∀ g off', v = .ptr (.global g off') →
      ∃ values, c.lookupStorage (.global g) = some values ∧ off' < values.length)
    (hnoSlotsV : ∀ g off', v = .ptr (.global g off') → g ∉ SlotsGlobals c.module)
    (hnotSlots : ∀ g, sid = .global g → g ∉ SlotsGlobals c.module) :
    WFState contract s' := by
  have hw0 := hw
  unfold IRState.writeStorage at hw
  cases hlook : c.lookupStorage sid with
  | none => simp [hlook] at hw
  | some values =>
      simp only [hlook] at hw
      by_cases hoff : off < values.length
      · rw [ite_eq_left hoff] at hw
        cases hw
        have hvaluesLen : ∀ vs, c.lookupStorage sid = some vs →
            (values.take off ++ v :: values.drop (off + 1)).length = vs.length := by
          intro vs hvs
          have hlook' := hlook
          rw [hvs] at hlook'
          have hvsEq : vs = values := Option.some.inj hlook'
          rw [hvsEq, length_take_cons_drop hoff]
        have hmemLive : ∀ w, w ∈ values →
            (∀ slot offp, w = .ptr (.root slot offp) →
              ∃ vs, c.lookupStorage (.root slot) = some vs ∧ offp < vs.length) ∧
            (∀ g offp, w = .ptr (.global g offp) →
              ∃ vs, c.lookupStorage (.global g) = some vs ∧ offp < vs.length) := by
          intro w hwmem
          obtain ⟨n, hn⟩ := List.getElem?_of_mem hwmem
          have hnLt : n < values.length := (List.getElem?_eq_some_iff.mp hn).choose
          have hlookAt : c.lookupStorageAt sid n = some w := by
            unfold IRState.lookupStorageAt
            simp only [hlook]
            rw [ite_eq_left hnLt]
            exact hn
          exact hwf.storageValuesLive sid n w hlookAt
        have hmemNoSlots : ∀ w, w ∈ values →
            ∀ gname offp, w = .ptr (.global gname offp) →
              gname ∉ SlotsGlobals c.module := by
          intro w hwmem gname offp hptr
          obtain ⟨n, hn⟩ := List.getElem?_of_mem hwmem
          have hnLt : n < values.length := (List.getElem?_eq_some_iff.mp hn).choose
          have hlookAt : c.lookupStorageAt sid n = some w := by
            unfold IRState.lookupStorageAt
            simp only [hlook]
            rw [ite_eq_left hnLt]
            exact hn
          exact hwf.noSlotsGlobalPointers.2.2 sid n w gname offp hlookAt hptr
        constructor
        · exact hwf.program
        · exact hwf.frameIdsNodup
        · exact hwf.frameIdsBound
        · exact hwf.frameFunction
        · exact hwf.frameBlock
        · exact hwf.framePrev
        · exact hwf.noBareUnreachable
        · exact hwf.localsInitialized
        · exact hwf.localsTyped
        · exact hwf.originsMatch
        · exact hwf.allocsWellFormed
        · exact hwf.stackPtrsBounded
        · exact hwf.memTyped
        · intro f hf
          have h := hwf.storagePtrsLive f hf
          constructor
          · intro name val slot offp hv hptr
            exact lookupStorage_setStorage_live
              (h.1 name val slot offp hv hptr) hvaluesLen
          · intro name val gname offp hv hptr
            exact lookupStorage_setStorage_live
              (h.2 name val gname offp hv hptr) hvaluesLen
        · intro f hf
          have h := hwf.slotsBase f hf
          refine ⟨h.1, h.2.1, ?_⟩
          intro base hbase g ty ptr align hdef hsym
          have hg : g ∈ SlotGlobalNames f.fn := by
            unfold SlotGlobalNames
            rw [List.mem_filterMap]
            exact ⟨base, hbase, by rw [hdef, hsym]⟩
          have hgSlots : g ∈ SlotsGlobals c.module := by
            unfold SlotsGlobals
            rw [List.mem_flatMap]
            exact ⟨f.fn, hwf.frameFunction f hf, hg⟩
          have hsid : sid ≠ .global g := by
            intro heq
            exact (hnotSlots g heq) hgSlots
          have hne : ((StoreId.global g) == sid) = false :=
            beq_eq_false_iff_ne.mpr (Ne.symm hsid)
          rw [lookupStorageAt_setStorage_ne hne]
          exact h.2.2 base hbase g ty ptr align hdef hsym
        · intro fn hfn base hbase g ty ptr align hdef hsym
          have hgSlots : g ∈ SlotsGlobals c.module :=
            mem_slotsGlobals_of_slotBase hfn hbase hdef hsym
          have hsid : sid ≠ .global g := by
            intro heq
            exact (hnotSlots g heq) hgSlots
          have hne : ((StoreId.global g) == sid) = false :=
            beq_eq_false_iff_ne.mpr (Ne.symm hsid)
          rw [lookupStorageAt_setStorage_ne hne]
          exact hwf.slotsStorage fn hfn base hbase g ty ptr align hdef hsym
        · intro sid' off' v0 hlookAt
          constructor
          · intro slot offp hptr
            rcases lookupStorageAt_setStorage_some_mem hlookAt with hmem | hold
            · rcases mem_take_cons_drop hmem with rfl | hmem
              · exact lookupStorage_setStorage_live (hvb slot offp hptr) hvaluesLen
              · exact lookupStorage_setStorage_live
                  ((hmemLive v0 hmem).1 slot offp hptr) hvaluesLen
            · exact lookupStorage_setStorage_live
                ((hwf.storageValuesLive sid' off' v0 hold).1 slot offp hptr) hvaluesLen
          · intro gname offp hptr
            rcases lookupStorageAt_setStorage_some_mem hlookAt with hmem | hold
            · rcases mem_take_cons_drop hmem with rfl | hmem
              · exact lookupStorage_setStorage_live (hvg gname offp hptr) hvaluesLen
              · exact lookupStorage_setStorage_live
                  ((hmemLive v0 hmem).2 gname offp hptr) hvaluesLen
            · exact lookupStorage_setStorage_live
                ((hwf.storageValuesLive sid' off' v0 hold).2 gname offp hptr) hvaluesLen
        · exact storageDomain_writeStorage hwf.storageDomain hw0
        · constructor
          · exact hwf.noSlotsGlobalPointers.1
          · constructor
            · exact hwf.noSlotsGlobalPointers.2.1
            · intro sid' off' v0 gname offp hlookAt hptr
              rcases lookupStorageAt_setStorage_some_mem hlookAt with hmem | hold
              · rcases mem_take_cons_drop hmem with rfl | hmem
                · exact hnoSlotsV gname offp hptr
                · exact hmemNoSlots v0 hmem gname offp hptr
              · exact hwf.noSlotsGlobalPointers.2.2 sid' off' v0 gname offp hold hptr
        · exact hwf.functionBindings
        · exact hwf.callTargets
        · exact hwf.returnLink
        · exact hwf.bottomRetDest
        · exact hwf.stackReturnsSafe
        · exact hwf.slotsParamsSafe
      · rw [ite_eq_right hoff] at hw
        simp at hw

theorem mem_fragValueSymbols_of_instr {frag : IRFrag} {fn : Function} {block : Block}
    {instr : Instr} (hfn : fn ∈ frag.functions) (hblock : block ∈ fn.blocks)
    (hinstr : instr ∈ block.instrs) {g : String} (hg : g ∈ instrValueSymbols instr) :
    g ∈ fragValueSymbols frag := by
  unfold fragValueSymbols
  rw [List.mem_flatMap]
  refine ⟨fn, hfn, ?_⟩
  rw [List.mem_flatMap]
  refine ⟨block, hblock, ?_⟩
  rw [List.mem_append]
  left
  rw [List.mem_flatMap]
  exact ⟨instr, hinstr, hg⟩

theorem mem_instrValueSymbols_store_value {ty : IrType} {value ptr : Operand}
    {align : Option Nat} {g : String} (h : g ∈ operandSymbols value) :
    g ∈ instrValueSymbols (.store ty value ptr align) := by
  unfold instrValueSymbols
  rw [List.mem_append]
  exact Or.inl h

theorem mem_instrValueSymbols_store_ptr {ty : IrType} {value ptr : Operand}
    {align : Option Nat} {g : String} (h : g ∈ operandSymbols ptr) :
    g ∈ instrValueSymbols (.store ty value ptr align) := by
  unfold instrValueSymbols
  rw [List.mem_append]
  exact Or.inr h

theorem noSlotsGlobal_of_evalOperand {c : IRState} {f : IRFrame}
    (hlocals : ∀ name v g off, f.lookupLocal name = some v →
      v = .ptr (.global g off) → g ∉ SlotsGlobals c.module)
    {operand : Operand} {g : String} {off : Nat}
    (hsyms : ∀ h, h ∈ operandSymbols operand → h ∉ SlotsGlobals c.module)
    (h : evalOperand f operand = some (.ptr (.global g off))) :
    g ∉ SlotsGlobals c.module := by
  cases operand with
  | «local» name => exact hlocals name _ g off h rfl
  | intLit ty val => simp [evalOperand] at h
  | nullPtr => simp [evalOperand] at h
  | symbol hname =>
      simp only [evalOperand, Option.some.injEq] at h
      have haddr : IRAddr.global hname 0 = IRAddr.global g off := IRValue.ptr.inj h
      have hnameEq : hname = g := (IRAddr.global.inj haddr).1
      have h' := hsyms hname (by simp [operandSymbols])
      rwa [hnameEq] at h'

theorem storageLive_of_evalOperand {c : IRState} {f : IRFrame}
    (hlocals : StoragePtrsLive c f) (hdoms : StorageDomain c)
    {operand : Operand} {v : IRValue}
    (hsyms : ∀ g, g ∈ operandSymbols operand → g ∈ fragValueSymbols c.module)
    (h : evalOperand f operand = some v) :
    (∀ slot off, v = .ptr (.root slot off) →
      ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length) ∧
    (∀ g off, v = .ptr (.global g off) →
      ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length) := by
  cases operand with
  | «local» name =>
      simp only [evalOperand] at h
      exact ⟨fun slot off hp => hlocals.1 name v slot off h hp,
        fun g off hp => hlocals.2 name v g off h hp⟩
  | intLit ty val =>
      simp only [evalOperand, Option.some.injEq] at h
      subst h
      exact ⟨fun slot off hp => by simp at hp, fun g off hp => by simp at hp⟩
  | nullPtr =>
      simp only [evalOperand, Option.some.injEq] at h
      subst h
      exact ⟨fun slot off hp => by simp at hp, fun g off hp => by simp at hp⟩
  | symbol hname =>
      simp only [evalOperand, Option.some.injEq] at h
      subst h
      refine ⟨fun slot off hp => by simp at hp, fun g off hp => ?_⟩
      have haddr : IRAddr.global hname 0 = IRAddr.global g off := IRValue.ptr.inj hp
      have hnameEq : hname = g := (IRAddr.global.inj haddr).1
      have hoff : off = 0 := (IRAddr.global.inj haddr).2.symm
      have hdom := hdoms.2 hname (hsyms hname (by simp [operandSymbols]))
      rw [hnameEq] at hdom
      rcases hdom with ⟨values, hlook, hpos⟩
      exact ⟨values, hlook, by rw [hoff]; exact hpos⟩

theorem memValueTyped_of_stackStoreTypeOK {c : IRState} {frame slot off : Nat} {ty : IrType}
    {v : IRValue} {g0 : IRFrame} {allocTy : IrType}
    (hfind : c.frameById? frame = some g0) (halloc : g0.allocType? slot = some allocTy)
    (hok : stackStoreTypeOK c (.stack frame slot off) ty = true)
    (htyped : v.type? = some ty) : MemValueTyped allocTy v := by
  simp only [stackStoreTypeOK, hfind] at hok
  simp only [halloc] at hok
  cases allocTy with
  | array elem count =>
      simp only [] at hok
      have hty : ty = elem := beq_iff_eq.mp hok
      simp [MemValueTyped, htyped, hty]
  | int bits =>
      simp only [] at hok
      have hty : ty = .int bits := beq_iff_eq.mp hok
      simp [MemValueTyped, htyped, hty]
  | ptr =>
      simp only [] at hok
      have hty : ty = .ptr := beq_iff_eq.mp hok
      simp [MemValueTyped, htyped, hty]
  | overflow bits =>
      simp only [] at hok
      have hty : ty = .overflow bits := beq_iff_eq.mp hok
      simp [MemValueTyped, htyped, hty]
  | void =>
      simp only [] at hok
      have hty : ty = .void := beq_iff_eq.mp hok
      simp [MemValueTyped, htyped, hty]

theorem getLast?_append_singleton (l : List Instr) (a : Instr) :
    (l ++ [a]).getLast? = some a := by
  induction l with
  | nil => rfl
  | cons x xs ih =>
      simp only [List.cons_append]
      cases xs with
      | nil => rfl
      | cons y ys => simpa using ih



theorem storeFrame_fn (frame slot off : Nat) (v : IRValue) (g : IRFrame) :
    (storeFrame frame slot off v g).fn = g.fn := by
  unfold storeFrame; split <;> rfl

theorem storeFrame_block (frame slot off : Nat) (v : IRValue) (g : IRFrame) :
    (storeFrame frame slot off v g).block = g.block := by
  unfold storeFrame; split <;> rfl

theorem storeFrame_terminator (frame slot off : Nat) (v : IRValue) (g : IRFrame) :
    (storeFrame frame slot off v g).terminator = g.terminator := by
  unfold storeFrame; split <;> rfl

theorem storeValue_stack_cases {s s' : IRState} {frame slot off : Nat} {v : IRValue}
    (hw : s.storeValue (.stack frame slot off) v = some s') :
    ∃ g0 allocTy, s.frameById? frame = some g0 ∧ g0.allocType? slot = some allocTy ∧
      off < allocSize allocTy ∧
      s' = { s with frames := s.frames.map fun g =>
        if g.id == frame then g.setMem (.stack frame slot off) v else g } := by
  unfold IRState.storeValue at hw
  cases hlook : s.frameById? frame with
  | none => simp [hlook] at hw
  | some g0 =>
      simp only [hlook] at hw
      cases halloc : g0.allocType? slot with
      | none => simp [halloc] at hw
      | some allocTy =>
          simp only [halloc] at hw
          by_cases hoff : off < allocSize allocTy
          · rw [ite_eq_left hoff] at hw
            cases hw
            exact ⟨g0, allocTy, rfl, halloc, hoff, rfl⟩
          · rw [ite_eq_right hoff] at hw
            simp at hw



theorem writeStorage_frames {s s' : IRState} {sid : StoreId} {off : Nat} {v : IRValue}
    (hw : s.writeStorage sid off v = some s') : s'.frames = s.frames := by
  unfold IRState.writeStorage at hw
  cases hlook : s.lookupStorage sid with
  | none => simp [hlook] at hw
  | some values =>
      simp only [hlook] at hw
      by_cases hoff : off < values.length
      · rw [ite_eq_left hoff] at hw
        cases hw
        rfl
      · rw [ite_eq_right hoff] at hw
        simp at hw

theorem wfState_storeStep {contract : IRContract} {c s' : IRState} {f : IRFrame}
    {restFrames : List IRFrame} {restInstrs : List Instr}
    {ty : IrType} {value ptr : Operand} {align : Option Nat} {v : IRValue} {addr : IRAddr}
    (hwf : WFState contract c)
    (hframes : c.frames = f :: restFrames)
    (hinstrs : f.instrs = .store ty value ptr align :: restInstrs)
    (hvalue : evalOperand f value = some v)
    (hptr : evalOperand f ptr = some (.ptr addr))
    (hcond : ((v.type? == some ty) && stackStoreTypeOK c addr ty) = true)
    (hstore : c.storeValue addr v = some s') :
    WFState contract (s'.withHeadInstrs restInstrs) := by
  have hf : f ∈ c.frames := by rw [hframes]; exact List.mem_cons_self ..
  have hfunMem : f.fn ∈ c.module.functions := hwf.frameFunction f hf
  rcases hwf.frameBlock f hf with ⟨block, hblock, hblockEq, htermEq, done, hdone⟩
  have hinstrMem : Instr.store ty value ptr align ∈ block.instrs := by
    rw [hdone, hinstrs]
    exact List.mem_append_right _ (List.mem_cons_self ..)
  have hslotsSyms := hwf.program.slotsSymbolsLoadOnly.1 f.fn hfunMem block hblock
    (.store ty value ptr align) hinstrMem
  have hsymsValue : ∀ g, g ∈ operandSymbols value → g ∉ SlotsGlobals c.module :=
    fun g hg => hslotsSyms g (mem_instrValueSymbols_store_value hg)
  have hsymsPtr : ∀ g, g ∈ operandSymbols ptr → g ∉ SlotsGlobals c.module :=
    fun g hg => hslotsSyms g (mem_instrValueSymbols_store_ptr hg)
  have hsymsValueFrag : ∀ g, g ∈ operandSymbols value → g ∈ fragValueSymbols c.module :=
    fun g hg => mem_fragValueSymbols_of_instr hfunMem hblock hinstrMem
      (mem_instrValueSymbols_store_value hg)
  have htyped : v.type? = some ty :=
    beq_iff_eq.mp (Bool.and_eq_true_iff.mp hcond).1
  have hnoSlotsLocals := hwf.noSlotsGlobalPointers.1 f hf
  have hblockShape : block.instrs = (done ++ [.store ty value ptr align]) ++ restInstrs := by
    rw [hdone, hinstrs, List.append_assoc]
    rfl
  have hnoBareInst : restInstrs = [] → block.terminator ≠ some .unreachable := by
    intro hres htermblock
    have hlast : block.instrs.getLast? = some (.store ty value ptr align) := by
      rw [hblockShape, hres]
      simpa using getLast?_append_singleton done (.store ty value ptr align)
    rcases hwf.program.unreachableGuarded f.fn hfunMem block hblock htermblock with
      ⟨instr0, hlast0, dest, ret, callee, args, hinstr0, _⟩
    rw [hlast] at hlast0
    have hinstrEq : Instr.store ty value ptr align = instr0 := Option.some.inj hlast0
    rw [← hinstrEq] at hinstr0
    cases hinstr0
  cases addr with
  | stack frame slot off =>
      rcases storeValue_stack_cases hstore with
        ⟨g0, allocTy, hfind, halloc, hoff, hs'⟩
      have hvt : MemValueTyped allocTy v :=
        memValueTyped_of_stackStoreTypeOK hfind halloc
          (Bool.and_eq_true_iff.mp hcond).2 htyped
      have hvb : MemValuePtrBounded c v :=
        memValuePtrBounded_of_evalOperand (hwf.stackPtrsBounded f hf) hvalue
      have hnoSlotsV : ∀ gname off', v = .ptr (.global gname off') →
          gname ∉ SlotsGlobals c.module := by
        intro gname off' hval
        exact noSlotsGlobal_of_evalOperand hnoSlotsLocals hsymsValue
          (by simpa [hval] using hvalue)
      have hstate : WFState contract
          { c with frames := c.frames.map fun g =>
              if g.id == frame then g.setMem (.stack frame slot off) v else g } :=
        wfState_storeFrames hwf hfind halloc hoff hvt hvb hnoSlotsV
      rw [hs']
      apply wfState_withHeadInstrs hstate
      · intro f' fs heq
        rw [hframes] at heq
        simp only [List.map_cons, List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        rw [← hf']
        refine ⟨block, ?_, ?_, ?_, done ++ [.store ty value ptr align], hblockShape⟩
        · split <;> exact hblock
        · split <;> exact hblockEq
        · split <;> exact htermEq
      · intro f' fs heq hres hterm
        rw [hframes] at heq
        simp only [List.map_cons, List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        have htermf : f.terminator = some .unreachable := by
          rw [← hf'] at hterm
          split at hterm
          · rwa [show (f.setMem (IRAddr.stack frame slot off) v).terminator =
              f.terminator from rfl] at hterm
          · exact hterm
        exact hnoBareInst hres (by rw [← htermEq]; exact htermf)
  | root slot off =>
      have hw : c.writeStorage (.root slot) off v = some s' := by
        simpa [IRState.storeValue] using hstore
      have hlive := storageLive_of_evalOperand (hwf.storagePtrsLive f hf)
        hwf.storageDomain hsymsValueFrag hvalue
      have hnoSlotsV : ∀ gname off', v = .ptr (.global gname off') →
          gname ∉ SlotsGlobals c.module := by
        intro gname off' hval
        exact noSlotsGlobal_of_evalOperand hnoSlotsLocals hsymsValue
          (by simpa [hval] using hvalue)
      have hnotSlots : ∀ g, (StoreId.root slot) = .global g →
          g ∉ SlotsGlobals c.module := by
        intro g heq
        cases heq
      have hs'frames : s'.frames = c.frames := writeStorage_frames hw
      have hstate : WFState contract s' :=
        wfState_storeStorage hwf hw hlive.1 hlive.2 hnoSlotsV hnotSlots
      apply wfState_withHeadInstrs hstate
      · intro f' fs heq
        rw [hs'frames, hframes] at heq
        simp only [List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        rw [← hf']
        exact ⟨block, hblock, hblockEq, htermEq,
          done ++ [.store ty value ptr align], hblockShape⟩
      · intro f' fs heq hres hterm
        rw [hs'frames, hframes] at heq
        simp only [List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        have htermf : f.terminator = some .unreachable := by
          rw [← hf'] at hterm
          exact hterm
        exact hnoBareInst hres (by rw [← htermEq]; exact htermf)
  | global name off =>
      have hw : c.writeStorage (.global name) off v = some s' := by
        simpa [IRState.storeValue] using hstore
      have hlive := storageLive_of_evalOperand (hwf.storagePtrsLive f hf)
        hwf.storageDomain hsymsValueFrag hvalue
      have hnoSlotsV : ∀ gname off', v = .ptr (.global gname off') →
          gname ∉ SlotsGlobals c.module := by
        intro gname off' hval
        exact noSlotsGlobal_of_evalOperand hnoSlotsLocals hsymsValue
          (by simpa [hval] using hvalue)
      have hnotSlots : ∀ g, (StoreId.global name) = .global g →
          g ∉ SlotsGlobals c.module := by
        intro g heq
        have hg : name = g := StoreId.global.inj heq
        rw [← hg]
        exact noSlotsGlobal_of_evalOperand hnoSlotsLocals hsymsPtr hptr
      have hs'frames : s'.frames = c.frames := writeStorage_frames hw
      have hstate : WFState contract s' :=
        wfState_storeStorage hwf hw hlive.1 hlive.2 hnoSlotsV hnotSlots
      apply wfState_withHeadInstrs hstate
      · intro f' fs heq
        rw [hs'frames, hframes] at heq
        simp only [List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        rw [← hf']
        exact ⟨block, hblock, hblockEq, htermEq,
          done ++ [.store ty value ptr align], hblockShape⟩
      · intro f' fs heq hres hterm
        rw [hs'frames, hframes] at heq
        simp only [List.cons.injEq] at heq
        rcases heq with ⟨hf', _⟩
        have htermf : f.terminator = some .unreachable := by
          rw [← hf'] at hterm
          exact hterm
        exact hnoBareInst hres (by rw [← htermEq]; exact htermf)
  | slotEntry slot => simp [IRState.storeValue] at hstore
  | cell slot => simp [IRState.storeValue] at hstore
  | slotTable => simp [IRState.storeValue] at hstore
  | function name => simp [IRState.storeValue] at hstore
  | importCell slot => simp [IRState.storeValue] at hstore

/-- The callee frame pushed by `callFunction`. -/
def calleeFrame (s : IRState) (fn : Function) (block : Block) (args : List IRValue)
    (dest : Option Ssa) : IRFrame :=
  { id := s.nextFrame, fn := fn, block := block, instrs := block.instrs,
    terminator := block.terminator, locals := initLocals fn args, mem := [],
    allocs := [], nextAlloc := 0, prev := none, retDest := dest }

theorem lookupStorage_push (c : IRState) (fs : List IRFrame) (n : Nat) (id : StoreId) :
    ({ c with frames := fs, nextFrame := n } : IRState).lookupStorage id =
      c.lookupStorage id := rfl

theorem lookupStorageAt_push (c : IRState) (fs : List IRFrame) (n : Nat)
    (id : StoreId) (off : Nat) :
    ({ c with frames := fs, nextFrame := n } : IRState).lookupStorageAt id off =
      c.lookupStorageAt id off := rfl

theorem lookupFunction_push (c : IRState) (fs : List IRFrame) (n : Nat) (slot : Nat) :
    ({ c with frames := fs, nextFrame := n } : IRState).lookupFunction slot =
      c.lookupFunction slot := rfl

theorem slotKind?_push (c : IRState) (fs : List IRFrame) (n : Nat) (slot : Nat) :
    ({ c with frames := fs, nextFrame := n } : IRState).slotKind? slot =
      c.slotKind? slot := rfl

theorem module_push (c : IRState) (fs : List IRFrame) (n : Nat) :
    ({ c with frames := fs, nextFrame := n } : IRState).module = c.module := rfl

theorem frameLookups_push {contract : IRContract} {c : IRState} {caller : IRFrame}
    {tail : List IRFrame} {fn : Function} {block : Block} {args : List IRValue}
    {dest : Option Ssa} (hwf : WFState contract c) (hframes : c.frames = caller :: tail) :
    FrameLookupsPreserved c
      { c with
        frames := calleeFrame c fn block args dest :: caller :: tail,
        nextFrame := c.nextFrame + 1 } := by
  intro sid g hfind
  unfold IRState.frameById? at hfind
  rw [hframes] at hfind
  have hbound : g.id < c.nextFrame := by
    apply hwf.frameIdsBound
    rw [hframes]
    exact List.mem_of_find?_eq_some hfind
  have hgid : g.id = sid := beq_iff_eq.mp
    (List.find?_some (p := fun frame : IRFrame => frame.id == sid) (a := g) hfind)
  have hneg : ¬ ((calleeFrame c fn block args dest).id == sid) = true := by
    intro hb
    have hcf : (calleeFrame c fn block args dest).id = c.nextFrame := rfl
    have hidEq : c.nextFrame = sid := by
      rw [hcf] at hb
      exact beq_iff_eq.mp hb
    omega
  refine ⟨g, ?_, ?_⟩
  · unfold IRState.frameById?
    show (calleeFrame c fn block args dest :: caller :: tail).find?
      (fun frame : IRFrame => frame.id == sid) = some g
    rw [List.find?_cons_of_neg (p := fun frame : IRFrame => frame.id == sid)
      (a := calleeFrame c fn block args dest) hneg]
    exact hfind
  · intro slot ty hty
    exact hty

theorem eq_of_mem_of_nodup_map {α β : Type} {f : α → β} {l : List α} {a b : α}
    (hnd : (l.map f).Nodup) (ha : a ∈ l) (hb : b ∈ l) (h : f a = f b) : a = b := by
  obtain ⟨i, hi⟩ := List.getElem?_of_mem ha
  obtain ⟨j, hj⟩ := List.getElem?_of_mem hb
  have hiLt : i < l.length := (List.getElem?_eq_some_iff.mp hi).choose
  have hmap : (l.map f)[i]? = (l.map f)[j]? := by
    rw [List.getElem?_map, List.getElem?_map, hi, hj]
    change some (f a) = some (f b)
    rw [h]
  have hij : i = j := (hnd.getElem?_inj (by simpa using hiLt)).mp hmap
  rw [hij] at hi
  rw [hi] at hj
  exact Option.some.inj hj

theorem block?_eq_some {fn : Function} {label : Label} {block : Block}
    (hnd : (fn.blocks.map (·.label)).Nodup) (hmem : block ∈ fn.blocks)
    (hlabel : block.label = label) : fn.block? label = some block := by
  have hisSome : (fn.block? label).isSome := by
    unfold Function.block?
    rw [List.find?_isSome]
    exact ⟨block, hmem, by rw [hlabel]; simp⟩
  cases hfind : fn.block? label with
  | none => rw [hfind] at hisSome; simp at hisSome
  | some b =>
      have hbmem : b ∈ fn.blocks := by
        unfold Function.block? at hfind
        exact List.mem_of_find?_eq_some (p := fun block : Block => block.label == label)
          (a := b) hfind
      have hblabel : b.label = label := by
        unfold Function.block? at hfind
        exact beq_iff_eq.mp (List.find?_some (p := fun block : Block => block.label == label) (a := b) hfind)
      have hbeq : b = block :=
        eq_of_mem_of_nodup_map (f := Block.label) hnd hbmem hmem (by rw [hblabel, hlabel])
      rw [hbeq]

theorem firstIndex?_some {α : Type} {p : α → Bool} {l : List α} :
    ∀ {i : Nat}, firstIndex? p l = some i → ∃ h : i < l.length, p l[i] = true := by
  induction l with
  | nil => intro i h; simp [firstIndex?] at h
  | cons a rest ih =>
      intro i h
      simp only [firstIndex?] at h
      split at h
      · rename_i hp
        simp only [Option.some.injEq] at h
        subst h
        exact ⟨Nat.zero_lt_succ _, hp⟩
      · simp only [Option.map_eq_some_iff] at h
        rcases h with ⟨j, hj, hji⟩
        rcases ih hj with ⟨hlt, hpj⟩
        subst hji
        exact ⟨Nat.succ_lt_succ hlt, by simpa using hpj⟩

theorem defSite?_some {fn : Function} {name : String} {label : Label} {index : Nat}
    (hnd : (fn.blocks.map (·.label)).Nodup)
    (h : fn.defSite? name = some (label, index)) :
    ∃ block, fn.block? label = some block ∧
      ∃ hlt : index < block.instrs.length, block.instrs[index].dest? = some name := by
  unfold Function.defSite? at h
  cases hf : fn.blocks.filterMap (fun block =>
      (firstIndex? (fun instr => instr.dest? == some name) block.instrs).map
        (fun index => (block.label, index))) with
  | nil => simp [hf] at h
  | cons x xs =>
      rw [hf] at h
      simp only [List.head?_cons, Option.some.injEq] at h
      subst h
      have hxmem : (label, index) ∈ fn.blocks.filterMap (fun block =>
          (firstIndex? (fun instr => instr.dest? == some name) block.instrs).map
            (fun index => (block.label, index))) := by
        rw [hf]
        exact List.mem_cons_self ..
      rw [List.mem_filterMap] at hxmem
      rcases hxmem with ⟨block, hblock, hfmap⟩
      rw [Option.map_eq_some_iff] at hfmap
      rcases hfmap with ⟨j, hj, hji⟩
      rcases firstIndex?_some hj with ⟨hlt, hp⟩
      injection hji with hlabel hindex
      subst hlabel
      subst hindex
      refine ⟨block, block?_eq_some hnd hblock rfl, hlt, ?_⟩
      exact beq_iff_eq.mp hp

theorem mem_of_defInstr? {fn : Function} {name : String} {instr : Instr}
    (h : defInstr? fn name = some instr) :
    ∃ block ∈ fn.blocks, instr ∈ block.instrs := by
  unfold defInstr? at h
  cases hsite : fn.defSite? name with
  | none => simp [hsite] at h
  | some site =>
      rcases site with ⟨label, index⟩
      simp only [hsite] at h
      cases hblock : fn.block? label with
      | none => simp [hblock] at h
      | some block =>
          simp only [hblock] at h
          refine ⟨block, ?_, ?_⟩
          · unfold Function.block? at hblock
            exact List.mem_of_find?_eq_some hblock
          · exact List.mem_of_getElem? h

theorem defInstr?_dest {fn : Function} {name : String} {instr : Instr}
    (hnd : (fn.blocks.map (·.label)).Nodup)
    (h : defInstr? fn name = some instr) : instr.dest? = some name := by
  unfold defInstr? at h
  cases hsite : fn.defSite? name with
  | none => simp [hsite] at h
  | some site =>
      rcases site with ⟨label, index⟩
      simp only [hsite] at h
      rcases defSite?_some hnd hsite with ⟨block, hblock, hlt, hdest⟩
      cases hb : fn.block? label with
      | none => simp [hb] at h
      | some block' =>
          simp only [hb] at h
          rw [hb] at hblock
          have hblocks : block' = block := Option.some.inj hblock
          subst block'
          rw [List.getElem?_eq_getElem hlt] at h
          have hinstr : block.instrs[index] = instr := Option.some.inj h
          rw [← hinstr]
          exact hdest



theorem slotBase_gep {fn : Function} {base : String} (h : base ∈ SlotBases fn) :
    ∃ gepName dest ty indices inbounds,
      defInstr? fn gepName = some (.gep dest ty (.local base) indices inbounds) := by
  unfold SlotBases at h
  rw [List.mem_filterMap] at h
  rcases h with ⟨entry, hentry, hval⟩
  rcases entry with ⟨name, origin⟩
  cases origin with
  | cell slot isTrue =>
      cases isTrue with
      | true => simp at hval
      | false =>
          cases hload : defInstr? fn name with
          | none => simp [hload] at hval
          | some instr =>
              simp only [hload] at hval
              cases instr with
              | load dest ty ptr align =>
                  cases ptr with
                  | «local» gepName =>
                      cases hgep : defInstr? fn gepName with
                      | none => simp [hgep] at hval
                      | some gep =>
                          simp only [hgep] at hval
                          cases gep with
                          | gep dest2 ty2 baseOp indices inbounds =>
                              cases baseOp with
                              | «local» baseName =>
                                  injection hval with hbn
                                  subst hbn
                                  exact ⟨gepName, dest2, ty2, indices, inbounds, hgep⟩
                              | intLit ty3 val =>
                                  simp [operandSymbols] at hval
                              | nullPtr => simp at hval
                              | symbol name2 => simp at hval
                          | alloca d t => simp at hval
                          | store t v p a => simp at hval
                          | load d t p a => simp at hval
                          | binop d op t l r => simp at hval
                          | icmp d pr t l r => simp at hval
                          | select d t c tv ev => simp at hval
                          | cast d op v ft tt => simp at hval
                          | phi d t inc => simp at hval
                          | call d r c a => simp at hval
                          | callPtr d r c a => simp at hval
                          | extractValue d agg i t => simp at hval
                  | intLit ty2 val => simp at hval
                  | nullPtr => simp at hval
                  | symbol name2 => simp at hval
              | alloca d t => simp at hval
              | store t v p a => simp at hval
              | gep d t b i ib => simp at hval
              | binop d op t l r => simp at hval
              | icmp d pr t l r => simp at hval
              | select d t c tv ev => simp at hval
              | cast d op v ft tt => simp at hval
              | phi d t inc => simp at hval
              | call d r c a => simp at hval
              | callPtr d r c a => simp at hval
              | extractValue d agg i t => simp at hval
  | global g => simp at hval
  | vtableCode => simp at hval



theorem slotEntry_gep {fn : Function} {gepName : String} {slot : Nat}
    (h : (gepName, slot) ∈ SlotEntries fn) :
    ∃ entry ∈ fn.facts.origins, ∃ dest ty align,
      defInstr? fn entry.1 = some (.load dest ty (.local gepName) align) ∧
      ∃ dest2 ty2 base indices inbounds,
        defInstr? fn gepName = some (.gep dest2 ty2 base indices inbounds) := by
  unfold SlotEntries at h
  rw [List.mem_filterMap] at h
  rcases h with ⟨entry, hentry, hval⟩
  rcases entry with ⟨name, origin⟩
  cases origin with
  | cell slot' isTrue =>
      cases isTrue with
      | true => simp at hval
      | false =>
          cases hload : defInstr? fn name with
          | none => simp [hload] at hval
          | some instr =>
              simp only [hload] at hval
              cases instr with
              | load dest ty ptr align =>
                  cases ptr with
                  | «local» gepName' =>
                      cases hgep : defInstr? fn gepName' with
                      | none => simp [hgep] at hval
                      | some gep =>
                          simp only [hgep] at hval
                          cases gep with
                          | gep dest2 ty2 baseOp indices inbounds =>
                              cases baseOp with
                              | «local» baseName =>
                                  simp only [] at hval
                                  by_cases hcond : indices =
                                      [(.int 64, .intLit (.int 64) (Int.ofNat slot'))]
                                  · rw [ite_eq_left hcond] at hval
                                    simp only [Option.some.injEq, Prod.mk.injEq] at hval
                                    rcases hval with ⟨hgn, _⟩
                                    subst hgn
                                    exact ⟨(name, PointerOrigin.cell slot' false), hentry,
                                      dest, ty, align, hload, dest2, ty2, .local baseName,
                                      indices, inbounds, hgep⟩
                                  · rw [ite_eq_right hcond] at hval
                                    simp at hval
                              | intLit ty3 val => simp at hval
                              | nullPtr => simp at hval
                              | symbol name2 => simp at hval
                          | alloca d t => simp at hval
                          | store t v p a => simp at hval
                          | load d t p a => simp at hval
                          | binop d op t l r => simp at hval
                          | icmp d pr t l r => simp at hval
                          | select d t c tv ev => simp at hval
                          | cast d op v ft tt => simp at hval
                          | phi d t inc => simp at hval
                          | call d r c a => simp at hval
                          | callPtr d r c a => simp at hval
                          | extractValue d agg i t => simp at hval
                  | intLit ty2 val => simp at hval
                  | nullPtr => simp at hval
                  | symbol name2 => simp at hval
              | alloca d t => simp at hval
              | store t v p a => simp at hval
              | gep d t b i ib => simp at hval
              | binop d op t l r => simp at hval
              | icmp d pr t l r => simp at hval
              | select d t c tv ev => simp at hval
              | cast d op v ft tt => simp at hval
              | phi d t inc => simp at hval
              | call d r c a => simp at hval
              | callPtr d r c a => simp at hval
              | extractValue d agg i t => simp at hval
  | global g => simp at hval
  | vtableCode => simp at hval

theorem slotBase_localType {contract : IRContract} {frag : IRFrag} {fn : Function}
    {base : String} (hprog : WFProgram contract frag) (hfn : fn ∈ frag.functions)
    (hbase : base ∈ SlotBases fn) : fn.localType? base = some .ptr := by
  rcases slotBase_gep hbase with ⟨gepName, dest, ty, indices, inbounds, hgep⟩
  rcases mem_of_defInstr? hgep with ⟨block, hblock, hinstr⟩
  have hwell := (hprog.wellFormed.typesCorrect fn hfn).instructions block hblock
    (.gep dest ty (.local base) indices inbounds) hinstr
  simp only [Instr.WellTyped] at hwell
  exact hwell.2.1

theorem slotEntry_localType {contract : IRContract} {frag : IRFrag} {fn : Function}
    {gepName : String} {slot : Nat} (hprog : WFProgram contract frag)
    (hfn : fn ∈ frag.functions) (h : (gepName, slot) ∈ SlotEntries fn) :
    fn.localType? gepName = some .ptr := by
  rcases slotEntry_gep h with
    ⟨entry, hentry, dest, ty, align, hload, dest2, ty2, base, indices, inbounds, hgep⟩
  rcases mem_of_defInstr? hload with ⟨block, hblock, hinstr⟩
  have hwell := (hprog.wellFormed.typesCorrect fn hfn).instructions block hblock
    (.load dest ty (.local gepName) align) hinstr
  simp only [Instr.WellTyped] at hwell
  exact hwell.2

theorem slotEntry_name_notParam {fn : Function} {gepName : String} {slot : Nat}
    (hnd : (fn.blocks.map (·.label)).Nodup) (hud : FunctionUsesDefined fn)
    (h : (gepName, slot) ∈ SlotEntries fn) : gepName ∉ fn.params.map (·.2) := by
  rcases slotEntry_gep h with
    ⟨entry, hentry, dest, ty, align, hload, dest2, ty2, base, indices, inbounds, hgep⟩
  have hdest : (Instr.gep dest2 ty2 base indices inbounds).dest? = some gepName :=
    defInstr?_dest hnd hgep
  rcases mem_of_defInstr? hgep with ⟨block, hblock, hinstr⟩
  have hdestMem : gepName ∈ fn.blocks.flatMap (fun b => b.instrs.filterMap Instr.dest?) :=
    mem_dests_of_block hblock hinstr hdest
  intro hmemP
  have hall := hud.allNodup
  rw [List.nodup_append] at hall
  exact hall.2.2 gepName hmemP gepName hdestMem rfl



theorem wfState_calleePush {contract : IRContract} {c : IRState} {caller : IRFrame}
    {tail : List IRFrame} {fn : Function} {block : Block} {restBlocks : List Block}
    {args : List IRValue} {dest : Option Ssa}
    (hwf : WFState contract c) (hframes : c.frames = caller :: tail)
    (hfnMem : fn ∈ c.module.functions) (hblocks : fn.blocks = block :: restBlocks)
    (hlen : args.length = fn.params.length) (hargs : ArgsTyped fn args)
    (hargsLive : ∀ p a, (p, a) ∈ fn.params.zip args →
      (∀ slot off, a = .ptr (.root slot off) →
        ∃ values, c.lookupStorage (.root slot) = some values ∧ off < values.length) ∧
      (∀ g off, a = .ptr (.global g off) →
        ∃ values, c.lookupStorage (.global g) = some values ∧ off < values.length))
    (hargsNoSlots : ∀ p a, (p, a) ∈ fn.params.zip args → ∀ g off,
      a = .ptr (.global g off) → g ∉ SlotsGlobals c.module)
    (hargsBounded : ∀ p a, (p, a) ∈ fn.params.zip args → ∀ frame slot off,
      a = .ptr (.stack frame slot off) →
        ∃ g, c.frameById? frame = some g ∧ ∃ ty, g.allocType? slot = some ty ∧
          off < allocSize ty)
    (hretDest : ∀ d, dest = some d → fn.returnType ≠ .void ∧
      caller.fn.localType? d = some fn.returnType)
    (hslotsParam : ∀ p, fn.params.getLast? = some p → p.2 = "%slots" →
      ∃ a, (p, a) ∈ fn.params.zip args ∧ a = .ptr .slotTable) :
    WFState contract
      { c with
        frames := calleeFrame c fn block args dest :: caller :: tail,
        nextFrame := c.nextFrame + 1 } := by
  have hud : FunctionUsesDefined fn := hwf.program.wellFormed.usesDefined fn hfnMem
  have hrep : ∀ name ty, fn.localType? name = some ty → ty.isRepresentable = true :=
    hwf.program.localTypesRepresentable fn hfnMem
  have hnd : (fn.blocks.map (·.label)).Nodup :=
    (hwf.program.wellFormed.typesCorrect fn hfnMem).labelsUnique
  have hlook : FrameLookupsPreserved c
      { c with
        frames := calleeFrame c fn block args dest :: caller :: tail,
        nextFrame := c.nextFrame + 1 } :=
    frameLookups_push (contract := contract) hwf hframes
  have hframes' : ({ c with
        frames := calleeFrame c fn block args dest :: caller :: tail,
        nextFrame := c.nextFrame + 1 } : IRState).frames =
      calleeFrame c fn block args dest :: c.frames := by
    rw [hframes]
  constructor
  · exact hwf.program
  · rw [hframes', List.map_cons, List.nodup_cons]
    constructor
    · intro hmem
      rcases List.mem_map.mp hmem with ⟨g, hg, hgid⟩
      have hlt : g.id < c.nextFrame := hwf.frameIdsBound g hg
      have hcfid : (calleeFrame c fn block args dest).id = c.nextFrame := rfl
      rw [hcfid] at hgid
      omega
    · exact hwf.frameIdsNodup
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · have hcfid : (calleeFrame c fn block args dest).id = c.nextFrame := rfl
      rw [hcfid]
      show c.nextFrame < c.nextFrame + 1
      omega
    · have hlt := hwf.frameIdsBound f' hf'
      show f'.id < c.nextFrame + 1
      omega
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact hfnMem
    · exact hwf.frameFunction f' hf'
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · refine ⟨block, ?_, rfl, rfl, [], rfl⟩
      change block ∈ fn.blocks
      rw [hblocks]
      exact List.mem_cons_self ..
    · exact hwf.frameBlock f' hf'
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact Or.inl ⟨rfl, ⟨restBlocks, hblocks⟩⟩
    · exact hwf.framePrev f' hf'
  · intro f' fs heq hinst hterm
    rw [hframes'] at heq
    injection heq with hf' _
    rw [← hf'] at hinst hterm
    have hinstrs : block.instrs = [] := hinst
    have htermBlock : block.terminator = some .unreachable := hterm
    rcases hwf.program.unreachableGuarded fn hfnMem block
      (by rw [hblocks]; exact List.mem_cons_self ..) htermBlock with ⟨instr, hlast, _⟩
    rw [hinstrs] at hlast
    simp at hlast
  · intro f' hf' name ty hty
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · rcases lookupLocal_initLocals_of_localType hud hlen hargs hrep hty with
        ⟨v, hfind, _⟩
      refine ⟨v, ?_⟩
      unfold IRFrame.lookupLocal
      show (List.find? (fun entry => entry.1 == name) (initLocals fn args)).map
        (fun x => x.snd) = some v
      rw [hfind]
      rfl
    · exact hwf.localsInitialized f' hf' name ty hty
  · intro f' hf' name v hv ty hty
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · exact initLocals_value_typed hud hlen hargs hrep (lookupLocal_find?_eq hv) hty
    · exact hwf.localsTyped f' hf' name v hv ty hty
  · intro f' hf' entry hentry
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · rcases hwf.program.originsTrusted fn hfnMem with ⟨_, htrusted⟩
      rcases htrusted entry hentry with ⟨hlocTy, hnotParam, _⟩
      have hfind := initLocals_lookup_default (args := args) hud hnotParam hlocTy
      refine ⟨.null, ?_, hlocTy, Or.inl rfl⟩
      unfold IRFrame.lookupLocal
      show (List.find? (fun e => e.1 == entry.1) (initLocals fn args)).map
        (fun x => x.snd) = some .null
      rw [hfind]
      rfl
    · exact hwf.originsMatch f' hf' entry hentry
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · simp [calleeFrame, AllocsWellFormed, IRFrame.allocType?]
    · exact hwf.allocsWellFormed f' hf'
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · have hb : StackPtrsBounded c (calleeFrame c fn block args dest) := by
        intro name v addr hv haddr
        cases addr with
        | stack frame slot off =>
            rcases initLocals_find_cases (lookupLocal_find?_eq hv) with
              ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
            · exact hargsBounded p a hzip frame slot off (by rw [ha]; exact haddr)
            · exact absurd (by rw [← hvdef]; exact haddr) (default_ne_stack ty frame slot off)
        | slotEntry slot => trivial
        | cell slot => trivial
        | root slot off => trivial
        | global name off => trivial
        | slotTable => trivial
        | function name => trivial
        | importCell slot => trivial
      exact stackPtrsBounded_of_frameLookups hlook hb
    · exact stackPtrsBounded_of_frameLookups hlook (hwf.stackPtrsBounded f' hf')
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · intro addr v hmem
      simp [calleeFrame, IRFrame.lookupMem] at hmem
    · exact memTyped_of_frameLookups hlook (hwf.memTyped f' hf')
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · constructor
      · intro name v slot off hv hptr
        rcases initLocals_find_cases (lookupLocal_find?_eq hv) with
          ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
        · have hlive := (hargsLive p a hzip).1 slot off (by rw [ha]; exact hptr)
          simpa [lookupStorage_push] using hlive
        · exact absurd (by rw [← hvdef]; exact hptr) (default_ne_root ty slot off)
      · intro name v g off hv hptr
        rcases initLocals_find_cases (lookupLocal_find?_eq hv) with
          ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
        · have hlive := (hargsLive p a hzip).2 g off (by rw [ha]; exact hptr)
          simpa [lookupStorage_push] using hlive
        · exact absurd (by rw [← hvdef]; exact hptr) (default_ne_global ty g off)
    · have h := hwf.storagePtrsLive f' hf'
      simpa [StoragePtrsLive, IRFrame.lookupLocal, lookupStorage_push] using h
  · intro f' hf'
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · refine ⟨?_, ?_, ?_⟩
      · intro base hbase
        have hnotParam : base ∉ fn.params.map (·.2) :=
          hwf.program.slotBasesNonParam fn hfnMem base hbase
        have hlocTy : fn.localType? base = some .ptr :=
          slotBase_localType hwf.program hfnMem hbase
        have hfind := initLocals_lookup_default (args := args) hud hnotParam hlocTy
        right
        unfold IRFrame.lookupLocal
        show (List.find? (fun e => e.1 == base) (initLocals fn args)).map
          (fun x => x.snd) = some .null
        rw [hfind]
        rfl
      · intro gepName slot hgep
        have hnotParam : gepName ∉ fn.params.map (·.2) :=
          slotEntry_name_notParam hnd hud hgep
        have hlocTy : fn.localType? gepName = some .ptr :=
          slotEntry_localType hwf.program hfnMem hgep
        have hfind := initLocals_lookup_default (args := args) hud hnotParam hlocTy
        right
        unfold IRFrame.lookupLocal
        show (List.find? (fun e => e.1 == gepName) (initLocals fn args)).map
          (fun x => x.snd) = some .null
        rw [hfind]
        rfl
      · intro base hbase g ty ptr align hdef hsym
        exact hwf.slotsStorage fn hfnMem base hbase g ty ptr align hdef hsym
    · have h := hwf.slotsBase f' hf'
      simpa [SlotsBase, SlotsGlobalStored, IRFrame.lookupLocal, lookupStorageAt_push] using h
  · intro fn' hfn' base hbase g ty ptr align hdef hsym
    simpa [lookupStorageAt_push] using
      hwf.slotsStorage fn' hfn' base hbase g ty ptr align hdef hsym
  · exact hwf.storageValuesLive
  · exact hwf.storageDomain
  · constructor
    · intro f' hf' name v g off hv hval
      rw [hframes'] at hf'
      rcases List.mem_cons.mp hf' with rfl | hf'
      · rcases initLocals_find_cases (lookupLocal_find?_eq hv) with
          ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
        · exact hargsNoSlots p a hzip g off (by rw [ha]; exact hval)
        · exact absurd (by rw [← hvdef]; exact hval) (default_ne_global ty g off)
      · exact hwf.noSlotsGlobalPointers.1 f' hf' name v g off hv hval
    · constructor
      · intro f' hf' addr v g off hv hval
        rw [hframes'] at hf'
        rcases List.mem_cons.mp hf' with rfl | hf'
        · simp [calleeFrame, IRFrame.lookupMem] at hv
        · exact hwf.noSlotsGlobalPointers.2.1 f' hf' addr v g off hv hval
      · intro id off v g off' hv hval
        exact hwf.noSlotsGlobalPointers.2.2 id off v g off'
          (by simpa [lookupStorageAt_push] using hv) hval
  · exact hwf.functionBindings
  · exact hwf.callTargets
  · intro pre callee' caller' suffix heq
    rw [hframes'] at heq
    cases pre with
    | nil =>
        simp only [List.nil_append] at heq
        injection heq with hcallee hrest
        subst hcallee
        have hcaller : caller' = caller := by
          rw [hframes] at hrest
          exact (List.cons.inj hrest).1.symm
        subst hcaller
        rw [show (calleeFrame c fn block args dest).retDest = dest from rfl]
        cases hd : dest with
        | none => trivial
        | some d => exact hretDest d hd
    | cons a as =>
        simp only [List.cons_append] at heq
        injection heq with hcallee hrest
        subst hcallee
        exact hwf.returnLink as callee' caller' suffix hrest
  · intro last hlast
    rw [hframes'] at hlast
    cases hc : c.frames with
    | nil => rw [hc] at hframes; simp at hframes
    | cons h t =>
        have hlast' : c.frames.getLast? = some last := by
          rw [hc] at hlast ⊢
          simpa using hlast
        exact hwf.bottomRetDest last hlast'
  · intro f' hf' operand v htermv heval frame slot off hv
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · change block.terminator = some (Terminator.ret (some operand)) at htermv
      cases operand with
      | «local» name =>
          simp only [evalOperand] at heval
          rcases initLocals_find_cases (lookupLocal_find?_eq heval) with
            ⟨p, a, _, _, hzip, ha⟩ | ⟨ty, _, hvdef⟩
          · have hb := hargsBounded p a hzip frame slot off (by rw [ha]; exact hv)
            rcases hb with ⟨g, hfind, ty, hty, hoff⟩
            have hlt : g.id < c.nextFrame := hwf.frameIdsBound g
              (List.mem_of_find?_eq_some (by simpa [IRState.frameById?] using hfind))
            have hne : frame ≠ c.nextFrame := by
              intro heq
              have hgid : g.id = frame :=
                beq_iff_eq.mp (List.find?_some
                  (p := fun f : IRFrame => f.id == frame) (a := g)
                  (by simpa [IRState.frameById?] using hfind))
              rw [hgid] at hlt
              rw [heq] at hlt
              omega
            have hcfid : (calleeFrame c fn block args dest).id = c.nextFrame := rfl
            rw [hcfid]
            exact hne
          · exact absurd (by rw [← hvdef]; exact hv) (default_ne_stack ty frame slot off)
      | intLit ty val =>
          simp only [evalOperand] at heval
          have hveq : v = .int ty val := (Option.some.inj heval).symm
          rw [hveq] at hv
          simp at hv
      | nullPtr =>
          simp only [evalOperand] at heval
          have hveq : v = .null := (Option.some.inj heval).symm
          rw [hveq] at hv
          simp at hv
      | symbol name =>
          simp only [evalOperand] at heval
          have hveq : v = .ptr (.global name 0) := (Option.some.inj heval).symm
          rw [hveq] at hv
          simp at hv
    · exact hwf.stackReturnsSafe f' hf' operand v htermv heval frame slot off hv
  · intro f' hf' p hp hname
    rw [hframes'] at hf'
    rcases List.mem_cons.mp hf' with rfl | hf'
    · rcases hslotsParam p hp hname with ⟨a, hzip, ha⟩
      have hpMem : p ∈ fn.params := mem_left_of_zip hzip
      have hfind := find?_initLocals_param hud hlen hpMem hzip
      show (List.find? (fun entry => entry.1 == p.2) (initLocals fn args)).map
        (fun x => x.snd) = some (IRValue.ptr IRAddr.slotTable)
      rw [hfind]
      rw [ha]
      rfl
    · exact hwf.slotsParamsSafe f' hf' p hp hname

end Stc.Codegen
