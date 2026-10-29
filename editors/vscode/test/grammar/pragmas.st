// SYNTAX TEST "source.st" "pragmas"

{stc:test}
// <---------- meta.pragma.st

{stc:test {"cycles": 25}}
// <------------------------- meta.pragma.st

{stc:io {"device":"drive1","channel":5}}
// <---------------------------------------- meta.pragma.st

{stc:deprecated {"message": "use `Other`"}}
// <------------------------------------------- meta.pragma.st
