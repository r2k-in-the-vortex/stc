//! Typed AST views over CST nodes (AST §4).
//!
//! These are zero-cost handles, not a second tree. Accessors return `Option`
//! because a malformed tree is still representable; the parser guarantees the
//! shape of trees it produces.

use stc_span::Span;

use crate::{PragmaAnnotation, SyntaxKind, SyntaxNode, SyntaxToken};

/// A typed view over a CST node.
pub trait AstNode: Clone {
    /// Casts a CST node to this view.
    fn cast(node: &SyntaxNode) -> Option<Self>;

    /// Returns the underlying CST node.
    fn syntax(&self) -> &SyntaxNode;

    /// Returns the node span (including attached trivia).
    fn span(&self) -> Span {
        self.syntax().span()
    }

    /// Returns the token-only extent, suitable for diagnostics.
    fn token_span(&self) -> Span {
        self.syntax().token_span()
    }
}

macro_rules! ast_node {
    ($name:ident, $kind:ident) => {
        #[doc = concat!("Typed view of a `", stringify!($kind), "` node.")]
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            fn cast(node: &SyntaxNode) -> Option<Self> {
                (node.kind() == SyntaxKind::$kind).then(|| Self(node.clone()))
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

ast_node!(SourceFile, SourceFile);
ast_node!(UsingDirective, UsingDirective);
ast_node!(NamespaceDecl, NamespaceDecl);
ast_node!(DataTypeDecl, DataTypeDecl);
ast_node!(SimpleTypeDecl, SimpleTypeDecl);
ast_node!(SubrangeTypeDecl, SubrangeTypeDecl);
ast_node!(EnumTypeDecl, EnumTypeDecl);
ast_node!(EnumValue, EnumValue);
ast_node!(NamedValueTypeDecl, NamedValueTypeDecl);
ast_node!(NamedValue, NamedValue);
ast_node!(ArrayTypeDecl, ArrayTypeDecl);
ast_node!(StructTypeDecl, StructTypeDecl);
ast_node!(StructElem, StructElem);
ast_node!(StringTypeDecl, StringTypeDecl);
ast_node!(RefTypeDecl, RefTypeDecl);
ast_node!(VarSection, VarSection);
ast_node!(VarDecl, VarDecl);
ast_node!(EdgeDecl, EdgeDecl);
ast_node!(AccessDecl, AccessDecl);
ast_node!(FunctionDecl, FunctionDecl);
ast_node!(FunctionBlockDecl, FunctionBlockDecl);
ast_node!(ProgramDecl, ProgramDecl);
ast_node!(ClassDecl, ClassDecl);
ast_node!(InterfaceDecl, InterfaceDecl);
ast_node!(MethodDecl, MethodDecl);
ast_node!(MethodPrototype, MethodPrototype);
ast_node!(ConfigDecl, ConfigDecl);
ast_node!(ResourceDecl, ResourceDecl);
ast_node!(TaskConfig, TaskConfig);
ast_node!(TaskAttr, TaskAttr);
ast_node!(ProgramConfig, ProgramConfig);
ast_node!(ProgConfElem, ProgConfElem);
ast_node!(VarAccessSection, VarAccessSection);
ast_node!(ConfigInit, ConfigInit);
ast_node!(ConfigInstInit, ConfigInstInit);
ast_node!(StmtList, StmtList);
ast_node!(AssignStmt, AssignStmt);
ast_node!(RefAssignStmt, RefAssignStmt);
ast_node!(AssignmentAttemptStmt, AssignmentAttemptStmt);
ast_node!(CallStmt, CallStmt);
ast_node!(ReturnStmt, ReturnStmt);
ast_node!(IfStmt, IfStmt);
ast_node!(CaseStmt, CaseStmt);
ast_node!(CaseSelection, CaseSelection);
ast_node!(CaseElement, CaseElement);
ast_node!(CaseLabel, CaseLabel);
ast_node!(ForStmt, ForStmt);
ast_node!(WhileStmt, WhileStmt);
ast_node!(RepeatStmt, RepeatStmt);
ast_node!(ExitStmt, ExitStmt);
ast_node!(ContinueStmt, ContinueStmt);
ast_node!(EmptyStmt, EmptyStmt);
ast_node!(ElseBranch, ElseBranch);
ast_node!(BinaryExpr, BinaryExpr);
ast_node!(UnaryExpr, UnaryExpr);
ast_node!(CallExpr, CallExpr);
ast_node!(MemberExpr, MemberExpr);
ast_node!(IndexExpr, IndexExpr);
ast_node!(DerefExpr, DerefExpr);
ast_node!(ParenExpr, ParenExpr);
ast_node!(LiteralExpr, LiteralExpr);
ast_node!(PathExpr, PathExpr);
ast_node!(RefValueExpr, RefValueExpr);
ast_node!(Name, Name);
ast_node!(Path, Path);
ast_node!(TypeRef, TypeRef);
ast_node!(ParamList, ParamList);
ast_node!(Param, Param);
ast_node!(Pragma, Pragma);
ast_node!(ErrorNode, Error);
ast_node!(ArrayInit, ArrayInit);
ast_node!(StructInit, StructInit);
ast_node!(StructFieldInit, StructFieldInit);
ast_node!(ArrayBound, ArrayBound);

/// One top-level source item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// `USING`.
    Using(UsingDirective),
    /// `NAMESPACE`.
    Namespace(NamespaceDecl),
    /// `TYPE`.
    DataType(DataTypeDecl),
    /// `FUNCTION`.
    Function(FunctionDecl),
    /// `FUNCTION_BLOCK`.
    FunctionBlock(FunctionBlockDecl),
    /// `PROGRAM`.
    Program(ProgramDecl),
    /// `CLASS`.
    Class(ClassDecl),
    /// `INTERFACE`.
    Interface(InterfaceDecl),
    /// `CONFIGURATION`.
    Configuration(ConfigDecl),
    /// A global variable section.
    GlobalVars(VarSection),
    /// A recovered error.
    Error(ErrorNode),
}

impl Item {
    /// Casts a CST node to an item.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::UsingDirective => Self::Using(UsingDirective::cast(node)?),
            SyntaxKind::NamespaceDecl => Self::Namespace(NamespaceDecl::cast(node)?),
            SyntaxKind::DataTypeDecl => Self::DataType(DataTypeDecl::cast(node)?),
            SyntaxKind::FunctionDecl => Self::Function(FunctionDecl::cast(node)?),
            SyntaxKind::FunctionBlockDecl => Self::FunctionBlock(FunctionBlockDecl::cast(node)?),
            SyntaxKind::ProgramDecl => Self::Program(ProgramDecl::cast(node)?),
            SyntaxKind::ClassDecl => Self::Class(ClassDecl::cast(node)?),
            SyntaxKind::InterfaceDecl => Self::Interface(InterfaceDecl::cast(node)?),
            SyntaxKind::ConfigDecl => Self::Configuration(ConfigDecl::cast(node)?),
            SyntaxKind::VarSection => Self::GlobalVars(VarSection::cast(node)?),
            SyntaxKind::Error => Self::Error(ErrorNode::cast(node)?),
            _ => return None,
        })
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Using(node) => node.syntax(),
            Self::Namespace(node) => node.syntax(),
            Self::DataType(node) => node.syntax(),
            Self::Function(node) => node.syntax(),
            Self::FunctionBlock(node) => node.syntax(),
            Self::Program(node) => node.syntax(),
            Self::Class(node) => node.syntax(),
            Self::Interface(node) => node.syntax(),
            Self::Configuration(node) => node.syntax(),
            Self::GlobalVars(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }

    /// Returns the item span.
    #[must_use]
    pub fn span(&self) -> Span {
        self.syntax().span()
    }
}

