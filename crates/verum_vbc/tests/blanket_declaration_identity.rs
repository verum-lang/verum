#![cfg(feature = "codegen")]
//! Source collection must not erase distinct conditional implementations.
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn imports_keep_distinct_owners_and_complete_declarations() {
    let sources = [
        ("alpha", "implement<T: Base> Granted for T {}"),
        ("beta", "implement<T: Base> Granted for T {}"),
        ("alpha", "implement<T: Base + Extra> Granted for T {}"),
        ("alpha", "implement<T: Base> Granted for T where type T: Extra {}"),
    ];
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for (owner, source) in sources {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
        producer.collect_blanket_impls(&ast).unwrap();
        producer.collect_blanket_impls(&ast).unwrap();
        let expected = if source.contains("Base + Extra") { 2 } else { 1 };
        assert_eq!(producer.blanket_impls().len(), expected);
        consumer.import_blanket_impls(producer.blanket_impls());
        consumer.import_blanket_impls(producer.blanket_impls());
    }
    // One row per trigger. The conjunctive declaration contributes two rows,
    // retaining the same complete header in each, rather than two new rules.
    assert_eq!(consumer.blanket_impls().len(), 5);
    let rows = consumer.blanket_impls();
    assert!(!rows[0].same_declaration(&rows[1]));
    assert!(!rows[0].same_declaration(&rows[2]));
    assert!(!rows[0].same_declaration(&rows[4]));
    assert_eq!(rows[2].declaration, rows[3].declaration);
    assert_ne!(rows[2].prerequisite, rows[3].prerequisite);
}
