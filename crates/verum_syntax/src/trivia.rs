//! Trivia types for non-semantic source elements.
//!
//! Trivia represents whitespace, comments, and other elements that don't affect
//! program semantics but are essential for lossless source reconstruction.
//!
//! Trivia Preservation Rules:
//! Trivia is attached to tokens (not stored as separate tree nodes):
//! - Leading trivia: all whitespace/comments from start of line up to the token
//! - Trailing trivia: whitespace/comments after token up to (not including) newline
//! - Newlines become leading trivia of the next token
//! This enables perfect source reconstruction: reconstruct(tokenize(source)) == source

use smallvec::SmallVec;
use std::fmt;

use crate::SyntaxKind;

/// Trivia represents non-semantic source elements.
///
/// These are attached to tokens (leading or trailing) rather than stored
/// as separate nodes in the syntax tree.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Trivia {
    /// Contiguous whitespace (spaces, tabs)
    Whitespace(TriviaText),
    /// Single newline (LF or CRLF)
    Newline(TriviaText),
    /// Line comment: `// ...`
    LineComment(TriviaText),
    /// Block comment: `/* ... */`
    BlockComment(TriviaText),
    /// Doc comment: `/// ...`
    DocComment(DocCommentKind, TriviaText),
}

/// Kind of doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocCommentKind {
    /// `/// ...` - outer doc comment (documents the following item)
    Outer,
    /// `//! ...` - inner doc comment (documents the enclosing item)
    Inner,
}

/// Compact text storage for trivia content.
///
/// For short trivia (most whitespace), we inline the text.
/// For longer trivia (comments), we use heap allocation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TriviaText {
    inner: TriviaTextInner,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum TriviaTextInner {
    /// Inline storage for short text (up to 22 bytes on 64-bit)
    Inline { len: u8, data: [u8; 22] },
    /// Heap allocation for longer text
    Heap(Box<str>),
}

impl TriviaText {
    /// Maximum length for inline storage.
    const MAX_INLINE_LEN: usize = 22;

    /// Create a new TriviaText from a string.
    pub fn new(s: &str) -> Self {
        if s.len() <= Self::MAX_INLINE_LEN {
            let mut data = [0u8; 22];
            data[..s.len()].copy_from_slice(s.as_bytes());
            Self {
                inner: TriviaTextInner::Inline {
                    len: s.len() as u8,
                    data,
                },
            }
        } else {
            Self {
                inner: TriviaTextInner::Heap(s.into()),
            }
        }
    }

    /// Get the text as a string slice.
    pub fn as_str(&self) -> &str {
        match &self.inner {
            TriviaTextInner::Inline { len, data } => {
                // SAFETY: We only store valid UTF-8 in inline storage
                unsafe { std::str::from_utf8_unchecked(&data[..*len as usize]) }
            }
            TriviaTextInner::Heap(s) => s,
        }
    }

    /// Get the length in bytes.
    pub fn len(&self) -> usize {
        match &self.inner {
            TriviaTextInner::Inline { len, .. } => *len as usize,
            TriviaTextInner::Heap(s) => s.len(),
        }
    }

    /// Returns true if the text is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for TriviaText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl fmt::Display for TriviaText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for TriviaText {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for TriviaText {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl Trivia {
    /// Create a whitespace trivia.
    pub fn whitespace(text: impl Into<TriviaText>) -> Self {
        Trivia::Whitespace(text.into())
    }

    /// Create a newline trivia.
    pub fn newline(text: impl Into<TriviaText>) -> Self {
        Trivia::Newline(text.into())
    }

    /// Create a line comment trivia.
    pub fn line_comment(text: impl Into<TriviaText>) -> Self {
        Trivia::LineComment(text.into())
    }

    /// Create a block comment trivia.
    pub fn block_comment(text: impl Into<TriviaText>) -> Self {
        Trivia::BlockComment(text.into())
    }

    /// Create an outer doc comment trivia.
    pub fn outer_doc_comment(text: impl Into<TriviaText>) -> Self {
        Trivia::DocComment(DocCommentKind::Outer, text.into())
    }

    /// Create an inner doc comment trivia.
    pub fn inner_doc_comment(text: impl Into<TriviaText>) -> Self {
        Trivia::DocComment(DocCommentKind::Inner, text.into())
    }

    /// Get the SyntaxKind corresponding to this trivia.
    pub fn syntax_kind(&self) -> SyntaxKind {
        match self {
            Trivia::Whitespace(_) => SyntaxKind::WHITESPACE,
            Trivia::Newline(_) => SyntaxKind::NEWLINE,
            Trivia::LineComment(_) => SyntaxKind::LINE_COMMENT,
            Trivia::BlockComment(_) => SyntaxKind::BLOCK_COMMENT,
            Trivia::DocComment(DocCommentKind::Outer, _) => SyntaxKind::DOC_COMMENT,
            Trivia::DocComment(DocCommentKind::Inner, _) => SyntaxKind::INNER_DOC_COMMENT,
        }
    }

