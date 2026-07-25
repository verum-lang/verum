//! Green tree - the immutable, persistent core of the syntax tree.
//!
//! Green trees store relative widths (not absolute offsets), enabling O(log n)
//! updates when source is edited. They form the "spine" of the red-green tree
//! pattern used by Roslyn and rust-analyzer.
//!
//! Green Tree (Immutable Core):
//! Green nodes store relative widths (not absolute offsets), enabling O(log n)
//! updates when source is edited -- only the path from edit to root needs recreation.
//! Forms the "spine" of the red-green tree pattern (Roslyn/rust-analyzer).
//! Token storage optimization: short tokens (up to 22 bytes) are stored inline
//! to avoid allocation; longer tokens use Arc<str> for shared allocation.
//! Green trees persist across edits via structural sharing of unchanged subtrees.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::SyntaxKind;

/// Text size type (byte offset).
pub type TextSize = u32;

/// A range in the text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    /// Create a new text range.
    pub const fn new(start: TextSize, end: TextSize) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    /// Create a range at a given offset with a given length.
    pub const fn at(offset: TextSize, len: TextSize) -> Self {
        Self {
            start: offset,
            end: offset + len,
        }
    }

    /// Create an empty range at the given offset.
    pub const fn empty(offset: TextSize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Start of the range.
    pub const fn start(self) -> TextSize {
        self.start
    }

    /// End of the range (exclusive).
    pub const fn end(self) -> TextSize {
        self.end
    }

    /// Length of the range.
    pub const fn len(self) -> TextSize {
        self.end - self.start
    }

    /// Returns true if the range is empty.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns true if the range contains the given offset.
    pub const fn contains(self, offset: TextSize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Returns true if this range contains the other range.
    pub const fn contains_range(self, other: TextRange) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Returns the intersection of two ranges, if any.
    pub fn intersect(self, other: TextRange) -> Option<TextRange> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start <= end {
            Some(TextRange::new(start, end))
        } else {
            None
        }
    }

    /// Extend the range to cover another range.
    pub fn cover(self, other: TextRange) -> TextRange {
        TextRange::new(self.start.min(other.start), self.end.max(other.end))
    }
}

impl fmt::Debug for TextRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl fmt::Display for TextRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// Immutable green node - the core of the syntax tree.
///
/// Green nodes store relative widths, not absolute offsets. This allows
/// efficient structural sharing when source is edited - only the path from
/// the edit to the root needs to be recreated.
#[derive(Clone)]
pub struct GreenNode {
    /// Syntax kind of this node
    kind: SyntaxKind,
    /// Total width (in bytes) of this node's text
    width: TextSize,
    /// Children of this node (nodes and tokens)
    children: Arc<[GreenChild]>,
}

impl GreenNode {
    /// Create a new green node.
    pub fn new(kind: SyntaxKind, children: Vec<GreenChild>) -> Self {
        let width = children.iter().map(|c| c.width()).sum();
        Self {
            kind,
            width,
            children: children.into(),
        }
    }

    /// Create a new green node from a slice of children.
    pub fn new_from_slice(kind: SyntaxKind, children: &[GreenChild]) -> Self {
        let width = children.iter().map(|c| c.width()).sum();
        Self {
            kind,
            width,
            children: children.into(),
        }
    }

    /// Create a leaf node with no children.
    pub fn leaf(kind: SyntaxKind) -> Self {
        Self {
            kind,
            width: 0,
            children: Arc::new([]),
        }
    }

    /// Get the syntax kind of this node.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// Get the width (in bytes) of this node's text.
    #[inline]
    pub fn width(&self) -> TextSize {
        self.width
    }

    /// Get the children of this node.
    #[inline]
    pub fn children(&self) -> &[GreenChild] {
        &self.children
    }

    /// Get the number of children.
    #[inline]
    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    /// Get a child by index.
    #[inline]
    pub fn child(&self, index: usize) -> Option<&GreenChild> {
        self.children.get(index)
    }

    /// Returns true if this node has no children.
    #[inline]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// Create new node with a child replaced (for incremental updates).
    pub fn replace_child(&self, index: usize, new_child: GreenChild) -> Self {
        let mut children: Vec<_> = self.children.iter().cloned().collect();
        if index < children.len() {
            children[index] = new_child;
        }
        Self::new(self.kind, children)
    }

