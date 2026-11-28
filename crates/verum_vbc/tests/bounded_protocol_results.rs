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

#[test]
fn static_protocol_calls_bind_the_implementation_receiver_in_its_own_order() {
    let source = parse(concat!("module consumer;\n", include_str!("fixtures/static_receiver_application.vr")));
    let mut module = compile(&[&source]).unwrap();
    execute_source_and_wire(module.clone(), 1441);

    use verum_vbc::types::{StringId, TypeId, TypeRef};
    module.resolve_protocol_dispatch();
    let relay = module.get_function(module.find_function_by_name("consumer.relay").unwrap()).unwrap();
    let start = relay.bytecode_offset as usize;
    let body = decode_instructions(&module.bytecode[start..start + relay.bytecode_length as usize]).unwrap();
    let method = body.iter().find_map(|op| match op {
        Instruction::CallM { method_id, .. } => Some(StringId(*method_id)),
        _ => None,
    }).unwrap();
    let pair = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Pair")).unwrap().id;
    let u8_ty = TypeRef::Concrete(TypeId::U8);
    let u32_ty = TypeRef::Concrete(TypeId::U32);
    let application = TypeRef::instantiated(pair, [u8_ty.clone(), u32_ty.clone()]);
    let (target, arguments) = module.try_bound_method(&application, method, 0).unwrap().unwrap();
    let parameters = &module.get_function(target).unwrap().type_params;
    let bound = |name| parameters.iter().position(|p| module.get_string(p.name) == Some(name))
        .and_then(|i| arguments.get(i));
    assert_eq!(bound("A"), Some(&u8_ty));
    assert_eq!(bound("B"), Some(&u32_ty));
    assert!(module.try_bound_method(&TypeRef::Concrete(TypeId::INT), method, 0).is_err());
    module.get_function_mut(target).unwrap().source_receiver = None;
    assert!(module.try_bound_method(&application, method, 0).is_err());
}

