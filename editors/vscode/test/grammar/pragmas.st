// SYNTAX TEST "source.st" "pragmas"

{stc:test}
// <---------- meta.pragma.st
// <----- punctuation.definition.pragma.begin.st
//   ^^^^ keyword.other.pragma.st
//   ^^^^ - variable.parameter.pragma.st

{stc:test {"cycles": 25}}
// <------------------------- meta.pragma.st
//   ^^^^ keyword.other.pragma.st
//         ^^^^^^^^ string.quoted.double.st
//         ^^^^^^^^ - keyword.other.pragma.st
//                   ^^ constant.numeric.pragma.st

{stc:io {"device":"drive1","channel":5}}
// <---------------------------------------- meta.pragma.st
//   ^^ keyword.other.pragma.st
//                                   ^ constant.numeric.pragma.st

{stc:deprecated {"message": "use `Other`"}}
// <------------------------------------------- meta.pragma.st
//   ^^^^^^^^^^ keyword.other.pragma.st
