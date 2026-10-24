use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use stc_lexer::{Literal, TriviaKind};
use stc_span::{FileId, Span};

use crate::SyntaxKind;
use crate::pragma::PragmaAnnotation;

/// Index of a node in a [`SyntaxTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) usize);

/// Index of a token leaf in a [`SyntaxTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenId(pub(crate) usize);

/// Index of a trivia leaf in a [`SyntaxTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TriviaId(pub(crate) usize);

impl NodeId {
    /// Returns the arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl TokenId {
    /// Returns the arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl TriviaId {
    /// Returns the arena index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One ordered child of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    /// A non-terminal node.
    Node(NodeId),
    /// A token leaf.
    Token(TokenId),
    /// A trivia leaf.
    Trivia(TriviaId),
}

/// One token leaf. Tokens are leaves even when the source text is empty
/// because a parser inserted a missing token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxTokenData {
    pub kind: SyntaxKind,
    pub span: Span,
    pub literal: Option<Box<Literal>>,
    pub missing: bool,
}

/// One trivia leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxTriviaData {
    pub kind: TriviaKind,
    pub span: Span,
}

/// One non-terminal node.
#[derive(Clone, Debug)]
pub struct NodeData {
    pub kind: SyntaxKind,
    pub span: Span,
    pub parent: Option<NodeId>,
    pub children: Vec<Element>,
    /// First non-trivia, non-missing token in the subtree, computed at close.
    pub first_token: Option<TokenId>,
    /// Last non-trivia, non-missing token in the subtree, computed at close.
    pub last_token: Option<TokenId>,
}

pub(crate) struct TreeData {
    pub file: FileId,
    pub source: Arc<str>,
    pub nodes: Vec<NodeData>,
    pub tokens: Vec<SyntaxTokenData>,
    pub trivia: Vec<SyntaxTriviaData>,
    pub pragmas: BTreeMap<TokenId, PragmaAnnotation>,
    pub root: NodeId,
}

/// An immutable lossless syntax tree for one file.
///
/// Cloning a tree or a node handle is cheap: both share the same arena through
/// an [`Arc`].
#[derive(Clone)]
pub struct SyntaxTree {
    pub(crate) data: Arc<TreeData>,
}

impl fmt::Debug for SyntaxTree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntaxTree")
            .field("file", &self.data.file)
            .field("nodes", &self.data.nodes.len())
            .field("tokens", &self.data.tokens.len())
            .field("trivia", &self.data.trivia.len())
            .field("root", &self.data.root)
            .finish()
    }
}

impl PartialEq for SyntaxTree {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

impl Eq for SyntaxTree {}

impl SyntaxTree {
    /// Returns the file identifier used by every span in this tree.
    #[must_use]
    pub fn file(&self) -> FileId {
        self.data.file
    }

    /// Returns the owned source text.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.data.source
    }

    /// Returns the root node (normally `SourceFile`).
    #[must_use]
    pub fn root(&self) -> SyntaxNode {
        self.node(self.data.root)
    }

    /// Returns a node handle.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not part of this tree.
    #[must_use]
    pub fn node(&self, id: NodeId) -> SyntaxNode {
        assert!(id.0 < self.data.nodes.len(), "node id out of range");
        SyntaxNode {
            tree: self.clone(),
            id,
        }
    }

    /// Returns a token handle.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not part of this tree.
    #[must_use]
    pub fn token(&self, id: TokenId) -> SyntaxToken {
        assert!(id.0 < self.data.tokens.len(), "token id out of range");
        SyntaxToken {
            tree: self.clone(),
            id,
        }
    }

    /// Returns a trivia handle.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not part of this tree.
    #[must_use]
    pub fn trivia(&self, id: TriviaId) -> SyntaxTrivia {
        assert!(id.0 < self.data.trivia.len(), "trivia id out of range");
        SyntaxTrivia {
            tree: self.clone(),
            id,
        }
    }

    /// Returns the parsed annotation for a pragma token.
    #[must_use]
    pub fn pragma_annotation(&self, token: TokenId) -> Option<&PragmaAnnotation> {
        self.data.pragmas.get(&token)
    }

