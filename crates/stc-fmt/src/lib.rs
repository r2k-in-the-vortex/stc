//! Fixed-style formatter over the lossless CST (M6 phase 7, D-159).
//!
//! The formatter re-emits the CST leaves in source order. Tokens it does not
//! understand keep their exact spelling; comments, blank lines, and existing
//! line breaks are preserved; horizontal spacing, indentation, keyword case,
//! and strict statement semicolons are normalized. Formatting an already
//! formatted file produces no change.
//!
//! Fixed style: uppercase keywords, original identifier spelling, two-space
//! indentation, one statement per line, one space around binary operators,
//! after commas, and around `:=`/`=>`, no space before a call's `(`, named
//! declarations written `name : TYPE`, every `END_*` on its own line, and a
//! strict `;` after every statement. There is no configuration, range
//! formatting, or line wrapping.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::BTreeSet;

use stc_diag::{Diagnostic, DiagnosticSink};
use stc_lexer::{TriviaKind, tokenize_bytes};
use stc_parser::{ParseOptions, parse};
use stc_span::{FileId, Span};
use stc_syntax::{
    SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, SyntaxTree, SyntaxTrivia, TokenId,
};

/// `STC-E-FMT-001`: syntax errors prevent formatting.
pub const SYNTAX_ERROR: &str = "STC-E-FMT-001";
/// `STC-E-FMT-002`: a source file cannot be read or written.
pub const IO_ERROR: &str = "STC-E-FMT-002";

/// The result of formatting one file.
#[derive(Clone, Debug)]
pub struct FormatOutcome {
    /// Decoded source text. Spans in the reported diagnostics refer to it.
    pub source: String,
    /// The formatted text, or `None` when syntax errors prevented formatting.
    pub formatted: Option<String>,
}

/// Formats one encoded source file.
///
/// The file is parsed with relaxed semicolons so that omitted terminators are
/// accepted; the output always uses strict semicolons. Lexer, parser, and
/// formatter diagnostics are appended to `diagnostics`. A file with syntax
/// errors is not formatted and reports [`SYNTAX_ERROR`].
pub fn format_bytes(file: FileId, bytes: &[u8], diagnostics: &mut DiagnosticSink) -> FormatOutcome {
    // Parse into a local sink so that errors from an earlier file do not block
    // formatting this one.
    let mut local = DiagnosticSink::unlimited();
    let lexed = tokenize_bytes(file, bytes, &mut local);
    let parsed = parse(
        file,
        lexed.text.as_str(),
        &lexed.stream,
        ParseOptions {
            relaxed_semicolons: true,
            ..ParseOptions::default()
        },
        &mut local,
    );
    let formatted = if local.has_errors() {
        let span = local
            .iter()
            .find(|diagnostic| diagnostic.is_error())
            .map_or_else(|| Span::empty(file, 0), |diagnostic| diagnostic.primary);
        local.push(Diagnostic::error(
            SYNTAX_ERROR,
            "syntax errors prevent formatting",
            span,
        ));
        None
    } else {
        Some(format_tree(parsed.tree()))
    };
    for diagnostic in local.into_diagnostics() {
        diagnostics.push(diagnostic);
    }
    FormatOutcome {
        source: lexed.text,
        formatted,
    }
}

/// Formats an already-parsed tree. The tree must not contain recovery nodes.
#[must_use]
pub fn format_tree(tree: &SyntaxTree) -> String {
    let mut formatter = Formatter::new(tree);
    formatter.emit_node(&tree.root(), &mut Vec::new());
    formatter.finish();
    formatter.out
}

/// One line-ending encoding. Existing line breaks keep their own encoding;
/// inserted line breaks use the file's dominant encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eol {
    Lf,
    Crlf,
    Cr,
}

