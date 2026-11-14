//! Borrowing a resource preserves its ownership and its access restrictions.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_module_path("sample");
    let mut errors = checker.prepare_module_declarations(&module);
    errors.extend(
        module
            .items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

#[test]
fn borrowed_resource_patterns_do_not_consume_the_reference() {
    for discipline in ["affine", "linear"] {
        for reference in [
            "&",
            "&checked",
            "&unsafe",
            "&mut",
            "&checked mut",
            "&unsafe mut",
        ] {
            for body in [
                "let first=match value {Token.Full(_)=>1,Token.Empty=>0}; let second=match value {Token.Full(_)=>1,Token.Empty=>0}; first+second",
                "match value {Token.Full(_)=>inspect(value),Token.Empty=>inspect(value)}",
            ] {
                let source = format!(
                    "type {discipline} Token is Full(Int) | Empty; fn inspect(value:{reference} Token)->Int {{1}} fn probe(value:{reference} Token)->Int {{{body}}}"
                );
                let errors = errors(&source);
                assert!(errors.is_empty(), "{source}: {errors:?}");
            }
        }
    }
}

#[test]
fn declared_reference_aliases_preserve_the_same_resource_boundary() {
    for reference in ["&", "&checked", "&unsafe"] {
        let source = format!(
            "type affine Token is Full(Int) | Empty; type Borrowed<T> is {reference} T; fn probe(value:Borrowed<Token>)->Int {{let first=match value {{Token.Full(_)=>1,Token.Empty=>0}}; let second=match value {{Token.Full(_)=>1,Token.Empty=>0}}; first+second}}"
        );
        let errors = errors(&source);
        assert!(errors.is_empty(), "{source}: {errors:?}");
    }
}

#[test]
fn borrowing_does_not_allow_copying_the_owner_or_an_owning_aggregate() {
    for body in [
        "fn probe(value:Token) {let first=value; let second=value;}",
        "type Holder is {item:Token}; fn probe(value:Holder) {let first=value; let second=value;}",
        "fn probe(value:&Token) {let first=*value; let second=*value;}",
    ] {
        let source = format!("type affine Token is {{value:Int}}; {body}");
        let errors = errors(&source);
        assert!(!errors.is_empty(), "resource duplicated: {source}");
    }
}

#[test]
fn reference_lifetimes_and_exclusive_access_are_still_checked() {
    for body in [
        "fn invalid()->&Token {let value=Token{value:73}; &value}",
        "fn invalid(value:&mut Token)->Int {let borrowed=&*value; value.value=4; borrowed.value}",
    ] {
        let source = format!("type affine Token is {{value:Int}}; {body}");
        let errors = errors(&source);
        assert!(
            errors.iter().any(
                |error| error.contains("borrow") || error.contains("does not live long enough")
            ),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn resource_reads_from_borrowed_places_require_ownership() {
    for reference in ["&", "&checked", "&unsafe", "&mut", "&checked mut", "&unsafe mut"] {
        for body in [
            "let copy=*value;",
            "let copy:Token=value;",
            "consume(*value);",
            "consume(value);",
            "let copy=Holder{item:*value};",
            "consume_holder(Holder{item:*value});",
            "let copy=(*value,);",
            "let copy=[*value];",
            "return *value;",
        ] {
            let result = if body.starts_with("return") {"Token"} else {"Unit"};
            let source=format!("type affine Token is {{value:Int}}; type Holder is {{item:Token}}; fn consume(value:Token) {{}} fn consume_holder(value:Holder) {{}} fn probe(value:{reference} Token)->{result} {{{body}}}");
            let found=errors(&source);
            assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "{source}: {found:?}");
        }
        for (parameter, body) in [
            ("Holder", "value.item"),
            ("Holder", "(*value).item"),
            ("(Token,Int)", "value.0"),
            ("[Token;1]", "value[0]"),
        ] {
            let source=format!("type affine Token is {{value:Int}}; type Holder is {{item:Token}}; fn probe(value:{reference} {parameter})->Token {{{body}}}");
            let found=errors(&source);
            assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "{source}: {found:?}");
        }
    }
}

#[test]
fn borrowed_resource_places_still_allow_reborrows_and_scalar_reads() {
    for body in [
        "fn probe(value:&Token)->Int { let first=&*value; let second=&*value; first.value+second.value }",
        "fn probe(value:&Holder)->Int { let first=&value.item; let second=&(*value).item; first.value+second.value }",
        "fn probe(value:&(Token,Int))->Int { let first=&value.0; let second=&value.0; first.value+second.value }",
        "fn probe(value:&[Token;1])->Int { let first=&value[0]; let second=&value[0]; first.value+second.value }",
        "fn inspect(value:&Token)->Int {value.value} fn probe(value:&Token)->Int {inspect(*value)+inspect(*value)}",
    ] {
        let source=format!("type affine Token is {{value:Int}}; type Holder is {{item:Token}}; {body}");
        let found=errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn by_value_receivers_use_the_same_resource_boundary() {
    let ordinary="type State is Closed | Open; implement State { fn code(self)->Int { match self {State.Closed=>73,State.Open=>0} } fn borrowed(&self)->Int {self.code()+self.code()} }";
    let found=errors(ordinary);
    assert!(found.is_empty(), "{found:?}");
    for receiver in ["&Token", "&mut Token", "&checked Token", "&unsafe Token"] {
        let source=format!("type affine Token is {{value:Int}}; implement Token {{fn consume(self)->Int {{self.value}}}} fn probe(value:{receiver})->Int {{value.consume()}}");
        let found=errors(&source);
        assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "{source}: {found:?}");
    }
}

#[test]
fn owned_call_arguments_consume_only_the_selected_resource_place() {
    for body in [
        "fn probe(value:Holder) { consume(value.item); consume(value.item); }",
        "fn probe(value:(Token,Int)) { consume(value.0); consume(value.0); }",
        "fn probe(value:Token) { consume((value)); consume(value); }",
    ] {
        let source=format!("type affine Token is {{value:Int}}; type Holder is {{item:Token}}; fn consume(value:Token) {{}} {body}");
        let found=errors(&source);
        assert!(found.iter().any(|error|error.contains("moved") || error.contains("already used")), "{source}: {found:?}");
    }
}

#[test]
fn repeated_borrow_aliases_are_not_owning_cycles() {
    let source="type affine Token is {value:Int}; type Borrowed<T> is &T; type Views is {first:Borrowed<Token>, second:Borrowed<Token>}; fn probe(value:Views) {let first=value; let second=value;}";
    let found=errors(source);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_declared_resource_alias_keeps_its_copy_restriction() {
    let found=errors("type affine Flag is Bool; fn probe(value:&Flag)->Flag {*value}");
    assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "{found:?}");
}

#[test]
fn addressing_a_dereference_chain_never_materializes_its_resource() {
    for depth in [1,8,32] {
        let mut references = Text::new();
        for n in 1..=depth {
            references.push_str(&format!("type Ref{n} is &Ref{};", n - 1));
        }
        let dereferences="* ".repeat(depth);
        let source=format!("type affine Token is {{value:Int}}; type Ref0 is Token; {references} fn probe(value:Ref{depth})->Int {{let item=&({dereferences}value); item.value}}");
        let found=errors(&source);
        assert!(found.is_empty(), "depth {depth}: {found:?}");
        let source=format!("type affine Token is {{value:Int}}; type Ref0 is Token; {references} fn probe(value:Ref{depth})->Token {{{dereferences}value}}");
        let found=errors(&source);
        assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "depth {depth}: {found:?}");
    }
}

#[test]
fn selected_deref_receivers_do_not_transfer_the_borrowed_resource() {
    let declarations="type Deref is protocol {type Target; fn deref(&self)->&Self.Target;}; type affine Token is {value:Int}; type Holder is {item:Token}; type Outer is {item:Holder}; implement Deref for Outer { type Target=Holder; fn deref(&self)->&Holder {&self.item} } implement Deref for Holder { type Target=Token; fn deref(&self)->&Token {&self.item} } implement Token { fn consume(self)->Int {self.value} fn inspect(&self)->Int {self.value} } type Borrowed<T> is &T;";
    for receiver in ["Borrowed<Token>", "Holder", "&Holder", "Outer", "&Outer"] {
        let source=format!("{declarations} fn probe(value:{receiver})->Int {{value.inspect()+value.inspect()}}");
        let found=errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");
        let source=format!("{declarations} fn probe(value:{receiver})->Int {{value.consume()}}");
        let found=errors(&source);
        assert!(found.iter().any(|error|error.contains("out of borrowed storage")), "{source}: {found:?}");
    }
    let source=format!("{declarations} implement Holder {{fn consume(self)->Int {{self.item.value}}}} fn probe(value:Holder)->Int {{value.consume()}}");
    let found=errors(&source);
    assert!(found.is_empty(), "owning wrapper's own method: {found:?}");
}