/// One data type declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeDecl {
    /// A simple type alias.
    Simple(SimpleTypeDecl),
    /// A subrange type.
    Subrange(SubrangeTypeDecl),
    /// An enumeration.
    Enum(EnumTypeDecl),
    /// An elementary type with named values.
    NamedValue(NamedValueTypeDecl),
    /// An array type.
    Array(ArrayTypeDecl),
    /// A struct type.
    Struct(StructTypeDecl),
    /// A string type.
    String(StringTypeDecl),
    /// A reference type.
    Ref(RefTypeDecl),
    /// A recovered error.
    Error(ErrorNode),
}

impl TypeDecl {
    /// Casts a CST node to a type declaration.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::SimpleTypeDecl => Self::Simple(SimpleTypeDecl::cast(node)?),
            SyntaxKind::SubrangeTypeDecl => Self::Subrange(SubrangeTypeDecl::cast(node)?),
            SyntaxKind::EnumTypeDecl => Self::Enum(EnumTypeDecl::cast(node)?),
            SyntaxKind::NamedValueTypeDecl => Self::NamedValue(NamedValueTypeDecl::cast(node)?),
            SyntaxKind::ArrayTypeDecl => Self::Array(ArrayTypeDecl::cast(node)?),
            SyntaxKind::StructTypeDecl => Self::Struct(StructTypeDecl::cast(node)?),
            SyntaxKind::StringTypeDecl => Self::String(StringTypeDecl::cast(node)?),
            SyntaxKind::RefTypeDecl => Self::Ref(RefTypeDecl::cast(node)?),
            SyntaxKind::Error => Self::Error(ErrorNode::cast(node)?),
            _ => return None,
        })
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Simple(node) => node.syntax(),
            Self::Subrange(node) => node.syntax(),
            Self::Enum(node) => node.syntax(),
            Self::NamedValue(node) => node.syntax(),
            Self::Array(node) => node.syntax(),
            Self::Struct(node) => node.syntax(),
            Self::String(node) => node.syntax(),
            Self::Ref(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }
}

/// One variable declaration form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VarDeclLike {
    /// A normal declaration.
    Var(VarDecl),
    /// An `R_EDGE`/`F_EDGE` declaration.
    Edge(EdgeDecl),
}

impl VarDeclLike {
    /// Casts a CST node.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        match node.kind() {
            SyntaxKind::VarDecl => Some(Self::Var(VarDecl::cast(node)?)),
            SyntaxKind::EdgeDecl => Some(Self::Edge(EdgeDecl::cast(node)?)),
            _ => None,
        }
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Var(node) => node.syntax(),
            Self::Edge(node) => node.syntax(),
        }
    }
}

/// One statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// Assignment.
    Assign(AssignStmt),
    /// Reference assignment.
    RefAssign(RefAssignStmt),
    /// Assignment attempt.
    Attempt(AssignmentAttemptStmt),
    /// A call used as a statement.
    Call(CallStmt),
    /// `RETURN`.
    Return(ReturnStmt),
    /// `IF`.
    If(IfStmt),
    /// `CASE`.
    Case(CaseStmt),
    /// `FOR`.
    For(ForStmt),
    /// `WHILE`.
    While(WhileStmt),
    /// `REPEAT`.
    Repeat(RepeatStmt),
    /// `EXIT`.
    Exit(ExitStmt),
    /// `CONTINUE`.
    Continue(ContinueStmt),
    /// An empty statement (`;`).
    Empty(EmptyStmt),
    /// A recovered error.
    Error(ErrorNode),
}

impl Stmt {
    /// Casts a CST node to a statement.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::AssignStmt => Self::Assign(AssignStmt::cast(node)?),
            SyntaxKind::RefAssignStmt => Self::RefAssign(RefAssignStmt::cast(node)?),
            SyntaxKind::AssignmentAttemptStmt => Self::Attempt(AssignmentAttemptStmt::cast(node)?),
            SyntaxKind::CallStmt => Self::Call(CallStmt::cast(node)?),
            SyntaxKind::ReturnStmt => Self::Return(ReturnStmt::cast(node)?),
            SyntaxKind::IfStmt => Self::If(IfStmt::cast(node)?),
            SyntaxKind::CaseStmt => Self::Case(CaseStmt::cast(node)?),
            SyntaxKind::ForStmt => Self::For(ForStmt::cast(node)?),
            SyntaxKind::WhileStmt => Self::While(WhileStmt::cast(node)?),
            SyntaxKind::RepeatStmt => Self::Repeat(RepeatStmt::cast(node)?),
            SyntaxKind::ExitStmt => Self::Exit(ExitStmt::cast(node)?),
            SyntaxKind::ContinueStmt => Self::Continue(ContinueStmt::cast(node)?),
            SyntaxKind::EmptyStmt => Self::Empty(EmptyStmt::cast(node)?),
            SyntaxKind::Error => Self::Error(ErrorNode::cast(node)?),
            _ => return None,
        })
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Assign(node) => node.syntax(),
            Self::RefAssign(node) => node.syntax(),
            Self::Attempt(node) => node.syntax(),
            Self::Call(node) => node.syntax(),
            Self::Return(node) => node.syntax(),
            Self::If(node) => node.syntax(),
            Self::Case(node) => node.syntax(),
            Self::For(node) => node.syntax(),
            Self::While(node) => node.syntax(),
            Self::Repeat(node) => node.syntax(),
            Self::Exit(node) => node.syntax(),
            Self::Continue(node) => node.syntax(),
            Self::Empty(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }
}