impl Eol {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
            Self::Cr => "\r",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LastItem {
    Token {
        kind: SyntaxKind,
        parent: SyntaxKind,
    },
    Comment,
}

struct Formatter {
    out: String,
    dominant: Eol,
    /// Line breaks seen since the last emitted leaf, in source encoding.
    pending_breaks: Vec<Eol>,
    /// A style rule requires the next token to start on a new line.
    force_break: bool,
    /// The current output line already contains text.
    line_has_content: bool,
    /// Some token or comment has been written (a leading BOM does not count).
    has_token: bool,
    last: Option<LastItem>,
    /// Last tokens of block statements that omitted their `;` (relaxed input).
    semicolons_after: BTreeSet<TokenId>,
}

impl Formatter {
    fn new(tree: &SyntaxTree) -> Self {
        Self {
            out: String::with_capacity(tree.source().len() + 64),
            dominant: dominant_eol(tree.source()),
            pending_breaks: Vec::new(),
            force_break: false,
            line_has_content: false,
            has_token: false,
            last: None,
            semicolons_after: BTreeSet::new(),
        }
    }

    fn finish(&mut self) {
        if !self.pending_breaks.is_empty() {
            self.flush_breaks();
        } else if self.force_break && self.has_token && !self.out.is_empty() {
            self.push_eol(self.dominant);
        }
    }

    fn emit_node(&mut self, node: &SyntaxNode, stack: &mut Vec<SyntaxKind>) {
        stack.push(node.kind());
        if is_statement(node.kind()) {
            // Under `--relaxed`/relaxed manifest syntax the parser omits the
            // terminator after a block statement. Register the last real token
            // so the formatter can append the strict `;` before trailing
            // trivia such as a line comment.
            let tokens = node.child_tokens();
            if let Some(last) = tokens.last()
                && last.kind() != SyntaxKind::PunctSemicolon
            {
                self.semicolons_after.insert(last.id());
            }
        }
        for element in node.children() {
            self.emit_element(&element, stack);
        }
        stack.pop();
    }

    fn emit_element(&mut self, element: &SyntaxElement, stack: &mut Vec<SyntaxKind>) {
        match element {
            SyntaxElement::Node(node) => {
                if is_statement(node.kind())
                    || is_declaration_line(node.kind())
                    || node.kind() == SyntaxKind::CaseSelection
                {
                    self.force_break = true;
                }
                self.emit_node(node, stack);
            }
            SyntaxElement::Token(token) => {
                let parent = stack.last().copied().unwrap_or(SyntaxKind::SourceFile);
                self.emit_token(token, parent, stack);
            }
            SyntaxElement::Trivia(trivia) => self.emit_trivia(trivia, stack),
        }
    }

    fn emit_token(&mut self, token: &SyntaxToken, parent: SyntaxKind, stack: &[SyntaxKind]) {
        let kind = token.kind();
        if token.is_missing() || kind == SyntaxKind::Eof {
            return;
        }
        if breaks_before(kind) {
            self.force_break = true;
        }
        let depth = indent_level(stack);
        self.begin_content(depth, kind, parent);
        let text = if kind.is_keyword() {
            Cow::Owned(token.text().to_ascii_uppercase())
        } else {
            Cow::Borrowed(token.text())
        };
        self.push_text(&text);
        if self.semicolons_after.remove(&token.id()) {
            self.push_text(";");
            self.force_break = true;
            self.last = Some(LastItem::Token {
                kind: SyntaxKind::PunctSemicolon,
                parent: SyntaxKind::StmtList,
            });
        } else {
            self.last = Some(LastItem::Token { kind, parent });
        }
        if kind == SyntaxKind::PunctSemicolon || breaks_after(kind) {
            self.force_break = true;
        }
    }