    /// Create new node with a child inserted at the given index.
    pub fn insert_child(&self, index: usize, new_child: GreenChild) -> Self {
        let mut children: Vec<_> = self.children.iter().cloned().collect();
        children.insert(index.min(children.len()), new_child);
        Self::new(self.kind, children)
    }

    /// Create new node with the child at the given index removed.
    pub fn remove_child(&self, index: usize) -> Self {
        let mut children: Vec<_> = self.children.iter().cloned().collect();
        if index < children.len() {
            children.remove(index);
        }
        Self::new(self.kind, children)
    }

    /// Get the offset of a child within this node.
    pub fn child_offset(&self, index: usize) -> Option<TextSize> {
        if index >= self.children.len() {
            return None;
        }
        Some(self.children[..index].iter().map(|c| c.width()).sum())
    }

    /// Find the child at the given relative offset.
    pub fn child_at_offset(&self, offset: TextSize) -> Option<(usize, TextSize)> {
        let mut current_offset = 0;
        for (i, child) in self.children.iter().enumerate() {
            let child_width = child.width();
            if offset < current_offset + child_width {
                return Some((i, current_offset));
            }
            current_offset += child_width;
        }
        None
    }

    /// Iterate over all tokens in this node (depth-first).
    pub fn tokens(&self) -> impl Iterator<Item = &GreenToken> {
        GreenTokenIter {
            stack: vec![self.children.iter()],
        }
    }

    /// Collect the text of this node.
    pub fn text(&self) -> String {
        let mut result = String::with_capacity(self.width as usize);
        self.collect_text(&mut result);
        result
    }

    /// Collect text into a string buffer.
    fn collect_text(&self, buf: &mut String) {
        for child in self.children.iter() {
            match child {
                GreenChild::Node(n) => n.collect_text(buf),
                GreenChild::Token(t) => buf.push_str(t.text()),
            }
        }
    }
}

impl fmt::Debug for GreenNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GreenNode")
            .field("kind", &self.kind)
            .field("width", &self.width)
            .field("children", &self.children.len())
            .finish()
    }
}

impl PartialEq for GreenNode {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.width == other.width && self.children == other.children
    }
}

impl Eq for GreenNode {}

impl Hash for GreenNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.width.hash(state);
        self.children.hash(state);
    }
}

/// Iterator over all tokens in a green tree.
struct GreenTokenIter<'a> {
    stack: Vec<std::slice::Iter<'a, GreenChild>>,
}

impl<'a> Iterator for GreenTokenIter<'a> {
    type Item = &'a GreenToken;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let iter = self.stack.last_mut()?;
            match iter.next() {
                Some(GreenChild::Token(t)) => return Some(t),
                Some(GreenChild::Node(n)) => {
                    self.stack.push(n.children.iter());
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

/// Immutable green token - a leaf in the syntax tree.
///
/// Tokens store their actual text content along with their kind.
#[derive(Clone)]
pub struct GreenToken {
    /// Syntax kind of this token
    kind: SyntaxKind,
    /// The actual text of this token
    text: GreenTokenText,
}

/// Compact text storage for tokens.
#[derive(Clone)]
enum GreenTokenText {
    /// Inline storage for short text (up to 22 bytes)
    Inline { len: u8, data: [u8; 22] },
    /// Arc storage for longer text (shared)
    Arc(Arc<str>),
}

impl GreenToken {
    /// Maximum length for inline storage.
    const MAX_INLINE_LEN: usize = 22;

    /// Create a new green token.
    pub fn new(kind: SyntaxKind, text: &str) -> Self {
        let text = if text.len() <= Self::MAX_INLINE_LEN {
            let mut data = [0u8; 22];
            data[..text.len()].copy_from_slice(text.as_bytes());
            GreenTokenText::Inline {
                len: text.len() as u8,
                data,
            }
        } else {
            GreenTokenText::Arc(text.into())
        };
        Self { kind, text }
    }

    /// Create a token from an Arc<str>.
    pub fn from_arc(kind: SyntaxKind, text: Arc<str>) -> Self {
        Self {
            kind,
            text: GreenTokenText::Arc(text),
        }
    }

    /// Get the syntax kind of this token.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// Get the text of this token.
    #[inline]
    pub fn text(&self) -> &str {
        match &self.text {
            GreenTokenText::Inline { len, data } => {
                // SAFETY: We only store valid UTF-8
                unsafe { std::str::from_utf8_unchecked(&data[..*len as usize]) }
            }
            GreenTokenText::Arc(s) => s,
        }
    }

    /// Get the width (in bytes) of this token.
    #[inline]
    pub fn width(&self) -> TextSize {
        self.text().len() as TextSize
    }
}

impl fmt::Debug for GreenToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GreenToken")
            .field("kind", &self.kind)
            .field("text", &self.text())
            .finish()
    }
}

impl PartialEq for GreenToken {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.text() == other.text()
    }
}