    /// Returns the number of nodes, including the root.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.data.nodes.len()
    }

    /// Returns the number of token leaves.
    #[must_use]
    pub fn token_count(&self) -> usize {
        self.data.tokens.len()
    }

    /// Returns the number of trivia leaves.
    #[must_use]
    pub fn trivia_count(&self) -> usize {
        self.data.trivia.len()
    }

    /// Returns a slice of source text, or `""` for an invalid range.
    #[must_use]
    pub fn text(&self, span: Span) -> &str {
        if span.file != self.data.file {
            return "";
        }
        self.data
            .source
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    /// Checks the lossless-tree invariants from AST §3.4.
    ///
    /// # Errors
    ///
    /// Returns a description of the first violated invariant.
    pub fn check_invariants(&self) -> Result<(), String> {
        let source_len = u32::try_from(self.data.source.len())
            .map_err(|_| "source exceeds the 4 GiB span limit".to_string())?;
        let root = &self.data.nodes[self.data.root.0];
        if root.parent.is_some() {
            return Err("root node has a parent".into());
        }
        if root.span.start != 0 || root.span.end != source_len {
            return Err(format!(
                "root span {}..{} does not cover the source (0..{source_len})",
                root.span.start, root.span.end
            ));
        }
        for (index, node) in self.data.nodes.iter().enumerate() {
            // An empty statement list is grammatical (`(stmt? ';')*`), so it is
            // the one node kind allowed to be empty alongside Error.
            if node.children.is_empty()
                && node.kind != SyntaxKind::Error
                && node.kind != SyntaxKind::StmtList
            {
                return Err(format!(
                    "node {index} ({}) has no children and is not an Error or empty StmtList node",
                    node.kind.as_str()
                ));
            }
            let mut union: Option<Span> = None;
            for child in &node.children {
                let span = self.element_span(*child);
                union = Some(match union {
                    Some(current) => current.join(span),
                    None => span,
                });
            }
            let expected = union.unwrap_or_else(|| Span::empty(self.data.file, node.span.start));
            if expected != node.span {
                return Err(format!(
                    "node {index} ({}) span {}..{} is not the union of its children ({}..{})",
                    node.kind.as_str(),
                    node.span.start,
                    node.span.end,
                    expected.start,
                    expected.end
                ));
            }
        }
        for (index, node) in self.data.nodes.iter().enumerate() {
            for child in &node.children {
                if let Element::Node(child_id) = child
                    && self.data.nodes[child_id.0].parent != Some(NodeId(index))
                {
                    return Err(format!(
                        "node {index} has child {} with a wrong parent",
                        child_id.0
                    ));
                }
            }
        }
        if self.root().text() != &*self.data.source {
            return Err("root text does not reproduce the source".into());
        }
        Ok(())
    }

    fn element_span(&self, element: Element) -> Span {
        match element {
            Element::Node(id) => self.data.nodes[id.0].span,
            Element::Token(id) => self.data.tokens[id.0].span,
            Element::Trivia(id) => self.data.trivia[id.0].span,
        }
    }
}

/// A non-terminal CST node. Handles are cheap to clone and always reference the
/// tree that produced them.
#[derive(Clone)]
pub struct SyntaxNode {
    tree: SyntaxTree,
    id: NodeId,
}

impl fmt::Debug for SyntaxNode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntaxNode")
            .field("kind", &self.kind())
            .field("span", &self.span())
            .finish()
    }
}

impl PartialEq for SyntaxNode {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.tree == other.tree
    }
}

impl Eq for SyntaxNode {}

impl SyntaxNode {
    /// Returns the node kind.
    #[must_use]
    pub fn kind(&self) -> SyntaxKind {
        self.tree.data.nodes[self.id.0].kind
    }

    /// Returns the node span, including attached trivia.
    #[must_use]
    pub fn span(&self) -> Span {
        self.tree.data.nodes[self.id.0].span
    }

    /// Returns the token-only extent of the node: the first and last non-trivia
    /// leaves, ignoring attached trivia. Missing tokens do not extend it.
    #[must_use]
    pub fn token_span(&self) -> Span {
        match (self.first_token(), self.last_token()) {
            (Some(first), Some(last)) => first.span().join(last.span()),
            _ => self.span(),
        }
    }

