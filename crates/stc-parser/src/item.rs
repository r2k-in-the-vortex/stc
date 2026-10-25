//! Source items, namespaces, types, and variable declarations
//! (Grammar §2–§5).

use stc_lexer::{Keyword, Punct, TokenKind};
use stc_syntax::SyntaxKind;

use crate::codes;
use crate::parser::{
    Parser, is_int_type_keyword, is_sfc_keyword, is_string_type_keyword, is_type_name_keyword,
    is_var_kind,
};

impl Parser<'_> {
    pub(crate) fn parse_source_file(&mut self) {
        self.start_node(SyntaxKind::SourceFile);
        while !self.at_eof() {
            let before = self.pos;
            self.parse_top_item();
            if self.pos == before && !self.at_eof() {
                self.recover_declaration(true);
            }
        }
        self.emit_rest_of_file();
        self.finish_node();
    }

    fn parse_top_item(&mut self) {
        let count = self.pragma_count();
        let item = self.kind_at(count);
        if item == SyntaxKind::Eof && count > 0 {
            self.start_node(SyntaxKind::Error);
            let annotations = self.collect_pragmas(count, false);
            self.emit_pragmas(&annotations);
            self.finish_node();
            return;
        }
        match item {
            SyntaxKind::KwUsing => {
                self.start_node(SyntaxKind::UsingDirective);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_using_body();
                self.finish_node();
            }
            SyntaxKind::KwNamespace => {
                self.start_node(SyntaxKind::NamespaceDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_namespace_body();
                self.finish_node();
            }
            SyntaxKind::KwType => {
                self.start_node(SyntaxKind::DataTypeDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_data_type_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwFunction => {
                self.start_node(SyntaxKind::FunctionDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_function_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwFunctionBlock => {
                self.start_node(SyntaxKind::FunctionBlockDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_function_block_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwProgram => {
                self.start_node(SyntaxKind::ProgramDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_program_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwClass => {
                self.start_node(SyntaxKind::ClassDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_class_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwInterface => {
                self.start_node(SyntaxKind::InterfaceDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_interface_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwConfiguration => {
                self.start_node(SyntaxKind::ConfigDecl);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.parse_configuration_body();
                self.finish_node();
            }
            kind if is_var_kind(kind) => {
                self.start_node(SyntaxKind::VarSection);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                if kind != SyntaxKind::KwVarGlobal {
                    self.error(
                        codes::UNEXPECTED_TOKEN,
                        "only a VAR_GLOBAL section is allowed at top level",
                        self.current_span(),
                    );
                }
                self.parse_var_section_body();
                self.finish_node();
            }
            SyntaxKind::PunctHash => {
                self.start_node(SyntaxKind::Error);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.error(
                    codes::NO_PREPROCESSOR,
                    "preprocessor directives are not in the profile",
                    self.current_span(),
                );
                self.recover_declaration(true);
                self.finish_node();
            }
            kind if is_sfc_keyword(kind) => {
                self.start_node(SyntaxKind::Error);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.error(
                    codes::SFC_UNSUPPORTED,
                    "SFC declarations are not in the profile",
                    self.current_span(),
                );
                self.recover_declaration(true);
                self.finish_node();
            }
            SyntaxKind::Ident if self.at_ed4_item(count) => {
                self.start_node(SyntaxKind::Error);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.error(
                    codes::NOT_IN_PROFILE,
                    "this Ed. 4 construct is not in the profile",
                    self.current_span(),
                );
                self.recover_declaration(true);
                self.finish_node();
            }
            _ => {
                self.start_node(SyntaxKind::Error);
                let annotations = self.collect_pragmas(count, true);
                self.emit_pragmas(&annotations);
                self.unexpected("expected a declaration");
                self.recover_declaration(true);
                self.finish_node();
            }
        }
    }

    fn at_ed4_item(&self, pragma_count: usize) -> bool {
        let offset = pragma_count;
        matches!(
            &self.nth(offset).kind,
            TokenKind::Ident(key)
                if key.as_ref() == "PROPERTY" || key.as_ref() == "MUTEX" || key.as_ref() == "SEMA"
        )
    }

    // ---- USING and namespaces --------------------------------------------

    fn parse_using_body(&mut self) {
        self.expect_keyword(Keyword::Using);
        loop {
            self.parse_path();
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        self.expect_semicolon("USING directive");
    }

    pub(crate) fn parse_using_directive(&mut self) {
        self.start_node(SyntaxKind::UsingDirective);
        self.parse_using_body();
        self.finish_node();
    }

    fn parse_namespace_body(&mut self) {
        if !self.enter_depth() {
            self.recover_until(|kind| kind == SyntaxKind::KwEndNamespace);
            if self.at_keyword(Keyword::EndNamespace) {
                self.bump();
            } else {
                self.insert_missing(SyntaxKind::KwEndNamespace);
            }
            return;
        }
        let opener = self.current_span();
        self.expect_keyword(Keyword::Namespace);
        self.eat_keyword(Keyword::Internal);
        self.parse_path();
        while self.at_keyword(Keyword::Using) {
            self.parse_using_directive();
        }
        let mut elements = 0;
        while !self.at_eof() && !self.at_keyword(Keyword::EndNamespace) {
            if self.at_any_end() {
                break;
            }
            let before = self.pos;
            self.parse_namespace_element();
            elements += 1;
            if self.pos == before && !self.at_eof() {
                self.recover_declaration(true);
            }
        }
        if elements == 0 {
            self.expected("a namespace element");
        }
        self.expect_end(Keyword::EndNamespace, opener);
        self.leave_depth();
    }

    fn parse_namespace_element(&mut self) {
        let count = self.pragma_count();
        match self.kind_at(count) {
            SyntaxKind::KwType => {
                self.start_node(SyntaxKind::DataTypeDecl);
                self.namespace_pragmas(count);
                self.parse_data_type_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwFunction => {
                self.start_node(SyntaxKind::FunctionDecl);
                self.namespace_pragmas(count);
                self.parse_function_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwFunctionBlock => {
                self.start_node(SyntaxKind::FunctionBlockDecl);
                self.namespace_pragmas(count);
                self.parse_function_block_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwClass => {
                self.start_node(SyntaxKind::ClassDecl);
                self.namespace_pragmas(count);
                self.parse_class_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwInterface => {
                self.start_node(SyntaxKind::InterfaceDecl);
                self.namespace_pragmas(count);
                self.parse_interface_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwNamespace => {
                self.start_node(SyntaxKind::NamespaceDecl);
                self.namespace_pragmas(count);
                self.parse_namespace_body();
                self.finish_node();
            }
            SyntaxKind::KwProgram => {
                self.start_node(SyntaxKind::ProgramDecl);
                self.namespace_pragmas(count);
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "PROGRAM declarations are only allowed at the top level",
                    self.current_span(),
                );
                self.parse_program_decl_body();
                self.finish_node();
            }
            SyntaxKind::KwConfiguration => {
                self.start_node(SyntaxKind::ConfigDecl);
                self.namespace_pragmas(count);
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "CONFIGURATION declarations are only allowed at the top level",
                    self.current_span(),
                );
                self.parse_configuration_body();
                self.finish_node();
            }
            kind if is_var_kind(kind) => {
                self.start_node(SyntaxKind::VarSection);
                self.namespace_pragmas(count);
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "variable sections are not allowed in a namespace",
                    self.current_span(),
                );
                self.parse_var_section_body();
                self.finish_node();
            }
            SyntaxKind::PunctHash => {
                self.start_node(SyntaxKind::Error);
                self.namespace_pragmas(count);
                self.error(
                    codes::NO_PREPROCESSOR,
                    "preprocessor directives are not in the profile",
                    self.current_span(),
                );
                self.recover_declaration(true);
                self.finish_node();
            }
            kind if is_sfc_keyword(kind) => {
                self.start_node(SyntaxKind::Error);
                self.namespace_pragmas(count);
                self.error(
                    codes::SFC_UNSUPPORTED,
                    "SFC declarations are not in the profile",
                    self.current_span(),
                );
                self.recover_declaration(true);
                self.finish_node();
            }
            _ => {
                self.start_node(SyntaxKind::Error);
                self.namespace_pragmas(count);
                self.unexpected("expected a namespace element");
                self.recover_declaration(true);
                self.finish_node();
            }
        }
    }

    fn namespace_pragmas(&mut self, count: usize) {
        let annotations = self.collect_pragmas(count, true);
        self.emit_pragmas(&annotations);
    }

    // ---- data types -------------------------------------------------------

    fn parse_data_type_decl_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Type);
        let mut declarations = 0;
        while !self.at_eof() && !self.at_keyword(Keyword::EndType) {
            if self.at_any_end() {
                break;
            }
            if self.at_ident() {
                self.parse_type_decl();
                declarations += 1;
            } else {
                self.unexpected("expected a type declaration");
                let before = self.pos;
                self.recover_declaration(true);
                if self.pos == before {
                    break;
                }
            }
        }
        if declarations == 0 {
            self.expected("a type declaration");
        }
        self.expect_end(Keyword::EndType, opener);
    }

    fn parse_type_decl(&mut self) {
        let kind = self.type_decl_kind();
        self.start_node(kind);
        match kind {
            SyntaxKind::EnumTypeDecl => self.parse_enum_type_decl_body(),
            SyntaxKind::SubrangeTypeDecl => self.parse_subrange_type_decl_body(),
            SyntaxKind::NamedValueTypeDecl => self.parse_named_value_type_decl_body(),
            _ => self.parse_general_type_decl_body(),
        }
        self.finish_node();
    }

    fn type_decl_kind(&self) -> SyntaxKind {
        if self.kind_at(1) != SyntaxKind::PunctColon {
            return SyntaxKind::SimpleTypeDecl;
        }
        let rhs = self.kind_at(2);
        match rhs {
            SyntaxKind::PunctLParen => SyntaxKind::EnumTypeDecl,
            SyntaxKind::KwArray => SyntaxKind::ArrayTypeDecl,
            SyntaxKind::KwStruct => SyntaxKind::StructTypeDecl,
            SyntaxKind::KwRefTo => SyntaxKind::RefTypeDecl,
            kind if is_string_type_keyword(kind) => SyntaxKind::StringTypeDecl,
            kind if is_int_type_keyword(kind) && self.range_in_parens(3) => {
                SyntaxKind::SubrangeTypeDecl
            }
            kind if is_type_name_keyword(kind) && self.kind_at(3) == SyntaxKind::PunctLParen => {
                SyntaxKind::NamedValueTypeDecl
            }
            _ => SyntaxKind::SimpleTypeDecl,
        }
    }

    fn range_in_parens(&self, offset: usize) -> bool {
        if self.kind_at(offset) != SyntaxKind::PunctLParen {
            return false;
        }
        let mut depth = 0i32;
        for step in offset..offset + 4096 {
            match self.kind_at(step) {
                SyntaxKind::Eof => return false,
                SyntaxKind::PunctLParen => depth += 1,
                SyntaxKind::PunctRParen => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                SyntaxKind::PunctRange if depth == 1 => return true,
                _ => {}
            }
        }
        false
    }

    fn parse_enum_type_decl_body(&mut self) {
        self.parse_name();
        self.expect_punct(Punct::Colon);
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            loop {
                self.start_node(SyntaxKind::EnumValue);
                self.parse_name();
                if self.eat_punct(Punct::Assign) {
                    self.parse_expression();
                }
                self.finish_node();
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen);
        if self.eat_punct(Punct::Assign) {
            self.parse_name();
        }
        self.expect_semicolon("type declaration");
    }

    fn parse_subrange_type_decl_body(&mut self) {
        self.parse_name();
        self.expect_punct(Punct::Colon);
        if is_int_type_keyword(self.kind_at(0)) {
            self.bump();
        } else {
            self.expected("an integer type name");
            self.insert_missing(SyntaxKind::KwInt);
        }
        self.expect_punct(Punct::LParen);
        self.parse_expression();
        self.expect_punct(Punct::Range);
        self.parse_expression();
        self.expect_punct(Punct::RParen);
        if self.eat_punct(Punct::Assign) {
            self.parse_expression();
        }
        self.expect_semicolon("type declaration");
    }

    fn parse_named_value_type_decl_body(&mut self) {
        self.parse_name();
        self.expect_punct(Punct::Colon);
        if is_type_name_keyword(self.kind_at(0)) {
            self.bump();
        } else if self.at_ident() {
            self.parse_path();
        } else {
            self.expected("an elementary type name");
            self.insert_missing(SyntaxKind::Ident);
        }
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            loop {
                self.start_node(SyntaxKind::NamedValue);
                self.parse_name();
                if self.eat_punct(Punct::Assign) {
                    self.parse_expression();
                }
                self.finish_node();
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen);
        if self.eat_punct(Punct::Assign) {
            self.parse_name();
        }
        self.expect_semicolon("type declaration");
    }

    fn parse_general_type_decl_body(&mut self) {
        self.parse_name();
        self.expect_punct(Punct::Colon);
        self.parse_type_spec();
        if self.eat_punct(Punct::Assign) {
            self.parse_initializer();
        }
        self.expect_semicolon("type declaration");
    }

    // ---- type specs -------------------------------------------------------

    pub(crate) fn parse_type_spec(&mut self) {
        if !self.enter_depth() {
            self.recover_until(|kind| {
                matches!(
                    kind,
                    SyntaxKind::PunctSemicolon
                        | SyntaxKind::PunctComma
                        | SyntaxKind::PunctRParen
                        | SyntaxKind::PunctRBracket
                        | SyntaxKind::KwEndVar
                )
            });
            return;
        }
        self.start_node(SyntaxKind::TypeRef);
        self.parse_type_spec_inner();
        self.finish_node();
        self.leave_depth();
    }

    fn parse_type_spec_inner(&mut self) {
        let kind = self.kind_at(0);
        if kind == SyntaxKind::KwArray {
            self.parse_array_shape();
        } else if kind == SyntaxKind::KwStruct {
            self.parse_struct_shape();
        } else if kind == SyntaxKind::KwRefTo {
            self.bump();
            self.parse_type_spec();
        } else if kind == SyntaxKind::KwPointer {
            self.bump();
            self.expect_keyword(Keyword::To);
            self.parse_type_spec();
        } else if is_string_type_keyword(kind) {
            self.bump();
            if self.at_punct(Punct::LBracket)
                && matches!(
                    kind,
                    SyntaxKind::KwString | SyntaxKind::KwWstring | SyntaxKind::KwUstring
                )
            {
                self.bump();
                if self.at_kind(SyntaxKind::IntLiteral) {
                    self.bump();
                } else {
                    self.expected("a string capacity");
                    self.insert_missing(SyntaxKind::IntLiteral);
                }
                self.expect_punct(Punct::RBracket);
            }
        } else if is_type_name_keyword(kind) {
            self.bump();
        } else if kind == SyntaxKind::Ident {
            self.parse_path();
        } else {
            self.expected("a type");
            self.insert_missing(SyntaxKind::Ident);
        }
    }

    pub(crate) fn parse_array_shape(&mut self) {
        self.expect_keyword(Keyword::Array);
        self.expect_punct(Punct::LBracket);
        if !self.at_punct(Punct::RBracket) {
            loop {
                self.parse_array_bound();
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RBracket);
        self.expect_keyword(Keyword::Of);
        self.parse_type_spec();
    }

    fn parse_array_bound(&mut self) {
        self.start_node(SyntaxKind::ArrayBound);
        if self.at_punct(Punct::Star) {
            self.bump();
        } else {
            self.parse_expression();
            if self.eat_punct(Punct::Range) {
                self.parse_expression();
            }
        }
        self.finish_node();
    }

    pub(crate) fn parse_struct_shape(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Struct);
        self.eat_keyword(Keyword::Overlap);
        let mut elements = 0;
        while !self.at_eof() && !self.at_keyword(Keyword::EndStruct) {
            if self.at_any_end() {
                break;
            }
            let before = self.pos;
            self.parse_struct_elem();
            elements += 1;
            self.expect_semicolon("struct element");
            if self.pos == before {
                self.recover_declaration(true);
                if self.pos == before {
                    break;
                }
            }
        }
        if elements == 0 {
            self.expected("a struct element");
        }
        self.expect_end(Keyword::EndStruct, opener);
    }

    fn parse_struct_elem(&mut self) {
        self.start_node(SyntaxKind::StructElem);
        self.parse_name();
        if self.eat_keyword(Keyword::At) {
            if self.at_kind(SyntaxKind::DirectVariable) {
                self.bump();
            } else {
                self.expected("a direct-variable address");
                self.insert_missing(SyntaxKind::DirectVariable);
            }
        }
        self.expect_punct(Punct::Colon);
        self.parse_type_spec();
        if self.eat_punct(Punct::Assign) {
            self.parse_initializer();
        }
        self.finish_node();
    }

    // ---- initializers -----------------------------------------------------

    pub(crate) fn parse_initializer(&mut self) {
        if !self.enter_depth() {
            self.recover_expression();
            return;
        }
        if self.at_punct(Punct::LBracket) {
            self.start_node(SyntaxKind::ArrayInit);
            self.bump();
            if !self.at_punct(Punct::RBracket) {
                loop {
                    self.parse_initializer();
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
            }
            self.expect_punct(Punct::RBracket);
            self.finish_node();
        } else if self.at_punct(Punct::LParen) && self.looks_like_struct_init() {
            self.start_node(SyntaxKind::StructInit);
            self.bump();
            if !self.at_punct(Punct::RParen) {
                loop {
                    self.start_node(SyntaxKind::StructFieldInit);
                    self.parse_struct_field_name();
                    self.expect_punct(Punct::Assign);
                    self.parse_initializer();
                    self.finish_node();
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
            }
            self.expect_punct(Punct::RParen);
            self.finish_node();
        } else {
            self.parse_expression();
        }
        self.leave_depth();
    }

    fn parse_struct_field_name(&mut self) {
        self.parse_name();
        while self.at_punct(Punct::Dot) && self.kind_at(1) == SyntaxKind::Ident {
            self.bump();
            self.parse_name();
        }
    }

    fn looks_like_struct_init(&self) -> bool {
        if self.kind_at(1) != SyntaxKind::Ident {
            return false;
        }
        let mut offset = 2;
        loop {
            match self.kind_at(offset) {
                SyntaxKind::PunctAssign => return true,
                SyntaxKind::PunctDot if self.kind_at(offset + 1) == SyntaxKind::Ident => {
                    offset += 2;
                }
                _ => return false,
            }
        }
    }

    // ---- variable sections ------------------------------------------------

    pub(crate) fn parse_var_section(&mut self) {
        self.start_node(SyntaxKind::VarSection);
        self.parse_var_section_body();
        self.finish_node();
    }

    fn parse_var_section_body(&mut self) {
        let opener = self.current_span();
        if is_var_kind(self.kind_at(0)) {
            self.bump();
        } else {
            self.expected("a variable section");
            self.insert_missing(SyntaxKind::KwVar);
        }
        while matches!(
            self.kind_at(0),
            SyntaxKind::KwConstant
                | SyntaxKind::KwRetain
                | SyntaxKind::KwNonRetain
                | SyntaxKind::KwPublic
                | SyntaxKind::KwProtected
                | SyntaxKind::KwPrivate
                | SyntaxKind::KwInternal
        ) {
            self.bump();
        }
        while !self.at_eof() && !self.at_keyword(Keyword::EndVar) {
            if self.at_any_end() {
                break;
            }
            if self.at_ident() || self.at_pragma() {
                let before = self.pos;
                self.parse_var_decl();
                if self.pos == before {
                    self.recover_declaration(true);
                }
            } else {
                self.unexpected("expected a variable declaration");
                let before = self.pos;
                self.recover_declaration(true);
                if self.pos == before {
                    break;
                }
            }
        }
        self.expect_end(Keyword::EndVar, opener);
    }

    fn parse_var_decl(&mut self) {
        let count = self.pragma_count();
        let edge = self.looks_like_edge(count);
        if edge {
            self.start_node(SyntaxKind::EdgeDecl);
            let annotations = self.collect_pragmas(count, true);
            self.emit_pragmas(&annotations);
            self.check_io_pragmas(&annotations, false);
            self.parse_names();
            self.expect_punct(Punct::Colon);
            self.expect_keyword(Keyword::Bool);
            if !self.eat_keyword(Keyword::REdge) {
                self.expect_keyword(Keyword::FEdge);
            }
            self.expect_semicolon("edge declaration");
            self.finish_node();
            return;
        }
        self.start_node(SyntaxKind::VarDecl);
        let mut annotations = self.collect_pragmas(count, true);
        self.emit_pragmas(&annotations);
        self.parse_names();
        let has_at = self.eat_keyword(Keyword::At);
        if has_at {
            if self.at_kind(SyntaxKind::DirectVariable) {
                self.bump();
            } else {
                self.expected("a direct-variable address");
                self.insert_missing(SyntaxKind::DirectVariable);
            }
        }
        self.expect_punct(Punct::Colon);
        self.parse_type_spec();
        if self.eat_punct(Punct::Assign) {
            self.parse_initializer();
        }
        let trailing = self.pragma_count();
        let trailing = self.collect_pragmas(trailing, true);
        self.emit_pragmas(&trailing);
        annotations.extend(trailing);
        self.check_io_pragmas(&annotations, has_at);
        self.expect_semicolon("variable declaration");
        self.finish_node();
    }

    fn parse_names(&mut self) {
        loop {
            self.parse_name();
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
    }

    fn looks_like_edge(&self, offset: usize) -> bool {
        let mut index = offset;
        if self.kind_at(index) != SyntaxKind::Ident {
            return false;
        }
        index += 1;
        while self.kind_at(index) == SyntaxKind::PunctComma {
            index += 1;
            if self.kind_at(index) != SyntaxKind::Ident {
                return false;
            }
            index += 1;
        }
        if self.kind_at(index) != SyntaxKind::PunctColon {
            return false;
        }
        index += 1;
        if self.kind_at(index) != SyntaxKind::KwBool {
            return false;
        }
        index += 1;
        matches!(
            self.kind_at(index),
            SyntaxKind::KwREdge | SyntaxKind::KwFEdge
        )
    }

    // ---- VAR_ACCESS -------------------------------------------------------

    pub(crate) fn parse_var_access_section(&mut self) {
        let opener = self.current_span();
        self.start_node(SyntaxKind::VarAccessSection);
        self.expect_keyword(Keyword::VarAccess);
        while self.at_ident() {
            self.parse_access_decl();
        }
        self.expect_end(Keyword::EndVar, opener);
        self.finish_node();
    }

    fn parse_access_decl(&mut self) {
        self.start_node(SyntaxKind::AccessDecl);
        self.parse_name();
        self.expect_punct(Punct::Colon);
        self.parse_access_path();
        self.expect_punct(Punct::Colon);
        self.parse_type_spec();
        if !self.eat_keyword(Keyword::ReadOnly) {
            self.eat_keyword(Keyword::ReadWrite);
        }
        self.expect_semicolon("VAR_ACCESS declaration");
        self.finish_node();
    }

    fn parse_access_path(&mut self) {
        if self.at_kind(SyntaxKind::DirectVariable) {
            self.bump();
            return;
        }
        self.parse_name();
        loop {
            if !self.at_punct(Punct::Dot) {
                break;
            }
            if self.kind_at(1) == SyntaxKind::Ident {
                self.bump();
                self.parse_name();
            } else if self.kind_at(1) == SyntaxKind::DirectVariable {
                self.bump();
                self.bump();
                break;
            } else {
                break;
            }
        }
    }
}
