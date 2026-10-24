//! Lossless concrete syntax tree, stable syntax kinds, typed AST views, and
//! pragma annotations for the `stc-2013-oop-utf8-v1` profile.
//!
//! The CST is a hand-rolled immutable arena tree (AST §3). It owns the source
//! text and preserves every byte as a token or trivia leaf, so
//! `tree.root().text() == tree.source()` for every input, valid or malformed.
//! Node spans include the trivia attached to their first and last tokens,
//! matching the rowan-style convention; diagnostics use token extents.

#![forbid(unsafe_code)]

pub mod ast;
pub mod builder;
pub mod codes;
pub mod dump;
mod kind;
pub mod pragma;
mod tree;

pub use builder::{Checkpoint, TreeBuilder, build_flat_tree};
pub use dump::{dump_cst, dump_typed_ast};
pub use kind::{NODE_BASE, SyntaxKind};
pub use pragma::PragmaAnnotation;
pub use tree::{
    Element, NodeId, SyntaxElement, SyntaxNode, SyntaxToken, SyntaxTree, SyntaxTrivia, TokenId,
    TriviaId,
};