/// One expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// Binary operation.
    Binary(BinaryExpr),
    /// Unary operation.
    Unary(UnaryExpr),
    /// A call.
    Call(CallExpr),
    /// Member access.
    Member(MemberExpr),
    /// Subscripting.
    Index(IndexExpr),
    /// Dereference.
    Deref(DerefExpr),
    /// Parenthesized expression.
    Paren(ParenExpr),
    /// A literal.
    Literal(LiteralExpr),
    /// A name/path.
    Path(PathExpr),
    /// `REF(...)` or `NULL`.
    RefValue(RefValueExpr),
    /// A recovered error.
    Error(ErrorNode),
}

impl Expr {
    /// Casts a CST node to an expression.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::BinaryExpr => Self::Binary(BinaryExpr::cast(node)?),
            SyntaxKind::UnaryExpr => Self::Unary(UnaryExpr::cast(node)?),
            SyntaxKind::CallExpr => Self::Call(CallExpr::cast(node)?),
            SyntaxKind::MemberExpr => Self::Member(MemberExpr::cast(node)?),
            SyntaxKind::IndexExpr => Self::Index(IndexExpr::cast(node)?),
            SyntaxKind::DerefExpr => Self::Deref(DerefExpr::cast(node)?),
            SyntaxKind::ParenExpr => Self::Paren(ParenExpr::cast(node)?),
            SyntaxKind::LiteralExpr => Self::Literal(LiteralExpr::cast(node)?),
            SyntaxKind::PathExpr => Self::Path(PathExpr::cast(node)?),
            SyntaxKind::RefValueExpr => Self::RefValue(RefValueExpr::cast(node)?),
            SyntaxKind::Error => Self::Error(ErrorNode::cast(node)?),
            _ => return None,
        })
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Binary(node) => node.syntax(),
            Self::Unary(node) => node.syntax(),
            Self::Call(node) => node.syntax(),
            Self::Member(node) => node.syntax(),
            Self::Index(node) => node.syntax(),
            Self::Deref(node) => node.syntax(),
            Self::Paren(node) => node.syntax(),
            Self::Literal(node) => node.syntax(),
            Self::Path(node) => node.syntax(),
            Self::RefValue(node) => node.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }

    /// Returns the expression span.
    #[must_use]
    pub fn span(&self) -> Span {
        self.syntax().span()
    }

    /// Returns the token-only extent, suitable for diagnostics.
    #[must_use]
    pub fn token_span(&self) -> Span {
        self.syntax().token_span()
    }
}

/// One initializer value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Initializer {
    /// An expression initializer.
    Expr(Expr),
    /// `[a, b, ...]`.
    Array(ArrayInit),
    /// `(field := value, ...)`.
    Struct(StructInit),
    /// A recovered error.
    Error(ErrorNode),
}

impl Initializer {
    /// Casts a CST node to an initializer.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::ArrayInit => Self::Array(ArrayInit::cast(node)?),
            SyntaxKind::StructInit => Self::Struct(StructInit::cast(node)?),
            SyntaxKind::Error => Self::Error(ErrorNode::cast(node)?),
            _ => Self::Expr(Expr::cast(node)?),
        })
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Expr(expr) => expr.syntax(),
            Self::Array(array) => array.syntax(),
            Self::Struct(structure) => structure.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }

    /// Returns the initializer span.
    #[must_use]
    pub fn span(&self) -> Span {
        self.syntax().span()
    }

    /// Returns the token-only extent.
    #[must_use]
    pub fn token_span(&self) -> Span {
        self.syntax().token_span()
    }
}

/// One call argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// Positional input argument.
    Positional(Expr),
    /// Named input argument (`name := value`).
    Named(Name, Expr),
    /// Output binding (`value => target`).
    Output(Expr, Expr),
    /// Negated output binding (`NOT value => target`).
    NegatedOutput(Expr, Expr),
    /// A reference assignment argument.
    RefAssign(Name, Expr),
    /// A recovered error.
    Error(ErrorNode),
}

impl Arg {
    /// Casts a `Param` node to an argument.
    #[must_use]
    pub fn cast(node: &SyntaxNode) -> Option<Self> {
        let param = Param::cast(node)?;
        let expressions = param.expressions();
        if param
            .syntax()
            .has_direct_token(SyntaxKind::PunctOutputAssign)
        {
            let mut expressions = expressions.into_iter();
            let source = expressions.next()?;
            let target = expressions.next()?;
            let negated = matches!(
                &source,
                Expr::Unary(unary)
                    if unary
                        .operator()
                        .is_some_and(|token| token.kind() == SyntaxKind::KwNot)
            );
            return Some(if negated {
                Self::NegatedOutput(source, target)
            } else {
                Self::Output(source, target)
            });
        }
        if let Some(name) = param.name() {
            let value = expressions.into_iter().next()?;
            return Some(Self::Named(name, value));
        }
        expressions.into_iter().next().map(Self::Positional)
    }

    /// Returns the underlying node.
    #[must_use]
    pub fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Positional(expr)
            | Self::Output(expr, _)
            | Self::NegatedOutput(expr, _)
            | Self::RefAssign(_, expr) => expr.syntax(),
            Self::Named(name, _) => name.syntax(),
            Self::Error(node) => node.syntax(),
        }
    }
}

/// Kinds of program organization unit that can enclose a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PouKind {
    /// `FUNCTION`.
    Function,
    /// `FUNCTION_BLOCK`.
    FunctionBlock,
    /// `PROGRAM`.
    Program,
    /// `CLASS`.
    Class,
    /// `INTERFACE`.
    Interface,
}

