//! Apply a pretty-printer's layout without losing lexical source information.
use verum_ast::FileId;
use verum_common::Text;
use verum_lexer::Lexer;

/// A layout proposal may change whitespace, but every code lexeme must match.
/// Lexer gaps carry comments, documentation and shebangs; copy these verbatim
/// between the same code tokens. This also prevents unsupported AST printing
/// from silently deleting constructs or rewriting literal payloads.
pub(super) fn preserve(source: &str, layout: &str) -> Option<Text> {
    let original = Lexer::new(source, FileId::new(0)).tokenize().ok()?;
    let proposed = Lexer::new(layout, FileId::new(0)).tokenize().ok()?;
    if original.len() != proposed.len() {
        return None;
    }
    let mut result = Text::new();
    let (mut original_end, mut proposed_end) = (0, 0);
    for (before, after) in original.iter().zip(proposed.iter()) {
        let token = source.get(before.span.start as usize..before.span.end as usize)?;
        if token != layout.get(after.span.start as usize..after.span.end as usize)? {
            return None;
        }
        let gap = source.get(original_end..before.span.start as usize)?;
        let suggested = layout.get(proposed_end..after.span.start as usize)?;
        result.push_str(
            if gap.chars().all(char::is_whitespace) && suggested.chars().all(char::is_whitespace) {
                suggested
            } else {
                gap
            },
        );
        result.push_str(token);
        original_end = before.span.end as usize;
        proposed_end = after.span.end as usize;
    }
    Some(result)
}

/// Line-based sorting cannot establish comment attachment or modify the lines
/// inside a literal. Keep their ordering until an AST attachment model is used.
pub(super) fn can_reorder_lines(source: &str) -> bool {
    let Ok(tokens) = Lexer::new(source, FileId::new(0)).tokenize() else {
        return false;
    };
    let mut end = 0;
    for token in tokens {
        let start = token.span.start as usize;
        let next = token.span.end as usize;
        if !source[end..start].chars().all(char::is_whitespace)
            || source[start..next].contains(['\n', '\r'])
        {
            return false;
        }
        end = next;
    }
    true
}
