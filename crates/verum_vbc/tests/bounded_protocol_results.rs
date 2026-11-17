#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module, TypeDeclBody, decl::ProtocolItemKind};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, bytecode::decode_instructions,
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    module::VbcModule,
};

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("parse source")
}

fn compile(files: &[&Module]) -> CodegenResult<VbcModule> {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.collect_unit_declarations(files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(files, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn associated_results_complete_protocol_defaults_before_matching_the_implementation() {
    for (application, operand, accepted) in [("Pick", "Self", true), ("Pick<Bool>", "Bool", false)] {
        let source = parse(&format!(r#"
            module consumer;
            type Pick<Rhs = Self> is protocol {{
                type Output;
                fn choose(self, rhs: Rhs) -> Self.Output;
            }};
            type Number is {{ value: Int }};
            implement {application} for Number {{
                type Output = Self;
                fn choose(self, rhs: {operand}) -> Self {{ self }}
            }}
            fn apply<T: Pick<Output = T>>(left: T, right: T) -> T {{ left.choose(right) }}
            fn probe() -> Int {{ apply(Number {{ value: 73 }}, Number {{ value: 0 }}).value }}
        "#));
        let result = compile(&[&source]);
        if !accepted {
            assert!(result.is_err(), "a different protocol application cannot supply the associated result");
            continue;
        }
        let module = result.expect("defaulted protocol application");
        let wire = verum_vbc::deserialize::deserialize_module_validated(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        ).unwrap();
        for module in [module, wire] {
            let entry = module.find_function_by_name("consumer.probe").unwrap();
            let mut interpreter = verum_vbc::interpreter::Interpreter::try_new_validated(module.into()).unwrap();
            assert_eq!(interpreter.execute_function(entry).unwrap().try_as_i64(), Some(73));
        }
    }
}

fn assert_tags(module: VbcModule, method: &str, expected: &[u32]) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let function = module.functions.iter().find(|function| {
            module.get_string(function.name).is_some_and(|name| name.ends_with(method))
        }).expect(method);
        let start = function.bytecode_offset as usize;
        let instructions = decode_instructions(
            &module.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        let tags: List<_> = instructions.iter().filter_map(|instruction| {
            match instruction { Instruction::IsVar { tag, .. } => Some(*tag), _ => None }
        }).collect();
        assert_eq!(tags.as_slice(), expected, "{method}: {instructions:?}");
    }
}

fn constrained(body: &str) -> CodegenResult<VbcModule> {
    let provider = parse(r#"
        module provider;
        public type Outcome is Idle | Selected(Int);
        public type Picker is protocol { fn pick(&self) -> Outcome; };
        public type Family is protocol { type Item; };
    "#);
    let consumer = parse(&format!(r#"
        module consumer;
        mount provider.{{Picker, Family}};
        type Outcome is Selected(Int) | Idle | Foreign;
        type Rival is protocol {{ fn pick(&self) -> Outcome; }};
        type Holder<T> is {{ item: T }};
        {body}
    "#));
    compile(&[&provider, &consumer])
}

#[test]
fn synchronous_instance_result_uses_its_bound_declaration_through_source_and_wire() {
    let module = constrained(r#"
        fn argument<T: Picker>(item: T) -> Int {
            match item.pick() { Selected(value) => value, _ => 0 }
        }
        implement<T: Picker> Holder<T> {
            fn field(&self) -> Int {
                match self.item.pick() { Selected(value) => value, _ => 0 }
            }
            fn shadow<T: Rival>(&self, item: T) -> Int {
                match item.pick() { Selected(value) => value, _ => 0 }
            }
        }
    "#).unwrap();
    assert_tags(module.clone(), ".argument", &[1]);
    assert_tags(module.clone(), ".field", &[1]);
    assert_tags(module, ".shadow", &[0]);
}

#[test]
fn projection_where_subject_retains_its_own_protocol_evidence() {
    let module = constrained(r#"
        fn projected<T: Family>(item: T.Item) -> Int where T.Item: Picker {
            match item.pick() { Selected(value) => value, _ => 0 }
        }
    "#).unwrap();
    assert_tags(module, ".projected", &[1]);
}

#[test]
fn missing_or_ambiguous_bounds_cannot_select_an_ambient_same_leaf_result() {
    for bound in ["", ": Picker + Rival"] {
        let error = constrained(&format!(r#"
            fn rejected<T{bound}>(item: T) -> Int {{
                match item.pick() {{ Selected(value) => value, _ => 0 }}
            }}
        "#)).err().expect("unproved or ambiguous bound must refuse").to_string();
        let reason = if bound.is_empty() { "no declared protocol method `pick`" }
            else { "ambiguous protocol method `pick`" };
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn selected_protocol_cannot_acquire_a_variant_from_a_same_leaf_sum() {
    let error = constrained(r#"
        fn rejected<T: Picker>(item: T) -> Int {
            match item.pick() { Foreign => 1, _ => 0 }
        }
    "#).err().expect("foreign variant must refuse").to_string();
    assert!(error.contains("no declared tag") && error.contains("Foreign"), "{error}");
}

#[test]
fn conditional_default_keeps_a_nominal_self_subject() {
    let module = constrained(r#"
        type Probe is protocol {
            fn inspect(self) -> Int where Self: Picker {
                match self.pick() { Selected(value) => value, _ => 0 }
            }
        };
        type Receiver is { marker: Int };
        implement Probe for Receiver {}
    "#).unwrap();
    assert_tags(module, "Receiver.inspect", &[1]);
}

#[test]
fn a_projection_bound_cannot_prove_a_different_subject_with_the_same_member() {
    let error = constrained(r#"
        fn rejected<T: Family, U: Family>(item: U.Item) -> Int where T.Item: Picker {
            match item.pick() { Selected(value) => value, _ => 0 }
        }
    "#).err().expect("T.Item evidence cannot prove U.Item").to_string();
    assert!(error.contains("no declared protocol method `pick`"), "{error}");
}

fn execute_source_and_wire(module: VbcModule, expected: i64) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("consumer.run").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(module.into())
            .execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn constrained_dispatch_executes_the_selected_protocol_implementation() {
    let module = constrained(r#"
        mount provider.Outcome as OwnedOutcome;
        type Value is { marker: Int };
        implement Picker for Value {
            fn pick(&self) -> OwnedOutcome { OwnedOutcome.Selected(self.marker) }
        }
        fn inspect<T: Picker>(item: T) -> Int {
            match item.pick() { Selected(value) => value, _ => 0 }
        }
        fn run() -> Int { inspect(Value { marker: 37 }) }
    "#).unwrap();
    assert_tags(module.clone(), ".inspect", &[1]);
    execute_source_and_wire(module, 37);
}

#[test]
fn a_nominal_method_keeps_its_own_result_without_protocol_bounds() {
    let module = constrained(r#"
        type Value is { marker: Int };
        implement Value {
            fn pick(&self) -> Outcome { Outcome.Selected(self.marker) }
        }
        fn run() -> Int {
            let item = Value { marker: 29 };
            match item.pick() { Selected(value) => value, _ => 0 }
        }
    "#).unwrap();
    assert_tags(module.clone(), ".run", &[0]);
    execute_source_and_wire(module, 29);
}

fn actual_iterator(methods: &[&str]) -> Module {
    let mut iterator = parse(include_str!("../../../core/base/iterator.vr"));
    let mut originals = List::new();
    iterator.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) if declaration.name.as_str() == "Iterator" => {
            let TypeDeclBody::Protocol(body) = &mut declaration.body else { panic!("Iterator protocol") };
            originals = body.items.iter().filter_map(|item| match &item.kind {
                ProtocolItemKind::Function { decl, .. } if methods.contains(&decl.name.as_str()) => Some(item.kind.clone()),
                _ => None,
            }).collect();
            body.items.retain(|item| match &item.kind {
                ProtocolItemKind::Type { name, .. } => name.as_str() == "Item",
                ProtocolItemKind::Function { decl, .. } => decl.name.as_str() == "next" || methods.contains(&decl.name.as_str()),
                _ => false,
            });
            assert!(originals.iter().all(|original| body.items.iter().any(|item| &item.kind == original)),
                "the actual signatures, constraints and bodies must remain unchanged");
            true
        }
        _ => false,
    });
    assert_eq!(originals.len(), methods.len());
    iterator
}

#[test]
fn actual_iterator_cmp_keeps_the_declared_ordering_in_source_and_wire() {
    let mut iterator = actual_iterator(&["cmp"]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.ordering.Ordering; mount core.base.protocols.Ord;").items);

    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "PartialEq" | "Eq" | "Ord"),
        _ => false,
    });
    protocols.items.extend(parse("mount core.base.ordering.Ordering;").items);
    let [ordering, maybe] = [
        (include_str!("../../../core/base/ordering.vr"), "Ordering"),
        (include_str!("../../../core/base/maybe.vr"), "Maybe"),
    ].map(|(source, name)| {
        let mut module = parse(source);
        module.items.retain(|item| match &item.kind {
            ItemKind::Module(declaration) => declaration.items.is_none(),
            ItemKind::Type(declaration) => declaration.name.as_str() == name,
            _ => false,
        });
        module
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        type Ordering is Equal | Less | Greater;
        type Sequence<T> is { marker: Int };
        implement<T> Iterator for Sequence<T> {
            type Item = T;
            fn next(&mut self) -> Maybe<T> { Maybe.None }
        }
    "#);
    let module = compile(&[&ordering, &maybe, &protocols, &iterator, &consumer]).unwrap();
    // Four Maybe tuple arms emit eight tests; the final test belongs to the
    // real Ord result, whose Equal tag is 1, despite the consumer's homonym.
    assert_tags(module, "Sequence.cmp", &[0, 0, 0, 1, 1, 0, 1, 1, 1]);
}

#[test]
fn actual_iterator_product_preserves_projected_static_and_binary_results() {
    use verum_vbc::types::TypeRef;
    let mut iterator = actual_iterator(&["product", "fold"]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.protocols.{Mul, One};").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "Mul" | "One"),
        _ => false,
    });
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "Maybe",
        _ => false,
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        mount core.base.protocols.{Mul, One};
        type Number is { value: Int };
        implement One for Number {
            fn one() -> Self { Number { value: 1 } }
            fn is_one(&self) -> Bool { self.value == 1 }
        }
        implement Mul for Number {
            type Output = Self;
            fn mul(self, rhs: Self) -> Self { Number { value: self.value * rhs.value } }
        }
        type Sequence<T> is { first: T, second: T, remaining: Int };
        implement<T> Iterator for Sequence<T> {
            type Item = T;
            fn next(&mut self) -> Maybe<T> {
                if self.remaining == 2 {
                    self.remaining = 1;
                    Maybe.Some(self.first)
                } else if self.remaining == 1 {
                    self.remaining = 0;
                    Maybe.Some(self.second)
                } else { Maybe.None }
            }
        }
        fn run() -> Int {
            let values = Sequence { first: Number { value: 3 }, second: Number { value: 7 }, remaining: 2 };
            let product = values.product();
            product.value
        }
    "#);
    let module = compile(&[&maybe, &protocols, &iterator, &consumer]).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    for module in [module, wire] {
        let product = module.functions.iter().find(|function|
            module.get_string(function.name).is_some_and(|name| name.ends_with("Sequence.product"))).unwrap();
        let result = product.semantic_return.as_ref().unwrap();
        let TypeRef::AssociatedProjection { base: receiver, qualifier: None, assoc, args: projection_args } = result else {
            panic!("the retained source signature declares Self.Item: {result:?}")
        };
        assert_eq!(assoc, "Item");
        assert!(projection_args.is_empty());
        assert_eq!(product.source_receiver.as_ref(), Some(receiver.as_ref()));
        let TypeRef::Instantiated { base, args } = receiver.as_ref() else { panic!("{receiver:?}") };
        let [item] = args.as_slice() else { panic!("the declared receiver has one parameter") };
        assert!(matches!(item, TypeRef::Generic(_)), "{item:?}");
        let iterator = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Iterator")
            && ty.origin_module.and_then(|owner| module.get_string(owner)) == Some("core.base.iterator")).unwrap();
        let implementation = module.get_type(*base).unwrap().protocols.iter()
            .find(|implementation| implementation.protocol.0 == iterator.id.0).unwrap();
        assert!(implementation.associated_types.iter().any(|(name, ty)|
            module.get_string(*name) == Some("Item") && ty == item));
        let closures: List<_> = module.functions.iter().filter(|function| function.closure_capture_count.is_some()).collect();
        assert_eq!(closures.len(), 1, "product's actual binary callback");
        assert_eq!(module.get_string(closures[0].params[0].name), Some("__closure_env"));
        assert_eq!(closures[0].closure_capture_count, Some(0));
        assert_eq!(closures[0].semantic_parameter_types().unwrap(), &[None, Some(item.clone()), Some(item.clone())]);
        assert_eq!(closures[0].semantic_return.as_ref(), Some(item));
        let entry = module.find_function_by_name("consumer.run").unwrap();
        let actual = verum_vbc::interpreter::Interpreter::new(module.into()).execute_function(entry).unwrap();
        assert_eq!(actual.try_as_i64(), Some(21));
    }
}


#[test]
fn actual_iterator_product_by_preserves_its_generic_accumulator_argument() {
    let mut iterator = actual_iterator(&["product_by", "fold"]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.protocols.{Mul, One};").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "Mul" | "One"),
        _ => false,
    });
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "Maybe",
        _ => false,
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        mount core.base.protocols.{Mul, One};
        type Number is { value: Int };
        implement One for Number {
            fn one() -> Self { Number { value: 1 } }
            fn is_one(&self) -> Bool { self.value == 1 }
        }
        implement Mul for Number {
            type Output = Self;
            fn mul(self, rhs: Self) -> Self { Number { value: self.value * rhs.value } }
        }
        type Sequence is { first: Int, second: Int, remaining: Int };
        implement Iterator for Sequence {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> {
                if self.remaining == 2 {
                    self.remaining = 1;
                    Maybe.Some(self.first)
                } else if self.remaining == 1 {
                    self.remaining = 0;
                    Maybe.Some(self.second)
                } else { Maybe.None }
            }
        }
        fn run() -> Int {
            let values = Sequence { first: 3, second: 7, remaining: 2 };
            let product = values.product_by(|item| Number { value: *item });
            product.value
        }
    "#);
    let module = compile(&[&maybe, &protocols, &iterator, &consumer]).unwrap();
    execute_source_and_wire(module, 21);
}

#[test]
fn a_static_protocol_result_supplies_the_callers_generic_argument() {
    let module = parse(r#"
        module consumer;
        type Seed is protocol { fn seed() -> Self; };
        type Number is { value: Int };
        implement Seed for Number { fn seed() -> Self { Number { value: 37 } } }
        fn relay<A, F: fn(A) -> A>(initial: A, transform: F) -> A { transform(initial) }
        fn forward<Unused, B: Seed>(marker: Unused) -> B {
            relay(B.seed(), |value| value)
        }
        fn run() -> Int { forward<Int, Number>(0).value }
    "#);
    execute_source_and_wire(compile(&[&module]).unwrap(), 37);
}


#[test]
fn actual_iterator_max_keeps_symbolic_callback_inputs_in_a_generic_adapter() {
    let mut iterator = actual_iterator(&["max", "reduce", "fold"]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.ordering.Ordering; mount core.base.protocols.Ord;").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "PartialEq" | "Eq" | "Ord"),
        _ => false,
    });
    protocols.items.extend(parse("mount core.base.ordering.Ordering;").items);
    let [ordering, maybe] = [
        (include_str!("../../../core/base/ordering.vr"), "Ordering"),
        (include_str!("../../../core/base/maybe.vr"), "Maybe"),
    ].map(|(source, name)| {
        let mut module = parse(source);
        module.items.retain(|item| match &item.kind {
            ItemKind::Module(declaration) => declaration.items.is_none(),
            ItemKind::Type(declaration) => declaration.name.as_str() == name,
            _ => false,
        });
        module
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        type Sequence is { first: Int, second: Int, remaining: Int };
        implement Iterator for Sequence {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> {
                if self.remaining == 2 {
                    self.remaining = 1;
                    Maybe.Some(self.first)
                } else if self.remaining == 1 {
                    self.remaining = 0;
                    Maybe.Some(self.second)
                } else { Maybe.None }
            }
        }
        type Adapter<I> is { iter: I };
        implement<I: Iterator> Iterator for Adapter<I> {
            type Item = I.Item;
            fn next(&mut self) -> Maybe<I.Item> { self.iter.next() }
        }
        fn run() -> Int {
            let values = Adapter { iter: Sequence { first: 3, second: 7, remaining: 2 } };
            match values.max() { Maybe.Some(value) => value, Maybe.None => 0 }
        }
    "#);
    execute_source_and_wire(compile(&[&ordering, &maybe, &protocols, &iterator, &consumer]).unwrap(), 7);
}

#[test]
fn implementation_local_member_equations_retain_residual_binders_for_all_bodies() {
    let provider = parse(r#"
        module provider;
        public type Outcome is Idle | Selected(Int);
        public type Picker is protocol { fn pick(&self) -> Outcome; };
        public type Maker is protocol {
            type Output;
            fn produce(&self) -> Self.Output;
            fn inherited(&self) -> Int where Self.Output: Picker {
                match self.produce().pick() { Selected(value) => value, _ => 0 }
            }
        };
    "#);
    let consumer = parse(r#"
        module consumer;
        mount provider.{Maker, Picker};
        type Outcome is Selected(Int) | Idle;
        type Wrapper<F> is { make: F };
        implement<Produced, Factory: fn() -> Produced> Maker for Wrapper<Factory> {
            type Output = Produced;
            fn produce(&self) -> Produced { (self.make)() }
            fn explicit(&self, item: Produced) -> Int where Self.Output: Picker {
                match item.pick() { Selected(value) => value, _ => 0 }
            }
        }
        type Another<G> is { make: G };
        implement<Result, Generator: fn() -> Result> Maker for Another<Generator> {
            type Output = Result;
            fn produce(&self) -> Result { (self.make)() }
        }
    "#);
    let module = compile(&[&provider, &consumer]).unwrap();
    assert_tags(module.clone(), "Wrapper.inherited", &[1]);
    assert_tags(module.clone(), "Wrapper.explicit", &[1]);
    assert_tags(module.clone(), "Another.inherited", &[1]);
    for name in ["Wrapper", "Another"] {
        let descriptor = module.types.iter().find(|ty| module.get_string(ty.name) == Some(name)).unwrap();
        assert_eq!(descriptor.type_params.len(), 1);
        assert!(descriptor.protocols.iter().all(|implementation|
            implementation.associated_types.is_empty()),
            "an implementation-only binder must not escape into the nominal telescope");
    }
}

#[test]
fn implementation_equations_cannot_supply_a_sibling_or_unrelated_subject_bound() {
    for clause in ["", "where Other.Item: Picker"] {
        let module = parse(&format!(r#"
            module consumer;
            type Outcome is Idle | Selected(Int);
            type Picker is protocol {{ fn pick(&self) -> Outcome; }};
            type Family is protocol {{ type Item; }};
            type Maker is protocol {{ type Output; }};
            type Wrapper<F> is {{ make: F }};
            implement<Produced, Factory: fn() -> Produced> Maker for Wrapper<Factory> {{
                type Output = Produced;
                fn proved(&self, item: Produced) -> Int where Self.Output: Picker {{
                    match item.pick() {{ Selected(value) => value, _ => 0 }}
                }}
                fn unproved<Other: Family>(&self, item: Produced) -> Int {clause} {{
                    match item.pick() {{ Selected(value) => value, _ => 0 }}
                }}
            }}
        "#));
        let error = compile(&[&module]).err().expect("an equation is not a protocol proof").to_string();
        assert!(error.contains("no declared protocol method `pick`") && error.contains("unproved"), "{error}");
    }

    let module = parse(r#"
        module consumer;
        type Outcome is Idle | Selected(Int);
        type Picker is protocol { fn pick(&self) -> Outcome; };
        type Maker is protocol { type Output; };
        type Wrapper<F> is { make: F };
        implement<Produced, Factory: fn() -> Produced> Maker for Wrapper<Factory> {
            type Output = Produced;
            fn proved(item: Produced) -> Int where Self.Output: Picker {
                match item.pick() { Selected(value) => value, _ => 0 }
            }
        }
        type Sibling<F> is { make: F };
        implement<Produced, Factory: fn() -> Produced> Maker for Sibling<Factory> {
            type Output = Int;
            fn unproved(item: Produced) -> Int where Self.Output: Picker {
                match item.pick() { Selected(value) => value, _ => 0 }
            }
        }
    "#);
    let error = compile(&[&module]).err().expect("a sibling's premise cannot prove Produced").to_string();
    assert!(error.contains("no declared protocol method `pick`") && error.contains("unproved"), "{error}");
}

#[test]
fn an_implementation_equation_cannot_capture_a_method_only_type_parameter() {
    let module = parse(r#"
        module consumer;
        type Maker is protocol { type Output; };
        type Wrapper is { marker: Int };
        implement Maker for Wrapper {
            type Output = Undeclared;
            fn shadow<Undeclared>(item: Undeclared) -> Undeclared { item }
        }
    "#);
    let error = compile(&[&module]).err().expect("method binders cannot declare an implementation member").to_string();
    assert!(error.contains("associated implementation equation has no declared target")
        || error.contains("Undeclared"), "{error}");
}

#[test]
fn actual_mapped_iterator_cmp_retains_its_implementation_only_item_parameter() {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    let mut iterator = actual_iterator(&["cmp"]);
    let actual = parse(include_str!("../../../core/base/iterator.vr"));
    let mapped_type = actual.items.iter().find(|item|
        matches!(&item.kind, ItemKind::Type(declaration) if declaration.name.as_str() == "MappedIter")).unwrap();
    let fused_type = actual.items.iter().find(|item|
        matches!(&item.kind, ItemKind::Type(declaration) if declaration.name.as_str() == "FusedIterator")).unwrap();
    let fused_impl = actual.items.iter().find(|item| {
        let ItemKind::Impl(declaration) = &item.kind else { return false; };
        let ImplKind::Protocol { protocol, for_type, .. } = &declaration.kind else { return false; };
        protocol.to_string() == "FusedIterator" && verum_ast::pretty::format_type(for_type) == "MappedIter<I, F>"
    }).unwrap();
    let mut mapped_impl = actual.items.iter().find(|item| {
        let ItemKind::Impl(declaration) = &item.kind else { return false; };
        let ImplKind::Protocol { protocol, for_type, .. } = &declaration.kind else { return false; };
        protocol.to_string() == "Iterator" && verum_ast::pretty::format_type(for_type) == "MappedIter<I, F>"
    }).unwrap().clone();
    let ItemKind::Impl(declaration) = &mut mapped_impl.kind else { unreachable!() };
    let selected: List<_> = declaration.items.iter().filter(|item| match &item.kind {
        ImplItemKind::Type { name, .. } => name.as_str() == "Item",
        ImplItemKind::Function(function) => function.name.as_str() == "next",
        _ => false,
    }).cloned().collect();
    assert_eq!(selected.len(), 2);
    declaration.items = selected.clone();
    assert_eq!(declaration.items, selected, "the selected associated equation and next body remain unchanged");
    iterator.items.push(mapped_type.clone());
    iterator.items.push(fused_type.clone());
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.ordering.Ordering; mount core.base.protocols.Ord;").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "PartialEq" | "Eq" | "Ord"),
        _ => false,
    });
    protocols.items.extend(parse("mount core.base.ordering.Ordering;").items);
    let mut ordering = parse(include_str!("../../../core/base/ordering.vr"));
    ordering.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "Ordering",
        _ => false,
    });
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "Maybe",
        ItemKind::Impl(declaration) if matches!(&declaration.kind,
            ImplKind::Inherent(ty) if verum_ast::pretty::format_type(ty) == "Maybe<T>") => {
            declaration.items.retain(|item| matches!(&item.kind,
                ImplItemKind::Function(function) if function.name.as_str() == "map"));
            !declaration.items.is_empty()
        }
        _ => false,
    });
    let mut primitives = parse(include_str!("../../../core/base/primitives.vr"));
    primitives.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Impl(declaration) => matches!(&declaration.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if matches!(protocol.to_string().as_str(), "PartialEq" | "Eq" | "Ord")
                    && matches!(for_type.kind, verum_ast::ty::TypeKind::Int)),
        _ => false,
    });
    assert_eq!(primitives.items.iter().filter(|item| matches!(item.kind, ItemKind::Impl(_))).count(), 3,
        "the actual Int equality and ordering implementations must be present");
    primitives.items.extend(parse(r#"
        mount core.base.protocols.{PartialEq, Eq, Ord};
        mount core.base.ordering.Ordering;
        mount core.intrinsics.arithmetic.eq;
    "#).items);
    let mut arithmetic = parse(include_str!("../../../core/intrinsics/arithmetic.vr"));
    arithmetic.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Function(function) => function.name.as_str() == "eq",
        _ => false,
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{Iterator, MappedIter};
        mount core.base.maybe.Maybe;
        mount core.base.ordering.Ordering;
        type Sequence is { value: Int, remaining: Bool };
        implement Iterator for Sequence {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> {
                if self.remaining { self.remaining = false; Maybe.Some(self.value) }
                else { Maybe.None }
            }
        }
        fn increment(value: Int) -> Int { value + 1 }
        fn run() -> Int {
            let mapped = MappedIter { iter: Sequence { value: 40, remaining: true }, f: increment };
            let other = Sequence { value: 41, remaining: true };
            match mapped.cmp(other) { Ordering.Equal => 43, _ => 0 }
        }
    "#);
    for direct_first in [false, true] {
        let mut iterator = iterator.clone();
        let implementations = if direct_first { [mapped_impl.clone(), fused_impl.clone()] }
            else { [fused_impl.clone(), mapped_impl.clone()] };
        iterator.items.extend(implementations);
        let module = compile(&[&ordering, &maybe, &protocols, &arithmetic, &primitives, &iterator, &consumer]).unwrap();
        assert_tags(module.clone(), "MappedIter.cmp", &[0, 0, 0, 1, 1, 0, 1, 1, 1]);
        execute_source_and_wire(module, 43);
    }
}

#[test]
fn declared_function_values_supply_record_arguments_and_preserve_lexical_shadowing() {
    let provider = parse("module provider; public fn increment(value: Int) -> Int { value + 1 }");
    let consumer = parse(r#"
        module consumer;
        mount provider.increment as mounted;
        type Holder<F> is { callback: F };
        fn increment(value: Int) -> Int { value + 20 }
        fn run() -> Int {
            fn increment(value: Int) -> Int { value + 2 }
            let first = Holder { callback: mounted };
            let second = Holder { callback: increment };
            let increment = 7;
            let third = Holder { callback: increment };
            (first.callback)(10) + (second.callback)(20) + third.callback
        }
    "#);
    execute_source_and_wire(compile(&[&provider, &consumer]).unwrap(), 40);
}

#[test]
fn function_values_refuse_unmounted_and_ambiguous_source_declarations() {
    let provider = parse("module provider; public fn increment(value: Int) -> Int { value + 1 }");
    for source in [
        r#"module consumer; type Holder<F> is { callback: F };
           fn run() { let value = Holder { callback: increment }; }"#,
        r#"module consumer; type Holder<F> is { callback: F };
           fn increment(value: Int) -> Int { value }
           fn increment(left: Int, right: Int) -> Int { left + right }
           fn run() { let value = Holder { callback: increment }; }"#,
    ] {
        assert!(compile(&[&provider, &parse(source)]).is_err(), "{source}");
    }
}

#[test]
fn nominal_constructors_do_not_borrow_a_function_family_from_shared_synthetic_ids() {
    let provider = parse(r#"
        module provider;
        public type Mounted is ();
        public fn callback(value: Int) -> Int { value + 1 }
    "#);
    let consumer = parse(r#"
        module consumer;
        mount provider.Mounted as Remote;
        mount provider.callback as selected;
        type Count is { value: Int };
        type Marker is ();
        type Holder<F> is { callback: F };
        fn run() -> Int {
            let marker = Marker();
            let remote = Remote();
            let count = Count { value: 42 };
            let holder = Holder { callback: selected };
            (holder.callback)(count.value)
        }
    "#);
    execute_source_and_wire(compile(&[&provider, &consumer]).unwrap(), 43);
}

#[test]
fn a_static_member_called_through_a_value_keeps_only_its_declared_parameters() {
    let module = parse(r#"
        module consumer;
        type Factory<T> is { marker: T };
        implement<T> Factory<T> {
            fn identity<U>(value: U) -> U { value }
            fn answer(value: Int) -> Int { value + 7 }
        }
        fn run() -> Int {
            let factory = Factory { marker: true };
            factory.answer(factory.identity(35))
        }
    "#);
    execute_source_and_wire(compile(&[&module]).unwrap(), 42);
    for call in ["factory.answer()", "factory.foreign(35)"] {
        let module = parse(&format!(r#"
            module consumer;
            type Factory is {{ marker: Bool }};
            implement Factory {{ fn answer(value: Int) -> Int {{ value + 7 }} }}
            type Other is {{ marker: Bool }};
            implement Other {{ fn foreign(value: Int) -> Int {{ value + 7 }} }}
            fn rejected(factory: &Factory) -> Int {{ {call} }}
        "#));
        let error = compile(&[&module]).err().expect("a value receiver does not change static arity or member owner");
        assert!(matches!(&error.kind, verum_vbc::codegen::CodegenErrorKind::UndefinedFunction(_)), "{error}");
    }
}

fn associated_argument_provider() -> Module {
    parse(r#"
        module provider;
        public type Family<Tag> is protocol { type Item; };
        public type Pair<A, B> is { first: A, second: B };
        implement<Result, Left: Family<Int, Item = Result>, Right: Family<Int, Item = Result>> Pair<Left, Right> {
            fn answer(&self) -> Int { 43 }
            fn forwarded(&self) -> Int { self.answer() }
        }
    "#)
}

#[test]
fn associated_premises_bind_residual_arguments_in_the_selected_declaration_scope() {
    let provider = associated_argument_provider();
    let consumer = parse(r#"
        module consumer;
        mount provider.{Family, Pair};
        type Source is { marker: Int };
        implement Family<Int> for Source { type Item = Int; }
        fn relay<Unused, Other: Family<Int, Item = Int>, First: Family<Int, Item = Int>>(
            marker: Unused, value: Pair<First, Other>
        ) -> Int { value.forwarded() }
        fn run() -> Int {
            relay(true, Pair { first: Source { marker: 1 }, second: Source { marker: 2 } })
        }
    "#);
    execute_source_and_wire(compile(&[&provider, &consumer]).unwrap(), 43);
}

#[test]
fn absent_or_conflicting_associated_premises_cannot_fill_residual_slots() {
    let provider = associated_argument_provider();
    for bounds in [
        "First, Second",
        "First: Family<Int, Item = Int>, Second: Family<Int, Item = Bool>",
        "First: Family<Bool, Item = Int>, Second: Family<Bool, Item = Int>",
    ] {
        let consumer = parse(&format!(r#"
            module consumer;
            mount provider.{{Family, Pair}};
            fn rejected<{bounds}>(value: Pair<First, Second>) -> Int {{ value.answer() }}
        "#));
        let error = compile(&[&provider, &consumer]).err().expect("unproved associated evidence must refuse");
        assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::TypeInference(_)), "{error}");
    }
}

#[test]
fn a_same_leaf_protocol_cannot_supply_another_owners_associated_argument() {
    let provider = associated_argument_provider();
    let consumer = parse(r#"
        module consumer;
        mount provider.Pair;
        type Family<Tag> is protocol { type Item; };
        fn rejected<First: Family<Int, Item = Int>, Second: Family<Int, Item = Int>>(
            value: Pair<First, Second>
        ) -> Int { value.answer() }
    "#);
    let error = compile(&[&provider, &consumer]).err().expect("same member and application spelling are not owner evidence");
    assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::TypeInference(_)), "{error}");
}

#[test]
fn explicit_arguments_cannot_discharge_an_unproved_associated_premise() {
    for bound in ["", ": Family<Bool, Item = Int>", ": Family<Int, Item = Bool>"] {
        let source = parse(&format!(r#"
            module consumer;
            type Family<Tag> is protocol {{ type Item; }};
            fn selected<Value, Source: Family<Int, Item = Value>>(source: Source) -> Int {{ 43 }}
            fn rejected<Actual{bound}>(source: Actual) -> Int {{ selected<Int, Actual>(source) }}
        "#));
        let error = compile(&[&source]).err().expect("explicit arguments do not prove their own bounds");
        assert!(matches!(&error.kind, verum_vbc::codegen::CodegenErrorKind::TypeInference(_)), "{error}");
    }
}

#[test]
fn inherited_associated_premises_keep_the_parent_application_and_source_owner() {
    let provider = parse(r#"
        module provider;
        public type Family<Tag> is protocol { type Item; };
        public type Child<Tag> is protocol extends Family<Tag> {};
        public type Pair<First, Second> is { first: First, second: Second };
        implement<Result, First: Child<Int, Item = Result>, Second: Child<Int, Item = Result>> Pair<First, Second> {
            fn answer(&self) -> Int { 43 }
        }
    "#);
    let consumer = parse(r#"
        module consumer;
        mount provider.{Child, Pair};
        type Family<Tag> is protocol { type Item; };
        type Source is { marker: Int };
        implement Child<Int> for Source { type Item = Int; }
        fn relay<First: Child<Int, Item = Int>, Second: Child<Int, Item = Int>>(value: Pair<First, Second>) -> Int {
            value.answer()
        }
        fn run() -> Int {
            relay(Pair { first: Source { marker: 1 }, second: Source { marker: 2 } })
        }
    "#);
    execute_source_and_wire(compile(&[&provider, &consumer]).unwrap(), 43);
}

#[test]
fn inherited_associated_premises_refuse_distinct_members_and_applications() {
    for (parents, bound) in [
        ("Left<Tag>", "Child<Bool, Item = Int>"),
        ("Left<Tag> + Right<Tag>", "Child<Int, Item = Int>"),
    ] {
        let source = parse(&format!(r#"
            module consumer;
            type Left<Tag> is protocol {{ type Item; }};
            type Right<Tag> is protocol {{ type Item; }};
            type Child<Tag> is protocol extends {parents} {{}};
            type Holder<Source> is {{ source: Source }};
            implement<Value, Source: Child<Int, Item = Value>> Holder<Source> {{
                fn answer(&self) -> Int {{ 43 }}
            }}
            fn rejected<Source: {bound}>(value: Holder<Source>) -> Int {{ value.answer() }}
        "#));
        let error = compile(&[&source]).err().expect("an inherited member requires one exact declaring application");
        assert!(matches!(&error.kind, verum_vbc::codegen::CodegenErrorKind::TypeInference(_)), "{error}");
    }
}

#[test]
fn actual_chain_iterator_max_completes_its_associated_implementation_parameter() {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    let mut iterator = actual_iterator(&["max", "reduce", "fold"]);
    let actual = parse(include_str!("../../../core/base/iterator.vr"));
    iterator.items.push(actual.items.iter().find(|item| matches!(&item.kind,
        ItemKind::Type(declaration) if declaration.name.as_str() == "ChainIter")).unwrap().clone());
    let mut implementation = actual.items.iter().find(|item| matches!(&item.kind,
        ItemKind::Impl(declaration) if matches!(&declaration.kind,
            ImplKind::Protocol { protocol, for_type, .. } if protocol.to_string() == "Iterator"
                && verum_ast::pretty::format_type(for_type) == "ChainIter<A, B>"))).unwrap().clone();
    let ItemKind::Impl(declaration) = &mut implementation.kind else { unreachable!() };
    let originals: List<_> = declaration.items.iter().filter(|item| match &item.kind {
        ImplItemKind::Type { name, .. } => name.as_str() == "Item",
        ImplItemKind::Function(function) => function.name.as_str() == "next",
        _ => false,
    }).cloned().collect();
    assert_eq!(originals.len(), 2);
    declaration.items = originals.clone();
    assert_eq!(declaration.items, originals, "the actual associated equation and next body remain unchanged");
    iterator.items.push(implementation);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.ordering.Ordering; mount core.base.protocols.Ord;").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => matches!(declaration.name.as_str(), "PartialEq" | "Eq" | "Ord"),
        _ => false,
    });
    protocols.items.extend(parse("mount core.base.ordering.Ordering;").items);
    let [ordering, maybe] = [
        (include_str!("../../../core/base/ordering.vr"), "Ordering"),
        (include_str!("../../../core/base/maybe.vr"), "Maybe"),
    ].map(|(source, name)| {
        let mut module = parse(source);
        module.items.retain(|item| match &item.kind {
            ItemKind::Module(declaration) => declaration.items.is_none(),
            ItemKind::Type(declaration) => declaration.name.as_str() == name,
            _ => false,
        });
        module
    });
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{Iterator, ChainIter};
        mount core.base.maybe.Maybe;
        type Sequence is { value: Int, remaining: Bool };
        implement Iterator for Sequence {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> {
                if self.remaining { self.remaining = false; Maybe.Some(self.value) }
                else { Maybe.None }
            }
        }
        fn run() -> Int {
            let values = ChainIter {
                first: Maybe.Some(Sequence { value: 3, remaining: true }),
                second: Sequence { value: 7, remaining: true },
            };
            match values.max() { Maybe.Some(value) => value, Maybe.None => 0 }
        }
    "#);
    execute_source_and_wire(compile(&[&ordering, &maybe, &protocols, &iterator, &consumer]).unwrap(), 7);
}