impl Eq for GreenToken {}

impl Hash for GreenToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.text().hash(state);
    }
}

/// Either a node or a token.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum GreenChild {
    /// A composite node
    Node(GreenNode),
    /// A leaf token
    Token(GreenToken),
}

impl GreenChild {
    /// Get the width of this child.
    #[inline]
    pub fn width(&self) -> TextSize {
        match self {
            GreenChild::Node(n) => n.width(),
            GreenChild::Token(t) => t.width(),
        }
    }

    /// Get the syntax kind of this child.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        match self {
            GreenChild::Node(n) => n.kind(),
            GreenChild::Token(t) => t.kind(),
        }
    }

    /// Returns true if this is a node.
    #[inline]
    pub fn is_node(&self) -> bool {
        matches!(self, GreenChild::Node(_))
    }

    /// Returns true if this is a token.
    #[inline]
    pub fn is_token(&self) -> bool {
        matches!(self, GreenChild::Token(_))
    }

    /// Get as a node, if this is a node.
    #[inline]
    pub fn as_node(&self) -> Option<&GreenNode> {
        match self {
            GreenChild::Node(n) => Some(n),
            _ => None,
        }
    }

    /// Get as a token, if this is a token.
    #[inline]
    pub fn as_token(&self) -> Option<&GreenToken> {
        match self {
            GreenChild::Token(t) => Some(t),
            _ => None,
        }
    }

    /// Convert into a node, if this is a node.
    pub fn into_node(self) -> Option<GreenNode> {
        match self {
            GreenChild::Node(n) => Some(n),
            _ => None,
        }
    }

    /// Convert into a token, if this is a token.
    pub fn into_token(self) -> Option<GreenToken> {
        match self {
            GreenChild::Token(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Debug for GreenChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GreenChild::Node(n) => n.fmt(f),
            GreenChild::Token(t) => t.fmt(f),
        }
    }
}

impl From<GreenNode> for GreenChild {
    fn from(node: GreenNode) -> Self {
        GreenChild::Node(node)
    }
}

impl From<GreenToken> for GreenChild {
    fn from(token: GreenToken) -> Self {
        GreenChild::Token(token)
    }
}

/// Builder for green trees using stack-based construction.
///
/// This builder efficiently constructs green trees without intermediate
/// allocations for children lists.
pub struct GreenBuilder {
    /// Stack of parent nodes being built
    parents: Vec<(SyntaxKind, Vec<GreenChild>)>,
}

impl GreenBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            parents: Vec::new(),
        }
    }

    /// Start a new node.
    pub fn start_node(&mut self, kind: SyntaxKind) {
        self.parents.push((kind, Vec::new()));
    }

    /// Add a token to the current node.
    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let token = GreenToken::new(kind, text);
        match self.parents.last_mut() {
            Some((_, children)) => children.push(GreenChild::Token(token)),
            None => panic!("token without parent node"),
        }
    }

    /// Finish the current node.
    pub fn finish_node(&mut self) {
        let (kind, children) = self.parents.pop().expect("finish without start");
        let node = GreenNode::new(kind, children);

        match self.parents.last_mut() {
            Some((_, parent_children)) => {
                parent_children.push(GreenChild::Node(node));
            }
            None => {
                // Root node - push back to be retrieved by finish()
                self.parents.push((kind, vec![GreenChild::Node(node)]));
            }
        }
    }

    /// Finish building and return the root node.
    pub fn finish(mut self) -> GreenNode {
        assert_eq!(self.parents.len(), 1, "unbalanced start/finish");
        let (_, mut children) = self.parents.pop().unwrap();
        match children.pop() {
            Some(GreenChild::Node(root)) => root,
            _ => panic!("no root node"),
        }
    }

    /// Check if we're currently inside a node.
    pub fn is_building(&self) -> bool {
        !self.parents.is_empty()
    }

    /// Get the current depth (number of open nodes).
    pub fn depth(&self) -> usize {
        self.parents.len()
    }

    /// Abandon the current node (for error recovery).
    pub fn abandon_node(&mut self) {
        if let Some((_, children)) = self.parents.pop() {
            // Move children to parent
            if let Some((_, parent_children)) = self.parents.last_mut() {
                parent_children.extend(children);
            }
        }
    }
}

