//! Source documents use the compiler parser's script entry and original spans.
use verum_ast::{FileId, Module};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_lexer::Lexer;
use verum_common::source_mode::SourceMode;
use verum_parser::{ParseError, ParseResult, VerumParser};

pub(crate) fn is_script(source: &str, file_id: FileId) -> bool {
    Lexer::new(source, file_id).had_shebang()
}

pub(crate) fn parse(source: &str, file_id: FileId) -> ParseResult<Module> {
    parse_with_mode(source, file_id, SourceMode::Source)
}

pub(crate) fn parse_with_mode(source: &str, file_id: FileId, mode: SourceMode) -> ParseResult<Module> {
    let parser = VerumParser::new();
    if mode == SourceMode::Script || is_script(source, file_id) {
        parser.parse_module_script_str(source, file_id)
    } else {
        parser.parse_module_str(source, file_id)
    }
}

pub(crate) fn diagnostic(error: ParseError, source: &str) -> Diagnostic {
    DiagnosticBuilder::new(Severity::Error)
        .message(error.to_string())
        .span(verum_common::span_utils::span_to_line_col_span(
            error.span, source, "",
        ))
        .build()
}
