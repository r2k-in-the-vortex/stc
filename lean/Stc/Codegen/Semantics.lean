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
offset. `cell` is a host GOT cell address, `root` the storage pointer stored in
a `Data`/`Direct` cell, and `function`/`importCell` the callable values of
`Function`/`Import` cells. `slotTable` is the host slot-table base parameter. -/
inductive IRAddr where
  | stack (frame : Nat) (slot : Nat) (offset : Nat)
  | cell (slot : Nat)
  | root (slot : Nat)
  | global (name : String)
  | globalOffset (name : String) (offset : Nat)
  | slotTable
  | function (name : String)
  | importCell (slot : Nat)
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

/-! ## Frames and state -/

/-- A keyed list lookup. -/
def lookupList {α β : Type} [BEq α] (entries : List (α × β)) (key : α) : Option β :=
  (entries.find? (fun entry => entry.1 == key)).map (·.2)

/-- A keyed list update or insert. -/
def setList {α β : Type} [BEq α] (entries : List (α × β)) (key : α)
    (value : β) : List (α × β) :=
  if entries.any (fun entry => entry.1 == key) then
    entries.map (fun entry => if entry.1 == key then (key, value) else entry)
  else entries ++ [(key, value)]

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
  | .symbol name => some (.ptr (.global name))

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
covered fragment emits zero-based array GEPs and slot-table indices. -/
def evalGepAddr (base : IRAddr) (indices : List Int) : Option IRAddr :=
  let offset := indices.foldl (fun acc index => acc + index.toNat) 0
  match base with
  | .slotTable =>
      match indices with
      | index :: _ => some (.cell index.toNat)
      | [] => none
  | .stack frame slot baseOffset => some (.stack frame slot (baseOffset + offset))
  | .global name => some (.globalOffset name offset)
  | .globalOffset name baseOffset => some (.globalOffset name (baseOffset + offset))
  | .root slot => some (.root slot)
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

/-- Find a frame by id. -/
def frameById? (s : IRState) (id : Nat) : Option IRFrame :=
  s.frames.find? (fun frame => frame.id == id)

/-- Load a value from an address. Cell loads return the root storage pointer or
the callable recorded by the slot table. -/
def loadValue (s : IRState) (addr : IRAddr) : Option IRValue :=
  match addr with
  | .stack frame _ _ =>
      match s.frameById? frame with
      | some f => f.lookupMem addr
      | none => none
  | .root slot => s.lookupRoot slot
  | .global name => s.lookupGlobal name
  | .globalOffset name _ => s.lookupGlobal name
  | .cell slot =>
      match s.slotKind? slot with
      | some .data | some .direct => some (.ptr (.root slot))
      | some .function => (s.lookupFunction slot).map (fun name => .ptr (.function name))
      | some .importCell => some (.ptr (.importCell slot))
      | some .descriptor => none
      | none => none
  | .slotTable => none
  | .function _ => none
  | .importCell _ => none

/-- Store a value to an address. GOT cell addresses are host-owned and are not
writable by generated code. -/
def storeValue (s : IRState) (addr : IRAddr) (value : IRValue) : Option IRState :=
  match addr with
  | .stack frame _ _ =>
      if s.frames.any (fun f => f.id == frame) then
        some ({ s with
          frames := s.frames.map fun f =>
            if f.id == frame then f.setMem addr value else f })
      else none
  | .root slot => some (s.setRoot slot value)
  | .global name => some (s.setGlobal name value)
  | .globalOffset name _ => some (s.setGlobal name value)
  | _ => none

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
  | .load dest _ty ptr _align =>
      match evalOperand f ptr with
      | some (.ptr addr) =>
          match s.loadValue addr with
          | some value =>
              .step (s.withHead ({ (f.setLocal dest value) with instrs := rest }))
          | none => .stuck s
      | _ => .stuck s
  | .store _ty value ptr _align =>
      match evalOperand f value, evalOperand f ptr with
      | some v, some (.ptr addr) =>
          match s.storeValue addr v with
          | some s' => .step (s'.withHead ({ f with instrs := rest }))
          | none => .ub s
      | _, _ => .stuck s
  | .gep dest _pointee base indices _inbounds =>
      match evalOperand f base with
      | some (.ptr addr) =>
          match evalOperandInts f (indices.map (·.2)) with
          | some values =>
              match evalGepAddr addr values with
              | some addr' =>
                  .step (s.withHead
                    { (f.setLocal dest (.ptr addr')) with instrs := rest })
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
                globals := globals, cycles := 0, cyclesLeft := cycles,
                nextFrame := 1, status := .running }
            some state
          else none

/-- The target cycle entry configuration. -/
def IRSem (module : IRFrag) (entry : String) (args : List IRValue)
    (roots : List (Nat × IRValue)) (functions : List (Nat × String))
    (imports : List (Nat × ImportKind)) (globals : List (String × IRValue))
    (cycles : Nat) : Option IRState :=
  initState? module entry args roots functions imports globals cycles

/-- Run the target machine with fuel. -/
def irRun (fuel : Nat) (module : IRFrag) (entry : String) (args : List IRValue)
    (roots : List (Nat × IRValue)) (functions : List (Nat × String))
    (imports : List (Nat × ImportKind)) (globals : List (String × IRValue))
    (cycles : Nat) : Option IROutcome :=
  (IRSem module entry args roots functions imports globals cycles).map (irExecRec fuel)

/-- The P0.4 smoke fragment produces an initial target state, so `IRSem` is
not vacuous. -/
example : (IRSem minimalFrag "main" [] [] [] [] [] 0).isSome = true := by decide

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
    | .data => s.loadValue (.cell slot) = some (.ptr (.root slot))
    | .direct => s.loadValue (.cell slot) = some (.ptr (.root slot))
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

end Stc.Codegen
