use verum_ast::{FileId, ItemKind};
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

#[test]
fn declaration_names_retain_the_consumed_identifier_not_the_next_token() {
    for source in [
        "public fn identity<A>(x:A)->A { x }",
        "fn show() {}",
        "fn имя() {}",
        "type Point is { x: Int };",
        "type Wrapper<A> is { value: A };",
        "context Logger { fn log(message:Text); }",
        "protocol Marker {}",
        "const answer: Int = 42;",
        "module nested { fn inner() {} }",
        "module runtime.recovery;",
        "theorem identity(A:Type,x:A):A proof=x;",
        "theorem trivial() ensures true proof by trivial;",
        "lemma helper(A:Type,x:A):A proof=x;",
        "corollary consequence(A:Type,x:A):A proof=x;",
        "axiom assumed(A:Type) ensures A;",
    ] {
        let source = format!("// Unicode prefix: Ω🙂\n{source}");
        let file = FileId::new(37);
        let module = VerumParser::new().parse_module(Lexer::new(&source, file), file)
            .unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let name = match &module.items[0].kind {
            ItemKind::Function(d) => &d.name,
            ItemKind::Type(d) => &d.name,
            ItemKind::Context(d) => &d.name,
            ItemKind::Protocol(d) => &d.name,
            ItemKind::Const(d) => &d.name,
            ItemKind::Module(d) => &d.name,
            ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => &d.name,
            ItemKind::Axiom(d) => &d.name,
            other => panic!("unexpected declaration {other:?}"),
        };
        assert_eq!(name.span.file_id, file);
        assert_eq!(source.get(name.span.start as usize..name.span.end as usize),
            Some(name.as_str()), "{source}");
    }
}
