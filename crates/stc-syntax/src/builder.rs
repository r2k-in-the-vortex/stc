use std::collections::BTreeMap;
use std::sync::Arc;

use stc_lexer::{Literal, Token, TokenStream, TriviaKind};
use stc_span::{FileId, Span};

use crate::SyntaxKind;
use crate::pragma::PragmaAnnotation;
use crate::tree::{
    Element, NodeData, NodeId, SyntaxTokenData, SyntaxTree, SyntaxTriviaData, TokenId, TreeData,
    TriviaId,
};

/// A position in the parent node's child list, used to wrap already-emitted
/// children into a new node after the fact (Pratt expression trees).
#[derive(Clone, Copy, Debug)]
pub struct Checkpoint {
    parent: Option<NodeId>,
    child_count: usize,
}

/// Builds one immutable [`SyntaxTree`].
///
/// The parser opens and closes nodes in source order. Every token and trivia
/// item is emitted at most once; the builder records the global order so the
/// root text reproduces the source exactly.
pub struct TreeBuilder {
    file: FileId,
    source: Arc<str>,
    nodes: Vec<NodeData>,
    tokens: Vec<SyntaxTokenData>,
    trivia: Vec<SyntaxTriviaData>,
    pragmas: BTreeMap<TokenId, PragmaAnnotation>,
    stack: Vec<NodeId>,
    root: Option<NodeId>,
    position: u32,
}

impl TreeBuilder {
    /// Creates a builder for `source`.
    #[must_use]
    pub fn new(file: FileId, source: Arc<str>) -> Self {
        Self {
            file,
            source,
            nodes: Vec::new(),
            tokens: Vec::new(),
            trivia: Vec::new(),
            pragmas: BTreeMap::new(),
            stack: Vec::new(),
            root: None,
            position: 0,
        }
    }

    /// Returns the file identifier.
    #[must_use]
    pub fn file(&self) -> FileId {
        self.file
    }

    /// Returns the current insertion offset (the end of the last emitted leaf).
    #[must_use]
    pub fn current_offset(&self) -> u32 {
        self.position
    }

    /// Returns the number of currently open nodes.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Opens a node. It becomes a child of the innermost open node when
    /// [`TreeBuilder::finish_node`] is called.
    ///
    /// # Panics
    ///
    /// Panics if called after [`TreeBuilder::finish`].
    pub fn start_node(&mut self, kind: SyntaxKind) -> NodeId {
        assert!(kind.is_node(), "start_node requires a node kind");
        let id = NodeId(self.nodes.len());
        self.nodes.push(NodeData {
            kind,
            span: Span::empty(self.file, self.position),
            parent: self.stack.last().copied(),
            children: Vec::new(),
            first_token: None,
            last_token: None,
        });
        self.stack.push(id);
        id
    }

    /// Changes the kind of a node that is still open.
    pub fn set_node_kind(&mut self, id: NodeId, kind: SyntaxKind) {
        assert!(kind.is_node(), "set_node_kind requires a node kind");
        self.nodes[id.0].kind = kind;
    }

    /// Returns a checkpoint at the end of the current node's children.
    #[must_use]
    pub fn checkpoint(&self) -> Checkpoint {
        let parent = self.stack.last().copied();
        let child_count = parent.map_or(0, |parent| self.nodes[parent.0].children.len());
        Checkpoint {
            parent,
            child_count,
        }
    }