fn compile(files: &[&Module]) -> CodegenResult<VbcModule> {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.collect_unit_declarations(files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(files, ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn actual_maybe_iterator_collect_passes_its_owned_receiver_to_from_iter() {
    use verum_ast::decl::{ImplItemKind, ImplKind};

    let mut iterator = actual_iterator(&["collect"]);
    let from_iterator = parse(include_str!("../../../core/base/iterator.vr")).items.into_iter()
        .find(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.as_str() == "FromIterator")).unwrap();
    iterator.items.push(from_iterator);
    iterator.items.extend(parse("mount core.base.maybe.Maybe;").items);
    let mut memory = parse(include_str!("../../../core/base/memory.vr"));
    memory.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Function(decl) => decl.name.as_str() == "replace",
        _ => false,
    });
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "Maybe" | "MaybeIter"),
        ItemKind::Impl(decl) => {
            match &decl.kind {
                ImplKind::Inherent(ty) if verum_ast::pretty::format_type(ty) == "Maybe<T>" => {
                    decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(fun) if fun.name.as_str() == "take"));
                }
                ImplKind::Protocol { protocol, for_type, .. } if protocol.to_string() == "Iterator"
                    && verum_ast::pretty::format_type(for_type) == "MaybeIter<T>" => {
                    decl.items.retain(|item| match &item.kind {
                        ImplItemKind::Type { name, .. } => name.as_str() == "Item",
                        ImplItemKind::Function(fun) => fun.name.as_str() == "next",
                        _ => false,
                    });
                }
                _ => return false,
            }
            !decl.items.is_empty()
        }
        _ => false,
    });
    maybe.items.extend(parse(r#"
        mount core.base.memory.replace as replace_value;
        mount core.base.iterator.Iterator;
        public fn single<T>(value: T) -> MaybeIter<T> { MaybeIter { inner: Maybe.Some(value) } }
    "#).items);
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{Iterator, FromIterator};
        mount core.base.maybe.{Maybe, single};
        type Captured<T> is { value: T };
        implement<T> FromIterator<T> for Captured<T> {
            fn from_iter<Sequence: Iterator<Item = T>>(iter: Sequence) -> Self {
                let mut source = iter;
                match source.next() {
                    Maybe.Some(value) => Captured { value },
                    Maybe.None => panic("expected a value"),
                }
            }
        }
        fn run() -> Int {
            let row: Captured<Int> = single(73).collect<Captured<Int>>();
            row.value
        }
    "#);
    let module = compile(&[&memory, &maybe, &iterator, &consumer]).unwrap();
    execute_source_and_wire(module, 73);
}

#[test]
fn static_protocol_method_arguments_respect_the_selected_declaration() {
    let source = |call: &str| parse(&format!(r#"
        module consumer;
        type Factory is protocol {{ fn measure<U>(left: U, right: U) -> Int; }};
        type Pair<A,B> is {{ first: A, second: B }};
        implement<B,A> Factory for Pair<A,B> {{
            fn measure<Value>(left: Value, right: Value) -> Int {{ A.size * 100 + B.size * 10 + Value.size }}
        }}
        fn width<C: Factory, V>(value: V) -> Int {{ {call} }}
        fn run() -> Int {{ width<Pair<UInt8, UInt32>, UInt16>(1_u16) }}
    "#));
    for call in ["C.measure(value, value)", "C.measure<V>(value, value)", "C.measure<_>(value, value)"] {
        let mut module = compile(&[&source(call)]).unwrap();
        execute_source_and_wire(module.clone(), 142);
        module.resolve_protocol_dispatch();
        let graph = verum_vbc::mono::module_instantiations(&module).unwrap();
        let specialized = verum_vbc::mono::monomorphize_minimal(module, &graph).unwrap().module;
        let target = specialized.find_function_by_name("consumer.Pair.measure").unwrap();
        use verum_vbc::types::{TypeId, TypeRef};
        let arguments: List<_> = specialized.get_function(target).unwrap().type_params.iter().map(|parameter| {
            TypeRef::Concrete(match specialized.get_string(parameter.name).unwrap() {
                "A" => TypeId::U8, "B" => TypeId::U32, "Value" => TypeId::U16,
                other => panic!("unexpected implementation binder {other}"),
            })
        }).collect();
        assert!(specialized.specialized_function_id(target, &arguments).is_some());
        execute_source_and_wire(specialized, 142);
    }
    for call in ["C.measure(value, true)", "C.measure<Bool>(value, value)"] {
        let error = compile(&[&source(call)]).unwrap_err().to_string();
        assert!(error.contains("operand does not establish its declared type"), "{error}");
    }
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
fn range_syntax_and_iterator_declaration_share_the_runtime_owner() {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    use verum_vbc::types::TypeId;

    let mut iterator = actual_iterator(&[]);
    let mut source = parse(include_str!("../../../core/base/iterator.vr"));
    source.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "Range" | "IntoIterator"),
        ItemKind::Function(decl) => decl.name.as_str() == "range",
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if matches!(protocol.to_string().as_str(), "Iterator" | "IntoIterator")
                && verum_ast::pretty::format_type(for_type) == "Range<Int>") => {
            decl.items.retain(|item| match &item.kind {
                ImplItemKind::Type { .. } => true,
                ImplItemKind::Function(fun) => matches!(fun.name.as_str(), "next" | "into_iter"),
                _ => false,
            });
            true
        }
        _ => false,
    });
    iterator.items.extend(source.items);
    iterator.items.extend(parse("mount core.base.maybe.Maybe;").items);
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.as_str() == "Maybe",
        _ => false,
    });
    let foreign = parse(r#"
        module foreign;
        public type Range is { marker: Int };
        public fn make() -> Range { Range { marker: 30 } }
        public fn marker() -> Int { make().marker }
    "#);
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{Range, Iterator, IntoIterator, range};
        mount foreign.marker;
        fn sum(span: Range<Int>) -> Int {
            let mut total = 0;
            for n in span { total += n; }
            total
        }
        fn run() -> Int { marker() + sum(2..2) + sum(1..4) + sum(1..=4) + sum(range(8, 11)) }
    "#);
    for (first, second) in [(&foreign, &iterator), (&iterator, &foreign)] {
        let module = compile(&[&maybe, first, second, &consumer]).unwrap();
        let returned = |name| module.get_function(module.find_function_by_name(name).unwrap())
            .unwrap().semantic_return.as_ref().unwrap().base_type_id().unwrap();
        assert_eq!(returned("core.base.iterator.range"), TypeId::RANGE);
        assert_ne!(returned("foreign.make"), TypeId::RANGE);
        execute_source_and_wire(module, 73);
    }
}

