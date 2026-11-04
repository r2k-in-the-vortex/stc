//! Document symbols from the typed AST.
//!
//! One hierarchical tree per file: namespaces and configurations contain their
//! declarations, POUs contain their variable sections (one symbol per declared
//! name) and methods, and data types contain enum values or struct fields.
//! Ranges are token extents (without attached trivia); the selection range is
//! the name token.

use stc_parser::ParsedFile;
use stc_span::Span;
use stc_syntax::SyntaxKind;
use stc_syntax::SyntaxNode;
use stc_syntax::ast::{
    AstNode, ConfigDecl, DataTypeDecl, Item, MethodDecl, MethodPrototype, Name, ProgramConfig,
    SourceFile, TypeDecl, VarAccessSection, VarDeclLike, VarSection,
};

use crate::snapshot::Snapshot;

use stc_span::FileId;

/// The category of a document symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolKind {
    /// A `NAMESPACE`.
    Namespace,
    /// A `FUNCTION`.
    Function,
    /// A `FUNCTION_BLOCK`.
    FunctionBlock,
    /// A `PROGRAM`.
    Program,
    /// A `CLASS`.
    Class,
    /// An `INTERFACE`.
    Interface,
    /// A method or interface prototype.
    Method,
    /// A data type declaration.
    Type,
    /// A variable declaration.
    Variable,
    /// A `CONSTANT` declaration.
    Constant,
    /// An enumeration value.
    EnumValue,
    /// A `CONFIGURATION`.
    Configuration,
    /// A `RESOURCE`.
    Resource,
    /// A task declaration.
    Task,
    /// A program instance in a configuration.
    ProgramInstance,
}

/// One hierarchical document symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSymbol {
    /// Declared name as written.
    pub name: String,
    /// Symbol category.
    pub kind: SymbolKind,
    /// Declared type or result type, when known.
    pub detail: Option<String>,
    /// Token extent of the whole declaration.
    pub range: Span,
    /// Span of the name token.
    pub selection: Span,
    /// Nested symbols in source order.
    pub children: Vec<DocumentSymbol>,
}

impl Snapshot {
    /// Returns the hierarchical document symbols of a file.
    ///
    /// The tree comes from the typed AST of the file's current text, so an
    /// overlay is reflected immediately. An unknown file returns an empty list.
    #[must_use]
    pub fn document_symbols(&self, file: FileId) -> Vec<DocumentSymbol> {
        let Some(parsed) = self.parsed(file) else {
            return Vec::new();
        };
        file_symbols(parsed)
    }
}

pub(crate) fn file_symbols(parsed: &ParsedFile) -> Vec<DocumentSymbol> {
    let Some(source) = SourceFile::cast(&parsed.root()) else {
        return Vec::new();
    };
    source.items().iter().flat_map(item_symbols).collect()
}

fn item_symbols(item: &Item) -> Vec<DocumentSymbol> {
    match item {
        Item::Using(_) | Item::Error(_) => Vec::new(),
        Item::Namespace(node) => {
            let children = node.elements().iter().flat_map(item_symbols).collect();
            path_symbol(
                node.syntax(),
                node.name(),
                SymbolKind::Namespace,
                None,
                children,
            )
            .into_iter()
            .collect()
        }
        Item::DataType(node) => type_symbols(node),
        Item::Function(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Function,
            node.result_type().map(type_text),
            member_children(node.syntax()),
        )
        .into_iter()
        .collect(),
        Item::FunctionBlock(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::FunctionBlock,
            None,
            member_children(node.syntax()),
        )
        .into_iter()
        .collect(),
        Item::Program(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Program,
            None,
            member_children(node.syntax()),
        )
        .into_iter()
        .collect(),
        Item::Class(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Class,
            None,
            member_children(node.syntax()),
        )
        .into_iter()
        .collect(),
        Item::Interface(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Interface,
            None,
            member_children(node.syntax()),
        )
        .into_iter()
        .collect(),
        Item::Configuration(node) => config_symbol(node).into_iter().collect(),
        Item::GlobalVars(section) => var_section_symbols(section),
    }
}

fn type_symbols(node: &DataTypeDecl) -> Vec<DocumentSymbol> {
    node.types().iter().filter_map(type_symbol).collect()
}

fn type_symbol(decl: &TypeDecl) -> Option<DocumentSymbol> {
    match decl {
        TypeDecl::Simple(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Type,
            node.type_ref().map(type_text),
            Vec::new(),
        ),
        TypeDecl::Subrange(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Type,
            node.base_token().map(|token| token.text().to_owned()),
            Vec::new(),
        ),
        TypeDecl::Enum(node) => {
            let values = node
                .values()
                .iter()
                .filter_map(|value| {
                    symbol(
                        value.syntax(),
                        value.name(),
                        SymbolKind::EnumValue,
                        None,
                        Vec::new(),
                    )
                })
                .collect();
            symbol(node.syntax(), node.name(), SymbolKind::Type, None, values)
        }
        TypeDecl::NamedValue(node) => {
            let values = node
                .values()
                .iter()
                .filter_map(|value| {
                    symbol(
                        value.syntax(),
                        value.name(),
                        SymbolKind::EnumValue,
                        None,
                        Vec::new(),
                    )
                })
                .collect();
            symbol(node.syntax(), node.name(), SymbolKind::Type, None, values)
        }
        TypeDecl::Array(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Type,
            node.type_ref().map(type_text),
            Vec::new(),
        ),
        TypeDecl::Struct(node) => {
            let fields = node
                .elements()
                .iter()
                .filter_map(|element| {
                    symbol(
                        element.syntax(),
                        element.name(),
                        SymbolKind::Variable,
                        element.type_ref().map(type_text),
                        Vec::new(),
                    )
                })
                .collect();
            symbol(node.syntax(), node.name(), SymbolKind::Type, None, fields)
        }
        TypeDecl::String(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Type,
            node.type_ref().map(type_text),
            Vec::new(),
        ),
        TypeDecl::Ref(node) => symbol(
            node.syntax(),
            node.name(),
            SymbolKind::Type,
            node.type_ref().map(type_text),
            Vec::new(),
        ),
        TypeDecl::Error(_) => None,
    }
}

