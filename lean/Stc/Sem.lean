/-
Static and dynamic semantics for the Phase 0 core subset (P0.2, P0.3).

`Stc.Sem.Typing` holds the declarative typing judgment `HasType`, the core
algorithmic checker `checkCore`, and well-formedness. `Stc.Sem.Eval` holds the
source small-step relation and the fuel-based recursive reference model.
-/
import Stc.Sem.Values
import Stc.Sem.Typing
import Stc.Sem.Eval

namespace Stc.Sem
end Stc.Sem
