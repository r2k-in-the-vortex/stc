/-
The reference-interpreter driver for a parsed core unit and case (P0.6).

`runReference` initializes the machine with the case's cycle count, runs it
with the case's fuel bound, and returns the terminal value or trap, the global
snapshot, the completed cycles, and an optional per-step trace. `encodeResult`
serializes that observation as a `stc-sem-result` `0.1.0` object with the dump,
case, and semantics versions recorded.

The driver does not compare against `expected`; it reports what the reference
machine did. `matchesExpected` is the comparison predicate the conformance
harness uses, and the unit tests exercise it directly.
-/
import Lean.Data.Json
import Verification.Semantics.CoreCase
import Verification.Semantics.Interpreter

namespace Verification.Semantics

open Lean (Json)
open Stc.Hir
open Stc.Sem
open Stc.Syntax

/-! ## Result format constants -/

/-- The reference-result format family. -/
def coreResultFormat : String := "stc-sem-result"

/-- The exact reference-result version emitted by this driver. -/
def coreResultFormatVersion : String := "0.1.0"

/-! ## Observation -/

/-- One global root in the terminal snapshot. -/
structure GlobalSnapshot where
  /-- The canonical global name. -/
  name : String
  /-- The declared global type. -/
  ty : Ty
  /-- The terminal value. -/
  value : Int

/-- One trace entry, recorded before a machine step. -/
structure TraceEntry where
  /-- The zero-based step index. -/
  step : Nat
  /-- Completed cycles before the step. -/
  cycles : Nat
  /-- Requested cycles remaining before the step. -/
  cyclesLeft : Nat
  /-- The focus tag: `expr`, `value`, `stmts`, `resume`, `returned`, `trapped`, or `halted`. -/
  focus : String
  /-- The number of frames on the call stack. -/
  frames : Nat
  /-- The number of continuations in the running frame. -/
  continuations : Nat

/-- The observable reference result of one case. -/
structure ReferenceResult where
  /-- The case name. -/
  caseName : String
  /-- The entry program name. -/
  entryName : String
  /-- `halted`, `trap`, `value`, `outOfFuel`, or `stuck`. -/
  status : String
  /-- The terminal value, when the machine stopped at one. -/
  value : Option Int
  /-- The terminal trap, for `status = "trap"`. -/
  trap : Option Trap
  /-- The trapping span, for `status = "trap"`. -/
  trapSpan : Option Span
  /-- The terminal global snapshot. -/
  globals : List GlobalSnapshot
  /-- Completed cycles. -/
  cycles : Nat
  /-- The requested cycle count. -/
  requestedCycles : Nat
  /-- The fuel bound used by the reference interpreter. -/
  fuel : Nat
  /-- The per-step trace, when requested. -/
  trace : Option (List TraceEntry)

/-- The machine state of a run result. -/
def RunResult.state : RunResult → State
  | .done c => c
  | .outOfFuel c => c
  | .stuck c => c

private def focusTag : Focus → String
  | .expr _ => "expr"
  | .value _ => "value"
  | .stmts _ _ => "stmts"
  | .resume => "resume"
  | .returned _ => "returned"
  | .trapped _ _ => "trapped"
  | .halted => "halted"

private def traceEntry (stepIndex : Nat) (c : State) : TraceEntry := {
  step := stepIndex
  cycles := c.cycles
  cyclesLeft := c.cyclesLeft
  focus := focusTag c.focus
  frames := c.frames.length
  continuations := c.k.length
}

private def runFuelTrace (u : HirUnit) : Nat → State → Array TraceEntry →
    RunResult × Array TraceEntry
  | 0, c, trace => (.outOfFuel c, trace)
  | fuel + 1, c, trace =>
      let entry := traceEntry trace.size c
      match step u c with
      | .step next => runFuelTrace u fuel next (trace.push entry)
      | .halt => (.done c, trace.push entry)
      | .stuck => (.stuck c, trace.push entry)

private def classifyFocus : Focus → String × Option Int × Option Trap × Option Span
  | .halted => ("halted", none, none, none)
  | .trapped trap span => ("trap", none, some trap, some span)
  | .value value => ("value", some value.value, none, none)
  | _ => ("stuck", none, none, none)

private def classify : RunResult → String × Option Int × Option Trap × Option Span
  | .done c => classifyFocus c.focus
  | .outOfFuel _ => ("outOfFuel", none, none, none)
  | .stuck _ => ("stuck", none, none, none)

