//! Text replacement must reach the borrowed Value slot, not its temporary register.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

const HELPERS: &str = r#"
type Fields is { padding: Int, word: Text, untouched: Text };
type OptionalFields is { padding: Int, description: Maybe<Text>, untouched: Text };
type Message is Named { padding: Int, word: Text, untouched: Text } | Empty;
fn append_optional(source: &mut Maybe<Text>) {
    match source { Maybe.Some(word) => word.push_str(" suffix"), Maybe.None => {}, }
}
fn append_optional_field(source: &mut OptionalFields) {
    match &mut source.description {
        Maybe.Some(word) => word.push_str(" suffix"), Maybe.None => {},
    }
}
fn append(source: &mut Text) { source.push_str(" suffix"); }
fn forward(source: &mut Text) { append(source); }
fn append_lambda(source: &mut Text) { source.push_byte(206); source.push_byte(187); }
"#;

fn check(body: &str, expected: &str) {
    check_with_declarations("", body, expected);
}

#[test]
fn transparent_text_field_projection_selects_its_declared_mutator() {
    check_with_declarations(r#"
        type Digest is (Text);
        type Envelope is { digest: Digest };
        type Stranger is { count: Int };
        implement Stranger { fn push_str(&mut self, text: &Text) { panic("wrong owner"); } }
    "#, r#"
        let mut record = Envelope { digest: Digest("original long text") };
        record.digest.0.push_str(" suffix");
        record.digest.0
    "#, "original long text suffix");
}

#[test]
fn transparent_indexed_projection_borrows_once() {
    check_with_declarations(r#"
        type Wrapped is (Text);
        fn index(calls: &mut Int) -> Int { *calls = *calls + 1; 0 }
    "#, r#"
        let mut calls = 0;
        let mut values = [Wrapped("original long text"), Wrapped("unchanged")];
        values[index(&mut calls)].0.push_str(" suffix");
        assert(calls == 1);
        assert(values[1].0 == "unchanged");
        values[0].0
    "#, "original long text suffix");
}

#[test]
fn explicit_transparent_field_borrow_replaces_the_same_slot() {
    check_with_declarations(r#"
        type Wrapped is (Text);
        type Envelope is { value: Wrapped };
    "#, r#"
        let mut record = Envelope { value: Wrapped("original long text") };
        append(&mut record.value.0);
        record.value.0
    "#, "original long text suffix");
}

#[test]
fn generic_transparent_projection_keeps_its_instantiated_inner_type() {
    check_with_declarations(r#"
        type Wrapped<T> is (T);
        type Envelope is { value: Wrapped<Text> };
    "#, r#"
        let mut record = Envelope { value: Wrapped("original long text") };
        record.value.0.push_str(" suffix");
        record.value.0
    "#, "original long text suffix");
}

#[test]
fn indexed_text_receiver_mutation_reaches_its_element() {
    check(r#"
        let mut words = ["original long text", "unchanged"];
        words[0].push_str(" suffix");
        assert(words[1] == "unchanged");
        words[0]
    "#, "original long text suffix");
}

#[test]
fn nested_indexed_text_receiver_mutation_reaches_its_element() {
    check_with_declarations(r#"
        type Words is { items: [Text; 2] };
        fn append_word(words: &mut Words) { (words.items[0]).push_str(" suffix"); }
    "#, r#"
        let mut words = Words { items: ["original long text", "unchanged"] };
        append_word(&mut words);
        assert(words.items[1] == "unchanged");
        words.items[0]
    "#, "original long text suffix");
}

#[test]
fn indexed_mutable_receiver_evaluates_its_index_once() {
    check_with_declarations(r#"
        fn index(calls: &mut Int) -> Int { *calls = *calls + 1; 0 }
    "#, r#"
        let mut calls = 0;
        let mut words = ["original long text"];
        words[index(&mut calls)].push_str(" suffix");
        assert(calls == 1);
        words[0]
    "#, "original long text suffix");
}

#[test]
fn indexed_user_method_replaces_the_element() {
    check_with_declarations(r#"
        type Entry is { word: Text };
        implement Entry { fn replace(&mut self) { *self = Entry { word: "replacement" }; } }
    "#, r#"
        let mut entries = [Entry { word: "original" }, Entry { word: "unchanged" }];
        entries[0].replace();
        assert(entries[1].word == "unchanged");
        entries[0].word
    "#, "replacement");
}

#[test]
fn indexed_packed_primitive_read_preserves_its_layout() {
    check(r#"
        let numbers: [Int; 2] = [4, 9];
        assert(numbers[1] == 9);
        numbers[0].to_text()
    "#, "4");
}

fn check_with_declarations(declarations: &str, body: &str, expected: &str) {
    let source = format!(
        "module text_field_mutation; mount core.base.maybe.Maybe; {HELPERS}\n{declarations}\nfn probe() -> Text {{ {body} }}"
    );
    let consumer = Parser::new(&source)
        .parse_module()
        .expect("fixture grammar");
    // Receiver ownership comes from the real declaration, not the method name.
    // Register the unchanged canonical record and actual method signatures.
    // Compile only consumer bodies: these controls exercise runtime intercepts,
    // not the complete stdlib method-body dependency closure.
    let mut ast = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module()
        .expect("actual Text grammar");
    let methods = [
        "push",
        "push_byte",
        "push_str",
        "truncate",
        "clear",
        "pop",
        "reserve",
        "capacity",
    ];
    ast.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(declaration) => declaration.name.name.as_str() == "Text",
        ItemKind::Impl(declaration) => {
            if !matches!(&declaration.kind, ImplKind::Inherent(ty)
                if matches!(&ty.kind, TypeKind::Text))
            {
                return false;
            }
            declaration.items.retain(|item| {
                matches!(&item.kind, ImplItemKind::Function(function)
                if methods.contains(&function.name.name.as_str()))
            });
            assert_eq!(
                declaration.items.len(),
                methods.len(),
                "complete method selection"
            );
            true
        }
        _ => false,
    });
    assert_eq!(
        ast.items.len(),
        3,
        "canonical module, record and inherent impl"
    );
    let mut maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module().expect("actual Maybe grammar");
    maybe.items.retain(|item| matches!(&item.kind, ItemKind::Module(_))
        || matches!(&item.kind, ItemKind::Type(declaration) if declaration.name.name.as_str() == "Maybe"));
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("text_field_mutation"));
    codegen.register_builtin_variants();
    codegen
        .collect_unit_declarations(&[&maybe, &ast, &consumer])
        .expect("actual declaration registration");
    let functions = codegen.export_functions();
    for name in methods {
        let info = functions
            .get(&format!("Text.{name}"))
            .expect("exact registered source signature");
        assert_eq!(
            info.takes_self_mut_ref,
            name != "capacity",
            "Text.{name} receiver ownership"
        );
    }
    codegen
        .compile_module_items(&consumer)
        .expect("strict consumer bodies");
    let module = codegen.finalize_module().expect("consumer module");
    assert!(
        module
            .types
            .iter()
            .any(|ty| module.get_string(ty.name) == Some("Text")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("core.text.text")
                && ty.kind == verum_vbc::types::TypeKind::Record),
        "exact source-owned Text record"
    );
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("decode wire");
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("text_field_mutation.probe")
            .expect("probe");
        let mut interpreter = Interpreter::new(Arc::new(module));
        match interpreter.execute_function(entry) {
            Ok(value) => {
                let actual = interpreter.read_text(value);
                if actual.as_deref() != Some(expected) {
                    failures.push(format!("{route}: expected {expected:?}, got {actual:?}").into());
                }
            }
            Err(error) => failures.push(format!("{route}: {error}").into()),
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn direct_local_text_append() {
    check(
        r#"let mut word = "seed"; word.push_str(" suffix"); word"#,
        "seed suffix",
    );
}

#[test]
fn forwarded_local_text_append() {
    check(
        r#"let mut word = "seed"; forward(&mut word); word"#,
        "seed suffix",
    );
}

#[test]
fn direct_small_text_field_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        fields.word.push_str(" suffix");
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn direct_heap_text_field_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "registry source", untouched: "sentinel" };
        fields.word.push_str(" λ");
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "registry source λ",
    );
}

#[test]
fn mutable_field_argument_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        append(&mut fields.word); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn forwarded_mutable_field_argument_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        forward(&mut fields.word); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn local_bytewise_unicode_append() {
    check(
        r#"let mut word = "seed"; append_lambda(&mut word); word"#,
        "seedλ",
    );
}

#[test]
fn field_bytewise_unicode_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        append_lambda(&mut fields.word); fields.word
    "#,
        "seedλ",
    );
}

#[test]
fn field_truncate_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        fields.word.truncate(4); fields.word
    "#,
        "seed",
    );
}

#[test]
fn field_pop_updates_owner_and_returns_character() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        match fields.word.pop() { Maybe.Some(ch) => assert(ch == 'λ'), Maybe.None => assert(false), }
        fields.word
    "#,
        "seed",
    );
}

