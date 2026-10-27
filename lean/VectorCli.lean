/-
`lake exe vectors`: writes the committed exact-function vectors.

The generator is deterministic: running it twice produces byte-identical
files, and CI fails when the committed copies differ.
-/
import Stc.Vectors.Gen

open Stc.Vectors

def main : IO Unit := do
  let directory := "../tests/vectors/exact"
  IO.FS.createDirAll directory
  for vector in vectors do
    IO.FS.writeFile (directory ++ "/" ++ vectorFile vector) (renderVector vector)
  IO.println s!"wrote {vectors.length} exact-function vector files to {directory}"