    fn emit_trivia(&mut self, trivia: &SyntaxTrivia, stack: &[SyntaxKind]) {
        match trivia.kind() {
            TriviaKind::Bom => {
                // STC-LEX-001 keeps an initial BOM as trivia so the CST stays
                // lossless; the formatter writes it back unchanged.
                self.out.push('\u{feff}');
                self.line_has_content = true;
            }
            TriviaKind::Whitespace => self.scan_whitespace(trivia.text()),
            TriviaKind::LineComment | TriviaKind::DocComment | TriviaKind::BlockComment => {
                let depth = indent_level(stack);
                if !self.pending_breaks.is_empty() {
                    self.flush_breaks();
                    self.indent(depth);
                } else if !self.line_has_content {
                    self.indent(depth);
                } else {
                    self.out.push(' ');
                }
                self.push_text(trivia.text());
                self.last = Some(LastItem::Comment);
            }
        }
    }

    fn begin_content(&mut self, depth: usize, kind: SyntaxKind, parent: SyntaxKind) {
        if !self.pending_breaks.is_empty() {
            self.flush_breaks();
            self.indent(depth);
            self.force_break = false;
        } else if self.force_break {
            if self.has_token {
                self.push_eol(self.dominant);
            }
            self.force_break = false;
            self.indent(depth);
        } else if !self.line_has_content {
            self.indent(depth);
        } else if self.needs_space(kind, parent) {
            self.out.push(' ');
        }
    }

    fn needs_space(&self, kind: SyntaxKind, parent: SyntaxKind) -> bool {
        // `STRING[32]` is a bound, not an index or an `ARRAY [..]` type; keep
        // it tight after the string type name.
        if kind == SyntaxKind::PunctLBracket
            && matches!(
                self.last,
                Some(LastItem::Token {
                    kind: SyntaxKind::KwString | SyntaxKind::KwWstring | SyntaxKind::KwUstring,
                    ..
                })
            )
        {
            return false;
        }
        if no_space_before(kind, parent) {
            return false;
        }
        match self.last {
            Some(LastItem::Token {
                kind: previous,
                parent: previous_parent,
            }) => !no_space_after(previous, previous_parent),
            Some(LastItem::Comment) => true,
            None => false,
        }
    }

