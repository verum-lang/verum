//! Red tree - the facade over the green tree providing parent pointers and absolute positions.
//!
//! Red nodes are built on-demand from green nodes. They provide:
//! - Parent pointers for navigating up the tree
//! - Absolute text positions (computed from relative widths)
//! - Convenient iteration and query methods
//!
//! Red nodes are cheap to create and should be discarded on each edit.
//!
//! Red Tree (Facade Layer):
//! Red nodes are built on-demand from green nodes and provide parent pointers
//! and absolute text positions (computed from relative widths stored in green nodes).
//! Navigation: children(), ancestors(), descendants(), token_at_offset(pos),
//! covering_element(range). Red nodes are cheap to create and should be discarded
//! on each edit since absolute positions change. The green tree persists across edits.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::green::{GreenChild, GreenNode, GreenToken, TextRange, TextSize};
use crate::SyntaxKind;

/// Red node - provides parent pointers and absolute positions.
///
/// Built on-demand from green nodes, discarded on edits.
/// Uses reference counting for efficient cloning.
#[derive(Clone)]
pub struct SyntaxNode {
    /// Underlying green node
    green: GreenNode,
    /// Parent node (None for root)
    parent: Option<Arc<SyntaxNodeData>>,
    /// Index in parent's children
    index_in_parent: u32,
    /// Absolute offset in the source text
    offset: TextSize,
}

/// Internal data for parent chain.
struct SyntaxNodeData {
    green: GreenNode,
    parent: Option<Arc<SyntaxNodeData>>,
    index_in_parent: u32,
    offset: TextSize,
}

impl SyntaxNode {
    /// Create a root syntax node from a green node.
    pub fn new_root(green: GreenNode) -> Self {
        Self {
            green,
            parent: None,
            index_in_parent: 0,
            offset: 0,
        }
    }

    /// Create a syntax node from a green node with parent info.
    fn new_child(green: GreenNode, parent: &SyntaxNode, index: usize, offset: TextSize) -> Self {
        Self {
            green,
            parent: Some(Arc::new(SyntaxNodeData {
                green: parent.green.clone(),
                parent: parent.parent.clone(),
                index_in_parent: parent.index_in_parent,
                offset: parent.offset,
            })),
            index_in_parent: index as u32,
            offset,
        }
    }