#[test]
fn field_clear_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "registry source", untouched: "sentinel" };
        fields.word.clear(); fields.word
    "#,
        "",
    );
}

#[test]
fn reused_mutable_field_alias_keeps_origin() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        let source = &mut fields.word;
        forward(source); forward(source);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seed suffix suffix",
    );
}

#[test]
fn field_reserve_retains_bytes_and_exposes_capacity() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        fields.word.reserve(32);
        assert(fields.word.capacity() >= 38);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seedλ",
    );
}

#[test]
fn field_reserve_then_append_updates_same_origin() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        fields.word.reserve(32);
        append_lambda(&mut fields.word);
        fields.word
    "#,
        "seedλ",
    );
}

#[test]
fn mutable_maybe_local_payload_updates_owner() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        match &mut value { Maybe.Some(word) => word.push_str(" suffix"), Maybe.None => {}, }
        match value { Maybe.Some(word) => word, Maybe.None => "missing", }
        "#,
        "seed suffix",
    );
}

#[test]
fn forwarded_mutable_maybe_payload_updates_owner() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("registry λ🙂");
        append_optional(&mut value);
        match value { Maybe.Some(word) => word, Maybe.None => "missing", }
        "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn mutable_maybe_field_payload_updates_owner() {
    check(
        r#"
        let mut fields = OptionalFields {
            padding: 17, description: Maybe.Some("registry λ🙂"), untouched: "sentinel",
        };
        match &mut fields.description {
            Maybe.Some(word) => word.push_str(" suffix"), Maybe.None => {},
        }
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        match fields.description { Maybe.Some(word) => word, Maybe.None => "missing", }
        "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn forwarded_mutable_maybe_field_payload_updates_owner() {
    check(
        r#"
        let mut fields = OptionalFields {
            padding: 17, description: Maybe.Some("registry λ🙂"), untouched: "sentinel",
        };
        append_optional_field(&mut fields);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        match fields.description { Maybe.Some(word) => word, Maybe.None => "missing", }
        "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn mutable_none_field_does_not_change_adjacent_values() {
    check(
        r#"
        let mut fields = OptionalFields {
            padding: 17, description: Maybe.None, untouched: "sentinel",
        };
        append_optional_field(&mut fields);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        match fields.description { Maybe.Some(_) => "unexpected", Maybe.None => "empty", }
        "#,
        "empty",
    );
}

#[test]
fn immutable_maybe_payload_can_be_read_and_copied() {
    check(
        r#"
        let value: Maybe<Text> = Maybe.Some("registry λ🙂");
        match &value {
            Maybe.Some(word) => { assert(word == "registry λ🙂"); *word },
            Maybe.None => "missing",
        }
    "#,
        "registry λ🙂",
    );
}

#[test]
fn a_mutable_reference_alias_keeps_the_variant_place() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let borrowed = &mut value;
        match borrowed { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn a_returned_mutable_reference_keeps_the_variant_place() {
    check_with_declarations(r#"
        fn borrowed<'a>(value: &'a mut Maybe<Text>) -> &'a mut Maybe<Text> { value }
    "#, r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let reference = borrowed(&mut value);
        match reference { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#, "seed suffix");
}

#[test]
fn a_generic_unwrapped_mutable_reference_keeps_the_variant_place() {
    check_with_declarations(r#"
        type Envelope<T> is { value: T };
        implement<T> Envelope<T> { fn take(self) -> T { self.value } }
        fn borrowed<'a>(value: &'a mut Maybe<Text>) -> Envelope<&'a mut Maybe<Text>> {
            Envelope { value: value }
        }
    "#, r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let reference = borrowed(&mut value).take();
        match reference { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#, "seed suffix");
}

#[test]
fn generic_reference_results_survive_named_intermediates_and_aliases() {
    check_with_declarations(r#"
        type Envelope<T> is { value: T };
        implement<T> Envelope<T> { fn take(self) -> T { self.value } }
        fn borrowed<'a>(value: &'a mut Maybe<Text>) -> Envelope<&'a mut Maybe<Text>> {
            Envelope { value: value }
        }
    "#, r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let envelope = borrowed(&mut value);
        let reference = envelope.take();
        let alias = reference;
        match alias { Maybe.Some(word) => {
            let forwarded = word;
            forwarded.push_str(" suffix");
        }, _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#, "seed suffix");
}

#[test]
fn generic_reference_results_preserve_checked_mutability() {
    check_with_declarations(r#"
        type Envelope<T> is { value: T };
        implement<T> Envelope<T> { fn take(self) -> T { self.value } }
        fn borrowed(value: &checked mut Maybe<Text>) -> Envelope<&checked mut Maybe<Text>> {
            Envelope { value: value }
        }
    "#, r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let reference = borrowed(&checked mut value).take();
        match reference { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#, "seed suffix");
}

#[test]
fn generic_immutable_reference_results_can_be_read() {
    check_with_declarations(r#"
        type Envelope<T> is { value: T };
        implement<T> Envelope<T> { fn take(self) -> T { self.value } }
        fn borrowed(value: &Maybe<Text>) -> Envelope<&Maybe<Text>> {
            Envelope { value: value }
        }
    "#, r#"
        let value: Maybe<Text> = Maybe.Some("seed");
        let reference = borrowed(&value).take();
        match reference { Maybe.Some(word) => *word, _ => "missing", }
    "#, "seed");
}

#[test]
fn nested_mutable_variant_pattern_keeps_the_inner_place() {
    check(
        r#"
        let mut value: Maybe<Maybe<Text>> = Maybe.Some(Maybe.Some("registry λ🙂"));
        match &mut value {
            Maybe.Some(Maybe.Some(word)) => word.push_str(" suffix"), _ => {},
        }
        match value { Maybe.Some(Maybe.Some(word)) => word, _ => "missing", }
    "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn nested_match_retains_the_inferred_payload_borrow() {
    check(
        r#"
        let mut value: Maybe<Maybe<Text>> = Maybe.Some(Maybe.Some("registry λ🙂"));
        match &mut value {
            Maybe.Some(inner) => {
                match inner { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
            },
            _ => {},
        }
        match value { Maybe.Some(Maybe.Some(word)) => word, _ => "missing", }
    "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn named_variant_field_mutation_keeps_adjacent_fields() {
    check(
        r#"
        let mut value = Message.Named { padding: 17, word: "registry λ🙂", untouched: "sentinel" };
        match &mut value { Message.Named { word, .. } => word.push_str(" suffix"), _ => {}, }
        match value {
            Message.Named { padding, word, untouched } => {
                assert(padding == 17); assert(untouched == "sentinel"); word
            },
            _ => "missing",
        }
    "#,
        "registry λ🙂 suffix",
    );
}

#[test]
fn mixed_tuple_scrutinee_preserves_each_binding_mode() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let other: Maybe<Text> = Maybe.Some("unchanged");
        match (&mut value, other) {
            (Maybe.Some(word), Maybe.Some(untouched)) => {
                word.push_str(" suffix"); assert(untouched == "unchanged");
            },
            _ => {},
        }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn if_let_preserves_the_payload_borrow() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        if let Maybe.Some(word) = &mut value { word.push_str(" suffix"); }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn while_let_preserves_the_payload_borrow() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        while let Maybe.Some(word) = &mut value { word.push_str(" suffix"); break; }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn let_else_preserves_the_payload_borrow() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        let Maybe.Some(word) = &mut value else { return "missing"; };
        word.push_str(" suffix");
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn owned_pattern_in_borrowed_arm_keeps_its_own_mode() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        match &mut value {
            Maybe.Some(word) => {
                let mut owned: Maybe<Text> = Maybe.Some("owned");
                match owned { Maybe.Some(mut word) => word.push_str(" lost"), _ => {}, }
                match owned { Maybe.Some(word) => assert(word == "owned"), _ => assert(false), }
                word.push_str(" suffix");
            },
            _ => {},
        }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn borrowed_plain_record_pattern_updates_its_field() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        match &mut fields { Fields { word, .. } => word.push_str(" suffix"), }
        assert(fields.padding == 17); assert(fields.untouched == "sentinel"); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn nested_record_payload_keeps_its_borrowed_field() {
    check(
        r#"
        let mut value: Maybe<Fields> = Maybe.Some(Fields { padding: 17, word: "seed", untouched: "sentinel" });
        match &mut value { Maybe.Some(Fields { word, .. }) => word.push_str(" suffix"), _ => {}, }
        match value {
            Maybe.Some(fields) => { assert(fields.padding == 17); assert(fields.untouched == "sentinel"); fields.word },
            _ => "missing",
        }
    "#,
        "seed suffix",
    );
}

#[test]
fn explicit_reference_pattern_retains_owned_payload_binding() {
    check(
        r#"
        let mut value: Maybe<Text> = Maybe.Some("seed");
        match &mut value { &mut Maybe.Some(mut word) => word.push_str(" local"), _ => {}, }
        match value { Maybe.Some(word) => word, _ => "missing", }
    "#,
        "seed",
    );
}

#[test]
fn a_scoped_variant_keeps_priority_over_a_same_named_record() {
    check_with_declarations(
        "type Selection is Fields { word: Text } | Empty;",
        r#"
        let mut selected = Selection.Fields { word: "seed" };
        match &mut selected { Fields { word } => word.push_str(" suffix"), _ => {}, }
        match selected { Selection.Fields { word } => word, _ => "missing", }
        "#,
        "seed suffix",
    );
}

#[test]
fn nested_match_retains_an_explicit_payload_borrow() {
    check(
        r#"
        let mut value: Maybe<Maybe<Text>> = Maybe.Some(Maybe.Some("seed"));
        match value {
            Maybe.Some(ref mut inner) => {
                match inner { Maybe.Some(word) => word.push_str(" suffix"), _ => {}, }
            },
            _ => {},
        }
        match value { Maybe.Some(Maybe.Some(word)) => word, _ => "missing", }
    "#,
        "seed suffix",
    );
}

#[test]
fn explicit_mutable_plain_record_field_borrow_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        match fields { Fields { word: ref mut word, .. } => word.push_str(" suffix"), _ => {}, }
        assert(fields.padding == 17); assert(fields.untouched == "sentinel"); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn explicit_immutable_plain_record_field_borrow_copies_on_dereference() {
    check(
        r#"
        let fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        let copied = match fields {
            Fields { word: ref word, .. } => { let mut copy = *word; copy.push_str(" suffix"); copy },
            _ => "missing",
        };
        assert(fields.padding == 17); assert(fields.untouched == "sentinel"); assert(fields.word == "seed");
        copied
    "#,
        "seed suffix",
    );
}
