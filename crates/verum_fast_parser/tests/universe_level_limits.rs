use verum_ast::FileId;
use verum_common::Text;
use verum_fast_parser::VerumParser;

fn nested_level(depth: usize, operand_on_left: bool) -> Text {
    let mut expression = Text::from("0");
    for _ in 0..depth {
        expression = if operand_on_left {
            format!("max({expression}, 1)").into()
        } else {
            format!("max(1, {expression})").into()
        };
    }
    expression
}

#[test]
fn ordinary_nested_maxima_remain_available() {
    for operand_on_left in [true, false] {
        let source = format!(
            "fn inspect(value: Type({})) {{}}",
            nested_level(16, operand_on_left)
        );
        VerumParser::new()
            .parse_module_str(&source, FileId::new(0))
            .unwrap();
    }
}

#[test]
fn universe_nesting_uses_the_parser_safety_budget() {
    for operand_on_left in [true, false] {
        let source = format!(
            "fn inspect(value: Type({})) {{}}",
            nested_level(256, operand_on_left)
        );
        let errors = VerumParser::new()
            .parse_module_str(&source, FileId::new(0))
            .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("Maximum nesting depth")),
            "{errors:?}"
        );
    }
}

#[test]
fn universe_literals_refuse_truncation_and_negative_levels() {
    for value in ["4294967296", "18446744073709551616", "-1"] {
        let source = format!("fn inspect(value: Type({value})) {{}}");
        assert!(
            VerumParser::new()
                .parse_module_str(&source, FileId::new(0))
                .is_err(),
            "{source}"
        );
    }
    VerumParser::new()
        .parse_module_str("fn inspect(value: Type(4294967295)) {}", FileId::new(0))
        .unwrap();
}