#[test]
fn a_protocol_call_passes_its_declared_shared_receiver_to_a_scalar_body() {
    let source = parse(concat!("module consumer;\n", include_str!("fixtures/bound_borrowed_byte.vr")));
    execute_source_and_wire(compile(&[&source]).unwrap(), 73);
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
        let TypeRef::AssociatedProjection { base: receiver, qualifier: Some(declaration), assoc, args: projection_args } = result else {
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
        assert_eq!(declaration.protocol.0, iterator.id.0);
        assert!(declaration.args.is_empty());
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
fn actual_mapped_iterator_nth_back_uses_the_inherited_item_equation() {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    let mut iterator = actual_iterator(&["nth", "advance_by"]);
    let mut source = parse(include_str!("../../../core/base/iterator.vr"));
    source.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(decl) if decl.name.as_str() == "DoubleEndedIterator" => {
            let TypeDeclBody::Protocol(body) = &mut decl.body else { unreachable!() };
            body.items.retain(|item| matches!(&item.kind, ProtocolItemKind::Function { decl, .. }
                if matches!(decl.name.as_str(), "next_back" | "advance_back_by" | "nth_back")));
            true
        }
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "MappedIter" | "Range" | "RepeatWith" | "IntoIterator"),
        ItemKind::Function(decl) => matches!(decl.name.as_str(), "range" | "repeat_with"),
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.to_string() == "IntoIterator" && verum_ast::pretty::format_type(for_type) == "I") => true,
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if matches!(protocol.to_string().as_str(), "Iterator" | "DoubleEndedIterator")
            && matches!(verum_ast::pretty::format_type(for_type).as_str(), "MappedIter<I, F>" | "RepeatWith<T, F>" | "Range<Int>")) => {
            decl.items.retain(|item| match &item.kind {
                ImplItemKind::Type { name, .. } => name.as_str() == "Item",
                ImplItemKind::Function(fun) => matches!(fun.name.as_str(), "next" | "next_back"),
                _ => false,
            });
            true
        }
        _ => false,
    });
    iterator.items.extend(source.items);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.result.Result;").items);
    iterator.items.extend(parse(r#"
        public type Alternate is protocol { type Item; };
        implement<I, F> Alternate for MappedIter<I, F> { type Item = UInt32; }
    "#).items);
    let mut maybe = parse(include_str!("../../../core/base/maybe.vr"));
    maybe.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.as_str() == "Maybe",
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Inherent(ty) if verum_ast::pretty::format_type(ty) == "Maybe<T>") => {
            decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(fun) if matches!(fun.name.as_str(), "map" | "is_none")));
            !decl.items.is_empty()
        }
        _ => false,
    });
    let result = parse("module core.base.result; public type Result<T,E> is Ok(T) | Err(E);");
    let consumer = r#"
        module consumer;
        mount core.base.iterator.{Iterator, DoubleEndedIterator, MappedIter, Alternate, repeat_with};
        mount core.base.maybe.Maybe;
        type Digits is { high: Int };
        implement Alternate for Digits { type Item = UInt32; }
        implement Iterator for Digits {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> { self.next_back() }
        }
        implement DoubleEndedIterator for Digits {
            fn next_back(&mut self) -> Maybe<Int> {
                if self.high == 0 { return Maybe.None; }
                let value = self.high;
                self.high -= 1;
                Maybe.Some(value)
            }
        }
        fn is_two(value: Int) -> Bool { value == 2 }
        fn three() -> Int { 3 }
        fn run() -> Int {
            let mut mapped = MappedIter { iter: Digits { high: 3 }, f: is_two };
            let first = match mapped.nth_back(1) { Maybe.Some(flag) => flag, _ => return 0 };
            let last = match mapped.next_back() { Maybe.Some(flag) => flag, _ => return 0 };
            if first && !last && mapped.next_back().is_none() {
                let mut repeated = repeat_with(three);
                match repeated.nth(4) { Maybe.Some(value) => 70 + value, _ => 0 }
            } else { 0 }
        }
    "#;
    execute_source_and_wire(compile(&[&maybe, &result, &iterator, &parse(consumer)]).unwrap(), 73);
    let wrong = parse(&consumer.replace("is_two(value: Int)", "is_two(value: UInt32)"));
    let mismatch = compile(&[&maybe, &result, &iterator, &wrong]).unwrap_err().to_string();
    assert!(mismatch.contains("conflicting source argument evidence"), "{mismatch}");
}