impl Default for GreenBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_range() {
        let range = TextRange::new(10, 20);
        assert_eq!(range.start(), 10);
        assert_eq!(range.end(), 20);
        assert_eq!(range.len(), 10);
        assert!(!range.is_empty());
        assert!(range.contains(15));
        assert!(!range.contains(5));
        assert!(!range.contains(20));
    }

    #[test]
    fn test_green_token() {
        let token = GreenToken::new(SyntaxKind::IDENT, "foo");
        assert_eq!(token.kind(), SyntaxKind::IDENT);
        assert_eq!(token.text(), "foo");
        assert_eq!(token.width(), 3);
    }

    #[test]
    fn test_green_token_long() {
        let long_text = "a".repeat(30);
        let token = GreenToken::new(SyntaxKind::STRING_LITERAL, &long_text);
        assert_eq!(token.text(), long_text);
        assert_eq!(token.width(), 30);
    }

    #[test]
    fn test_green_node() {
        let token1 = GreenToken::new(SyntaxKind::LET_KW, "let");
        let token2 = GreenToken::new(SyntaxKind::WHITESPACE, " ");
        let token3 = GreenToken::new(SyntaxKind::IDENT, "x");

        let node = GreenNode::new(
            SyntaxKind::LET_STMT,
            vec![
                GreenChild::Token(token1),
                GreenChild::Token(token2),
                GreenChild::Token(token3),
            ],
        );

        assert_eq!(node.kind(), SyntaxKind::LET_STMT);
        assert_eq!(node.width(), 5); // "let" + " " + "x"
        assert_eq!(node.children_count(), 3);
        assert_eq!(node.text(), "let x");
    }

    #[test]
    fn test_green_builder() {
        let mut builder = GreenBuilder::new();

        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.start_node(SyntaxKind::LET_STMT);
        builder.token(SyntaxKind::LET_KW, "let");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "x");
        builder.finish_node();
        builder.finish_node();

        let root = builder.finish();

        assert_eq!(root.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(root.text(), "let x");
    }

    #[test]
    fn test_node_replace_child() {
        let token1 = GreenToken::new(SyntaxKind::IDENT, "old");
        let token2 = GreenToken::new(SyntaxKind::IDENT, "new");

        let node = GreenNode::new(SyntaxKind::PATH, vec![GreenChild::Token(token1)]);

        let new_node = node.replace_child(0, GreenChild::Token(token2));

        assert_eq!(node.text(), "old");
        assert_eq!(new_node.text(), "new");
    }

    #[test]
    fn test_child_at_offset() {
        let token1 = GreenToken::new(SyntaxKind::LET_KW, "let");
        let token2 = GreenToken::new(SyntaxKind::WHITESPACE, " ");
        let token3 = GreenToken::new(SyntaxKind::IDENT, "x");

        let node = GreenNode::new(
            SyntaxKind::LET_STMT,
            vec![
                GreenChild::Token(token1),
                GreenChild::Token(token2),
                GreenChild::Token(token3),
            ],
        );

        assert_eq!(node.child_at_offset(0), Some((0, 0)));
        assert_eq!(node.child_at_offset(2), Some((0, 0)));
        assert_eq!(node.child_at_offset(3), Some((1, 3)));
        assert_eq!(node.child_at_offset(4), Some((2, 4)));
    }

    #[test]
    fn test_tokens_iterator() {
        let mut builder = GreenBuilder::new();

        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.start_node(SyntaxKind::LET_STMT);
        builder.token(SyntaxKind::LET_KW, "let");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "x");
        builder.finish_node();
        builder.finish_node();

        let root = builder.finish();
        let tokens: Vec<_> = root.tokens().collect();

        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].text(), "let");
        assert_eq!(tokens[1].text(), " ");
        assert_eq!(tokens[2].text(), "x");
    }
}
