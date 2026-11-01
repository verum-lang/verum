use verum_ast::FileId;
use verum_cli::commands::fmt::format_string;
use verum_common::{List, Text};
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn lexical_content(source: &str) -> (List<Text>, List<Text>) {
    let tokens = Lexer::new(source, FileId::new(0)).tokenize().unwrap();
    let (mut code, mut comments) = (List::new(), List::new());
    let mut end = 0;
    for token in tokens {
        let gap = &source[end..token.span.start as usize];
        if !gap.chars().all(char::is_whitespace) {
            comments.push(Text::from(gap));
        }
        code.push(Text::from(
            &source[token.span.start as usize..token.span.end as usize],
        ));
        end = token.span.end as usize;
    }
    (code, comments)
}

#[test]
fn comments_stay_between_the_same_exact_tokens_and_formatting_is_idempotent() {
    let sources = [
        "// leading Ω😀\nfn main()->Int{42}\n",
        "/// documentation\nfn main(/* parameter boundary */)->Int{\n// body\n42 /* outer /* nested */ tail */\n} // trailing\n",
        "// import attachment\nmount core.text;\nmount core.base;\nfn main()->Int{42}\n",
        "/* comment-only Ω😀 */\n// last line",
    ];
    for source in sources {
        VerumParser::new()
            .parse_module(Lexer::new(source, FileId::new(0)), FileId::new(0))
            .unwrap();
        let output = format_string(source).unwrap();
        // The configured final newline terminates an EOF line comment; all
        // original comment bytes and their attachment to code remain exact.
        let expected = if source.ends_with('\n') {
            source.to_owned()
        } else {
            format!("{source}\n")
        };
        assert_eq!(
            lexical_content(&expected),
            lexical_content(&output),
            "{output}"
        );
        assert_eq!(format_string(&output).unwrap(), output, "not a fixed point");
        VerumParser::new()
            .parse_module(Lexer::new(&output, FileId::new(0)), FileId::new(0))
            .unwrap();
    }
    assert_eq!(
        format_string(sources[0]).unwrap().as_str(),
        "// leading Ω😀\nfn main() -> Int {\n    42\n}\n"
    );
}

#[test]
fn literal_payloads_are_data_not_lines_to_trim_or_sort() {
    for source in [
        "fn text()->Text{\"mount z;\nmount a;\n\n  Ω😀  \"}\n",
        "fn value()->Int{0x00ff}\n",
    ] {
        VerumParser::new()
            .parse_module(Lexer::new(source, FileId::new(0)), FileId::new(0))
            .unwrap();
        let output = format_string(source).unwrap();
        assert_eq!(lexical_content(source), lexical_content(&output));
        assert_eq!(format_string(&output).unwrap(), output);
    }
}

#[test]
fn lexical_errors_are_left_intact_instead_of_rewriting_their_payload() {
    let source = "fn text() { \"unterminated\n  payload  ";
    assert_eq!(format_string(source).unwrap().as_str(), source);
}