#[test]
fn actual_filtered_try_collect_preserves_the_declared_predicate_and_result() {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    let mut iterator = actual_iterator(&["try_collect"]);
    let mut actual = parse(include_str!("../../../core/base/iterator.vr"));
    actual.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(decl) => matches!(decl.name.as_str(), "FilterIter" | "Extend"),
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.to_string() == "Iterator" && verum_ast::pretty::format_type(for_type) == "FilterIter<I, P>") => {
            decl.items.retain(|item| match &item.kind {
                ImplItemKind::Type { name, .. } => name.as_str() == "Item",
                ImplItemKind::Function(fun) => fun.name.as_str() == "next",
                _ => false,
            });
            true
        }
        _ => false,
    });
    assert_eq!(actual.items.len(), 3);
    iterator.items.extend(actual.items);
    iterator.items.extend(parse("mount core.base.maybe.Maybe; mount core.base.result.Result; mount core.base.protocols.Default;").items);
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.as_str() == "Default",
        _ => false,
    });
    let maybe = parse("module core.base.maybe; public type Maybe<T> is None | Some(T);");
    let result = parse("module core.base.result; public type Result<T,E> is Ok(T) | Err(E);");
    let consumer = r#"
        module consumer;
        mount core.base.iterator.{Iterator, FilterIter, Extend};
        mount core.base.maybe.Maybe;
        mount core.base.result.Result;
        mount core.base.protocols.Default;
        type Emissions is { position: Int, fails: Bool };
        implement Iterator for Emissions {
            type Item = Result<Int,Int>;
            fn next(&mut self) -> Maybe<Result<Int,Int>> {
                self.position += 1;
                if self.position == 1 { return Maybe.Some(Result.Ok(-6)); }
                if self.position == 2 { return Maybe.Some(Result.Ok(3)); }
                if self.position == 3 && self.fails { return Maybe.Some(Result.Err(9)); }
                if self.position == 3 { return Maybe.Some(Result.Ok(4)); }
                Maybe.None
            }
        }
        type Total is { value: Int };
        implement Default for Total { fn default() -> Total { Total { value: 0 } } }
        implement Extend<Int> for Total {
            fn extend<I: Iterator<Item = Int>>(&mut self, mut iter: I) {
                while let Maybe.Some(value) = iter.next() { self.extend_one(value); }
            }
            fn extend_one(&mut self, value: Int) { self.value += value; }
        }
        fn keep(input: &Result<Int,Int>) -> Bool {
            match *input { Result.Ok(value) => value > 0, Result.Err(_) => true }
        }
        fn collect_values(fails: Bool) -> Result<Total,Int> {
            let producer = FilterIter { iter: Emissions { position: 0, fails: fails }, predicate: keep };
            producer.try_collect<Int,Int,Total>()
        }
        fn run() -> Int {
            let sum = match collect_values(false) { Result.Ok(total) => total.value, _ => return 0 };
            match collect_values(true) { Result.Err(code) => 10 * sum + code, _ => 0 }
        }
    "#;
    execute_source_and_wire(compile(&[&maybe, &result, &protocols, &iterator, &parse(consumer)]).unwrap(), 79);
    let wrong = parse(&consumer.replace("keep(input: &Result<Int,Int>)", "keep(input: &Result<UInt32,Int>)"));
    let error = compile(&[&maybe, &result, &protocols, &iterator, &wrong]).unwrap_err().to_string();
    assert!(error.contains("conflicting source argument evidence"), "{error}");
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
fn mounted_variants_keep_the_selected_sum_declaration_in_value_position() {
    let provider = parse("module provider; public type Direction is West | East | Offset(Int);");
    let consumer = parse(r#"
        module consumer;
        mount provider.{Direction, East, West, Offset};
        fn choose(value: Int) -> Direction {
            if value < 0 { West } else if value > 0 { Offset(value) } else { East }
        }
        fn run() -> Int {
            let west = match choose(-1) { Direction.West => 30, _ => 0 };
            let east = match choose(0) { Direction.East => 40, _ => 0 };
            let offset = match choose(3) { Direction.Offset(n) => n, _ => 0 };
            west + east + offset
        }
    "#);
    execute_source_and_wire(compile(&[&provider, &consumer]).unwrap(), 73);

    let other = parse("module other; public type Status is East | Closed;");
    let ambiguous = parse(r#"
        module consumer;
        mount provider.{Direction, East};
        mount other.East;
        fn selected() -> Direction { East }
    "#);
    let error = compile(&[&provider, &other, &ambiguous]).expect_err("two constructor declarations");
    assert!(error.to_string().contains("conflict") || error.to_string().contains("ambig"), "{error}");
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
    let source = r#"
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
    "#;
    let consumer = parse(source);
    let module = compile(&[&ordering, &maybe, &protocols, &iterator, &consumer]).unwrap();
    let run = module.get_function(module.find_function_by_name("consumer.run").unwrap()).unwrap();
    let start = run.bytecode_offset as usize;
    let body = decode_instructions(&module.bytecode[start..start + run.bytecode_length as usize]).unwrap();
    use verum_vbc::types::{TypeId, TypeRef};
    assert!(body.iter().any(|operation| matches!(operation,
        Instruction::DropTyped { type_ref: TypeRef::Instantiated { base, args }, .. }
        if module.get_type(*base).and_then(|ty| module.get_string(ty.name)) == Some("Maybe")
            && args.as_slice() == [TypeRef::Concrete(TypeId::INT)])),
        "the result must retire Maybe<Int> under the proved Item = Int equation");
    execute_source_and_wire(module, 7);

    for (item, value) in [("Int", "7"), ("Bool", "true")] {
        let declaration = format!("type Other is {{ remaining: Bool }}; implement Iterator for Other {{
            type Item = {item}; fn next(&mut self) -> Maybe<{item}> {{
                if self.remaining {{ self.remaining = false; Maybe.Some({value}) }} else {{ Maybe.None }}
            }}
        }} fn run()");
        let mixed = parse(&source.replace("fn run()", &declaration)
            .replace("second: Sequence { value: 7, remaining: true }", "second: Other { remaining: true }"));
        let result = compile(&[&ordering, &maybe, &protocols, &iterator, &mixed]);
        if item == "Int" {
            execute_source_and_wire(result.unwrap(), 7);
        } else {
            let error = result.err().expect("the two independently selected Iterator.Item declarations must agree");
            assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::TypeInference(_)), "{error}");
        }
    }
}