impl SourceFile {
    /// Returns the top-level items in source order.
    #[must_use]
    pub fn items(&self) -> Vec<Item> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Item::cast)
            .collect()
    }

    /// Returns leading pragmas that could not be attached to an item.
    #[must_use]
    pub fn pragmas(&self) -> Vec<Pragma> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Pragma::cast)
            .collect()
    }

    /// Returns the item node with the given name, if any.
    #[must_use]
    pub fn item_named(&self, name: &str) -> Option<Item> {
        self.items().into_iter().find(|item| {
            declaration_name(item.syntax())
                .is_some_and(|candidate| candidate.text().eq_ignore_ascii_case(name))
        })
    }
}

impl Pragma {
    /// Returns the parsed annotation, when the pragma was valid.
    #[must_use]
    pub fn annotation(&self) -> Option<&PragmaAnnotation> {
        let tree = self.syntax().tree();
        let token = self.syntax().first_token_of_kind(SyntaxKind::PragmaToken)?;
        tree.pragma_annotation(token.id())
    }

    /// Returns the pragma token text including braces.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }
}

impl Name {
    /// Returns the identifier spelling as written.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }

    /// Returns the case-insensitive symbol key.
    #[must_use]
    pub fn normalized(&self) -> String {
        self.text().to_ascii_uppercase()
    }

    /// Returns the identifier token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        self.syntax().first_token_of_kind(SyntaxKind::Ident)
    }
}

impl Path {
    /// Returns the path segments in order.
    #[must_use]
    pub fn segments(&self) -> Vec<Name> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Name::cast)
            .collect()
    }

    /// Returns the path text without attached trivia.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }
}

impl UsingDirective {
    /// Returns the imported namespaces.
    #[must_use]
    pub fn namespaces(&self) -> Vec<Path> {
        child_views(self.syntax(), Path::cast)
    }
}

impl NamespaceDecl {
    /// Returns the declared namespace name.
    #[must_use]
    pub fn name(&self) -> Option<Path> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Path)
            .and_then(|node| Path::cast(&node))
    }

    /// Returns `true` when the `INTERNAL` modifier is present.
    #[must_use]
    pub fn is_internal(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwInternal)
    }

    /// Returns the using directives.
    #[must_use]
    pub fn using_directives(&self) -> Vec<UsingDirective> {
        child_views(self.syntax(), UsingDirective::cast)
    }

    /// Returns the nested elements (types, POUs, namespaces).
    ///
    /// `USING` directives are not namespace elements and are excluded.
    #[must_use]
    pub fn elements(&self) -> Vec<Item> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Item::cast)
            .filter(|item| !matches!(item, Item::Using(_)))
            .collect()
    }
}

impl DataTypeDecl {
    /// Returns the declared types.
    #[must_use]
    pub fn types(&self) -> Vec<TypeDecl> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(TypeDecl::cast)
            .collect()
    }
}

impl EnumTypeDecl {
    /// Returns the enum values.
    #[must_use]
    pub fn values(&self) -> Vec<EnumValue> {
        child_views(self.syntax(), EnumValue::cast)
    }
}

impl NamedValueTypeDecl {
    /// Returns the named values.
    #[must_use]
    pub fn values(&self) -> Vec<NamedValue> {
        child_views(self.syntax(), NamedValue::cast)
    }
}

impl StructTypeDecl {
    /// Returns the struct elements.
    #[must_use]
    pub fn elements(&self) -> Vec<StructElem> {
        child_views(self.syntax(), StructElem::cast)
    }

    /// Returns `true` when `OVERLAP` is present.
    #[must_use]
    pub fn is_overlap(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwOverlap)
            || self
                .type_ref()
                .is_some_and(|type_ref| type_ref.syntax().has_direct_token(SyntaxKind::KwOverlap))
    }
}

impl StructElem {
    /// Returns the element name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Name)
            .and_then(|node| Name::cast(&node))
    }

    /// Returns the element type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }
}

impl VarSection {
    /// Returns the section kind token, for example `VAR_INPUT`.
    #[must_use]
    pub fn kind(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::KwVar
                    | SyntaxKind::KwVarInput
                    | SyntaxKind::KwVarOutput
                    | SyntaxKind::KwVarInOut
                    | SyntaxKind::KwVarTemp
                    | SyntaxKind::KwVarExternal
                    | SyntaxKind::KwVarGlobal
            )
        })
    }

    /// Returns the qualifier tokens (`CONSTANT`, `RETAIN`, access specifiers).
    #[must_use]
    pub fn qualifiers(&self) -> Vec<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .filter(|token| {
                matches!(
                    token.kind(),
                    SyntaxKind::KwConstant
                        | SyntaxKind::KwRetain
                        | SyntaxKind::KwNonRetain
                        | SyntaxKind::KwPublic
                        | SyntaxKind::KwProtected
                        | SyntaxKind::KwPrivate
                        | SyntaxKind::KwInternal
                )
            })
            .collect()
    }

    /// Returns the declarations in this section.
    #[must_use]
    pub fn declarations(&self) -> Vec<VarDeclLike> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(VarDeclLike::cast)
            .collect()
    }
}

impl VarDecl {
    /// Returns the declared names.
    #[must_use]
    pub fn names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns `true` when the declaration has an `AT` binding.
    #[must_use]
    pub fn has_at(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwAt)
    }

    /// Returns the initializer expression.
    #[must_use]
    pub fn initializer(&self) -> Option<Expr> {
        self.syntax().child_nodes().iter().find_map(Expr::cast)
    }

    /// Returns leading and trailing pragmas.
    #[must_use]
    pub fn pragmas(&self) -> Vec<Pragma> {
        child_views(self.syntax(), Pragma::cast)
    }
}

impl EdgeDecl {
    /// Returns the declared names.
    #[must_use]
    pub fn names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the edge kind token (`R_EDGE` or `F_EDGE`).
    #[must_use]
    pub fn edge_kind(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| matches!(token.kind(), SyntaxKind::KwREdge | SyntaxKind::KwFEdge))
    }

    /// Returns the pragmas.
    #[must_use]
    pub fn pragmas(&self) -> Vec<Pragma> {
        child_views(self.syntax(), Pragma::cast)
    }
}

impl AccessDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Name)
            .and_then(|node| Name::cast(&node))
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }
}