    /// Get the text of this trivia.
    pub fn text(&self) -> &str {
        match self {
            Trivia::Whitespace(t)
            | Trivia::Newline(t)
            | Trivia::LineComment(t)
            | Trivia::BlockComment(t)
            | Trivia::DocComment(_, t) => t.as_str(),
        }
    }

    /// Get the byte length of this trivia.
    pub fn len(&self) -> usize {
        self.text().len()
    }

    /// Returns true if this trivia is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns true if this is a whitespace trivia.
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Trivia::Whitespace(_))
    }

    /// Returns true if this is a newline trivia.
    pub fn is_newline(&self) -> bool {
        matches!(self, Trivia::Newline(_))
    }

    /// Returns true if this is a comment trivia.
    pub fn is_comment(&self) -> bool {
        matches!(
            self,
            Trivia::LineComment(_) | Trivia::BlockComment(_) | Trivia::DocComment(_, _)
        )
    }

    /// Returns true if this is a doc comment trivia.
    pub fn is_doc_comment(&self) -> bool {
        matches!(self, Trivia::DocComment(_, _))
    }
}

impl fmt::Debug for Trivia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trivia::Whitespace(t) => write!(f, "Whitespace({:?})", t.as_str()),
            Trivia::Newline(t) => write!(f, "Newline({:?})", t.as_str()),
            Trivia::LineComment(t) => write!(f, "LineComment({:?})", t.as_str()),
            Trivia::BlockComment(t) => write!(f, "BlockComment({:?})", t.as_str()),
            Trivia::DocComment(kind, t) => write!(f, "DocComment({:?}, {:?})", kind, t.as_str()),
        }
    }
}

/// Compact storage for trivia lists.
///
/// Most tokens have 0-2 trivia items attached, so we use SmallVec
/// to avoid heap allocation for common cases.
pub type TriviaList = SmallVec<[Trivia; 2]>;

/// Extension trait for TriviaList operations.
pub trait TriviaListExt {
    /// Get the total byte length of all trivia.
    fn total_len(&self) -> usize;

    /// Concatenate all trivia text.
    fn concat_text(&self) -> String;

    /// Returns true if any trivia is a comment.
    fn has_comment(&self) -> bool;

    /// Returns true if any trivia is a doc comment.
    fn has_doc_comment(&self) -> bool;

    /// Returns true if any trivia contains a newline.
    fn has_newline(&self) -> bool;
}

impl TriviaListExt for TriviaList {
    fn total_len(&self) -> usize {
        self.iter().map(|t| t.len()).sum()
    }

    fn concat_text(&self) -> String {
        let mut result = String::with_capacity(self.total_len());
        for trivia in self.iter() {
            result.push_str(trivia.text());
        }
        result
    }

    fn has_comment(&self) -> bool {
        self.iter().any(|t| t.is_comment())
    }

    fn has_doc_comment(&self) -> bool {
        self.iter().any(|t| t.is_doc_comment())
    }

    fn has_newline(&self) -> bool {
        self.iter().any(|t| t.is_newline())
    }
}

/// Trivia attachment rules following Swift-style ownership.
///
/// - Token owns TRAILING trivia up to (not including) next newline
/// - Token owns LEADING trivia starting from first newline sequence
///
/// This ensures that:
/// - Comments on the same line stay with their token
/// - Indentation belongs to the following token
/// - Blank lines are handled consistently
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriviaPosition {
    /// Trivia before the token (indentation, preceding blank lines)
    Leading,
    /// Trivia after the token (same-line comments, trailing spaces)
    Trailing,
}