    fn scan_whitespace(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                    self.pending_breaks.push(Eol::Crlf);
                    index += 2;
                }
                b'\r' => {
                    self.pending_breaks.push(Eol::Cr);
                    index += 1;
                }
                b'\n' => {
                    self.pending_breaks.push(Eol::Lf);
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn flush_breaks(&mut self) {
        for eol in std::mem::take(&mut self.pending_breaks) {
            self.push_eol(eol);
        }
    }

    fn push_eol(&mut self, eol: Eol) {
        self.out.push_str(eol.as_str());
        self.line_has_content = false;
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("  ");
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.out.push_str(text);
        self.line_has_content = !(text.ends_with('\n') || text.ends_with('\r'));
        self.has_token = true;
    }
}

/// Returns the two-space indentation depth of a leaf at `stack`.
///
/// Only declaration and statement structure contributes; expressions and
/// paths do not. POU, class, and function-block members sit at the construct's
/// level (matching the `stc new` scaffold), while var-section declarations,
/// type declarations, struct fields, interface members, namespace items,
/// resource programs, and statements are one level deeper than their owner.
fn indent_level(stack: &[SyntaxKind]) -> usize {
    let mut depth = 0;
    for (index, kind) in stack.iter().enumerate() {
        let rest = &stack[index + 1..];
        match kind {
            SyntaxKind::StmtList => depth += 1,
            SyntaxKind::VarSection
                if rest.iter().any(|kind| {
                    matches!(
                        kind,
                        SyntaxKind::VarDecl | SyntaxKind::EdgeDecl | SyntaxKind::AccessDecl
                    )
                }) =>
            {
                depth += 1;
            }
            SyntaxKind::VarAccessSection if rest.contains(&SyntaxKind::AccessDecl) => depth += 1,
            SyntaxKind::ConfigInit if rest.contains(&SyntaxKind::ConfigInstInit) => depth += 1,
            SyntaxKind::DataTypeDecl if rest.iter().any(|kind| is_type_decl_kind(*kind)) => {
                depth += 1;
            }
            SyntaxKind::EnumTypeDecl if rest.contains(&SyntaxKind::EnumValue) => depth += 1,
            SyntaxKind::StructTypeDecl if rest.contains(&SyntaxKind::StructElem) => depth += 1,
            SyntaxKind::TypeRef
                if !stack[..index].contains(&SyntaxKind::StructTypeDecl)
                    && rest.contains(&SyntaxKind::StructElem) =>
            {
                depth += 1;
            }
            SyntaxKind::InterfaceDecl
                if rest.iter().any(|kind| {
                    matches!(kind, SyntaxKind::MethodDecl | SyntaxKind::MethodPrototype)
                }) =>
            {
                depth += 1;
            }
            SyntaxKind::NamespaceDecl if rest.iter().any(|kind| is_item_kind(*kind)) => {
                depth += 1;
            }
            SyntaxKind::ResourceDecl
                if rest.iter().any(|kind| {
                    matches!(kind, SyntaxKind::TaskConfig | SyntaxKind::ProgramConfig)
                }) =>
            {
                depth += 1;
            }
            _ => {}
        }
    }
    depth
}

fn is_statement(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::AssignStmt
            | SyntaxKind::RefAssignStmt
            | SyntaxKind::AssignmentAttemptStmt
            | SyntaxKind::CallStmt
            | SyntaxKind::ReturnStmt
            | SyntaxKind::IfStmt
            | SyntaxKind::CaseStmt
            | SyntaxKind::ForStmt
            | SyntaxKind::WhileStmt
            | SyntaxKind::RepeatStmt
            | SyntaxKind::ExitStmt
            | SyntaxKind::ContinueStmt
            | SyntaxKind::EmptyStmt
    )
}

/// Declaration nodes whose first token starts a new line, so declarations are
/// never left on a section-header or previous-declaration line.
fn is_declaration_line(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::UsingDirective
            | SyntaxKind::VarDecl
            | SyntaxKind::EdgeDecl
            | SyntaxKind::AccessDecl
            | SyntaxKind::StructElem
            | SyntaxKind::ConfigInstInit
            | SyntaxKind::TaskConfig
            | SyntaxKind::ProgramConfig
            | SyntaxKind::MethodDecl
            | SyntaxKind::MethodPrototype
    ) || is_type_decl_kind(kind)
}

fn is_type_decl_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::SimpleTypeDecl
            | SyntaxKind::SubrangeTypeDecl
            | SyntaxKind::EnumTypeDecl
            | SyntaxKind::NamedValueTypeDecl
            | SyntaxKind::ArrayTypeDecl
            | SyntaxKind::StructTypeDecl
            | SyntaxKind::StringTypeDecl
            | SyntaxKind::RefTypeDecl
    )
}

fn is_item_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::UsingDirective
            | SyntaxKind::NamespaceDecl
            | SyntaxKind::DataTypeDecl
            | SyntaxKind::FunctionDecl
            | SyntaxKind::FunctionBlockDecl
            | SyntaxKind::ProgramDecl
            | SyntaxKind::ClassDecl
            | SyntaxKind::InterfaceDecl
            | SyntaxKind::ConfigDecl
            | SyntaxKind::VarSection
    )
}

fn is_end_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwEndVar
            | SyntaxKind::KwEndType
            | SyntaxKind::KwEndStruct
            | SyntaxKind::KwEndIf
            | SyntaxKind::KwEndCase
            | SyntaxKind::KwEndFor
            | SyntaxKind::KwEndWhile
            | SyntaxKind::KwEndRepeat
            | SyntaxKind::KwEndFunction
            | SyntaxKind::KwEndFunctionBlock
            | SyntaxKind::KwEndProgram
            | SyntaxKind::KwEndClass
            | SyntaxKind::KwEndInterface
            | SyntaxKind::KwEndMethod
            | SyntaxKind::KwEndNamespace
            | SyntaxKind::KwEndConfiguration
            | SyntaxKind::KwEndResource
            | SyntaxKind::KwEndAction
            | SyntaxKind::KwEndStep
            | SyntaxKind::KwEndTransition
    )
}

