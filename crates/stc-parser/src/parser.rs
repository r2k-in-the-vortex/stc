use std::sync::Arc;

use stc_diag::{Diagnostic, DiagnosticSink};
use stc_lexer::{Keyword, Punct, Token, TokenKind, TokenStream};
use stc_span::{FileId, Span};
use stc_syntax::pragma::{duplicate_io, parse_pragma};
use stc_syntax::{
    Checkpoint, NodeId, PragmaAnnotation, SyntaxKind, SyntaxNode, SyntaxTree, TreeBuilder,
};

use crate::codes;

/// Parser options.
#[derive(Clone, Copy, Debug)]
pub struct ParseOptions {
    /// Accept the documented omitted-semicolon-after-block superset (A-015).
    pub relaxed_semicolons: bool,
    /// Maximum nested expression/statement/type depth.
    pub max_depth: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            relaxed_semicolons: false,
            max_depth: 256,
        }
    }
}

/// A parsed file: the lossless CST plus pragma annotations.
#[derive(Clone, Debug)]
pub struct ParsedFile {
    tree: SyntaxTree,
}

impl ParsedFile {
    /// Returns the CST.
    #[must_use]
    pub fn tree(&self) -> &SyntaxTree {
        &self.tree
    }

    /// Returns the owned source text.
    #[must_use]
    pub fn source(&self) -> &str {
        self.tree.source()
    }

    /// Returns the root node.
    #[must_use]
    pub fn root(&self) -> SyntaxNode {
        self.tree.root()
    }

    /// Consumes the file and returns the CST.
    #[must_use]
    pub fn into_tree(self) -> SyntaxTree {
        self.tree
    }
}

/// Parses one token stream into a lossless CST.
///
/// Lexical diagnostics already pushed to `diagnostics` are preserved; parser
/// diagnostics are appended to the same sink.
pub fn parse<S: Into<Arc<str>>>(
    file: FileId,
    source: S,
    stream: &TokenStream,
    options: ParseOptions,
    diagnostics: &mut DiagnosticSink,
) -> ParsedFile {
    let source = source.into();
    let mut parser = Parser {
        builder: TreeBuilder::new(file, Arc::clone(&source)),
        source,
        stream,
        tokens: &stream.tokens,
        pos: 0,
        diagnostics,
        options,
        depth: 0,
    };
    parser.parse_source_file();
    ParsedFile {
        tree: parser.builder.finish(),
    }
}

pub(crate) struct Parser<'a> {
    pub(crate) source: Arc<str>,
    pub(crate) stream: &'a TokenStream,
    pub(crate) tokens: &'a [Token],
    pub(crate) pos: usize,
    pub(crate) builder: TreeBuilder,
    pub(crate) diagnostics: &'a mut DiagnosticSink,
    pub(crate) options: ParseOptions,
    pub(crate) depth: usize,
}