impl FunctionDecl {
    /// Returns the function name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the result type.
    #[must_use]
    pub fn result_type(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the variable sections.
    #[must_use]
    pub fn var_sections(&self) -> Vec<VarSection> {
        child_views(self.syntax(), VarSection::cast)
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }

    /// Returns the using directives.
    #[must_use]
    pub fn using_directives(&self) -> Vec<UsingDirective> {
        child_views(self.syntax(), UsingDirective::cast)
    }

    /// Returns the pragmas.
    #[must_use]
    pub fn pragmas(&self) -> Vec<Pragma> {
        child_views(self.syntax(), Pragma::cast)
    }
}

macro_rules! pou_common {
    ($name:ident) => {
        impl $name {
            /// Returns the POU name.
            #[must_use]
            pub fn name(&self) -> Option<Name> {
                declaration_name(self.syntax())
            }

            /// Returns the variable sections.
            #[must_use]
            pub fn var_sections(&self) -> Vec<VarSection> {
                child_views(self.syntax(), VarSection::cast)
            }

            /// Returns the using directives.
            #[must_use]
            pub fn using_directives(&self) -> Vec<UsingDirective> {
                child_views(self.syntax(), UsingDirective::cast)
            }

            /// Returns the pragmas.
            #[must_use]
            pub fn pragmas(&self) -> Vec<Pragma> {
                child_views(self.syntax(), Pragma::cast)
            }
        }
    };
}

pou_common!(FunctionBlockDecl);
pou_common!(ProgramDecl);
pou_common!(ClassDecl);
pou_common!(InterfaceDecl);

impl FunctionBlockDecl {
    /// Returns the methods.
    #[must_use]
    pub fn methods(&self) -> Vec<MethodDecl> {
        child_views(self.syntax(), MethodDecl::cast)
    }

    /// Returns the body, when present.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }

    /// Returns the base type name.
    #[must_use]
    pub fn extends(&self) -> Option<TypeRef> {
        extends_type(self.syntax())
    }

    /// Returns `true` when the FB is `FINAL`.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwFinal)
    }

    /// Returns `true` when the FB is `ABSTRACT`.
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwAbstract)
    }

    /// Returns the implemented interfaces.
    #[must_use]
    pub fn implements(&self) -> Vec<Path> {
        implements_list(self.syntax())
    }
}

impl ProgramDecl {
    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }

    /// Returns the `VAR_ACCESS` section, when present.
    #[must_use]
    pub fn var_access(&self) -> Option<VarAccessSection> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::VarAccessSection)
            .and_then(|node| VarAccessSection::cast(&node))
    }
}

impl ClassDecl {
    /// Returns the methods.
    #[must_use]
    pub fn methods(&self) -> Vec<MethodDecl> {
        child_views(self.syntax(), MethodDecl::cast)
    }

    /// Returns the base class name.
    #[must_use]
    pub fn extends(&self) -> Option<TypeRef> {
        extends_type(self.syntax())
    }

    /// Returns `true` when the class is `FINAL`.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwFinal)
    }

    /// Returns `true` when the class is `ABSTRACT`.
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwAbstract)
    }

    /// Returns the implemented interfaces.
    #[must_use]
    pub fn implements(&self) -> Vec<Path> {
        implements_list(self.syntax())
    }
}

impl InterfaceDecl {
    /// Returns the method prototypes.
    #[must_use]
    pub fn prototypes(&self) -> Vec<MethodPrototype> {
        child_views(self.syntax(), MethodPrototype::cast)
    }

    /// Returns the extended interfaces.
    #[must_use]
    pub fn extends(&self) -> Vec<Path> {
        child_views(self.syntax(), Path::cast)
    }
}

impl MethodDecl {
    /// Returns the method name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the result type.
    #[must_use]
    pub fn result_type(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the variable sections.
    #[must_use]
    pub fn var_sections(&self) -> Vec<VarSection> {
        child_views(self.syntax(), VarSection::cast)
    }

    /// Returns the body, when present.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }

    /// Returns the access specifier token.
    #[must_use]
    pub fn access(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::KwPublic
                    | SyntaxKind::KwProtected
                    | SyntaxKind::KwPrivate
                    | SyntaxKind::KwInternal
            )
        })
    }

    /// Returns the using directives.
    #[must_use]
    pub fn using_directives(&self) -> Vec<UsingDirective> {
        child_views(self.syntax(), UsingDirective::cast)
    }

    /// Returns the pragmas.
    #[must_use]
    pub fn pragmas(&self) -> Vec<Pragma> {
        child_views(self.syntax(), Pragma::cast)
    }

    /// Returns `true` when `OVERRIDE` is present.
    #[must_use]
    pub fn is_override(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwOverride)
    }

    /// Returns `true` when `FINAL` is present.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwFinal)
    }

    /// Returns `true` when `ABSTRACT` is present.
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwAbstract)
    }

    /// Returns the enclosing POU kind.
    #[must_use]
    pub fn pou_kind(&self) -> Option<PouKind> {
        let mut current = self.syntax().parent();
        while let Some(node) = current {
            match node.kind() {
                SyntaxKind::FunctionDecl => return Some(PouKind::Function),
                SyntaxKind::FunctionBlockDecl => return Some(PouKind::FunctionBlock),
                SyntaxKind::ProgramDecl => return Some(PouKind::Program),
                SyntaxKind::ClassDecl => return Some(PouKind::Class),
                SyntaxKind::InterfaceDecl => return Some(PouKind::Interface),
                _ => {}
            }
            current = node.parent();
        }
        None
    }
}

impl MethodPrototype {
    /// Returns the prototype name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the result type.
    #[must_use]
    pub fn result_type(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the variable sections.
    #[must_use]
    pub fn var_sections(&self) -> Vec<VarSection> {
        child_views(self.syntax(), VarSection::cast)
    }
}

impl ConfigDecl {
    /// Returns the configuration name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the resources.
    #[must_use]
    pub fn resources(&self) -> Vec<ResourceDecl> {
        child_views(self.syntax(), ResourceDecl::cast)
    }

    /// Returns the single-resource declarations owned directly by the
    /// configuration.
    #[must_use]
    pub fn program_configs(&self) -> Vec<ProgramConfig> {
        child_views(self.syntax(), ProgramConfig::cast)
    }

