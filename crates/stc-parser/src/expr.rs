//! Expressions: Grammar §9 and the Annex A precedence split (STC-GRAM-008–015).

use stc_lexer::{Keyword, Punct};
use stc_syntax::SyntaxKind;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses a full expression, guarding recursion depth.
    pub(crate) fn parse_expression(&mut self) {
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        self.parse_or();
        self.leave_depth();
    }

    fn parse_or(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_xor();
        while self.at_keyword(Keyword::Or) {
            self.bump();
            self.parse_xor();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    fn parse_xor(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_and();
        while self.at_keyword(Keyword::Xor) {
            self.bump();
            self.parse_and();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    fn parse_and(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_equality();
        while self.at_keyword(Keyword::And) || self.at_punct(Punct::Ampersand) {
            self.bump();
            self.parse_equality();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    /// Equality binds looser than relational (A-014, STC-GRAM-009).
    fn parse_equality(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_relational();
        while self.at_punct(Punct::Equal) || self.at_punct(Punct::NotEqual) {
            self.bump();
            self.parse_relational();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    fn parse_relational(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_additive();
        while self.at_punct(Punct::Less)
            || self.at_punct(Punct::Greater)
            || self.at_punct(Punct::LessEqual)
            || self.at_punct(Punct::GreaterEqual)
        {
            self.bump();
            self.parse_additive();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    fn parse_additive(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_term();
        while self.at_punct(Punct::Plus) || self.at_punct(Punct::Minus) {
            self.bump();
            self.parse_term();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    fn parse_term(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_power();
        while self.at_punct(Punct::Star)
            || self.at_punct(Punct::Slash)
            || self.at_keyword(Keyword::Mod)
        {
            self.bump();
            self.parse_power();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    /// `**` is left-associative (STC-GRAM-010).
    fn parse_power(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_unary();
        while self.at_punct(Punct::Power) {
            self.bump();
            self.parse_unary();
            self.wrap_children(checkpoint, SyntaxKind::BinaryExpr);
        }
    }

    /// Unary operators bind tighter than `**` (STC-GRAM-011).
    fn parse_unary(&mut self) {
        if self.at_punct(Punct::Minus)
            || self.at_punct(Punct::Plus)
            || self.at_keyword(Keyword::Not)
        {
            if !self.enter_depth() {
                self.recover_expression();
                return;
            }
            let checkpoint = self.checkpoint();
            self.bump();
            self.parse_unary();
            self.wrap_children(checkpoint, SyntaxKind::UnaryExpr);
            self.leave_depth();
        } else {
            self.parse_postfix_expr();
        }
    }

    /// Parses a primary expression followed by any postfix operations.
    pub(crate) fn parse_postfix_expr(&mut self) {
        let checkpoint = self.checkpoint();
        self.parse_primary();
        loop {
            if self.at_punct(Punct::LParen) {
                // The grammar allows calls on paths and call results, not on
                // literals or parenthesized expressions. The analyzer will
                // still resolve the callee in M3.
                let callee = self.last_child_node_kind_in_current();
                if !matches!(
                    callee,
                    Some(
                        SyntaxKind::PathExpr
                            | SyntaxKind::CallExpr
                            | SyntaxKind::MemberExpr
                            | SyntaxKind::IndexExpr
                            | SyntaxKind::DerefExpr
                            | SyntaxKind::Error
                    )
                ) {
                    self.unexpected("a call target must be a path or a call result");
                }
                self.parse_param_list();
                self.wrap_children(checkpoint, SyntaxKind::CallExpr);
            } else if self.at_punct(Punct::Dot) {
                self.parse_member_suffix();
                self.wrap_children(checkpoint, SyntaxKind::MemberExpr);
            } else if self.at_punct(Punct::LBracket) {
                self.parse_index_suffix();
                self.wrap_children(checkpoint, SyntaxKind::IndexExpr);
            } else if self.at_punct(Punct::Caret) {
                self.bump();
                self.wrap_children(checkpoint, SyntaxKind::DerefExpr);
            } else {
                break;
            }
        }
    }

    fn parse_member_suffix(&mut self) {
        self.bump(); // '.'
        match self.kind_at(0) {
            SyntaxKind::Ident => {
                self.parse_name();
            }
            SyntaxKind::IntLiteral => {
                self.bump();
            }
            SyntaxKind::DirectVariable => {
                self.bump();
            }
            SyntaxKind::PunctPercent => {
                self.bump();
                if matches!(self.kind_at(0), SyntaxKind::Ident | SyntaxKind::IntLiteral) {
                    self.bump();
                }
            }
            _ => {
                self.expected("member name, bit index, or direct-variable address");
                self.insert_missing(SyntaxKind::Ident);
            }
        }
    }

    fn parse_index_suffix(&mut self) {
        self.bump(); // '['
        if !self.at_punct(Punct::RBracket) {
            loop {
                self.parse_expression();
                if self.eat_punct(Punct::Comma) {
                    continue;
                }
                break;
            }
        }
        self.expect_punct(Punct::RBracket);
    }

    fn parse_primary(&mut self) {
        match self.kind_at(0) {
            SyntaxKind::IntLiteral
            | SyntaxKind::RealLiteral
            | SyntaxKind::BasedLiteral
            | SyntaxKind::TypedLiteral
            | SyntaxKind::StringLiteral
            | SyntaxKind::UStringLiteral
            | SyntaxKind::CharLiteral
            | SyntaxKind::DurationLiteral
            | SyntaxKind::DateLiteral
            | SyntaxKind::TodLiteral
            | SyntaxKind::DateTimeLiteral
            | SyntaxKind::DirectVariable
            | SyntaxKind::KwTrue
            | SyntaxKind::KwFalse => {
                self.start_node(SyntaxKind::LiteralExpr);
                self.bump();
                self.finish_node();
            }
            SyntaxKind::Ident => self.parse_path_expr(),
            SyntaxKind::KwThis | SyntaxKind::KwSuper => self.parse_this_super_path(),
            SyntaxKind::KwRef | SyntaxKind::KwNull => self.parse_ref_value(),
            SyntaxKind::KwAdr => self.parse_address_of(),
            SyntaxKind::KwSizeof => self.parse_sizeof(),
            SyntaxKind::KwRefTo | SyntaxKind::KwPointer => self.parse_pointer_conversion(),
            SyntaxKind::PunctLParen => {
                self.start_node(SyntaxKind::ParenExpr);
                self.bump();
                self.parse_expression();
                self.expect_punct(Punct::RParen);
                self.finish_node();
            }
            SyntaxKind::ErrorToken => {
                self.unexpected("malformed token");
                self.start_node(SyntaxKind::Error);
                self.bump();
                self.finish_node();
            }
            _ => {
                self.unexpected("expected an expression");
                self.start_node(SyntaxKind::Error);
                if !self.at_eof() && !self.at_expression_sync() {
                    self.bump();
                }
                self.finish_node();
            }
        }
    }

    fn parse_path_expr(&mut self) {
        self.start_node(SyntaxKind::PathExpr);
        self.parse_name();
        while self.at_punct(Punct::Dot) && self.kind_at(1) == SyntaxKind::Ident {
            self.bump();
            self.parse_name();
        }
        self.finish_node();
    }

    fn parse_this_super_path(&mut self) {
        self.start_node(SyntaxKind::PathExpr);
        self.bump();
        if self.at_punct(Punct::Dot) && self.kind_at(1) == SyntaxKind::Ident {
            self.bump();
            self.parse_name();
            while self.at_punct(Punct::Dot) && self.kind_at(1) == SyntaxKind::Ident {
                self.bump();
                self.parse_name();
            }
        }
        self.finish_node();
    }

    fn parse_ref_value(&mut self) {
        // `REF(REF(...))` recurses through `parse_primary` without going
        // through `parse_expression`, so guard it explicitly.
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        self.start_node(SyntaxKind::RefValueExpr);
        if self.at_keyword(Keyword::Null) {
            self.bump();
        } else {
            self.bump(); // REF
            self.expect_punct(Punct::LParen);
            if !self.at_punct(Punct::RParen) {
                self.parse_postfix_expr();
            }
            self.expect_punct(Punct::RParen);
        }
        self.finish_node();
        self.leave_depth();
    }

    fn parse_address_of(&mut self) {
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        self.start_node(SyntaxKind::AddressOfExpr);
        self.bump(); // ADR
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            self.parse_postfix_expr();
        }
        self.expect_punct(Punct::RParen);
        self.finish_node();
        self.leave_depth();
    }

    fn parse_sizeof(&mut self) {
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        self.start_node(SyntaxKind::SizeOfExpr);
        self.bump(); // SIZEOF
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            if starts_type_operand(self.kind_at(0)) {
                self.parse_type_spec();
            } else {
                self.parse_postfix_expr();
            }
        }
        self.expect_punct(Punct::RParen);
        self.finish_node();
        self.leave_depth();
    }

    fn parse_pointer_conversion(&mut self) {
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        self.start_node(SyntaxKind::PointerConversionExpr);
        let pointer = self.at_keyword(Keyword::Pointer);
        self.bump(); // REF_TO or POINTER
        if pointer {
            self.expect_keyword(Keyword::To);
        }
        self.parse_type_spec();
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            self.parse_expression();
        }
        self.expect_punct(Punct::RParen);
        self.finish_node();
        self.leave_depth();
    }

    fn parse_param_list(&mut self) {
        self.start_node(SyntaxKind::ParamList);
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            loop {
                self.parse_param();
                if self.eat_punct(Punct::Comma) {
                    continue;
                }
                break;
            }
        }
        self.expect_punct(Punct::RParen);
        self.finish_node();
    }

    fn parse_param(&mut self) {
        self.start_node(SyntaxKind::Param);
        if self.at_ident() && self.kind_at(1) == SyntaxKind::PunctAssign {
            self.parse_name();
            self.bump(); // ':='
            self.parse_expression();
        } else {
            self.parse_expression();
            if self.at_punct(Punct::OutputAssign) {
                self.bump();
                self.parse_postfix_expr();
            }
        }
        self.finish_node();
    }

    /// Parses a configuration data source or sink (Grammar §7).
    pub(crate) fn parse_prog_data_source(&mut self) {
        if self.at_kind(SyntaxKind::DirectVariable) {
            self.start_node(SyntaxKind::LiteralExpr);
            self.bump();
            self.finish_node();
        } else {
            self.parse_expression();
        }
    }
}

/// Returns `true` when the token can begin a `SIZEOF` type operand
/// (Grammar §11.11). A bare name is always parsed as a place; the analyzer
/// resolves it as a type when no value of that name is visible.
fn starts_type_operand(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::KwArray | SyntaxKind::KwStruct | SyntaxKind::KwRefTo | SyntaxKind::KwPointer
    ) || crate::parser::is_type_name_keyword(kind)
}
