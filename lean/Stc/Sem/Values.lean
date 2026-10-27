/-
Core values, widths, locals, globals, GOT roots, memory, and traps (P0.3).

The model has no heap and no aliasing beyond GOT cells, so a configuration is a
value store plus a root map plus explicit call and control stacks. Exact
operations reuse `Stc.Exact`; this module supplies the configuration-level
wrappers.
-/
namespace Stc.Sem
end Stc.Sem