impl Parser<'_> {
    // ---- token cursor -----------------------------------------------------

    pub(crate) fn current(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("token stream is empty"))
    }

    pub(crate) fn nth(&self, offset: usize) -> &Token {
        let index = (self.pos + offset).min(self.tokens.len().saturating_sub(1));
        &self.tokens[index]
    }

    pub(crate) fn kind_at(&self, offset: usize) -> SyntaxKind {
        SyntaxKind::from_token(&self.nth(offset).kind)
    }

    pub(crate) fn at_kind(&self, kind: SyntaxKind) -> bool {
        self.kind_at(0) == kind
    }

    pub(crate) fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current().kind, TokenKind::Keyword(found) if found == keyword)
    }

    pub(crate) fn at_punct(&self, punct: Punct) -> bool {
        matches!(self.current().kind, TokenKind::Punct(found) if found == punct)
    }

    pub(crate) fn at_ident(&self) -> bool {
        matches!(self.current().kind, TokenKind::Ident(_))
    }

    pub(crate) fn at_ident_named(&self, name: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Ident(key) if key.eq_ignore_ascii_case(name))
    }

    pub(crate) fn at_eof(&self) -> bool {
        self.current().kind.is_eof()
    }

    pub(crate) fn at_pragma(&self) -> bool {
        matches!(self.current().kind, TokenKind::Pragma { .. })
    }

    pub(crate) fn current_span(&self) -> Span {
        self.current().span
    }

    pub(crate) fn current_text(&self) -> &str {
        self.current().text(&self.source)
    }

    pub(crate) fn describe_current(&self) -> String {
        describe_token(self.current(), &self.source)
    }

    pub(crate) fn bump(&mut self) -> Option<stc_syntax::TokenId> {
        if self.at_eof() {
            return None;
        }
        let token = &self.tokens[self.pos];
        let id = self.builder.emit_token(self.stream, token);
        self.pos += 1;
        Some(id)
    }

    pub(crate) fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(crate) fn eat_punct(&mut self, punct: Punct) -> bool {
        if self.at_punct(punct) {
            self.bump();
            true
        } else {
            false
        }
    }

    // ---- nodes ------------------------------------------------------------

    pub(crate) fn start_node(&mut self, kind: SyntaxKind) -> NodeId {
        self.builder.start_node(kind)
    }

    pub(crate) fn finish_node(&mut self) {
        self.builder.finish_node();
    }

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        self.builder.checkpoint()
    }

    pub(crate) fn wrap_children(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        self.builder.wrap_children(checkpoint, kind);
    }

    pub(crate) fn set_node_kind(&mut self, id: NodeId, kind: SyntaxKind) {
        self.builder.set_node_kind(id, kind);
    }

    pub(crate) fn last_child_node_kind(&self, parent: NodeId) -> Option<SyntaxKind> {
        self.builder.last_child_node_kind(parent)
    }

    pub(crate) fn last_child_node_kind_in_current(&self) -> Option<SyntaxKind> {
        self.builder
            .current_parent()
            .and_then(|parent| self.builder.last_child_node_kind(parent))
    }

    // ---- diagnostics and recovery ----------------------------------------

    pub(crate) fn error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics.error(code, message, span);
    }

    pub(crate) fn warning(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics.warning(code, message, span);
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn unexpected(&mut self, message: impl Into<String>) {
        let span = self.current_span();
        self.error(codes::UNEXPECTED_TOKEN, message, span);
    }

    pub(crate) fn expected(&mut self, what: &str) {
        let span = self.current_span();
        let found = self.describe_current();
        self.error(
            codes::EXPECTED_TOKEN,
            format!("expected {what}, found {found}"),
            span,
        );
    }

    pub(crate) fn insert_missing(&mut self, kind: SyntaxKind) -> stc_syntax::TokenId {
        debug_assert!(kind.is_token());
        self.builder.missing_token(kind)
    }

    pub(crate) fn expect_keyword(&mut self, keyword: Keyword) -> bool {
        if self.eat_keyword(keyword) {
            true
        } else {
            self.expected(&format!("`{}`", keyword.as_str()));
            self.insert_missing(SyntaxKind::from_keyword(keyword));
            false
        }
    }

    pub(crate) fn expect_punct(&mut self, punct: Punct) -> bool {
        if self.eat_punct(punct) {
            true
        } else {
            self.expected(&format!("`{}`", punct.as_str()));
            self.insert_missing(SyntaxKind::from_punct(punct));
            false
        }
    }

    pub(crate) fn expect_semicolon(&mut self, context: &str) -> bool {
        if self.eat_punct(Punct::Semicolon) {
            true
        } else {
            let span = self.current_span();
            self.error(
                codes::MISSING_SEMICOLON,
                format!("expected `;` after {context}"),
                span,
            );
            self.insert_missing(SyntaxKind::PunctSemicolon);
            false
        }
    }

    /// Consumes the expected block terminator. A different `END_*` is reported
    /// as a mismatch and left for the enclosing construct; EOF or another token
    /// is reported as a missing terminator.
    pub(crate) fn expect_end(&mut self, expected: Keyword, opener: Span) {
        if self.eat_keyword(expected) {
            return;
        }
        let current = self.current_span();
        if self.at_any_end() {
            let diagnostic = Diagnostic::error(
                codes::MISMATCHED_END,
                format!(
                    "mismatched terminator: expected `{}`, found `{}`",
                    expected.as_str(),
                    self.current_text()
                ),
                current,
            )
            .with_label(opener, format!("`{}` starts here", expected.as_str()));
            self.push(diagnostic);
        } else {
            let diagnostic = Diagnostic::error(
                codes::MISSING_END,
                format!("missing `{}` terminator", expected.as_str()),
                current,
            )
            .with_label(opener, format!("`{}` starts here", expected.as_str()));
            self.push(diagnostic);
        }
        self.insert_missing(SyntaxKind::from_keyword(expected));
    }

    // ---- block and sync sets ---------------------------------------------

    pub(crate) fn at_any_end(&self) -> bool {
        is_end_keyword(self.kind_at(0))
    }

    pub(crate) fn at_block_delimiter(&self) -> bool {
        is_block_delimiter(self.kind_at(0))
    }

    pub(crate) fn at_declaration_sync(&self) -> bool {
        self.at_punct(Punct::Semicolon) || self.at_eof() || self.at_any_end()
    }

    pub(crate) fn at_expression_sync(&self) -> bool {
        matches!(
            self.kind_at(0),
            SyntaxKind::PunctRParen
                | SyntaxKind::PunctRBracket
                | SyntaxKind::PunctComma
                | SyntaxKind::PunctSemicolon
                | SyntaxKind::Eof
        ) || self.at_block_delimiter()
    }

    /// Consumes tokens into an `Error` node until a declaration sync point. The
    /// sync token is left for the caller unless it is a semicolon and
    /// `consume_semicolon` is set.
    pub(crate) fn recover_declaration(&mut self, consume_semicolon: bool) {
        self.start_node(SyntaxKind::Error);
        let start = self.pos;
        while !self.at_eof() && !self.at_declaration_sync() {
            self.bump();
        }
        if consume_semicolon && self.at_punct(Punct::Semicolon) {
            self.bump();
        }
        if self.pos == start && !self.at_eof() {
            // The malformed input is itself at a sync point (for example a
            // stray END_PROGRAM). Consume it so recovery always progresses.
            self.bump();
        }
        self.finish_node();
    }

    /// Consumes a malformed statement, including its semicolon.
    pub(crate) fn recover_statement(&mut self) {
        self.start_node(SyntaxKind::Error);
        let start = self.pos;
        while !self.at_eof() && !self.at_block_delimiter() && !self.at_punct(Punct::Semicolon) {
            self.bump();
        }
        if self.at_punct(Punct::Semicolon) {
            self.bump();
        }
        if self.pos == start && !self.at_eof() && !self.at_block_delimiter() {
            // Guarantee progress even if the sync set was already satisfied.
            self.bump();
        }
        self.finish_node();
    }

    /// Consumes malformed expression tokens into an `Error` node without
    /// consuming the delimiter that ends the surrounding construct.
    pub(crate) fn recover_expression(&mut self) {
        self.start_node(SyntaxKind::Error);
        while !self.at_eof() && !self.at_expression_sync() {
            self.bump();
        }
        self.finish_node();
    }

    /// Consumes tokens into an `Error` node until `stop` or EOF.
    pub(crate) fn recover_until(&mut self, stop: impl Fn(SyntaxKind) -> bool) {
        self.start_node(SyntaxKind::Error);
        while !self.at_eof() && !stop(self.kind_at(0)) {
            self.bump();
        }
        self.finish_node();
    }

    // ---- depth ------------------------------------------------------------

    pub(crate) fn enter_depth(&mut self) -> bool {
        if self.depth >= self.options.max_depth {
            let span = self.current_span();
            self.error(
                codes::DEPTH_LIMIT,
                format!(
                    "nesting exceeds the parser limit of {} levels",
                    self.options.max_depth
                ),
                span,
            );
            false
        } else {
            self.depth += 1;
            true
        }
    }

    pub(crate) fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ---- names and paths --------------------------------------------------

    /// Parses one identifier wrapped in a `Name` node. Returns `None` when the
    /// identifier was missing.
    pub(crate) fn parse_name(&mut self) -> Option<stc_syntax::TokenId> {
        self.start_node(SyntaxKind::Name);
        let id = if self.at_ident() {
            self.bump()
        } else {
            self.expected("identifier");
            Some(self.insert_missing(SyntaxKind::Ident))
        };
        self.finish_node();
        id
    }

    /// Parses `IDENT ('.' IDENT)*` into a `Path` node. Returns `false` when the
    /// first identifier was missing.
    pub(crate) fn parse_path(&mut self) -> bool {
        self.start_node(SyntaxKind::Path);
        let present = self.at_ident();
        self.parse_name();
        while self.at_punct(Punct::Dot) {
            self.bump();
            if self.at_ident() {
                self.parse_name();
            } else {
                self.expected("identifier");
                self.insert_missing(SyntaxKind::Ident);
                break;
            }
        }
        self.finish_node();
        present
    }

    // ---- pragmas ----------------------------------------------------------

    pub(crate) fn pragma_count(&self) -> usize {
        let mut count = 0;
        while matches!(self.nth(count).kind, TokenKind::Pragma { .. }) {
            count += 1;
        }
        count
    }

    /// Parses `count` consecutive pragma tokens without consuming them.
    pub(crate) fn collect_pragmas(
        &mut self,
        count: usize,
        allowed: bool,
    ) -> Vec<Option<PragmaAnnotation>> {
        let mut annotations = Vec::with_capacity(count);
        for offset in 0..count {
            let token = &self.tokens[self.pos + offset];
            let token_span = token.span;
            if !allowed {
                self.error(
                    codes::PRAGMA_PLACEMENT,
                    "a pragma is not allowed at this position",
                    token_span,
                );
            }
            let terminated = matches!(token.kind, TokenKind::Pragma { terminated: true });
            let text = token.text(&self.source);
            let annotation = parse_pragma(text, token_span, terminated, self.diagnostics);
            annotations.push(annotation);
        }
        annotations
    }

    /// Emits collected pragmas as `Pragma` nodes in the current node.
    pub(crate) fn emit_pragmas(&mut self, annotations: &[Option<PragmaAnnotation>]) {
        for annotation in annotations {
            self.start_node(SyntaxKind::Pragma);
            if let Some(token) = self.bump()
                && let Some(annotation) = annotation
            {
                self.builder.set_pragma(token, annotation.clone());
            }
            self.finish_node();
        }
    }

    /// Checks duplicate `stc:io` pragmas and `stc:io` without an `AT` binding.
    pub(crate) fn check_io_pragmas(
        &mut self,
        annotations: &[Option<PragmaAnnotation>],
        has_at: bool,
    ) {
        let mut first_io: Option<Span> = None;
        for annotation in annotations.iter().flatten() {
            if annotation.name != "io" {
                continue;
            }
            if let Some(first) = first_io {
                duplicate_io(self.diagnostics, annotation.span, first);
            } else {
                first_io = Some(annotation.span);
            }
        }
        if let Some(span) = first_io
            && !has_at
        {
            self.warning(
                stc_syntax::codes::IO_WITHOUT_AT,
                "`stc:io` has no effect without an `AT` binding",
                span,
            );
        }
    }

    // ---- file completion --------------------------------------------------

    /// Emits any unconsumed tokens (including `Eof` and its leading trivia).
    pub(crate) fn emit_rest_of_file(&mut self) {
        while self.pos < self.tokens.len() {
            let token = &self.tokens[self.pos];
            self.builder.emit_token(self.stream, token);
            if token.kind.is_eof() {
                break;
            }
            self.pos += 1;
        }
    }
}