#[test]
fn a_custom_for_loop_delivers_its_iterator_application_to_next() {
    let mut iterator = actual_iterator(&[]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe;").items);
    let maybe = parse("module core.base.maybe; public type Maybe<T> is None | Some(T);");
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        type Values<T> is { value: T, pending: Bool };
        implement<T> Iterator for Values<T> {
            type Item = T;
            fn next(&mut self) -> Maybe<T> {
                if !self.pending { return Maybe.None; }
                self.pending = false;
                let value = self.value;
                Maybe.Some(value)
            }
        }
        fn sum<U>(other: U, rows: Values<Int>) -> Int {
            let mut total = 0;
            for value in rows { total = total + value; }
            total
        }
        fn run() -> Int { sum(false, Values { value: 93, pending: true }) }
    "#);
    execute_source_and_wire(compile(&[&maybe, &iterator, &consumer]).unwrap(), 93);
}

fn actual_enumeration() -> (Module, Module) {
    use verum_ast::decl::{ImplItemKind, ImplKind};

    let mut iterator = actual_iterator(&["enumerate"]);
    iterator.items.extend(parse("mount core.base.maybe.Maybe;").items);
    let mut actual = parse(include_str!("../../../core/base/iterator.vr"));
    actual.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(declaration) => declaration.name.as_str() == "EnumerateIter",
        ItemKind::Impl(declaration) if matches!(&declaration.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if protocol.to_string() == "Iterator"
                    && verum_ast::pretty::format_type(for_type) == "EnumerateIter<I>") =>
        {
            declaration.items.retain(|item| match &item.kind {
                ImplItemKind::Type { name, .. } => name.as_str() == "Item",
                ImplItemKind::Function(function) => function.name.as_str() == "next",
                _ => false,
            });
            true
        }
        _ => false,
    });
    assert_eq!(actual.items.len(), 2);
    iterator.items.extend(actual.items);
    let maybe = parse("module core.base.maybe; public type Maybe<T> is None | Some(T);");
    (iterator, maybe)
}