    /// Returns the first non-trivia, non-missing token in the subtree.
    #[must_use]
    pub fn first_token(&self) -> Option<SyntaxToken> {
        self.tree.data.nodes[self.id.0]
            .first_token
            .map(|id| self.tree.token(id))
    }

    /// Returns the last non-trivia, non-missing token in the subtree.
    #[must_use]
    pub fn last_token(&self) -> Option<SyntaxToken> {
        self.tree.data.nodes[self.id.0]
            .last_token
            .map(|id| self.tree.token(id))
    }

    /// Returns the parent node, if any.
    #[must_use]
    pub fn parent(&self) -> Option<SyntaxNode> {
        self.tree.data.nodes[self.id.0]
            .parent
            .map(|parent| self.tree.node(parent))
    }

    /// Returns the arena id.
    #[must_use]
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Returns the tree this node belongs to.
    #[must_use]
    pub fn tree(&self) -> &SyntaxTree {
        &self.tree
    }

    /// Returns the source text covered by the node span.
    #[must_use]
    pub fn text(&self) -> &str {
        self.tree.text(self.span())
    }

    /// Returns the ordered children as handles.
    #[must_use]
    pub fn children(&self) -> Vec<SyntaxElement> {
        self.tree.data.nodes[self.id.0]
            .children
            .iter()
            .map(|element| self.tree.element(*element))
            .collect()
    }

    /// Returns the child nodes in order.
    #[must_use]
    pub fn child_nodes(&self) -> Vec<SyntaxNode> {
        self.tree.data.nodes[self.id.0]
            .children
            .iter()
            .filter_map(|element| match element {
                Element::Node(id) => Some(self.tree.node(*id)),
                _ => None,
            })
            .collect()
    }

    /// Returns the child tokens in order.
    #[must_use]
    pub fn child_tokens(&self) -> Vec<SyntaxToken> {
        self.tree.data.nodes[self.id.0]
            .children
            .iter()
            .filter_map(|element| match element {
                Element::Token(id) => Some(self.tree.token(*id)),
                _ => None,
            })
            .collect()
    }

    /// Returns the first child node with `kind`.
    #[must_use]
    pub fn first_child_of_kind(&self, kind: SyntaxKind) -> Option<SyntaxNode> {
        self.child_nodes()
            .into_iter()
            .find(|node| node.kind() == kind)
    }

    /// Returns the first token leaf with `kind`, directly or in a descendant.
    #[must_use]
    pub fn first_token_of_kind(&self, kind: SyntaxKind) -> Option<SyntaxToken> {
        self.descendants()
            .into_iter()
            .find_map(|element| match element {
                SyntaxElement::Token(token) if token.kind() == kind => Some(token),
                _ => None,
            })
    }

    /// Returns `true` if a direct child token has `kind`.
    #[must_use]
    pub fn has_direct_token(&self, kind: SyntaxKind) -> bool {
        self.child_tokens()
            .into_iter()
            .any(|token| token.kind() == kind)
    }

    /// Returns every child element in pre-order (excluding this node).
    #[must_use]
    pub fn descendants(&self) -> Vec<SyntaxElement> {
        let mut out = Vec::new();
        // Depth-first pre-order over the arena. Element order is preserved by
        // pushing children in reverse.
        let mut pending: Vec<SyntaxElement> = self.children();
        pending.reverse();
        while let Some(element) = pending.pop() {
            out.push(element.clone());
            if let SyntaxElement::Node(node) = element {
                let mut children = node.children();
                children.reverse();
                pending.extend(children);
            }
        }
        out
    }

    /// Returns every token leaf in pre-order.
    #[must_use]
    pub fn descendant_tokens(&self) -> Vec<SyntaxToken> {
        self.descendants()
            .into_iter()
            .filter_map(|element| match element {
                SyntaxElement::Token(token) => Some(token),
                _ => None,
            })
            .collect()
    }

    /// Returns every trivia leaf in pre-order.
    #[must_use]
    pub fn descendant_trivia(&self) -> Vec<SyntaxTrivia> {
        self.descendants()
            .into_iter()
            .filter_map(|element| match element {
                SyntaxElement::Trivia(trivia) => Some(trivia),
                _ => None,
            })
            .collect()
    }
}