/// Collects the variable sections, `VAR_ACCESS` sections, and methods of a POU
/// in source order.
fn member_children(node: &SyntaxNode) -> Vec<DocumentSymbol> {
    let mut children = Vec::new();
    for child in node.child_nodes() {
        match child.kind() {
            SyntaxKind::VarSection => {
                if let Some(section) = VarSection::cast(&child) {
                    children.extend(var_section_symbols(&section));
                }
            }
            SyntaxKind::VarAccessSection => {
                if let Some(section) = VarAccessSection::cast(&child) {
                    children.extend(access_symbols(&section));
                }
            }
            SyntaxKind::MethodDecl => {
                if let Some(method) = MethodDecl::cast(&child) {
                    children.extend(method_symbol(
                        method.syntax(),
                        method.name(),
                        method.result_type(),
                    ));
                }
            }
            SyntaxKind::MethodPrototype => {
                if let Some(method) = MethodPrototype::cast(&child) {
                    children.extend(method_symbol(
                        method.syntax(),
                        method.name(),
                        method.result_type(),
                    ));
                }
            }
            _ => {}
        }
    }
    children
}

fn method_symbol(
    node: &SyntaxNode,
    name: Option<Name>,
    result: Option<stc_syntax::ast::TypeRef>,
) -> Option<DocumentSymbol> {
    symbol(
        node,
        name,
        SymbolKind::Method,
        result.map(type_text),
        Vec::new(),
    )
}

fn var_section_symbols(section: &VarSection) -> Vec<DocumentSymbol> {
    let constant = section
        .qualifiers()
        .iter()
        .any(|token| token.kind() == SyntaxKind::KwConstant);
    let kind = if constant {
        SymbolKind::Constant
    } else {
        SymbolKind::Variable
    };
    let mut symbols = Vec::new();
    for declaration in section.declarations() {
        match declaration {
            VarDeclLike::Var(node) => {
                let detail = node.type_ref().map(type_text);
                for name in node.names() {
                    symbols.extend(symbol(
                        node.syntax(),
                        Some(name),
                        kind,
                        detail.clone(),
                        Vec::new(),
                    ));
                }
            }
            VarDeclLike::Edge(node) => {
                for name in node.names() {
                    symbols.extend(symbol(node.syntax(), Some(name), kind, None, Vec::new()));
                }
            }
        }
    }
    symbols
}

fn access_symbols(section: &VarAccessSection) -> Vec<DocumentSymbol> {
    section
        .declarations()
        .iter()
        .filter_map(|declaration| {
            symbol(
                declaration.syntax(),
                declaration.name(),
                SymbolKind::Variable,
                declaration.type_ref().map(type_text),
                Vec::new(),
            )
        })
        .collect()
}

fn config_symbol(node: &ConfigDecl) -> Option<DocumentSymbol> {
    let mut children = Vec::new();
    for resource in node.resources() {
        let mut resource_children = Vec::new();
        for task in resource.tasks() {
            resource_children.extend(symbol(
                task.syntax(),
                task.name(),
                SymbolKind::Task,
                None,
                Vec::new(),
            ));
        }
        for program in resource.program_configs() {
            resource_children.extend(program_symbol(&program));
        }
        children.extend(symbol(
            resource.syntax(),
            resource.name(),
            SymbolKind::Resource,
            None,
            resource_children,
        ));
    }
    for program in node.program_configs() {
        children.extend(program_symbol(&program));
    }
    symbol(
        node.syntax(),
        node.name(),
        SymbolKind::Configuration,
        None,
        children,
    )
}

fn program_symbol(node: &ProgramConfig) -> Option<DocumentSymbol> {
    symbol(
        node.syntax(),
        node.name(),
        SymbolKind::ProgramInstance,
        node.program_path().map(|path| path.text().to_owned()),
        Vec::new(),
    )
}

fn symbol(
    node: &SyntaxNode,
    name: Option<Name>,
    kind: SymbolKind,
    detail: Option<String>,
    children: Vec<DocumentSymbol>,
) -> Option<DocumentSymbol> {
    let name = name?;
    let token = name.token()?;
    Some(DocumentSymbol {
        name: name.text().to_owned(),
        kind,
        detail,
        range: node_range(node),
        selection: token.span(),
        children,
    })
}

fn path_symbol(
    node: &SyntaxNode,
    path: Option<stc_syntax::ast::Path>,
    kind: SymbolKind,
    detail: Option<String>,
    children: Vec<DocumentSymbol>,
) -> Option<DocumentSymbol> {
    let path = path?;
    Some(DocumentSymbol {
        name: path.text().to_owned(),
        kind,
        detail,
        range: node_range(node),
        selection: path.syntax().token_span(),
        children,
    })
}

fn node_range(node: &SyntaxNode) -> Span {
    let span = node.token_span();
    if span.is_empty() { node.span() } else { span }
}

fn type_text(type_ref: stc_syntax::ast::TypeRef) -> String {
    type_ref.text().to_owned()
}