    /// Returns the `VAR_ACCESS` section.
    #[must_use]
    pub fn var_access(&self) -> Option<VarAccessSection> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::VarAccessSection)
            .and_then(|node| VarAccessSection::cast(&node))
    }

    /// Returns the `VAR_CONFIG` section.
    #[must_use]
    pub fn config_init(&self) -> Option<ConfigInit> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::ConfigInit)
            .and_then(|node| ConfigInit::cast(&node))
    }
}

impl ResourceDecl {
    /// Returns the resource name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the tasks.
    #[must_use]
    pub fn tasks(&self) -> Vec<TaskConfig> {
        child_views(self.syntax(), TaskConfig::cast)
    }

    /// Returns the program configurations.
    #[must_use]
    pub fn program_configs(&self) -> Vec<ProgramConfig> {
        child_views(self.syntax(), ProgramConfig::cast)
    }
}

impl VarAccessSection {
    /// Returns the access declarations.
    #[must_use]
    pub fn declarations(&self) -> Vec<AccessDecl> {
        child_views(self.syntax(), AccessDecl::cast)
    }
}

impl ConfigInit {
    /// Returns the configured instances.
    #[must_use]
    pub fn instances(&self) -> Vec<ConfigInstInit> {
        child_views(self.syntax(), ConfigInstInit::cast)
    }
}

impl StmtList {
    /// Returns the statements, including empty statements.
    #[must_use]
    pub fn statements(&self) -> Vec<Stmt> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Stmt::cast)
            .collect()
    }
}

macro_rules! binary_expr {
    ($name:ident) => {
        impl $name {
            /// Returns the left operand.
            #[must_use]
            pub fn lhs(&self) -> Option<Expr> {
                expr_children(self.syntax()).into_iter().next()
            }

            /// Returns the right operand.
            #[must_use]
            pub fn rhs(&self) -> Option<Expr> {
                expr_children(self.syntax()).into_iter().nth(1)
            }

            /// Returns the operator token.
            #[must_use]
            pub fn operator(&self) -> Option<SyntaxToken> {
                operator_token(self.syntax())
            }
        }
    };
}

binary_expr!(BinaryExpr);
binary_expr!(AssignStmt);
binary_expr!(RefAssignStmt);
binary_expr!(AssignmentAttemptStmt);

impl UnaryExpr {
    /// Returns the operand.
    #[must_use]
    pub fn operand(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the operator token.
    #[must_use]
    pub fn operator(&self) -> Option<SyntaxToken> {
        operator_token(self.syntax())
    }
}

impl CallStmt {
    /// Returns the call expression.
    #[must_use]
    pub fn call(&self) -> Option<CallExpr> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::CallExpr)
            .and_then(|node| CallExpr::cast(&node))
    }
}

impl ReturnStmt {
    /// Returns `true` when a value follows `RETURN`.
    #[must_use]
    pub fn has_value(&self) -> bool {
        self.syntax()
            .child_nodes()
            .iter()
            .any(|node| Expr::cast(node).is_some())
    }
}

impl IfStmt {
    /// Returns the `IF` condition.
    #[must_use]
    pub fn condition(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the `THEN` block.
    #[must_use]
    pub fn then_block(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }

    /// Returns the `ELSIF` branches in order.
    #[must_use]
    pub fn elsif_branches(&self) -> Vec<ElseBranch> {
        child_views(self.syntax(), ElseBranch::cast)
    }

    /// Returns the `ELSE` block.
    #[must_use]
    pub fn else_block(&self) -> Option<StmtList> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(StmtList::cast)
            .nth(1)
    }
}

impl ElseBranch {
    /// Returns the branch condition.
    #[must_use]
    pub fn condition(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the branch body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }
}

impl CaseStmt {
    /// Returns the selector expression.
    #[must_use]
    pub fn selector(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the selections.
    #[must_use]
    pub fn selections(&self) -> Vec<CaseSelection> {
        child_views(self.syntax(), CaseSelection::cast)
    }

    /// Returns the `ELSE` block.
    #[must_use]
    pub fn else_block(&self) -> Option<StmtList> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(StmtList::cast)
            .next_back()
    }
}

impl CaseSelection {
    /// Returns the labels.
    #[must_use]
    pub fn labels(&self) -> Vec<CaseLabel> {
        child_views(self.syntax(), CaseLabel::cast)
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }
}

impl CaseLabel {
    /// Returns the label expressions (one, or two for a range).
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        expr_children(self.syntax())
    }
}

impl ForStmt {
    /// Returns the loop variable name.
    #[must_use]
    pub fn variable(&self) -> Option<Name> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Name)
            .and_then(|node| Name::cast(&node))
    }

    /// Returns start, end, and optional step expressions.
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        expr_children(self.syntax())
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }
}

impl WhileStmt {
    /// Returns the condition.
    #[must_use]
    pub fn condition(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }
}

impl RepeatStmt {
    /// Returns the condition.
    #[must_use]
    pub fn condition(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().last()
    }

    /// Returns the body.
    #[must_use]
    pub fn body(&self) -> Option<StmtList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::StmtList)
            .and_then(|node| StmtList::cast(&node))
    }
}

impl CallExpr {
    /// Returns the callee expression.
    #[must_use]
    pub fn callee(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the arguments.
    #[must_use]
    pub fn arguments(&self) -> Vec<Arg> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::ParamList)
            .map(|node| child_views(&node, Arg::cast))
            .unwrap_or_default()
    }

    /// Returns the argument list node.
    #[must_use]
    pub fn param_list(&self) -> Option<ParamList> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::ParamList)
            .and_then(|node| ParamList::cast(&node))
    }
}

impl MemberExpr {
    /// Returns the base expression.
    #[must_use]
    pub fn base(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the member name, when the access uses an identifier.
    #[must_use]
    pub fn member(&self) -> Option<Name> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Name)
            .and_then(|node| Name::cast(&node))
    }

    /// Returns the bit index token for partial bit access (`a.0`, `a.%X0`).
    #[must_use]
    pub fn bit_index(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::IntLiteral | SyntaxKind::DirectVariable
            )
        })
    }
}

impl IndexExpr {
    /// Returns the base expression.
    #[must_use]
    pub fn base(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }

    /// Returns the subscript expressions.
    #[must_use]
    pub fn indices(&self) -> Vec<Expr> {
        expr_children(self.syntax()).into_iter().skip(1).collect()
    }
}