fn is_var_section_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwVar
            | SyntaxKind::KwVarInput
            | SyntaxKind::KwVarOutput
            | SyntaxKind::KwVarInOut
            | SyntaxKind::KwVarTemp
            | SyntaxKind::KwVarExternal
            | SyntaxKind::KwVarGlobal
            | SyntaxKind::KwVarAccess
            | SyntaxKind::KwVarConfig
    )
}

/// Tokens that always start a new output line.
fn breaks_before(kind: SyntaxKind) -> bool {
    is_end_keyword(kind)
        || is_var_section_keyword(kind)
        || matches!(
            kind,
            SyntaxKind::KwElse | SyntaxKind::KwElsif | SyntaxKind::KwUntil
        )
}

/// Tokens that always end an output line. Statement terminators (`END_IF` and
/// friends) are followed by a `;` or a formatter-inserted one, and `END_STRUCT`
/// is followed by the declaration's `;`.
fn breaks_after(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwEndVar
            | SyntaxKind::KwEndType
            | SyntaxKind::KwEndFunction
            | SyntaxKind::KwEndFunctionBlock
            | SyntaxKind::KwEndProgram
            | SyntaxKind::KwEndClass
            | SyntaxKind::KwEndInterface
            | SyntaxKind::KwEndMethod
            | SyntaxKind::KwEndNamespace
            | SyntaxKind::KwEndConfiguration
            | SyntaxKind::KwEndResource
            | SyntaxKind::KwEndAction
            | SyntaxKind::KwEndStep
            | SyntaxKind::KwEndTransition
    )
}

/// Tokens that never have a space before them.
fn no_space_before(kind: SyntaxKind, parent: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PunctRParen
            | SyntaxKind::PunctRBracket
            | SyntaxKind::PunctComma
            | SyntaxKind::PunctSemicolon
            | SyntaxKind::PunctDot
            | SyntaxKind::PunctRange
            | SyntaxKind::PunctCaret
    ) || (kind == SyntaxKind::PunctColon && parent == SyntaxKind::CaseSelection)
        || (kind == SyntaxKind::PunctLParen
            && matches!(
                parent,
                SyntaxKind::ParamList
                    | SyntaxKind::RefValueExpr
                    | SyntaxKind::AddressOfExpr
                    | SyntaxKind::SizeOfExpr
                    | SyntaxKind::PointerConversionExpr
                    | SyntaxKind::TaskConfig
                    | SyntaxKind::ProgramConfig
            ))
        || (kind == SyntaxKind::PunctLBracket
            && matches!(parent, SyntaxKind::IndexExpr | SyntaxKind::ArrayInit))
}

/// Tokens that never have a space after them.
///
/// A postfix dereference is not listed: `p^` glues to a following `[`, `.`,
/// `^`, or `)` through those tokens' own before-rules, while `p^ := ...` and
/// `IF p^ THEN` still need a separator.
fn no_space_after(kind: SyntaxKind, parent: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::PunctLParen
            | SyntaxKind::PunctLBracket
            | SyntaxKind::PunctDot
            | SyntaxKind::PunctRange
    ) || (matches!(kind, SyntaxKind::PunctPlus | SyntaxKind::PunctMinus)
        && parent == SyntaxKind::UnaryExpr)
}

/// Picks the line ending used for inserted line breaks: the file's dominant
/// encoding, with CRLF winning a tie over LF. New files without line breaks
/// use LF.
fn dominant_eol(source: &str) -> Eol {
    let bytes = source.as_bytes();
    let mut lf = 0_usize;
    let mut crlf = 0_usize;
    let mut cr = 0_usize;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' => {
                cr += 1;
                index += 1;
            }
            b'\n' => {
                lf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if crlf > 0 && crlf >= lf && crlf >= cr {
        Eol::Crlf
    } else if lf >= cr {
        Eol::Lf
    } else {
        Eol::Cr
    }
}