    /// Wraps the current node's children emitted since `checkpoint` into a new
    /// node of `kind`, inserted at the checkpoint position.
    ///
    /// This is the expression-tree primitive: parse a left operand, then wrap
    /// it together with the operator and right operand after the fact.
    ///
    /// # Panics
    ///
    /// Panics if no node is open or the checkpoint belongs elsewhere.
    pub fn wrap_children(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) -> NodeId {
        assert!(kind.is_node(), "wrap_children requires a node kind");
        let parent = checkpoint.parent.expect("checkpoint outside a node");
        assert_eq!(
            self.stack.last().copied(),
            Some(parent),
            "checkpoint parent is not the current node"
        );
        assert!(
            checkpoint.child_count <= self.nodes[parent.0].children.len(),
            "checkpoint is ahead of the children"
        );
        let children: Vec<Element> = self.nodes[parent.0]
            .children
            .drain(checkpoint.child_count..)
            .collect();
        let span = self.union_span(&children);
        let first_token = children.iter().find_map(|child| self.element_first(*child));
        let last_token = children
            .iter()
            .rev()
            .find_map(|child| self.element_last(*child));
        let id = NodeId(self.nodes.len());
        for child in &children {
            if let Element::Node(child) = child {
                self.nodes[child.0].parent = Some(id);
            }
        }
        self.nodes.push(NodeData {
            kind,
            span,
            parent: Some(parent),
            children,
            first_token,
            last_token,
        });
        self.nodes[parent.0]
            .children
            .insert(checkpoint.child_count, Element::Node(id));
        id
    }

    fn union_span(&self, children: &[Element]) -> Span {
        let mut span = Span::empty(self.file, self.position);
        for child in children {
            let child_span = match child {
                Element::Node(child) => self.nodes[child.0].span,
                Element::Token(child) => self.tokens[child.0].span,
                Element::Trivia(child) => self.trivia[child.0].span,
            };
            span = span.join(child_span);
        }
        span
    }

    /// Closes the innermost open node.
    ///
    /// # Panics
    ///
    /// Panics if no node is open.
    pub fn finish_node(&mut self) {
        let id = self.stack.pop().expect("finish_node without an open node");
        self.close_node(id);
    }

    /// Emits a token leaf and returns its id.
    pub fn token(
        &mut self,
        kind: SyntaxKind,
        span: Span,
        literal: Option<Box<Literal>>,
    ) -> TokenId {
        self.emit_leaf(kind, span, literal, false)
    }

    /// Emits a zero-width token that the parser inserted to recover.
    pub fn missing_token(&mut self, kind: SyntaxKind) -> TokenId {
        let span = Span::empty(self.file, self.position);
        self.emit_leaf(kind, span, None, true)
    }

    /// Emits a trivia leaf.
    pub fn trivia(&mut self, kind: TriviaKind, span: Span) -> TriviaId {
        let id = TriviaId(self.trivia.len());
        self.trivia.push(SyntaxTriviaData { kind, span });
        self.position = span.end;
        self.append(Element::Trivia(id));
        id
    }

    /// Emits a lexer token's leading trivia, the token, and its trailing
    /// trivia, in that order. Returns the token id.
    pub fn emit_token(&mut self, stream: &TokenStream, token: &Token) -> TokenId {
        for trivia in stream.leading_trivia(token) {
            self.trivia(trivia.kind, trivia.span);
        }
        let kind = SyntaxKind::from_token(&token.kind);
        let id = self.token(kind, token.span, token.literal.clone());
        for trivia in stream.trailing_trivia(token) {
            self.trivia(trivia.kind, trivia.span);
        }
        id
    }

    /// Returns the innermost open node, if any.
    #[must_use]
    pub fn current_parent(&self) -> Option<NodeId> {
        self.stack.last().copied()
    }

    /// Returns the most recently appended child node kind of `parent`.
    pub fn last_child_node_kind(&self, parent: NodeId) -> Option<SyntaxKind> {
        self.nodes
            .get(parent.0)?
            .children
            .iter()
            .rev()
            .find_map(|element| match element {
                Element::Node(id) => Some(self.nodes[id.0].kind),
                _ => None,
            })
    }

    /// Records the parsed annotation for a pragma token.
    pub fn set_pragma(&mut self, token: TokenId, annotation: PragmaAnnotation) {
        self.pragmas.insert(token, annotation);
    }