#[test]
fn enumeration_binds_the_index_and_the_declared_borrowed_item() {
    let (iterator, maybe) = actual_enumeration();
    let source = r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.base.maybe.Maybe;
        type Source<T> is { first: T, second: T, at: Int };
        type Other is protocol { type Item; };
        implement<T> Other for Source<T> { type Item = Bool; }
        implement<T> Iterator for Source<T> {
            type Item = &T;
            fn next(&mut self) -> Maybe<&T> {
                self.at += 1;
                if self.at == 1 { Maybe.Some(&self.first) }
                else if self.at == 2 { Maybe.Some(&self.second) }
                else { Maybe.None }
            }
        }
        fn accept<T>(value: T, exemplar: T) -> T { value }
        fn run() -> Int {
            let rows = Source { first: 37_u8, second: 55_u8, at: 0 };
            let mut total = 0;
            for (i, b) in rows.enumerate() { total += i + (accept(*b, 7_u8) as Int); }
            total
        }
    "#;
    let consumer = parse(source);
    execute_source_and_wire(compile(&[&maybe, &iterator, &consumer]).unwrap(), 93);
    let incompatible = parse(&source.replace("accept(*b, 7_u8)", "accept(*b, false)"));
    let error = compile(&[&maybe, &iterator, &incompatible]).unwrap_err();
    assert!(error.to_string().contains("source argument evidence"), "{error}");
}

