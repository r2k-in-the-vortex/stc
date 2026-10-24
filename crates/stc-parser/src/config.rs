//! Configurations, resources, tasks, and program configurations
//! (Grammar §7).

use stc_lexer::{Keyword, Punct};
use stc_syntax::SyntaxKind;

use crate::codes;
use crate::parser::{Parser, is_var_kind};

impl Parser<'_> {
    pub(crate) fn parse_configuration_body(&mut self) {
        let opener = self.current_span();
        self.expect_keyword(Keyword::Configuration);
        self.parse_name();
        if is_var_kind(self.kind_at(0)) {
            if !self.at_keyword(Keyword::VarGlobal) {
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "only a VAR_GLOBAL section is allowed here",
                    self.current_span(),
                );
            }
            self.parse_var_section();
        }
        if self.at_keyword(Keyword::Resource) {
            while self.at_keyword(Keyword::Resource) {
                self.parse_resource_decl();
            }
        } else {
            self.parse_single_resource_decl();
        }
        if self.at_keyword(Keyword::VarAccess) {
            self.parse_var_access_section();
        }
        if self.at_keyword(Keyword::VarConfig) {
            self.parse_config_init();
        }
        self.expect_end(Keyword::EndConfiguration, opener);
    }

    fn parse_resource_decl(&mut self) {
        let opener = self.current_span();
        self.start_node(SyntaxKind::ResourceDecl);
        self.expect_keyword(Keyword::Resource);
        self.parse_name();
        self.expect_keyword(Keyword::On);
        self.parse_name();
        if is_var_kind(self.kind_at(0)) {
            if !self.at_keyword(Keyword::VarGlobal) {
                self.error(
                    codes::UNEXPECTED_TOKEN,
                    "only a VAR_GLOBAL section is allowed here",
                    self.current_span(),
                );
            }
            self.parse_var_section();
        }
        self.parse_single_resource_decl();
        self.expect_end(Keyword::EndResource, opener);
        self.finish_node();
    }

    fn parse_single_resource_decl(&mut self) {
        while self.at_keyword(Keyword::Task) {
            self.parse_task_config();
            self.expect_semicolon("task declaration");
        }
        let mut programs = 0;
        while self.at_keyword(Keyword::Program) {
            self.parse_program_config();
            self.expect_semicolon("program configuration");
            programs += 1;
        }
        if programs == 0 {
            self.expected("a program configuration");
        }
    }

    fn parse_task_config(&mut self) {
        self.start_node(SyntaxKind::TaskConfig);
        self.expect_keyword(Keyword::Task);
        self.parse_name();
        self.expect_punct(Punct::LParen);
        if !self.at_punct(Punct::RParen) {
            loop {
                self.parse_task_attr();
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen);
        self.finish_node();
    }

    fn parse_task_attr(&mut self) {
        self.start_node(SyntaxKind::TaskAttr);
        if matches!(
            self.kind_at(0),
            SyntaxKind::KwSingle | SyntaxKind::KwInterval | SyntaxKind::KwPriority
        ) {
            self.bump();
        } else {
            self.expected("SINGLE, INTERVAL, or PRIORITY");
            self.insert_missing(SyntaxKind::KwInterval);
        }
        self.expect_punct(Punct::Assign);
        self.parse_prog_data_source();
        self.finish_node();
    }

    fn parse_program_config(&mut self) {
        self.start_node(SyntaxKind::ProgramConfig);
        self.expect_keyword(Keyword::Program);
        if !self.eat_keyword(Keyword::Retain) {
            self.eat_keyword(Keyword::NonRetain);
        }
        self.parse_name();
        if self.eat_keyword(Keyword::With) {
            self.parse_name();
        }
        self.expect_punct(Punct::Colon);
        self.parse_path();
        if self.eat_punct(Punct::LParen) {
            if !self.at_punct(Punct::RParen) {
                loop {
                    self.parse_prog_conf_elem();
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
            }
            self.expect_punct(Punct::RParen);
        }
        self.finish_node();
    }

    fn parse_prog_conf_elem(&mut self) {
        self.start_node(SyntaxKind::ProgConfElem);
        if self.at_ident() && self.kind_at(1) == SyntaxKind::KwWith {
            self.parse_name();
            self.bump();
            self.parse_name();
        } else {
            self.parse_postfix_expr();
            if self.at_punct(Punct::Assign) || self.at_punct(Punct::OutputAssign) {
                self.bump();
                self.parse_prog_data_source();
            } else {
                self.expected("`:=` or `=>`");
                self.recover_until(|kind| {
                    matches!(kind, SyntaxKind::PunctComma | SyntaxKind::PunctRParen)
                });
            }
        }
        self.finish_node();
    }

    fn parse_config_init(&mut self) {
        let opener = self.current_span();
        self.start_node(SyntaxKind::ConfigInit);
        self.expect_keyword(Keyword::VarConfig);
        while self.at_ident() {
            self.parse_config_inst_init();
            self.expect_semicolon("configuration instance");
        }
        self.expect_end(Keyword::EndVar, opener);
        self.finish_node();
    }

    fn parse_config_inst_init(&mut self) {
        self.start_node(SyntaxKind::ConfigInstInit);
        self.parse_name();
        self.expect_punct(Punct::Dot);
        self.parse_name();
        self.expect_punct(Punct::Dot);
        // `(IDENT '.')* IDENT`: the final identifier is the configured target.
        loop {
            self.parse_name();
            if self.eat_punct(Punct::Dot) {
                continue;
            }
            break;
        }
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
}
