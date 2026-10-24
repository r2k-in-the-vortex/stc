//! Program organization units and methods (Grammar §6).

use stc_lexer::{Keyword, Punct};
use stc_syntax::SyntaxKind;

use crate::codes;
use crate::parser::{Parser, is_access_spec, is_var_kind};

impl Parser<'_> {
    pub(crate) fn parse_function_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Function);
        self.parse_name();
        if self.eat_punct(Punct::Colon) {
            self.parse_type_spec();
        }
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        while is_var_kind(self.kind_at(0)) {
            self.parse_var_section();
        }
        self.parse_stmt_list(false, Some(Keyword::EndFunction));
        self.expect_end(Keyword::EndFunction, opener);
    }

    pub(crate) fn parse_function_block_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::FunctionBlock);
        if !self.eat_keyword(Keyword::Final) {
            self.eat_keyword(Keyword::Abstract);
        }
        self.parse_name();
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        if self.eat_keyword(Keyword::Extends) {
            self.parse_type_spec();
        }
        if self.eat_keyword(Keyword::Implements) {
            self.parse_interface_list();
        }
        while is_var_kind(self.kind_at(0)) {
            self.parse_var_section();
        }
        self.parse_methods();
        if !self.at_eof() && !self.at_any_end() {
            self.parse_stmt_list(false, Some(Keyword::EndFunctionBlock));
        }
        self.expect_end(Keyword::EndFunctionBlock, opener);
    }

    pub(crate) fn parse_program_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Program);
        self.parse_name();
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        while is_var_kind(self.kind_at(0)) {
            self.parse_var_section();
        }
        if self.at_keyword(Keyword::VarAccess) {
            self.parse_var_access_section();
        }
        self.parse_stmt_list(false, Some(Keyword::EndProgram));
        self.expect_end(Keyword::EndProgram, opener);
    }

    pub(crate) fn parse_class_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Class);
        if !self.eat_keyword(Keyword::Final) {
            self.eat_keyword(Keyword::Abstract);
        }
        self.parse_name();
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        if self.eat_keyword(Keyword::Extends) {
            self.parse_type_spec();
        }
        if self.eat_keyword(Keyword::Implements) {
            self.parse_interface_list();
        }
        while is_var_kind(self.kind_at(0)) {
            if !self.at_keyword(Keyword::Var) {
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "only a plain `VAR` section is allowed in a class",
                    self.current_span(),
                );
            }
            self.parse_var_section();
        }
        self.parse_methods();
        self.expect_end(Keyword::EndClass, opener);
    }

    pub(crate) fn parse_interface_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Interface);
        self.parse_name();
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        if self.eat_keyword(Keyword::Extends) {
            self.parse_interface_list();
        }
        while self.at_method_start() {
            self.parse_method_prototype_with_pragmas();
        }
        self.expect_end(Keyword::EndInterface, opener);
    }

    fn parse_interface_list(&mut self) {
        loop {
            self.parse_path();
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
    }

    fn at_method_start(&self) -> bool {
        self.at_keyword(Keyword::Method)
            || (self.at_pragma() && self.kind_at(self.pragma_count()) == SyntaxKind::KwMethod)
    }

    fn parse_methods(&mut self) {
        while self.at_method_start() {
            self.parse_method_with_pragmas();
        }
    }

    fn parse_method_with_pragmas(&mut self) {
        let count = self.pragma_count();
        self.start_node(SyntaxKind::MethodDecl);
        let annotations = self.collect_pragmas(count, true);
        self.emit_pragmas(&annotations);
        self.parse_method_body();
        self.finish_node();
    }

    fn parse_method_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Method);
        if is_access_spec(self.kind_at(0)) {
            self.bump();
        }
        if !self.eat_keyword(Keyword::Final) {
            self.eat_keyword(Keyword::Abstract);
        }
        self.eat_keyword(Keyword::Override);
        self.parse_name();
        if self.eat_punct(Punct::Colon) {
            self.parse_type_spec();
        }
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        while is_var_kind(self.kind_at(0)) {
            self.parse_var_section();
        }
        if !self.at_eof() && !self.at_any_end() {
            self.parse_stmt_list(false, Some(Keyword::EndMethod));
        }
        self.expect_end(Keyword::EndMethod, opener);
    }

    fn parse_method_prototype_with_pragmas(&mut self) {
        let count = self.pragma_count();
        self.start_node(SyntaxKind::MethodPrototype);
        let annotations = self.collect_pragmas(count, true);
        self.emit_pragmas(&annotations);
        self.parse_method_prototype_body();
        self.finish_node();
    }

    fn parse_method_prototype_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Method);
        if is_access_spec(self.kind_at(0)) {
            self.bump();
        }
        self.parse_name();
        if self.eat_punct(Punct::Colon) {
            self.parse_type_spec();
        }
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        while is_var_kind(self.kind_at(0)) {
            self.parse_var_section();
        }
        if !self.at_keyword(Keyword::EndMethod) && !self.at_eof() {
            self.error(
                codes::UNEXPECTED_TOKEN,
                "a method prototype cannot have a body",
                self.current_span(),
            );
            self.recover_until(|kind| kind == SyntaxKind::KwEndMethod);
        }
        self.expect_end(Keyword::EndMethod, opener);
    }
}