#[test]
fn a_for_loop_acquires_and_retires_the_declared_cursor() {
    let (mut iterator, maybe) = actual_enumeration();
    let actual = parse(include_str!("../../../core/base/iterator.vr"));
    iterator.items.extend(actual.items.into_iter().filter(|item| matches!(&item.kind,
        ItemKind::Type(declaration) if declaration.name.as_str() == "IntoIterator")));
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| matches!(&item.kind,
        ItemKind::Module(declaration) if declaration.items.is_none()) || matches!(&item.kind,
        ItemKind::Type(declaration) if declaration.name.as_str() == "Drop"));
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{IntoIterator, Iterator};
        mount core.base.maybe.Maybe;
        mount core.base.protocols.Drop;
        static mut RELEASED: Int = 0;
        type Sequence is { seed: Int };
        type Cursor is { remaining: Int };
        implement Drop for Cursor { fn drop(&mut self) { RELEASED = RELEASED + 1; } }
        implement IntoIterator for Sequence {
            type Item = Int;
            type IntoIter = Cursor;
            fn into_iter(self) -> Cursor { Cursor { remaining: self.seed + 1 } }
        }
        implement Iterator for Cursor {
            type Item = Int;
            fn next(&mut self) -> Maybe<Int> {
                if self.remaining == 0 { return Maybe.None; }
                let value = self.remaining;
                self.remaining = value - 1;
                Maybe.Some(value)
            }
        }
        fn first_value(source: Sequence) -> Int {
            for value in source { return value; }
            0
        }
        fn run() -> Int {
            RELEASED = 0;
            let mut total = 0;
            let first = Sequence { seed: 2 };
            let second = Sequence { seed: 3 };
            for n in first { total = total * 10 + n; }
            for n in second { total = total + n; break; }
            total = total + first_value(Sequence { seed: 4 });
            total * 10 + RELEASED
        }
    "#);
    let module = compile(&[&protocols, &maybe, &iterator, &consumer]).unwrap();
    execute_source_and_wire(module, 3303);
}

#[test]
fn bounded_slice_advancement_preserves_the_shared_iterator_contract() {
    use verum_ast::decl::ImplKind;
    let mut iterator = actual_iterator(&["count", "advance_by", "nth"]);
    let mut reverse = parse(include_str!("../../../core/base/iterator.vr"));
    reverse.items.retain_mut(|item| {
        let ItemKind::Type(decl) = &mut item.kind else { return false };
        if decl.name.as_str() != "DoubleEndedIterator" { return false }
        let TypeDeclBody::Protocol(body) = &mut decl.body else { panic!("DoubleEndedIterator protocol") };
        body.items.retain(|item| matches!(&item.kind, ProtocolItemKind::Function { decl, .. }
            if matches!(decl.name.as_str(), "advance_back_by" | "next_back" | "nth_back")));
        true
    });
    iterator.items.extend(reverse.items);
    let imports = "mount core.base.maybe.Maybe; mount core.base.result.Result;";
    iterator.items.extend(parse(imports).items);
    let mut slice = parse(include_str!("../../../core/collections/slice.vr"));
    slice.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => decl.name.as_str() == "SliceIter",
        ItemKind::Function(decl) => decl.name.as_str() == "advance_index",
        ItemKind::Impl(decl) => match &decl.kind {
            ImplKind::Protocol { protocol, for_type, .. } => matches!(protocol.to_string().as_str(), "Iterator" | "DoubleEndedIterator")
                && verum_ast::pretty::format_type(for_type) == "SliceIter<T>",
            ImplKind::Inherent(ty) => verum_ast::pretty::format_type(ty) == "SliceIter<T>",
        },
        _ => false,
    });
    slice.items.extend(parse(imports).items);
    slice.items.extend(parse("mount core.base.iterator.{Iterator, DoubleEndedIterator};").items);
    let maybe = parse("module core.base.maybe; public type Maybe<T> is None | Some(T);");
    let result = parse("module core.base.result; public type Result<T,E> is Ok(T) | Err(E);");
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.{Iterator, DoubleEndedIterator};
        mount core.base.maybe.Maybe;
        mount core.base.result.Result;
        mount core.collections.slice.SliceIter;
        fn scan(bytes:&[Byte])->Int {
            let mut rows = SliceIter.new(bytes);
            if let Maybe.Some(_) = rows.nth(-1) { return -1; }
            if let Maybe.Some(_) = rows.nth_back(-1) { return -2; }
            let a = match rows.nth(1) { Maybe.Some(x) => *x as Int, _ => return -3 };
            let b = match rows.nth_back(0) { Maybe.Some(x) => *x as Int, _ => return -4 };
            let advanced = match rows.advance_back_by(9223372036854775807) {
                Result.Err(n) => n, _ => return -5
            };
            let mut beyond = SliceIter.new(bytes);
            if let Maybe.Some(_) = beyond.nth(9223372036854775807) { return -6; }
            if beyond.count() != 0 { return -7; }
            a * 100 + b * 10 + advanced + rows.count()
        }
        fn run()->Int { let bytes = [3_u8, 5_u8, 7_u8, 9_u8]; scan(&bytes) }
    "#);
    execute_source_and_wire(compile(&[&maybe, &result, &iterator, &slice, &consumer]).unwrap(), 591);
}

