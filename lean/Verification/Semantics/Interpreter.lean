/-
The executable reference interpreter for the core HIR subset (P0.3/P0.6).

The interpreter is a fuel-based total function over the same configuration type
as the declarative semantics: it iterates `Stc.Sem.step` until the run halts,
gets stuck, or runs out of fuel. `interpreter_sound` says every reported result
is reachable by the step relation and terminal; `interpreter_complete` says
every terminating run is reproduced with enough fuel.

This is a verification tool, not a production interpreter; performance is not a
goal. The interpreter and the case/dump front ends arrive fully in P0.6.
-/
import Stc.Sem

namespace Verification.Semantics

open Stc.Sem
open Stc.Hir

/-- The interpreter's result. -/
inductive RunResult where
  /-- The run reached a terminal configuration. -/
  | done (c : State)
  /-- The fuel ran out at a non-terminal configuration. -/
  | outOfFuel (c : State)
  /-- The machine got stuck at a non-terminal configuration. -/
  | stuck (c : State)

/-- Run the machine for at most `fuel` steps. -/
def runFuel (u : HirUnit) : Nat → State → RunResult
  | 0, c => .outOfFuel c
  | n + 1, c =>
      match step u c with
      | .step c' => runFuel u n c'
      | .halt => .done c
      | .stuck => .stuck c

/-- The reference semantics entry: initialize, then run with fuel. -/
def stcSem (u : HirUnit) (fuel cycles : Nat) : RunResult :=
  runFuel u fuel (initState u cycles)

/-- Every reported result is reachable by `Steps` and terminal. -/
theorem interpreter_sound {u : HirUnit} {fuel : Nat} {c r : State}
    (h : runFuel u fuel c = .done r) :
    Steps u c r ∧ Terminal u r := by
  induction fuel generalizing c with
  | zero => simp [runFuel] at h
  | succ n ih =>
      cases hs : step u c with
      | step c' =>
          simp only [runFuel, hs] at h
          rcases ih h with ⟨hsteps, hterm⟩
          exact ⟨Steps.step hs hsteps, hterm⟩
      | halt =>
          simp only [runFuel, hs, RunResult.done.injEq] at h
          subst h
          exact ⟨Steps.refl _, hs⟩
      | stuck =>
          simp only [runFuel, hs] at h
          cases h

/-- Every terminating run is reproduced with enough fuel. -/
theorem interpreter_complete {u : HirUnit} {c c' : State}
    (hs : Steps u c c') (ht : Terminal u c') :
    ∃ fuel, runFuel u fuel c = .done c' := by
  induction hs with
  | refl => exact ⟨1, by unfold Terminal at ht; simp [runFuel, ht]⟩
  | step hstep _ ih =>
      rcases ih ht with ⟨n, hn⟩
      unfold Step at hstep
      exact ⟨n + 1, by simp [runFuel, hstep, hn]⟩

end Verification.Semantics