    /// Get the underlying green node.
    #[inline]
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Get the syntax kind of this node.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.green.kind()
    }

    /// Get the text range of this node in the source.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::at(self.offset, self.green.width())
    }

    /// Get the start offset of this node.
    #[inline]
    pub fn start(&self) -> TextSize {
        self.offset
    }

    /// Get the end offset of this node (exclusive).
    #[inline]
    pub fn end(&self) -> TextSize {
        self.offset + self.green.width()
    }

    /// Get the text of this node.
    pub fn text(&self) -> String {
        self.green.text()
    }

    /// Get the parent node.
    pub fn parent(&self) -> Option<SyntaxNode> {
        self.parent.as_ref().map(|p| SyntaxNode {
            green: p.green.clone(),
            parent: p.parent.clone(),
            index_in_parent: p.index_in_parent,
            offset: p.offset,
        })
    }

    /// Get the index of this node in its parent's children.
    pub fn index(&self) -> usize {
        self.index_in_parent as usize
    }

    /// Get all ancestors (parent, grandparent, etc.).
    pub fn ancestors(&self) -> impl Iterator<Item = SyntaxNode> {
        std::iter::successors(self.parent(), |node| node.parent())
    }

    /// Get the number of children.
    pub fn children_count(&self) -> usize {
        self.green.children_count()
    }

    /// Iterate over children (both nodes and tokens).
    pub fn children(&self) -> impl Iterator<Item = SyntaxElement> + '_ {
        let mut offset = self.offset;
        self.green.children().iter().enumerate().map(move |(i, child)| {
            let child_offset = offset;
            offset += child.width();
            match child {
                GreenChild::Node(n) => {
                    SyntaxElement::Node(SyntaxNode::new_child(n.clone(), self, i, child_offset))
                }
                GreenChild::Token(t) => {
                    SyntaxElement::Token(SyntaxToken::new(t.clone(), self.clone(), i, child_offset))
                }
            }
        })
    }

    /// Iterate over child nodes only.
    pub fn child_nodes(&self) -> impl Iterator<Item = SyntaxNode> + '_ {
        self.children().filter_map(|e| e.into_node())
    }

    /// Iterate over child tokens only.
    pub fn child_tokens(&self) -> impl Iterator<Item = SyntaxToken> + '_ {
        self.children().filter_map(|e| e.into_token())
    }

    /// Get a child by index.
    pub fn child(&self, index: usize) -> Option<SyntaxElement> {
        let offset = self.green.child_offset(index)?;
        let child = self.green.child(index)?;
        Some(match child {
            GreenChild::Node(n) => {
                SyntaxElement::Node(SyntaxNode::new_child(n.clone(), self, index, self.offset + offset))
            }
            GreenChild::Token(t) => {
                SyntaxElement::Token(SyntaxToken::new(t.clone(), self.clone(), index, self.offset + offset))
            }
        })
    }

    /// Get the first child.
    pub fn first_child(&self) -> Option<SyntaxElement> {
        self.child(0)
    }

    /// Get the last child.
    pub fn last_child(&self) -> Option<SyntaxElement> {
        if self.green.children_count() == 0 {
            None
        } else {
            self.child(self.green.children_count() - 1)
        }
    }

    /// Get the first child node.
    pub fn first_child_node(&self) -> Option<SyntaxNode> {
        self.child_nodes().next()
    }

    /// Get the last child node.
    pub fn last_child_node(&self) -> Option<SyntaxNode> {
        self.child_nodes().last()
    }

    /// Get the first token in this subtree (depth-first).
    pub fn first_token(&self) -> Option<SyntaxToken> {
        for child in self.children() {
            match child {
                SyntaxElement::Token(t) => return Some(t),
                SyntaxElement::Node(n) => {
                    if let Some(t) = n.first_token() {
                        return Some(t);
                    }
                }
            }
        }
        None
    }

    /// Get the last token in this subtree (depth-first).
    pub fn last_token(&self) -> Option<SyntaxToken> {
        for child in self.children().collect::<Vec<_>>().into_iter().rev() {
            match child {
                SyntaxElement::Token(t) => return Some(t),
                SyntaxElement::Node(n) => {
                    if let Some(t) = n.last_token() {
                        return Some(t);
                    }
                }
            }
        }
        None
    }

    /// Get the next sibling.
    pub fn next_sibling(&self) -> Option<SyntaxElement> {
        self.parent()?.child(self.index() + 1)
    }

    /// Get the previous sibling.
    pub fn prev_sibling(&self) -> Option<SyntaxElement> {
        if self.index() == 0 {
            return None;
        }
        self.parent()?.child(self.index() - 1)
    }

    /// Get the next sibling node.
    pub fn next_sibling_node(&self) -> Option<SyntaxNode> {
        std::iter::successors(self.next_sibling(), |e| match e {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(t) => t.next_sibling(),
        })
        .find_map(|e| e.into_node())
    }

    /// Get the previous sibling node.
    pub fn prev_sibling_node(&self) -> Option<SyntaxNode> {
        std::iter::successors(self.prev_sibling(), |e| match e {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(t) => t.prev_sibling(),
        })
        .find_map(|e| e.into_node())
    }

    /// Find the deepest token at the given offset.
    pub fn token_at_offset(&self, offset: TextSize) -> Option<SyntaxToken> {
        if !self.text_range().contains(offset) {
            return None;
        }

        for child in self.children() {
            let range = child.text_range();
            if range.contains(offset) {
                return match child {
                    SyntaxElement::Token(t) => Some(t),
                    SyntaxElement::Node(n) => n.token_at_offset(offset),
                };
            }
        }
        None
    }

    /// Find the deepest node at the given offset.
    pub fn node_at_offset(&self, offset: TextSize) -> Option<SyntaxNode> {
        if !self.text_range().contains(offset) {
            return None;
        }

        for child in self.child_nodes() {
            if child.text_range().contains(offset) {
                return child.node_at_offset(offset).or(Some(child));
            }
        }
        Some(self.clone())
    }

    /// Find the node covering the given range.
    pub fn covering_element(&self, range: TextRange) -> SyntaxElement {
        let mut node = self.clone();
        loop {
            let mut found = None;
            for child in node.children() {
                let child_range = child.text_range();
                if child_range.contains_range(range) {
                    found = Some(child);
                    break;
                }
            }
            match found {
                Some(SyntaxElement::Node(n)) => node = n,
                Some(e) => return e,
                None => return SyntaxElement::Node(node),
            }
        }
    }

    /// Iterate over all descendants (depth-first, pre-order).
    pub fn descendants(&self) -> impl Iterator<Item = SyntaxElement> {
        DescendantsIter {
            stack: vec![DescendantsFrame::new(self.clone())],
        }
    }

    /// Iterate over all descendant nodes.
    pub fn descendant_nodes(&self) -> impl Iterator<Item = SyntaxNode> {
        self.descendants().filter_map(|e| e.into_node())
    }

    /// Iterate over all descendant tokens.
    pub fn descendant_tokens(&self) -> impl Iterator<Item = SyntaxToken> {
        self.descendants().filter_map(|e| e.into_token())
    }

    /// Check if this node contains errors.
    pub fn contains_error(&self) -> bool {
        self.descendants().any(|e| e.kind().is_error())
    }

    /// Find the first child with the given kind.
    pub fn child_by_kind(&self, kind: SyntaxKind) -> Option<SyntaxElement> {
        self.children().find(|c| c.kind() == kind)
    }

    /// Find the first child node with the given kind.
    pub fn child_node_by_kind(&self, kind: SyntaxKind) -> Option<SyntaxNode> {
        self.child_nodes().find(|n| n.kind() == kind)
    }

    /// Find the first child token with the given kind.
    pub fn child_token_by_kind(&self, kind: SyntaxKind) -> Option<SyntaxToken> {
        self.child_tokens().find(|t| t.kind() == kind)
    }

    /// Check if this node is a specific kind.
    pub fn is(&self, kind: SyntaxKind) -> bool {
        self.kind() == kind
    }
}

