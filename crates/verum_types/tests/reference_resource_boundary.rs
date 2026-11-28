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
fn while_patterns_borrow_each_iteration_and_reject_repeated_affine_moves() {
    let declarations = "type affine Ticket is {value:Int}; type Batch is Empty | Entry(Ticket);";
    let borrow = "fn get(batch:&Batch)->Int {while let Batch.Entry(ref ticket)=batch {return ticket.value;} 0}";
    let found = errors(&format!("{declarations} {borrow}"));
    assert!(found.is_empty(), "{found:?}");

    let repeat = "fn consume(batch:Batch) {while let Batch.Entry(ticket)=batch {let spent=ticket;}}";
    let found = errors(&format!("{declarations} {repeat}"));
    assert!(found.iter().any(|error| error.contains("loop")), "{found:?}");
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
fn replacing_an_affine_referent_is_a_write_to_storage() {
    let types = "type affine Token is {value:Int}; type Choice is Empty | Full(Token);";
    for (reference, place) in [
        ("&mut Choice", "*dst"),
        ("&checked mut Choice", "(*dst)"),
        ("&unsafe mut Choice", "*dst"),
        ("&mut &mut Choice", "**dst"),
    ] {
        let source = format!("{types} fn store(dst:{reference}, item:Token) {{{place}=Choice.Full(item);}}");
        let found = errors(&source);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
    for (body, reason) in [
        ("fn bad(dst:&Choice,item:Token){*dst=Choice.Full(item);}", "shared"),
        ("fn bad(dst:&mut Choice){let held=&*dst;*dst=Choice.Empty;match held {_=>()};}", "borrow"),
        ("fn bad(dst:&mut Choice)->Choice {*dst}", "out of borrowed storage"),
        ("fn bad(dst:&mut Choice,item:Token){*dst=Choice.Full(item);*dst=Choice.Full(item);}", "moved"),
        ("fn bad(dst:&mut Choice){*dst=73;}", "mismatch"),
    ] {
        let found = errors(&format!("{types} {body}"));
        assert!(found.iter().any(|error| error.to_lowercase().contains(reason)), "{body}: {found:?}");
    }
}

#[test]
fn replacing_a_live_linear_referent_cannot_drop_the_old_obligation() {
    for (declarations, name) in [
        ("", "Ticket"),
        ("type Envelope is {payload:Ticket};", "Envelope"),
        ("type Packet is Missing | Present(Ticket);", "Packet"),
        ("type Envelope<T> is {payload:T};", "Envelope<Ticket>"),
        ("type Alias is Ticket;", "Alias"),
    ] {
        let source = format!("type linear Ticket is {{id:Int}}; {declarations} fn lose(dst:&mut {name}, replacement:{name}) {{*dst=replacement;}}");
        let found = errors(&source);
        assert!(found.iter().any(|error| error.contains("linear") && error.contains("overwrite")), "{source}: {found:?}");
    }
    let generic = errors("fn lose<T>(dst:&mut T,replacement:T){*dst=replacement;}");
    assert!(generic.iter().any(|error| error.contains("cannot overwrite")), "{generic:?}");
    for (carrier, place) in [
        ("Envelope", "dst.payload"),
        ("Envelope", "(*dst).payload"),
        ("(Ticket,Int)", "dst.0"),
        ("RefEnvelope", "dst.inner.payload"),
    ] {
        let source = format!("type linear Ticket is {{id:Int}}; type Envelope is {{payload:Ticket}}; type RefEnvelope is {{inner:&mut Envelope}}; fn lose(dst:&mut {carrier}, replacement:Ticket) {{{place}=replacement;}}");
        let found = errors(&source);
        assert!(found.iter().any(|error| error.contains("linear") && error.contains("overwrite")), "{source}: {found:?}");
    }
    let erased = errors("type Witness<A> is {value:A}; type Pack is {carrier:Type,witness:Witness<carrier>}; fn lose(dst:&mut Pack,replacement:Pack){*dst=replacement;}");
    assert!(erased.iter().any(|error| error.contains("cannot overwrite")), "{erased:?}");
    let phantom = errors("type linear Ticket is {id:Int}; type Tag<T> is {id:Int}; fn update(dst:&mut Tag<Ticket>,next:Tag<Ticket>){*dst=next;}");
    assert!(phantom.is_empty(), "{phantom:?}");
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

#[test]
fn indexed_slice_writes_check_the_element_discipline_of_the_selected_receiver() {
    let declarations = r#"
        type Default is protocol { fn default() -> Self; };
        type linear Ticket is { id: Int };
        type Token is { id: Int };
        implement Default for Token { fn default() -> Self { Token { id: 73 } } }
        implement Default for Ticket { fn default() -> Self { Ticket { id: 1 } } }
        implement<T> [T] {
            fn reset_at(&mut self, slot: Int) where T: Default {
                if slot >= 0 { self[slot] = T.default(); }
            }
            fn inspect(&self) -> Int { 73 }
        }
    "#;
    for caller in [
        "fn run(items: &mut [Token]) { items.reset_at(0); }",
        "fn run(items: &mut [Ticket]) -> Int { items.inspect() }",
    ] {
        let found = errors(&format!("{declarations} {caller}"));
        assert!(found.is_empty(), "{caller}: {found:?}");
    }
    for (caller, reason) in [
        ("fn lose(items: &mut [Ticket]) { items.reset_at(0); }", "Linear"),
        ("fn hide<T: Default>(items: &mut [T]) { items.reset_at(0); }", "discarded"),
    ] {
        let found = errors(&format!("{declarations} {caller}"));
        assert!(found.iter().any(|error| error.contains("overwrite") && error.to_lowercase().contains(&reason.to_lowercase())), "{caller}: {found:?}");
    }
    let erased = errors(
        "type Clear is protocol {fn clear(&mut self);}; type Default is protocol {fn default() -> Self;}; implement<T: Default> Clear for [T] {fn clear(&mut self) {self[0] = T.default();}}",
    );
    assert!(erased.iter().any(|error| error.contains("cannot overwrite")), "{erased:?}");
}