/// Classifies a sequence of trivia characters into structured trivia items.
///
/// This function parses raw source text and produces a list of trivia items.
pub fn classify_trivia(text: &str) -> TriviaList {
    let mut result = TriviaList::new();
    let mut chars = text.chars().peekable();
    let mut current_start = 0;
    let mut current_pos = 0;

    while let Some(c) = chars.next() {
        match c {
            // Newline handling
            '\n' => {
                if current_start < current_pos {
                    // Flush any accumulated whitespace
                    result.push(Trivia::whitespace(&text[current_start..current_pos]));
                }
                // Check for CRLF
                let newline_end = if chars.peek() == Some(&'\r') {
                    chars.next();
                    current_pos + 2
                } else {
                    current_pos + 1
                };
                result.push(Trivia::newline(&text[current_pos..newline_end]));
                current_pos = newline_end;
                current_start = current_pos;
            }
            '\r' => {
                if current_start < current_pos {
                    result.push(Trivia::whitespace(&text[current_start..current_pos]));
                }
                // Check for CRLF
                let newline_end = if chars.peek() == Some(&'\n') {
                    chars.next();
                    current_pos + 2
                } else {
                    current_pos + 1
                };
                result.push(Trivia::newline(&text[current_pos..newline_end]));
                current_pos = newline_end;
                current_start = current_pos;
            }
            // Line comment
            '/' if chars.peek() == Some(&'/') => {
                if current_start < current_pos {
                    result.push(Trivia::whitespace(&text[current_start..current_pos]));
                }
                chars.next(); // consume second /
                current_pos += 2;

                // Check for doc comment
                let is_inner_doc = chars.peek() == Some(&'!');
                let is_outer_doc = chars.peek() == Some(&'/');

                // Consume rest of line
                let comment_start = current_pos - 2;
                while let Some(&c) = chars.peek() {
                    if c == '\n' || c == '\r' {
                        break;
                    }
                    chars.next();
                    current_pos += c.len_utf8();
                }

                let comment_text = &text[comment_start..current_pos];
                if is_inner_doc {
                    result.push(Trivia::inner_doc_comment(comment_text));
                } else if is_outer_doc {
                    result.push(Trivia::outer_doc_comment(comment_text));
                } else {
                    result.push(Trivia::line_comment(comment_text));
                }
                current_start = current_pos;
            }
            // Block comment
            '/' if chars.peek() == Some(&'*') => {
                if current_start < current_pos {
                    result.push(Trivia::whitespace(&text[current_start..current_pos]));
                }
                chars.next(); // consume *
                current_pos += 2;

                let comment_start = current_pos - 2;
                let mut depth = 1;

                while depth > 0 {
                    match chars.next() {
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            current_pos += 2;
                            depth -= 1;
                        }
                        Some('/') if chars.peek() == Some(&'*') => {
                            chars.next();
                            current_pos += 2;
                            depth += 1;
                        }
                        Some(c) => {
                            current_pos += c.len_utf8();
                        }
                        None => break, // Unclosed comment
                    }
                }

                result.push(Trivia::block_comment(&text[comment_start..current_pos]));
                current_start = current_pos;
            }
            // Whitespace
            ' ' | '\t' => {
                current_pos += 1;
            }
            // Other characters (shouldn't happen in trivia, but handle gracefully)
            _ => {
                current_pos += c.len_utf8();
            }
        }
    }

    // Flush any remaining whitespace
    if current_start < current_pos {
        result.push(Trivia::whitespace(&text[current_start..current_pos]));
    }

    result
}

/// Split trivia into leading and trailing based on newline boundaries.
///
/// Trailing trivia: everything up to (not including) the first newline
/// Leading trivia: everything from the first newline onward
pub fn split_trivia(trivia: TriviaList) -> (TriviaList, TriviaList) {
    let mut leading = TriviaList::new();
    let mut trailing = TriviaList::new();
    let mut seen_newline = false;

    for item in trivia {
        if seen_newline {
            leading.push(item);
        } else if item.is_newline() {
            leading.push(item);
            seen_newline = true;
        } else {
            trailing.push(item);
        }
    }

    (leading, trailing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trivia_text_inline() {
        let text = TriviaText::new("  ");
        assert_eq!(text.as_str(), "  ");
        assert_eq!(text.len(), 2);
    }

    #[test]
    fn test_trivia_text_heap() {
        let long_text = "a".repeat(30);
        let text = TriviaText::new(&long_text);
        assert_eq!(text.as_str(), long_text);
        assert_eq!(text.len(), 30);
    }

    #[test]
    fn test_classify_whitespace() {
        let trivia = classify_trivia("  \t ");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_whitespace());
        assert_eq!(trivia[0].text(), "  \t ");
    }

    #[test]
    fn test_classify_newline() {
        let trivia = classify_trivia("\n");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_newline());
    }

    #[test]
    fn test_classify_line_comment() {
        let trivia = classify_trivia("// comment");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_comment());
        assert_eq!(trivia[0].text(), "// comment");
    }

    #[test]
    fn test_classify_doc_comment() {
        let trivia = classify_trivia("/// doc comment");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_doc_comment());
    }

    #[test]
    fn test_classify_block_comment() {
        let trivia = classify_trivia("/* block */");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_comment());
        assert_eq!(trivia[0].text(), "/* block */");
    }

    #[test]
    fn test_classify_nested_block_comment() {
        let trivia = classify_trivia("/* outer /* inner */ end */");
        assert_eq!(trivia.len(), 1);
        assert!(trivia[0].is_comment());
        assert_eq!(trivia[0].text(), "/* outer /* inner */ end */");
    }

    #[test]
    fn test_classify_mixed() {
        let trivia = classify_trivia("  // comment\n    ");
        assert_eq!(trivia.len(), 4);
        assert!(trivia[0].is_whitespace());
        assert!(trivia[1].is_comment());
        assert!(trivia[2].is_newline());
        assert!(trivia[3].is_whitespace());
    }

    #[test]
    fn test_split_trivia() {
        let trivia = classify_trivia("  // comment\n    ");
        let (leading, trailing) = split_trivia(trivia);

        assert_eq!(trailing.len(), 2); // whitespace, comment
        assert_eq!(leading.len(), 2); // newline, whitespace
    }

    #[test]
    fn test_trivia_list_ext() {
        let trivia = classify_trivia("  // comment\n");
        assert!(trivia.has_comment());
        assert!(trivia.has_newline());
        assert!(!trivia.has_doc_comment());
        assert_eq!(trivia.total_len(), "  // comment\n".len());
    }
}