impl fmt::Debug for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxNode")
            .field("kind", &self.kind())
            .field("range", &self.text_range())
            .finish()
    }
}

impl fmt::Display for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn display_tree(f: &mut fmt::Formatter<'_>, node: &SyntaxNode, indent: usize) -> fmt::Result {
            writeln!(f, "{}{:?}@{}", "  ".repeat(indent), node.kind(), node.text_range())?;
            for child in node.children() {
                match child {
                    SyntaxElement::Node(n) => display_tree(f, &n, indent + 1)?,
                    SyntaxElement::Token(t) => {
                        writeln!(f, "{}{:?}@{} {:?}", "  ".repeat(indent + 1), t.kind(), t.text_range(), t.text())?;
                    }
                }
            }
            Ok(())
        }
        display_tree(f, self, 0)
    }
}

impl PartialEq for SyntaxNode {
    fn eq(&self, other: &Self) -> bool {
        self.green == other.green && self.offset == other.offset
    }
}

impl Eq for SyntaxNode {}

impl Hash for SyntaxNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.green.hash(state);
        self.offset.hash(state);
    }
}

/// Iterator for descendants.
struct DescendantsIter {
    stack: Vec<DescendantsFrame>,
}

struct DescendantsFrame {
    node: SyntaxNode,
    index: usize,
    yielded_self: bool,
}

impl DescendantsFrame {
    fn new(node: SyntaxNode) -> Self {
        Self {
            node,
            index: 0,
            yielded_self: false,
        }
    }
}

impl Iterator for DescendantsIter {
    type Item = SyntaxElement;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let frame = self.stack.last_mut()?;

            if !frame.yielded_self {
                frame.yielded_self = true;
                return Some(SyntaxElement::Node(frame.node.clone()));
            }

            if let Some(child) = frame.node.child(frame.index) {
                frame.index += 1;
                match child {
                    SyntaxElement::Node(n) => {
                        self.stack.push(DescendantsFrame::new(n));
                    }
                    SyntaxElement::Token(t) => {
                        return Some(SyntaxElement::Token(t));
                    }
                }
            } else {
                self.stack.pop();
            }
        }
    }
}

/// Red token - provides absolute position for tokens.
#[derive(Clone)]
pub struct SyntaxToken {
    /// Underlying green token
    green: GreenToken,
    /// Parent node
    parent: SyntaxNode,
    /// Index in parent's children
    index_in_parent: usize,
    /// Absolute offset in the source text
    offset: TextSize,
}

impl SyntaxToken {
    /// Create a new syntax token.
    fn new(green: GreenToken, parent: SyntaxNode, index: usize, offset: TextSize) -> Self {
        Self {
            green,
            parent,
            index_in_parent: index,
            offset,
        }
    }

    /// Get the underlying green token.
    #[inline]
    pub fn green(&self) -> &GreenToken {
        &self.green
    }