pub(crate) fn is_end_keyword(kind: SyntaxKind) -> bool {
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

pub(crate) fn is_block_delimiter(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwElse | SyntaxKind::KwElsif | SyntaxKind::KwUntil
    ) || is_end_keyword(kind)
}

pub(crate) fn is_var_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwVar
            | SyntaxKind::KwVarInput
            | SyntaxKind::KwVarOutput
            | SyntaxKind::KwVarInOut
            | SyntaxKind::KwVarTemp
            | SyntaxKind::KwVarExternal
            | SyntaxKind::KwVarGlobal
    )
}

pub(crate) fn is_access_spec(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwPublic
            | SyntaxKind::KwProtected
            | SyntaxKind::KwPrivate
            | SyntaxKind::KwInternal
    )
}

pub(crate) fn is_sfc_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwAction
            | SyntaxKind::KwEndAction
            | SyntaxKind::KwStep
            | SyntaxKind::KwEndStep
            | SyntaxKind::KwInitialStep
            | SyntaxKind::KwTransition
            | SyntaxKind::KwEndTransition
    )
}

pub(crate) fn is_type_name_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwBool
            | SyntaxKind::KwSint
            | SyntaxKind::KwInt
            | SyntaxKind::KwDint
            | SyntaxKind::KwLint
            | SyntaxKind::KwUsint
            | SyntaxKind::KwUint
            | SyntaxKind::KwUdint
            | SyntaxKind::KwUlint
            | SyntaxKind::KwByte
            | SyntaxKind::KwWord
            | SyntaxKind::KwDword
            | SyntaxKind::KwLword
            | SyntaxKind::KwReal
            | SyntaxKind::KwLreal
            | SyntaxKind::KwTime
            | SyntaxKind::KwLtime
            | SyntaxKind::KwDate
            | SyntaxKind::KwLdate
            | SyntaxKind::KwLdateAndTime
            | SyntaxKind::KwTimeOfDay
            | SyntaxKind::KwTod
            | SyntaxKind::KwLtod
            | SyntaxKind::KwLtimeOfDay
            | SyntaxKind::KwDateAndTime
            | SyntaxKind::KwDt
            | SyntaxKind::KwLdt
            | SyntaxKind::KwString
            | SyntaxKind::KwWstring
            | SyntaxKind::KwUstring
            | SyntaxKind::KwChar
            | SyntaxKind::KwWchar
            | SyntaxKind::KwUchar
            | SyntaxKind::KwAny
            | SyntaxKind::KwAnyDerived
            | SyntaxKind::KwAnyElementary
            | SyntaxKind::KwAnyMagnitude
            | SyntaxKind::KwAnyNum
            | SyntaxKind::KwAnyReal
            | SyntaxKind::KwAnyInt
            | SyntaxKind::KwAnyUnsigned
            | SyntaxKind::KwAnySigned
            | SyntaxKind::KwAnyDuration
            | SyntaxKind::KwAnyBit
            | SyntaxKind::KwAnyChars
            | SyntaxKind::KwAnyString
            | SyntaxKind::KwAnyChar
            | SyntaxKind::KwAnyDate
    )
}

pub(crate) fn is_int_type_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwSint
            | SyntaxKind::KwInt
            | SyntaxKind::KwDint
            | SyntaxKind::KwLint
            | SyntaxKind::KwUsint
            | SyntaxKind::KwUint
            | SyntaxKind::KwUdint
            | SyntaxKind::KwUlint
    )
}

pub(crate) fn is_string_type_keyword(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwString
            | SyntaxKind::KwWstring
            | SyntaxKind::KwUstring
            | SyntaxKind::KwChar
            | SyntaxKind::KwWchar
            | SyntaxKind::KwUchar
    )
}

fn describe_token(token: &Token, source: &str) -> String {
    match &token.kind {
        TokenKind::Eof => "end of file".into(),
        TokenKind::Ident(_) => format!("identifier `{}`", token.text(source)),
        TokenKind::Keyword(keyword) => format!("`{}`", keyword.as_str()),
        TokenKind::Punct(punct) => format!("`{}`", punct.as_str()),
        TokenKind::Pragma { .. } => "pragma".into(),
        other => other.name().to_owned(),
    }
}
