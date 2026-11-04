// SYNTAX TEST "source.st" "strings and escapes"

'plain'
// <------- string.quoted.single.st

"double"
// <-------- string.quoted.double.st

'it$'s $N $T $00 $$ $L'
// <----------------------- string.quoted.single.st

"wide $"quote$" $HHHH"
// <---------------------- string.quoted.double.st

U#'utf8 ${1F600}'
// <----------------- string.quoted.single.st

STRING#'typed'
// <-------------- string.quoted.single.st

WCHAR#"wide char"
// <----------------- string.quoted.double.st

'$41'
// <~--- constant.character.escape.st

"$$"
// <~-- constant.character.escape.st