    /// Get the syntax kind of this token.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.green.kind()
    }

    /// Get the text of this token.
    #[inline]
    pub fn text(&self) -> &str {
        self.green.text()
    }

    /// Get the text range of this token.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::at(self.offset, self.green.width())
    }

    /// Get the start offset.
    #[inline]
    pub fn start(&self) -> TextSize {
        self.offset
    }

    /// Get the end offset.
    #[inline]
    pub fn end(&self) -> TextSize {
        self.offset + self.green.width()
    }

    /// Get the parent node.
    pub fn parent(&self) -> SyntaxNode {
        self.parent.clone()
    }

    /// Get the index in parent.
    pub fn index(&self) -> usize {
        self.index_in_parent
    }

    /// Get ancestors (starting with parent).
    pub fn ancestors(&self) -> impl Iterator<Item = SyntaxNode> {
        std::iter::once(self.parent()).chain(self.parent.ancestors())
    }

    /// Get the next sibling.
    pub fn next_sibling(&self) -> Option<SyntaxElement> {
        self.parent.child(self.index_in_parent + 1)
    }

    /// Get the previous sibling.
    pub fn prev_sibling(&self) -> Option<SyntaxElement> {
        if self.index_in_parent == 0 {
            return None;
        }
        self.parent.child(self.index_in_parent - 1)
    }

    /// Get the next token in the tree.
    pub fn next_token(&self) -> Option<SyntaxToken> {
        let mut current: SyntaxElement = SyntaxElement::Token(self.clone());
        loop {
            match current.next_sibling() {
                Some(SyntaxElement::Token(t)) => return Some(t),
                Some(SyntaxElement::Node(n)) => {
                    if let Some(t) = n.first_token() {
                        return Some(t);
                    }
                    current = SyntaxElement::Node(n);
                }
                None => {
                    let parent = current.parent()?;
                    current = SyntaxElement::Node(parent);
                }
            }
        }
    }

    /// Get the previous token in the tree.
    pub fn prev_token(&self) -> Option<SyntaxToken> {
        let mut current: SyntaxElement = SyntaxElement::Token(self.clone());
        loop {
            match current.prev_sibling() {
                Some(SyntaxElement::Token(t)) => return Some(t),
                Some(SyntaxElement::Node(n)) => {
                    if let Some(t) = n.last_token() {
                        return Some(t);
                    }
                    current = SyntaxElement::Node(n);
                }
                None => {
                    let parent = current.parent()?;
                    current = SyntaxElement::Node(parent);
                }
            }
        }
    }

    /// Check if this is a trivia token.
    pub fn is_trivia(&self) -> bool {
        self.kind().is_trivia()
    }

    /// Check if this token is a specific kind.
    pub fn is(&self, kind: SyntaxKind) -> bool {
        self.kind() == kind
    }
}

impl fmt::Debug for SyntaxToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxToken")
            .field("kind", &self.kind())
            .field("text", &self.text())
            .field("range", &self.text_range())
            .finish()
    }
}

impl PartialEq for SyntaxToken {
    fn eq(&self, other: &Self) -> bool {
        self.green == other.green && self.offset == other.offset
    }
}

impl Eq for SyntaxToken {}

impl Hash for SyntaxToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.green.hash(state);
        self.offset.hash(state);
    }
}

/// Either a syntax node or a syntax token.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SyntaxElement {
    /// A syntax node (interior node in the syntax tree).
    Node(SyntaxNode),
    /// A syntax token (leaf node in the syntax tree).
    Token(SyntaxToken),
}

impl SyntaxElement {
    /// Get the syntax kind.
    pub fn kind(&self) -> SyntaxKind {
        match self {
            SyntaxElement::Node(n) => n.kind(),
            SyntaxElement::Token(t) => t.kind(),
        }
    }

    /// Get the text range.
    pub fn text_range(&self) -> TextRange {
        match self {
            SyntaxElement::Node(n) => n.text_range(),
            SyntaxElement::Token(t) => t.text_range(),
        }
    }