impl DerefExpr {
    /// Returns the dereferenced expression.
    #[must_use]
    pub fn operand(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl ParenExpr {
    /// Returns the inner expression.
    #[must_use]
    pub fn inner(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl LiteralExpr {
    /// Returns the literal token.
    #[must_use]
    pub fn token(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().next()
    }

    /// Returns the literal source text.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }
}

impl PathExpr {
    /// Returns the path segments, including a leading `THIS`/`SUPER` when
    /// present.
    #[must_use]
    pub fn segments(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the path text without attached trivia.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }
}

impl RefValueExpr {
    /// Returns `true` for `NULL`.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwNull)
    }

    /// Returns the referenced expression or name.
    #[must_use]
    pub fn target(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl TypeRef {
    /// Returns the type text without attached trivia.
    #[must_use]
    pub fn text(&self) -> &str {
        let node = self.syntax();
        node.tree().text(node.token_span())
    }

    /// Returns the referenced path, when the type is a named type.
    #[must_use]
    pub fn path(&self) -> Option<Path> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Path)
            .and_then(|node| Path::cast(&node))
    }

    /// Returns the array bounds, when this is an array type.
    #[must_use]
    pub fn bounds(&self) -> Vec<ArrayBound> {
        child_views(self.syntax(), ArrayBound::cast)
    }
}

impl Param {
    /// Returns the argument name (`name :=` form).
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Name)
            .and_then(|node| Name::cast(&node))
    }

    /// Returns the expressions in source order.
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        expr_children(self.syntax())
    }
}

impl ParamList {
    /// Returns the parameters.
    #[must_use]
    pub fn params(&self) -> Vec<Param> {
        child_views(self.syntax(), Param::cast)
    }
}

impl ErrorNode {
    /// Builds an error view for a node that the parser marked as recovered.
    #[must_use]
    pub fn new(node: SyntaxNode) -> Option<Self> {
        Self::cast(&node)
    }
}

impl VarDecl {
    /// Returns the initializer, including aggregate forms.
    #[must_use]
    pub fn initializer_node(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }

    /// Returns the direct-variable address token after `AT`.
    #[must_use]
    pub fn at_token(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| token.kind() == SyntaxKind::DirectVariable)
    }
}

impl StructElem {
    /// Returns `true` when the element has an `AT` binding.
    #[must_use]
    pub fn has_at(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwAt)
    }

    /// Returns the direct-variable address token after `AT`.
    #[must_use]
    pub fn at_token(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| token.kind() == SyntaxKind::DirectVariable)
    }

    /// Returns the initializer, including aggregate forms.
    #[must_use]
    pub fn initializer_node(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl ArrayInit {
    /// Returns the element initializers.
    #[must_use]
    pub fn elements(&self) -> Vec<Initializer> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(Initializer::cast)
            .collect()
    }
}

impl StructInit {
    /// Returns the field initializers.
    #[must_use]
    pub fn fields(&self) -> Vec<StructFieldInit> {
        child_views(self.syntax(), StructFieldInit::cast)
    }
}

impl StructFieldInit {
    /// Returns the field path segments.
    #[must_use]
    pub fn names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the initializer value.
    #[must_use]
    pub fn value(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl ArrayBound {
    /// Returns `true` for the `*` conformant bound.
    #[must_use]
    pub fn is_star(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::PunctStar)
    }

    /// Returns the bound expressions (one, or two for a range).
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        expr_children(self.syntax())
    }
}

impl TypeRef {
    /// Returns the nested type for `ARRAY ... OF` and `REF_TO`.
    #[must_use]
    pub fn element_type(&self) -> Option<TypeRef> {
        self.syntax()
            .child_nodes()
            .iter()
            .filter_map(TypeRef::cast)
            .next()
    }

    /// Returns the struct elements of an inline struct type.
    #[must_use]
    pub fn struct_elements(&self) -> Vec<StructElem> {
        child_views(self.syntax(), StructElem::cast)
    }
}

impl SimpleTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the type's default initializer.
    #[must_use]
    pub fn initializer(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl ArrayTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the type's default initializer.
    #[must_use]
    pub fn initializer(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl StringTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the type's default initializer.
    #[must_use]
    pub fn initializer(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl RefTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the type's default initializer.
    #[must_use]
    pub fn initializer(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl StructTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the type's default initializer.
    #[must_use]
    pub fn initializer(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl SubrangeTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the integer base type token.
    #[must_use]
    pub fn base_token(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::KwSint
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
            )
        })
    }

    /// Returns the lower and upper bound expressions.
    #[must_use]
    pub fn bounds(&self) -> (Option<Expr>, Option<Expr>) {
        let mut expressions = expr_children(self.syntax()).into_iter();
        (expressions.next(), expressions.next())
    }

    /// Returns the declared default value.
    #[must_use]
    pub fn default_value(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().nth(2)
    }
}

impl EnumTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the declared default value name.
    #[must_use]
    pub fn default_value(&self) -> Option<Name> {
        let names: Vec<Name> = child_views(self.syntax(), Name::cast);
        names.into_iter().nth(1)
    }
}

impl NamedValueTypeDecl {
    /// Returns the declared name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the underlying elementary type keyword.
    #[must_use]
    pub fn base_token(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| token.kind() != SyntaxKind::PunctColon)
    }

    /// Returns the underlying elementary type path, when named.
    #[must_use]
    pub fn base_path(&self) -> Option<Path> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Path)
            .and_then(|node| Path::cast(&node))
    }

    /// Returns the declared default value name.
    #[must_use]
    pub fn default_value(&self) -> Option<Name> {
        let names: Vec<Name> = child_views(self.syntax(), Name::cast);
        names.into_iter().nth(1)
    }
}

impl EnumValue {
    /// Returns the value name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the explicit value expression, if any.
    #[must_use]
    pub fn value(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl NamedValue {
    /// Returns the value name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the explicit value expression, if any.
    #[must_use]
    pub fn value(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl MethodPrototype {
    /// Returns the access specifier token.
    #[must_use]
    pub fn access(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::KwPublic
                    | SyntaxKind::KwProtected
                    | SyntaxKind::KwPrivate
                    | SyntaxKind::KwInternal
            )
        })
    }
}

impl ConfigDecl {
    /// Returns the variable sections owned by the configuration.
    #[must_use]
    pub fn var_sections(&self) -> Vec<VarSection> {
        child_views(self.syntax(), VarSection::cast)
    }
}

impl ResourceDecl {
    /// Returns the `ON <processor>` name.
    #[must_use]
    pub fn on(&self) -> Option<Name> {
        let names: Vec<Name> = child_views(self.syntax(), Name::cast);
        names.into_iter().nth(1)
    }