    /// Finishes the tree, closing any nodes the parser left open.
    ///
    /// Closing open nodes rather than panicking keeps the fuzz target total;
    /// `check_invariants` still exposes malformed trees in tests.
    #[must_use]
    pub fn finish(mut self) -> SyntaxTree {
        while !self.stack.is_empty() {
            self.finish_node();
        }
        if self.root.is_none() {
            self.start_node(SyntaxKind::SourceFile);
            self.finish_node();
        }
        let root = self.root.expect("a tree always has a root");
        SyntaxTree {
            data: Arc::new(TreeData {
                file: self.file,
                source: self.source,
                nodes: self.nodes,
                tokens: self.tokens,
                trivia: self.trivia,
                pragmas: self.pragmas,
                root,
            }),
        }
    }

    fn emit_leaf(
        &mut self,
        kind: SyntaxKind,
        span: Span,
        literal: Option<Box<Literal>>,
        missing: bool,
    ) -> TokenId {
        assert!(kind.is_token(), "token requires a token kind");
        let id = TokenId(self.tokens.len());
        self.tokens.push(SyntaxTokenData {
            kind,
            span,
            literal,
            missing,
        });
        if span.end > self.position {
            self.position = span.end;
        }
        self.append(Element::Token(id));
        id
    }

    fn element_first(&self, element: Element) -> Option<TokenId> {
        match element {
            Element::Node(id) => self.nodes[id.0].first_token,
            Element::Token(id) => {
                let token = &self.tokens[id.0];
                (!token.missing && !token.span.is_empty()).then_some(id)
            }
            Element::Trivia(_) => None,
        }
    }

    fn element_last(&self, element: Element) -> Option<TokenId> {
        match element {
            Element::Node(id) => self.nodes[id.0].last_token,
            Element::Token(id) => {
                let token = &self.tokens[id.0];
                (!token.missing && !token.span.is_empty()).then_some(id)
            }
            Element::Trivia(_) => None,
        }
    }

    fn append(&mut self, element: Element) {
        if let Some(parent) = self.stack.last() {
            self.nodes[parent.0].children.push(element);
        }
    }

    fn close_node(&mut self, id: NodeId) {
        let start = self.nodes[id.0].span.start;
        let mut span = Span::empty(self.file, start);
        for child in &self.nodes[id.0].children {
            let child_span = match child {
                Element::Node(child) => self.nodes[child.0].span,
                Element::Token(child) => self.tokens[child.0].span,
                Element::Trivia(child) => self.trivia[child.0].span,
            };
            span = span.join(child_span);
        }
        self.nodes[id.0].span = span;
        let first_token = self.nodes[id.0]
            .children
            .iter()
            .find_map(|child| self.element_first(*child));
        let last_token = self.nodes[id.0]
            .children
            .iter()
            .rev()
            .find_map(|child| self.element_last(*child));
        self.nodes[id.0].first_token = first_token;
        self.nodes[id.0].last_token = last_token;
        if let Some(parent) = self.stack.last() {
            self.nodes[parent.0].children.push(Element::Node(id));
        } else if let Some(root) = self.root {
            // A second top-level node is a parser bug. Keep the tree total by
            // attaching it to the existing root.
            self.nodes[id.0].parent = Some(root);
            self.nodes[root.0].children.push(Element::Node(id));
            let root_span = self.nodes[root.0].span.join(span);
            self.nodes[root.0].span = root_span;
        } else {
            self.root = Some(id);
        }
    }
}

/// Builds a flat `SourceFile` from a token stream without applying grammar
/// rules. This is the round-trip proof used before the parser is involved.
#[must_use]
pub fn build_flat_tree(file: FileId, source: Arc<str>, stream: &TokenStream) -> SyntaxTree {
    let mut builder = TreeBuilder::new(file, source);
    builder.start_node(SyntaxKind::SourceFile);
    for token in &stream.tokens {
        builder.emit_token(stream, token);
    }
    builder.finish_node();
    builder.finish()
}