/// A token leaf.
#[derive(Clone)]
pub struct SyntaxToken {
    tree: SyntaxTree,
    id: TokenId,
}

impl fmt::Debug for SyntaxToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntaxToken")
            .field("kind", &self.kind())
            .field("span", &self.span())
            .field("missing", &self.is_missing())
            .finish()
    }
}

impl PartialEq for SyntaxToken {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.tree == other.tree
    }
}

impl Eq for SyntaxToken {}

impl SyntaxToken {
    /// Returns the token kind.
    #[must_use]
    pub fn kind(&self) -> SyntaxKind {
        self.tree.data.tokens[self.id.0].kind
    }

    /// Returns the token span. Missing tokens are zero-width.
    #[must_use]
    pub fn span(&self) -> Span {
        self.tree.data.tokens[self.id.0].span
    }

    /// Returns the token source text (`""` for a missing token).
    #[must_use]
    pub fn text(&self) -> &str {
        self.tree.text(self.span())
    }

    /// Returns the decoded literal payload, if any.
    #[must_use]
    pub fn literal(&self) -> Option<&Literal> {
        self.tree.data.tokens[self.id.0].literal.as_deref()
    }

    /// Returns `true` if the parser inserted this token to recover.
    #[must_use]
    pub fn is_missing(&self) -> bool {
        self.tree.data.tokens[self.id.0].missing
    }

    /// Returns the arena id.
    #[must_use]
    pub fn id(&self) -> TokenId {
        self.id
    }

    /// Returns the parsed annotation when this is a `Pragma` token.
    #[must_use]
    pub fn pragma_annotation(&self) -> Option<&PragmaAnnotation> {
        self.tree.pragma_annotation(self.id)
    }

    /// Returns the parent node.
    #[must_use]
    pub fn parent(&self) -> Option<SyntaxNode> {
        let mut parent = None;
        for (index, node) in self.tree.data.nodes.iter().enumerate() {
            if node.children.contains(&Element::Token(self.id)) {
                parent = Some(self.tree.node(NodeId(index)));
                break;
            }
        }
        parent
    }
}

/// A trivia leaf.
#[derive(Clone)]
pub struct SyntaxTrivia {
    tree: SyntaxTree,
    id: TriviaId,
}

impl fmt::Debug for SyntaxTrivia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntaxTrivia")
            .field("kind", &self.kind())
            .field("span", &self.span())
            .finish()
    }
}

impl PartialEq for SyntaxTrivia {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.tree == other.tree
    }
}

impl Eq for SyntaxTrivia {}

impl SyntaxTrivia {
    /// Returns the trivia kind.
    #[must_use]
    pub fn kind(&self) -> TriviaKind {
        self.tree.data.trivia[self.id.0].kind
    }

    /// Returns the trivia span.
    #[must_use]
    pub fn span(&self) -> Span {
        self.tree.data.trivia[self.id.0].span
    }

    /// Returns the trivia source text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.tree.text(self.span())
    }
}

/// One child of a node, as a typed handle.
#[derive(Clone, Debug)]
pub enum SyntaxElement {
    /// A non-terminal node.
    Node(SyntaxNode),
    /// A token leaf.
    Token(SyntaxToken),
    /// A trivia leaf.
    Trivia(SyntaxTrivia),
}

impl SyntaxElement {
    /// Returns the element span.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Node(node) => node.span(),
            Self::Token(token) => token.span(),
            Self::Trivia(trivia) => trivia.span(),
        }
    }

    /// Returns the element source text.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Node(node) => node.text(),
            Self::Token(token) => token.text(),
            Self::Trivia(trivia) => trivia.text(),
        }
    }

    /// Returns `true` for a trivia leaf.
    #[must_use]
    pub fn is_trivia(&self) -> bool {
        matches!(self, Self::Trivia(_))
    }
}

impl SyntaxTree {
    fn element(&self, element: Element) -> SyntaxElement {
        match element {
            Element::Node(id) => SyntaxElement::Node(self.node(id)),
            Element::Token(id) => SyntaxElement::Token(self.token(id)),
            Element::Trivia(id) => SyntaxElement::Trivia(self.trivia(id)),
        }
    }
}