    /// Returns the variable sections owned by the resource.
    #[must_use]
    pub fn var_sections(&self) -> Vec<VarSection> {
        child_views(self.syntax(), VarSection::cast)
    }
}

impl TaskConfig {
    /// Returns the task name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the task attributes.
    #[must_use]
    pub fn attributes(&self) -> Vec<TaskAttr> {
        child_views(self.syntax(), TaskAttr::cast)
    }
}

impl TaskAttr {
    /// Returns the attribute kind token (`SINGLE`, `INTERVAL`, `PRIORITY`).
    #[must_use]
    pub fn kind(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::KwSingle | SyntaxKind::KwInterval | SyntaxKind::KwPriority
            )
        })
    }

    /// Returns the attribute value expression.
    #[must_use]
    pub fn value(&self) -> Option<Expr> {
        expr_children(self.syntax()).into_iter().next()
    }
}

impl ProgramConfig {
    /// Returns the program instance name.
    #[must_use]
    pub fn name(&self) -> Option<Name> {
        declaration_name(self.syntax())
    }

    /// Returns the `WITH <task>` name, if present.
    #[must_use]
    pub fn with(&self) -> Option<Name> {
        let names: Vec<Name> = child_views(self.syntax(), Name::cast);
        names.into_iter().nth(1)
    }

    /// Returns the program type path after `:`.
    #[must_use]
    pub fn program_path(&self) -> Option<Path> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::Path)
            .and_then(|node| Path::cast(&node))
    }

    /// Returns the bindings.
    #[must_use]
    pub fn elements(&self) -> Vec<ProgConfElem> {
        child_views(self.syntax(), ProgConfElem::cast)
    }
}

impl ProgConfElem {
    /// Returns the destination/source expressions in source order.
    #[must_use]
    pub fn expressions(&self) -> Vec<Expr> {
        expr_children(self.syntax())
    }

    /// Returns the `IDENT WITH IDENT` names, when this is an FB task binding.
    #[must_use]
    pub fn names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the `:=`/`=>` operator token.
    #[must_use]
    pub fn operator(&self) -> Option<SyntaxToken> {
        self.syntax().child_tokens().into_iter().find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::PunctAssign | SyntaxKind::PunctOutputAssign
            )
        })
    }
}

impl AccessDecl {
    /// Returns the access path names after the declared name.
    #[must_use]
    pub fn path_names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
            .into_iter()
            .skip(1)
            .collect()
    }

    /// Returns a direct-variable address in the access path.
    #[must_use]
    pub fn direct_token(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| token.kind() == SyntaxKind::DirectVariable)
    }

    /// Returns `true` for `READ_ONLY`.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwReadOnly)
    }

    /// Returns `true` for `READ_WRITE`.
    #[must_use]
    pub fn is_read_write(&self) -> bool {
        self.syntax().has_direct_token(SyntaxKind::KwReadWrite)
    }
}

impl ConfigInstInit {
    /// Returns the dotted path segments.
    #[must_use]
    pub fn names(&self) -> Vec<Name> {
        child_views(self.syntax(), Name::cast)
    }

    /// Returns the optional `AT` address token.
    #[must_use]
    pub fn at_token(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| token.kind() == SyntaxKind::DirectVariable)
    }

    /// Returns the declared type.
    #[must_use]
    pub fn type_ref(&self) -> Option<TypeRef> {
        self.syntax()
            .first_child_of_kind(SyntaxKind::TypeRef)
            .and_then(|node| TypeRef::cast(&node))
    }

    /// Returns the initializer, including aggregate forms.
    #[must_use]
    pub fn initializer_node(&self) -> Option<Initializer> {
        initializer_child(self.syntax())
    }
}

impl PathExpr {
    /// Returns the `THIS`/`SUPER` prefix token, when present.
    #[must_use]
    pub fn prefix(&self) -> Option<SyntaxToken> {
        self.syntax()
            .child_tokens()
            .into_iter()
            .find(|token| matches!(token.kind(), SyntaxKind::KwThis | SyntaxKind::KwSuper))
    }
}

fn initializer_child(node: &SyntaxNode) -> Option<Initializer> {
    node.child_nodes().iter().find_map(Initializer::cast)
}

fn child_views<T>(node: &SyntaxNode, cast: fn(&SyntaxNode) -> Option<T>) -> Vec<T> {
    node.child_nodes().iter().filter_map(cast).collect()
}

fn expr_children(node: &SyntaxNode) -> Vec<Expr> {
    node.child_nodes().iter().filter_map(Expr::cast).collect()
}

fn declaration_name(node: &SyntaxNode) -> Option<Name> {
    node.child_nodes().iter().find_map(Name::cast)
}

fn implements_list(node: &SyntaxNode) -> Vec<Path> {
    let mut seen_implements = false;
    let mut paths = Vec::new();
    for child in node.children() {
        match &child {
            crate::SyntaxElement::Token(token) if token.kind() == SyntaxKind::KwImplements => {
                seen_implements = true;
            }
            crate::SyntaxElement::Node(child) if seen_implements => {
                if let Some(path) = Path::cast(child) {
                    paths.push(path);
                }
            }
            crate::SyntaxElement::Token(token)
                if seen_implements && token.kind() == SyntaxKind::PunctSemicolon =>
            {
                seen_implements = false;
            }
            _ => {}
        }
    }
    paths
}

fn extends_type(node: &SyntaxNode) -> Option<TypeRef> {
    let mut seen_extends = false;
    for child in node.children() {
        match &child {
            crate::SyntaxElement::Token(token) if token.kind() == SyntaxKind::KwExtends => {
                seen_extends = true;
            }
            crate::SyntaxElement::Node(child) if seen_extends => {
                if let Some(type_ref) = TypeRef::cast(child) {
                    return Some(type_ref);
                }
            }
            _ => {}
        }
    }
    None
}

fn operator_token(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.child_tokens()
        .into_iter()
        .find(|token| token.kind() != SyntaxKind::PunctSemicolon)
}