    /// Get the parent node.
    pub fn parent(&self) -> Option<SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => n.parent(),
            SyntaxElement::Token(t) => Some(t.parent()),
        }
    }

    /// Get the next sibling.
    pub fn next_sibling(&self) -> Option<SyntaxElement> {
        match self {
            SyntaxElement::Node(n) => n.next_sibling(),
            SyntaxElement::Token(t) => t.next_sibling(),
        }
    }

    /// Get the previous sibling.
    pub fn prev_sibling(&self) -> Option<SyntaxElement> {
        match self {
            SyntaxElement::Node(n) => n.prev_sibling(),
            SyntaxElement::Token(t) => t.prev_sibling(),
        }
    }

    /// Check if this is a node.
    pub fn is_node(&self) -> bool {
        matches!(self, SyntaxElement::Node(_))
    }

    /// Check if this is a token.
    pub fn is_token(&self) -> bool {
        matches!(self, SyntaxElement::Token(_))
    }

    /// Get as a node.
    pub fn as_node(&self) -> Option<&SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => Some(n),
            _ => None,
        }
    }

    /// Get as a token.
    pub fn as_token(&self) -> Option<&SyntaxToken> {
        match self {
            SyntaxElement::Token(t) => Some(t),
            _ => None,
        }
    }

    /// Convert into a node.
    pub fn into_node(self) -> Option<SyntaxNode> {
        match self {
            SyntaxElement::Node(n) => Some(n),
            _ => None,
        }
    }

    /// Convert into a token.
    pub fn into_token(self) -> Option<SyntaxToken> {
        match self {
            SyntaxElement::Token(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Debug for SyntaxElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntaxElement::Node(n) => n.fmt(f),
            SyntaxElement::Token(t) => t.fmt(f),
        }
    }
}

impl From<SyntaxNode> for SyntaxElement {
    fn from(node: SyntaxNode) -> Self {
        SyntaxElement::Node(node)
    }
}

impl From<SyntaxToken> for SyntaxElement {
    fn from(token: SyntaxToken) -> Self {
        SyntaxElement::Token(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::green::GreenBuilder;

    fn build_test_tree() -> SyntaxNode {
        let mut builder = GreenBuilder::new();

        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.start_node(SyntaxKind::FN_DEF);
        builder.token(SyntaxKind::FN_KW, "fn");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "foo");
        builder.token(SyntaxKind::L_PAREN, "(");
        builder.token(SyntaxKind::R_PAREN, ")");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.start_node(SyntaxKind::BLOCK);
        builder.token(SyntaxKind::L_BRACE, "{");
        builder.token(SyntaxKind::R_BRACE, "}");
        builder.finish_node();
        builder.finish_node();
        builder.finish_node();

        SyntaxNode::new_root(builder.finish())
    }

    #[test]
    fn test_syntax_node_basic() {
        let root = build_test_tree();

        assert_eq!(root.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(root.text(), "fn foo() {}");
        assert_eq!(root.text_range(), TextRange::new(0, 11));
    }

    #[test]
    fn test_navigation() {
        let root = build_test_tree();

        let fn_def = root.first_child_node().unwrap();
        assert_eq!(fn_def.kind(), SyntaxKind::FN_DEF);

        let parent = fn_def.parent().unwrap();
        assert_eq!(parent.kind(), SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn test_token_at_offset() {
        let root = build_test_tree();

        let token = root.token_at_offset(0).unwrap();
        assert_eq!(token.text(), "fn");

        let token = root.token_at_offset(3).unwrap();
        assert_eq!(token.text(), "foo");

        let token = root.token_at_offset(6).unwrap();
        assert_eq!(token.text(), "(");
    }

    #[test]
    fn test_descendants() {
        let root = build_test_tree();

        let kinds: Vec<_> = root.descendants().map(|e| e.kind()).collect();

        assert!(kinds.contains(&SyntaxKind::SOURCE_FILE));
        assert!(kinds.contains(&SyntaxKind::FN_DEF));
        assert!(kinds.contains(&SyntaxKind::FN_KW));
        assert!(kinds.contains(&SyntaxKind::IDENT));
        assert!(kinds.contains(&SyntaxKind::BLOCK));
    }

    #[test]
    fn test_next_token() {
        let root = build_test_tree();

        let first = root.first_token().unwrap();
        assert_eq!(first.text(), "fn");

        let next = first.next_token().unwrap();
        assert_eq!(next.text(), " ");

        let next = next.next_token().unwrap();
        assert_eq!(next.text(), "foo");
    }

    #[test]
    fn test_covering_element() {
        let root = build_test_tree();

        let covering = root.covering_element(TextRange::new(3, 6));
        assert_eq!(covering.kind(), SyntaxKind::IDENT);
        assert_eq!(covering.as_token().unwrap().text(), "foo");
    }
}
