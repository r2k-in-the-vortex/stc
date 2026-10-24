//! Statements: Grammar §8 and the strict/relaxed semicolon rules (STC-GRAM-016).

use stc_lexer::{Keyword, Punct};
use stc_syntax::{NodeId, SyntaxKind};

use crate::codes;
use crate::parser::{Parser, is_block_delimiter, is_sfc_keyword};

const IL_MNEMONICS: &[&str] = &[
    "LD", "LDN", "LDI", "ST", "STN", "S", "R", "CAL", "JMP", "RET", "ADD", "SUB", "MUL", "DIV",
    "MOD", "AND", "OR", "XOR", "NOT", "GT", "GE", "EQ", "NE", "LE", "LT", "MOVE",
];

impl Parser<'_> {
    /// Parses a statement list until a block delimiter (or a case label).
    ///
    /// When `expected_end` names the enclosing POU terminator, a stray
    /// statement terminator (for example a leftover `END_WHILE` from an inner
    /// mismatch) is consumed and parsing continues. Other `END_*` tokens are
    /// left for the enclosing construct, which reports the mismatch.
    pub(crate) fn parse_stmt_list(
        &mut self,
        stop_at_case_labels: bool,
        expected_end: Option<Keyword>,
    ) {
        self.start_node(SyntaxKind::StmtList);
        loop {
            if self.at_eof() {
                break;
            }
            if let Some(expected) = expected_end {
                if self.at_keyword(expected) {
                    break;
                }
                if self.at_block_delimiter() {
                    if is_statement_terminator(self.kind_at(0)) {
                        self.consume_stray_terminator();
                        continue;
                    }
                    break;
                }
            } else if self.at_block_delimiter() {
                break;
            }
            if stop_at_case_labels && self.at_case_label_start() {
                break;
            }
            let before = self.pos;
            if self.at_pragma() {
                let count = self.pragma_count();
                let next = self.kind_at(count);
                if is_block_delimiter(next) || next == SyntaxKind::Eof {
                    // A pragma at the end of a block attaches to the
                    // terminator and has no meaning (Grammar §11.10).
                    let annotations = self.collect_pragmas(count, false);
                    self.emit_pragmas(&annotations);
                } else {
                    self.parse_statement_with_pragmas(count);
                }
            } else if self.at_punct(Punct::Semicolon) {
                self.parse_empty_statement();
            } else {
                self.parse_statement_with_pragmas(0);
            }
            if self.pos == before && !self.at_eof() && !self.at_block_delimiter() {
                self.recover_statement();
                if self.pos == before {
                    self.start_node(SyntaxKind::Error);
                    self.bump();
                    self.finish_node();
                }
            }
        }
        self.finish_node();
    }

    fn consume_stray_terminator(&mut self) {
        let span = self.current_span();
        let already_reported = self.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == codes::MISMATCHED_END && diagnostic.primary == span
        });
        self.start_node(SyntaxKind::Error);
        if !already_reported {
            let message = format!("unexpected `{}`", self.current_text());
            self.error(codes::MISMATCHED_END, message, span);
        }
        self.bump();
        if self.at_punct(Punct::Semicolon) {
            self.bump();
        }
        self.finish_node();
    }

    fn parse_empty_statement(&mut self) {
        self.start_node(SyntaxKind::EmptyStmt);
        self.expect_punct(Punct::Semicolon);
        self.finish_node();
    }

    fn parse_statement_with_pragmas(&mut self, count: usize) {
        // The grammar allows pragmas before declarations and POUs, not before
        // statements; attach them anyway for a lossless, recoverable tree.
        let annotations = self.collect_pragmas(count, false);
        let statement = self.start_node(SyntaxKind::Error);
        self.emit_pragmas(&annotations);
        if !self.enter_depth() {
            self.recover_statement();
            self.finish_node();
            return;
        }
        self.parse_statement_body(statement);
        self.leave_depth();
    }

    fn parse_statement_body(&mut self, statement: NodeId) {
        match self.kind_at(0) {
            SyntaxKind::KwIf => {
                self.set_node_kind(statement, SyntaxKind::IfStmt);
                self.parse_if_body();
                self.finish_block_statement();
            }
            SyntaxKind::KwCase => {
                self.set_node_kind(statement, SyntaxKind::CaseStmt);
                self.parse_case_body();
                self.finish_block_statement();
            }
            SyntaxKind::KwFor => {
                self.set_node_kind(statement, SyntaxKind::ForStmt);
                self.parse_for_body();
                self.finish_block_statement();
            }
            SyntaxKind::KwWhile => {
                self.set_node_kind(statement, SyntaxKind::WhileStmt);
                self.parse_while_body();
                self.finish_block_statement();
            }
            SyntaxKind::KwRepeat => {
                self.set_node_kind(statement, SyntaxKind::RepeatStmt);
                self.parse_repeat_body();
                self.finish_block_statement();
            }
            SyntaxKind::KwReturn => {
                self.set_node_kind(statement, SyntaxKind::ReturnStmt);
                self.bump();
                self.finish_simple_statement("`RETURN`");
            }
            SyntaxKind::KwExit => {
                self.set_node_kind(statement, SyntaxKind::ExitStmt);
                self.bump();
                self.finish_simple_statement("`EXIT`");
            }
            SyntaxKind::KwContinue => {
                self.set_node_kind(statement, SyntaxKind::ContinueStmt);
                self.bump();
                self.finish_simple_statement("`CONTINUE`");
            }
            kind if is_sfc_keyword(kind) => {
                self.error(
                    codes::SFC_UNSUPPORTED,
                    "SFC constructs are not in the profile",
                    self.current_span(),
                );
                self.recover_statement();
                self.finish_node();
            }
            SyntaxKind::PunctHash => {
                self.error(
                    codes::NO_PREPROCESSOR,
                    "preprocessor directives are not in the profile",
                    self.current_span(),
                );
                self.recover_statement();
                self.finish_node();
            }
            SyntaxKind::Ident
                if self.at_ident_named("ASSERT") && self.kind_at(1) == SyntaxKind::PunctLParen =>
            {
                self.error(
                    codes::NOT_IN_PROFILE,
                    "`ASSERT` is an Ed. 4 construct and is not in the profile",
                    self.current_span(),
                );
                self.recover_statement();
                self.finish_node();
            }
            SyntaxKind::Ident
                if self.at_ident_named("PROPERTY")
                    || self.at_ident_named("MUTEX")
                    || self.at_ident_named("SEMA") =>
            {
                if self.kind_at(1) == SyntaxKind::Ident || self.kind_at(1) == SyntaxKind::PunctColon
                {
                    self.error(
                        codes::NOT_IN_PROFILE,
                        "this Ed. 4 construct is not in the profile",
                        self.current_span(),
                    );
                    self.recover_statement();
                    self.finish_node();
                } else {
                    self.parse_expression_statement(statement);
                }
            }
            SyntaxKind::Ident if self.looks_like_il() => {
                self.error(
                    codes::UNSUPPORTED_BODY,
                    "IL/LD/FBD instruction-list bodies are not in the profile",
                    self.current_span(),
                );
                self.recover_statement();
                self.finish_node();
            }
            _ => self.parse_expression_statement(statement),
        }
    }

    fn parse_expression_statement(&mut self, statement: NodeId) {
        self.parse_postfix_expr();
        let head_kind = self.last_child_node_kind(statement);
        if self.at_punct(Punct::Assign) {
            self.bump();
            self.parse_expression();
            let rhs_kind = self.last_child_node_kind(statement);
            let ref_assign = head_kind == Some(SyntaxKind::DerefExpr)
                || rhs_kind == Some(SyntaxKind::RefValueExpr);
            let kind = if ref_assign {
                SyntaxKind::RefAssignStmt
            } else {
                SyntaxKind::AssignStmt
            };
            self.finish_statement(statement, kind, false);
        } else if self.at_punct(Punct::AttemptAssign) {
            self.bump();
            self.parse_expression();
            self.finish_statement(statement, SyntaxKind::AssignmentAttemptStmt, false);
        } else {
            if head_kind != Some(SyntaxKind::CallExpr) {
                self.unexpected("expected an assignment (`:=`) or a call statement");
            }
            self.finish_statement(statement, SyntaxKind::CallStmt, false);
        }
    }

    fn finish_simple_statement(&mut self, context: &str) {
        self.expect_semicolon(context);
        self.finish_node();
    }

    fn finish_statement(&mut self, statement: NodeId, kind: SyntaxKind, block: bool) {
        self.set_node_kind(statement, kind);
        self.finish_statement_terminator(block);
        self.finish_node();
    }

    fn finish_block_statement(&mut self) {
        self.finish_statement_terminator(true);
        self.finish_node();
    }

    fn finish_statement_terminator(&mut self, block: bool) {
        if self.at_punct(Punct::Semicolon) {
            self.bump();
        } else if !(block && self.options.relaxed_semicolons) {
            let span = self.current_span();
            self.error(
                codes::MISSING_SEMICOLON,
                "expected `;` after statement",
                span,
            );
            self.insert_missing(SyntaxKind::PunctSemicolon);
        }
    }

    fn parse_if_body(&mut self) {
        let opener = self.current_span();
        self.bump(); // IF
        self.parse_expression();
        self.expect_keyword(Keyword::Then);
        self.parse_stmt_list(false, None);
        while self.at_keyword(Keyword::Elsif) {
            self.start_node(SyntaxKind::ElseBranch);
            self.bump();
            self.parse_expression();
            self.expect_keyword(Keyword::Then);
            self.parse_stmt_list(false, None);
            self.finish_node();
        }
        if self.eat_keyword(Keyword::Else) {
            self.parse_stmt_list(false, None);
        }
        self.expect_end(Keyword::EndIf, opener);
    }

    fn parse_case_body(&mut self) {
        let opener = self.current_span();
        self.bump(); // CASE
        self.parse_expression();
        self.expect_keyword(Keyword::Of);
        let mut selections = 0;
        while !self.at_eof()
            && !self.at_keyword(Keyword::Else)
            && !self.at_keyword(Keyword::EndCase)
        {
            let before = self.pos;
            self.parse_case_selection();
            selections += 1;
            if self.at_block_delimiter() {
                break;
            }
            if self.pos == before {
                self.recover_statement();
                if self.pos == before {
                    break;
                }
            }
        }
        if selections == 0 {
            self.expected("a CASE selection");
        }
        if self.eat_keyword(Keyword::Else) {
            self.parse_stmt_list(false, None);
        }
        self.expect_end(Keyword::EndCase, opener);
    }

    fn parse_case_selection(&mut self) {
        self.start_node(SyntaxKind::CaseSelection);
        loop {
            self.parse_case_label();
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_punct(Punct::Colon);
        self.parse_stmt_list(true, None);
        self.finish_node();
    }

    fn parse_case_label(&mut self) {
        self.start_node(SyntaxKind::CaseLabel);
        self.parse_expression();
        if self.eat_punct(Punct::Range) {
            self.parse_expression();
        }
        self.finish_node();
    }

    fn parse_for_body(&mut self) {
        let opener = self.current_span();
        self.bump(); // FOR
        self.parse_name();
        self.expect_punct(Punct::Assign);
        self.parse_expression();
        self.expect_keyword(Keyword::To);
        self.parse_expression();
        if self.eat_keyword(Keyword::By) {
            self.parse_expression();
        }
        self.expect_keyword(Keyword::Do);
        self.parse_stmt_list(false, None);
        self.expect_end(Keyword::EndFor, opener);
    }

    fn parse_while_body(&mut self) {
        let opener = self.current_span();
        self.bump(); // WHILE
        self.parse_expression();
        self.expect_keyword(Keyword::Do);
        self.parse_stmt_list(false, None);
        self.expect_end(Keyword::EndWhile, opener);
    }

    fn parse_repeat_body(&mut self) {
        let opener = self.current_span();
        self.bump(); // REPEAT
        self.parse_stmt_list(false, None);
        self.expect_keyword(Keyword::Until);
        self.parse_expression();
        self.expect_end(Keyword::EndRepeat, opener);
    }

    /// Detects a case label (`constant_expr [: constant_expr] ':'`) at the
    /// current position without consuming it.
    fn at_case_label_start(&self) -> bool {
        if !matches!(
            self.kind_at(0),
            SyntaxKind::Ident
                | SyntaxKind::IntLiteral
                | SyntaxKind::RealLiteral
                | SyntaxKind::BasedLiteral
                | SyntaxKind::TypedLiteral
                | SyntaxKind::CharLiteral
                | SyntaxKind::StringLiteral
                | SyntaxKind::UStringLiteral
                | SyntaxKind::DurationLiteral
                | SyntaxKind::DateLiteral
                | SyntaxKind::TodLiteral
                | SyntaxKind::DateTimeLiteral
                | SyntaxKind::KwTrue
                | SyntaxKind::KwFalse
                | SyntaxKind::PunctPlus
                | SyntaxKind::PunctMinus
                | SyntaxKind::PunctLParen
        ) {
            return false;
        }
        let mut depth = 0i32;
        for offset in 0..4096 {
            let kind = self.kind_at(offset);
            match kind {
                SyntaxKind::Eof => return false,
                SyntaxKind::PunctLParen | SyntaxKind::PunctLBracket => depth += 1,
                SyntaxKind::PunctRParen | SyntaxKind::PunctRBracket => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                SyntaxKind::PunctColon if depth == 0 => return true,
                SyntaxKind::PunctAssign
                | SyntaxKind::PunctOutputAssign
                | SyntaxKind::PunctAttemptAssign
                | SyntaxKind::PunctSemicolon
                    if depth == 0 =>
                {
                    return false;
                }
                kind if depth == 0 && is_block_delimiter(kind) => return false,
                _ => {}
            }
        }
        false
    }

    fn looks_like_il(&self) -> bool {
        let Some(name) = self.current().normalized() else {
            return false;
        };
        if !IL_MNEMONICS.contains(&name) {
            return false;
        }
        matches!(
            self.kind_at(1),
            SyntaxKind::Ident
                | SyntaxKind::IntLiteral
                | SyntaxKind::RealLiteral
                | SyntaxKind::BasedLiteral
                | SyntaxKind::TypedLiteral
                | SyntaxKind::DirectVariable
                | SyntaxKind::StringLiteral
                | SyntaxKind::UStringLiteral
        )
    }
}

pub(crate) fn is_statement_terminator(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwElse
            | SyntaxKind::KwElsif
            | SyntaxKind::KwUntil
            | SyntaxKind::KwEndIf
            | SyntaxKind::KwEndCase
            | SyntaxKind::KwEndFor
            | SyntaxKind::KwEndWhile
            | SyntaxKind::KwEndRepeat
            | SyntaxKind::KwEndAction
            | SyntaxKind::KwEndStep
            | SyntaxKind::KwEndTransition
    )
}
