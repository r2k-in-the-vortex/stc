//! Stable textual dumps for tests, snapshots, and `stc parse` (AST §12).

use std::fmt::Write as _;

use crate::ast::{AstNode, Expr, Item, Name, Stmt, TypeDecl, VarDeclLike};
use crate::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, SyntaxTrivia};

/// Maximum indentation printed by the dumps.
///
/// A left-associative operator chain produces a very deep tree without
/// recursion in the parser. Capping indentation keeps the output linear in the
/// number of nodes; the printed order and kinds still identify the structure.
const MAX_INDENT: usize = 64;

enum Work {
    Node(SyntaxNode, usize),
    Token(SyntaxToken, usize),
    Trivia(SyntaxTrivia, usize),
}

/// Returns a stable pre-order dump of the CST, including trivia.
///
/// Every leaf is printed so a diff shows exactly which byte moved. Missing
/// tokens are marked `Missing`.
#[must_use]
pub fn dump_cst(tree: &SyntaxTree) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# cst nodes={} tokens={} trivia={} bytes={}",
        tree.node_count(),
        tree.token_count(),
        tree.trivia_count(),
        tree.source().len()
    );
    let mut stack = vec![Work::Node(tree.root(), 0)];
    while let Some(work) = stack.pop() {
        match work {
            Work::Node(node, indent) => {
                write_indent(indent, &mut out);
                let _ = writeln!(out, "{} {}", node.kind().as_str(), span_text(node.span()));
                let mut children = node.children();
                children.reverse();
                for child in children {
                    let child_indent = indent.saturating_add(1);
                    stack.push(match child {
                        SyntaxElement::Node(child) => Work::Node(child, child_indent),
                        SyntaxElement::Token(token) => Work::Token(token, child_indent),
                        SyntaxElement::Trivia(trivia) => Work::Trivia(trivia, child_indent),
                    });
                }
            }
            Work::Token(token, indent) => {
                write_indent(indent, &mut out);
                let _ = writeln!(out, "{}", token_text(&token));
            }
            Work::Trivia(trivia, indent) => {
                write_indent(indent, &mut out);
                let _ = writeln!(out, "{}", trivia_text(&trivia));
            }
        }
    }
    out
}

fn token_text(token: &SyntaxToken) -> String {
    if token.is_missing() {
        return format!(
            "Missing({})@{}..{}",
            token.kind().as_str(),
            token.span().start,
            token.span().end
        );
    }
    format!(
        "{}@{}..{} {:?}",
        token.kind().as_str(),
        token.span().start,
        token.span().end,
        token.text()
    )
}

fn trivia_text(trivia: &SyntaxTrivia) -> String {
    format!(
        "Trivia({})@{}..{} {:?}",
        trivia.kind().name(),
        trivia.span().start,
        trivia.span().end,
        trivia.text()
    )
}

/// Returns a compact dump of the typed AST views over the CST.
///
/// Only declaration, statement, and expression nodes are printed. The label is
/// the declaration name when there is one and a short source excerpt
/// otherwise.
#[must_use]
pub fn dump_typed_ast(tree: &SyntaxTree) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# ast bytes={}", tree.source().len());
    let mut stack = vec![(tree.root(), 0)];
    while let Some((node, indent)) = stack.pop() {
        if is_typed(&node) {
            write_indent(indent, &mut out);
            let _ = write!(
                out,
                "{}@{}..{}",
                node.kind().as_str(),
                node.span().start,
                node.span().end
            );
            if let Some(label) = label_for(&node) {
                let _ = write!(out, " {label}");
            }
            out.push('\n');
        }
        let mut children = node.child_nodes();
        children.reverse();
        for child in children {
            stack.push((child, indent.saturating_add(1)));
        }
    }
    out
}

fn is_typed(node: &SyntaxNode) -> bool {
    Item::cast(node).is_some()
        || TypeDecl::cast(node).is_some()
        || VarDeclLike::cast(node).is_some()
        || Stmt::cast(node).is_some()
        || Expr::cast(node).is_some()
        || matches!(
            node.kind(),
            SyntaxKind::VarSection
                | SyntaxKind::MethodDecl
                | SyntaxKind::MethodPrototype
                | SyntaxKind::StructElem
                | SyntaxKind::AccessDecl
                | SyntaxKind::UsingDirective
                | SyntaxKind::NamespaceDecl
                | SyntaxKind::ResourceDecl
                | SyntaxKind::TaskConfig
                | SyntaxKind::ProgramConfig
                | SyntaxKind::VarAccessSection
                | SyntaxKind::ConfigInit
                | SyntaxKind::ConfigInstInit
                | SyntaxKind::Param
                | SyntaxKind::Name
                | SyntaxKind::Path
                | SyntaxKind::TypeRef
        )
}

fn label_for(node: &SyntaxNode) -> Option<String> {
    if let Some(name) = node.child_nodes().iter().find_map(Name::cast) {
        return Some(name.text().trim().to_owned());
    }
    let text = node.tree().text(node.token_span());
    let mut label = String::new();
    let mut started = false;
    for character in text.chars() {
        if !started && character.is_whitespace() {
            continue;
        }
        started = true;
        if label.len() >= 77 {
            label.push_str("...");
            break;
        }
        if character == '\n' || character == '\r' {
            label.push(' ');
        } else {
            label.push(character);
        }
    }
    let label = label.trim_end().to_owned();
    if label.is_empty() { None } else { Some(label) }
}

fn write_indent(indent: usize, out: &mut String) {
    for _ in 0..indent.min(MAX_INDENT) {
        out.push_str("  ");
    }
}

fn span_text(span: stc_span::Span) -> String {
    format!("{}..{}", span.start, span.end)
}
