// SYNTAX TEST "source.st" "comments"

// line comment
// <--------------- comment.line.double-slash.st

/// documentation line
// <---------------------- comment.line.documentation.st

(* block comment *)
// <------------------- comment.block.st

/* slash block comment */
// <------------------------- comment.block.st

(* outer (* inner *) outer *)
// <----------------------------- comment.block.st

(* ( * ) // /* not special *)
// <----------------------------- comment.block.st