private def snapshot (core : CoreUnit) (c : State) : List GlobalSnapshot :=
  (core.unit.globals.zip c.globals).map fun (global, value) =>
    { name := global.name, ty := global.ty, value := value.value }

/-- Run one case against a parsed core unit. -/
def runReference (core : CoreUnit) (descriptor : CoreCase) (withTrace : Bool) :
    Except String ReferenceResult := do
  if descriptor.entry.name != core.unit.program.name then
    throw s!"case.entry.name: `{descriptor.entry.name}` does not match the dump program `{core.unit.program.name}`"
  if descriptor.expected.globals.length != core.unit.globals.length then
    throw s!"case.expected.globals: expected {descriptor.expected.globals.length} value(s) for {core.unit.globals.length} global(s)"
  let (run, trace) ←
    if withTrace then
      let (run, trace) := runFuelTrace core.unit descriptor.fuel
        (initState core.unit descriptor.cycles) #[]
      pure (run, some trace.toList)
    else
      pure (stcSem core.unit descriptor.fuel descriptor.cycles, none)
  let state := run.state
  let (status, value, trap, trapSpan) := classify run
  pure {
    caseName := descriptor.name
    entryName := descriptor.entry.name
    status
    value
    trap
    trapSpan
    globals := snapshot core state
    cycles := state.cycles
    requestedCycles := descriptor.cycles
    fuel := descriptor.fuel
    trace
  }

/-- Whether a reference result agrees with a case's expected observation. -/
def ReferenceResult.matchesExpected (result : ReferenceResult) (expected : CaseExpected) : Bool :=
  result.status == expected.status &&
    result.trap == expected.trap &&
    result.globals.map (fun global => global.value) == expected.globals &&
    result.cycles == expected.cycles

/-! ## JSON encoding -/

/-- The canonical name of a trap. -/
def trapTag : Trap → String
  | .overflow => "OVERFLOW"
  | .divZero => "DIV_ZERO"
  | .range => "RANGE"

private def typeJson : Ty → Json
  | .bool => Json.mkObj [("kind", "bool")]
  | .int width signed =>
      Json.mkObj [("kind", "int"), ("width", width), ("signed", signed)]
  | .bits width =>
      Json.mkObj [("kind", "bits"), ("width", width)]
  | .char .char => Json.mkObj [("kind", "char"), ("charKind", "char")]
  | .char .wchar => Json.mkObj [("kind", "char"), ("charKind", "wchar")]
  | .char .uchar => Json.mkObj [("kind", "char"), ("charKind", "uchar")]

private def spanJson (core : CoreUnit) (span : Span) : Json :=
  Json.mkObj [
    ("file", core.fileOf span),
    ("start", span.start),
    ("end", span.stop)
  ]

private def globalJson (global : GlobalSnapshot) : Json :=
  Json.mkObj [
    ("name", global.name),
    ("type", typeJson global.ty),
    ("value", (global.value : Json))
  ]

private def traceJson (entry : TraceEntry) : Json :=
  Json.mkObj [
    ("step", entry.step),
    ("cycles", entry.cycles),
    ("cyclesLeft", entry.cyclesLeft),
    ("focus", entry.focus),
    ("frames", entry.frames),
    ("continuations", entry.continuations)
  ]

/-- Encode a reference result as a `stc-sem-result` `0.1.0` object. -/
def encodeResult (core : CoreUnit) (result : ReferenceResult) : Json :=
  let fields : List (String × Json) := [
    ("format", coreResultFormat),
    ("formatVersion", coreResultFormatVersion),
    ("semanticsVersion", coreDumpSemanticsVersion),
    ("dumpFormatVersion", coreDumpFormatVersion),
    ("caseFormatVersion", coreCaseFormatVersion),
    ("profile", coreDumpProfile),
    ("case", result.caseName),
    ("entry", Json.mkObj [("kind", "program"), ("name", result.entryName)]),
    ("status", result.status),
    ("value", match result.value with | some value => (value : Json) | none => Json.null),
    ("trap", match result.trap with | some trap => (trapTag trap : Json) | none => Json.null),
    ("trapSpan", match result.trapSpan with | some span => spanJson core span | none => Json.null),
    ("globals", Json.arr (result.globals.map globalJson).toArray),
    ("cycles", result.cycles),
    ("requestedCycles", result.requestedCycles),
    ("fuel", result.fuel)
  ]
  let fields :=
    match result.trace with
    | some trace => fields ++ [("trace", Json.arr (trace.map traceJson).toArray)]
    | none => fields
  Json.mkObj fields

end Verification.Semantics