#[test]
fn the_slice_iterator_keeps_its_borrow_through_enumerate() {
    use verum_ast::decl::{ImplItemKind, ImplKind};

    let (mut iterator, maybe) = actual_enumeration();
    let actual = parse(include_str!("../../../core/base/iterator.vr"));
    iterator.items.extend(actual.items.into_iter().filter(|item| matches!(&item.kind,
        ItemKind::Type(declaration) if declaration.name.as_str() == "IntoIterator")));
    let mut slice = parse(include_str!("../../../core/collections/slice.vr"));
    slice.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Type(declaration) => declaration.name.as_str() == "SliceIter",
        ItemKind::Impl(declaration) if matches!(&declaration.kind,
            ImplKind::Protocol { protocol, for_type, .. }
                if matches!(protocol.to_string().as_str(), "Iterator" | "IntoIterator")
                    && verum_ast::pretty::format_type(for_type) == "SliceIter<T>") =>
        {
            declaration.items.retain(|item| match &item.kind {
                ImplItemKind::Type { .. } => true,
                ImplItemKind::Function(function) => matches!(function.name.as_str(), "next" | "into_iter"),
                _ => false,
            });
            true
        }
        _ => false,
    });
    slice.items.extend(parse("mount core.base.iterator.{Iterator, IntoIterator}; mount core.base.maybe.Maybe;").items);
    let consumer = parse(r#"
        module consumer;
        mount core.base.iterator.Iterator;
        mount core.collections.slice.SliceIter;
        fn accept<T>(value: T, exemplar: T) -> T { value }
        fn sum(bytes: &[Byte]) -> Int {
            let rows = SliceIter { slice: bytes, front: 0, back: 2 };
            let mut total = 0;
            for (i, b) in rows.enumerate() { total += i + (accept(*b, 7_u8) as Int); }
            total
        }
        fn run() -> Int { let input = [37_u8, 55_u8]; sum(&input) }
    "#);
    execute_source_and_wire(compile(&[&maybe, &iterator, &slice, &consumer]).unwrap(), 93);
}

#[test]
fn the_receiver_selects_its_unparameterized_protocol_with_a_byte_argument() {
    let module = constrained(r#"
        type Reader is protocol { fn decode(&self, marker: Byte) -> Int; };
        type Source is { offset: Int };
        implement Reader for Source {
            fn decode(&self, marker: Byte) -> Int { self.offset + (marker as Int) }
        }
        implement Source { fn other(&self, marker: Byte) -> Int { 0 } }
        fn read<R: Reader>(reader: &R, marker: Byte) -> Int { reader.decode(marker) }
        fn run() -> Int { read(&Source { offset: 3 }, 23_u8) }
    "#).unwrap();
    execute_source_and_wire(module.clone(), 26);

    let source = module.types.iter()
        .position(|owner| module.get_string(owner.name) == Some("Source")).unwrap();
    let alternate = module.find_function_by_name("consumer.Source.other").unwrap();
    for competing in [false, true] {
        let mut altered = module.clone();
        let rows = &mut altered.types[source].protocols;
        if competing {
            let mut row = rows[0].clone();
            row.methods[0] = alternate.0;
            rows.push(row);
        } else {
            rows.clear();
        }
        let entry = altered.find_function_by_name("consumer.run").unwrap();
        let error = verum_vbc::interpreter::Interpreter::new(altered.into())
            .execute_function(entry).unwrap_err();
        let reason = if competing { "multiple implementations" } else { "no implementation" };
        assert!(error.to_string().contains(reason), "{error}");
    }
}
