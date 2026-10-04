#![cfg(feature = "codegen")]
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    module::VbcModule,
    types::{TypeId, TypeRef},
};

fn parse(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().expect("source")
}
fn producer() -> (VbcModule, VbcCodegen) {
    let source = parse(
        r#"
module alpha.adapters;
type Unused is { data: Int };
type Payload is { value: Int };
type Stage<T> is { value: T };
type Wrapped<I,F> is { owner: I, callback: F };
implement<T> Stage<T> {
 fn transform<R, F: fn(T) -> R>(self, f: F) -> Wrapped<Self,F> { Wrapped { owner: self, callback: f } }
}
implement<I,F> Wrapped<I,F> { fn finish<C>(self) -> C { C.default() } }
"#,
    );
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("alpha"));
    let module = cg.compile_module(&source).expect("producer");
    (module, cg)
}
fn consumer() -> (VbcModule, VbcCodegen) {
    let (provider, exported) = producer();
    let source = parse(
        r#"
module consumer;
mount alpha.adapters.{Stage,Payload};
fn probe(stage: Stage<Payload>) -> Int { stage.transform(|x| x).finish<Int>() }
"#,
    );
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.import_functions(&exported.export_functions());
    cg.import_type_layouts(&exported.export_type_layouts());
    cg.import_type_field_names(&exported.export_type_field_names());
    cg.import_bootstrap_nominal_dependencies(&[&source], &[&provider])
        .expect("nominal boundary");
    cg.collect_unit_declarations(&[&source])
        .expect("declarations");
    let module = cg.compile_function_bodies(&source).expect("consumer");
    (module, cg)
}
fn nominal(module: &VbcModule, name: &str) -> TypeRef {
    TypeRef::Concrete(
        module
            .types
            .iter()
            .find(|t| module.strings.get(t.name) == Some(name))
            .expect(name)
            .id,
    )
}
#[test]
fn bootstrap_callable_parameters_have_consumer_nominal_identity() {
    let (module, _) = consumer();
    let closure = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|s| s.contains("$closure$"))
        })
        .expect("closure");
    assert_eq!(closure.params[1].type_ref, nominal(&module, "Payload"));
    assert_eq!(closure.return_type, nominal(&module, "Payload"));
}
#[test]
fn bootstrap_method_result_and_callable_bindings_remain_structural() {
    let (module, _) = consumer();
    let mut pc = 0;
    let mut found = List::new();
    while pc < module.bytecode.len() {
        if let verum_vbc::instruction::Instruction::CallG {
            func_id, type_args, ..
        } = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).expect("decode")
        {
            if module
                .band_reference_name(func_id)
                .is_some_and(|n| n.ends_with("Wrapped.finish"))
            {
                found.push(type_args);
            }
        }
    }
    let TypeRef::Concrete(stage) = nominal(&module, "Stage") else {
        panic!("stage")
    };
    let payload = nominal(&module, "Payload");
    let expected = vec![
        TypeRef::Instantiated {
            base: stage,
            args: vec![payload.clone()],
        },
        TypeRef::Function {
            params: vec![payload.clone()],
            return_type: Box::new(payload),
            contexts: Default::default(),
        },
        TypeRef::Concrete(TypeId::INT),
    ];
    assert_eq!(found.as_slice(), &[expected]);
}

fn default_producer(owner: &str) -> (VbcModule, VbcCodegen) {
    let source=parse(&r#"
module alpha.adapters;
type Unused is { data: Int };
type Payload is { value: Int };
type Wrapped<I,F> is { owner: I, callback: F };
type Transform is protocol {
 type Item;
 fn transform<R,F: fn(Self.Item) -> R>(self,f:F) -> Wrapped<Self,F> { Wrapped { owner:self,callback:f } }
};
type Stage<T> is { value:T };
implement<T> Transform for Stage<T> { type Item=T; }
"#.replace("alpha",owner));
    let mut producer = VbcCodegen::with_config(CodegenConfig::new(owner));
    // Match stdlib_bootstrap: default methods precede the per-file body pass.
    producer
        .collect_unit_declarations(&[&source])
        .expect("collect provider");
    producer.resolve_pending_imports();
    producer
        .compile_pending_default_methods()
        .expect("defaults");
    producer
        .compile_items_into_state(&source)
        .expect("provider items");
    let provider = producer.finalize_module_from_state().expect("provider");
    (provider, producer)
}

#[test]
fn staged_default_method_exports_its_compilation_unit_identity() {
    let (provider, producer) = default_producer("alpha");
    let descriptor = provider
        .functions
        .iter()
        .find(|f| provider.strings.get(f.name) == Some("Stage.transform"))
        .expect("default method");
    assert_eq!(
        descriptor.origin_module, None,
        "test reaches pre-file default pass"
    );
    assert!(
        producer
            .export_functions()
            .contains_key("alpha.Stage.transform")
    );
}

#[test]
fn staged_default_method_remaps_only_its_source_owned_signature_in_both_orders() {
    let (alpha, ac) = default_producer("alpha");
    let (beta, bc) = default_producer("beta");
    let mut ar = ac.export_functions();
    let mut br = bc.export_functions();
    // Bootstrap's global allocator is independent of archive-local dense IDs.
    for info in ar.values_mut().filter(|info| info.id.0 < 100_000) {
        info.id.0 += 1000;
    }
    for info in br.values_mut().filter(|info| info.id.0 < 100_000) {
        info.id.0 += 2000;
    }
    let caller =
        parse("module consumer; mount alpha.adapters.Stage; fn probe(x:Stage<Int>)->Int { 0 }");
    for reverse in [false, true] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for registry in if reverse { [&ar, &br] } else { [&br, &ar] } {
            cg.import_functions(registry);
        }
        let beta_before = cg.ctx_mut().functions["beta.Stage.transform"]
            .return_type
            .clone();
        cg.import_bootstrap_nominal_dependencies(
            &[&caller],
            &if reverse {
                [&alpha, &beta]
            } else {
                [&beta, &alpha]
            },
        )
        .expect("import");
        let alpha_info = cg.ctx_mut().functions["alpha.Stage.transform"].clone();
        assert_eq!(
            cg.ctx_mut().functions["beta.Stage.transform"].return_type,
            beta_before,
            "foreign source-pool metadata must not be rewritten"
        );
        assert!(
            !cg.ctx_mut()
                .archive_fn_param_types
                .contains_key(&br["beta.Stage.transform"].id.0),
            "unselected sibling is not certified in consumer pool"
        );
        let carried = cg.ctx_mut().archive_fn_param_types[&alpha_info.id.0].clone();
        let generic = cg.ctx_mut().archive_fn_parameter_generics[&alpha_info.id.0].clone();
        cg.collect_unit_declarations(&[&caller])
            .expect("declarations");
        let module = cg.compile_function_bodies(&caller).expect("caller");
        let TypeRef::Concrete(stage) = nominal(&module, "Stage") else {
            panic!("stage")
        };
        let TypeRef::Concrete(wrapped) = nominal(&module, "Wrapped") else {
            panic!("wrapped")
        };
        assert_ne!(
            nominal(&alpha, "Stage"),
            nominal(&module, "Stage"),
            "the pin must cross different source and consumer numeric IDs"
        );
        let owner = TypeRef::Instantiated {
            base: stage,
            args: vec![TypeRef::Generic(verum_vbc::types::TypeParamId(0))],
        };
        assert_eq!(
            alpha_info.return_type,
            Some(TypeRef::Instantiated {
                base: wrapped,
                args: vec![
                    owner.clone(),
                    TypeRef::Generic(verum_vbc::types::TypeParamId(2))
                ]
            })
        );
        assert_eq!(carried[0], owner);
        assert!(
            matches!(&carried[1],TypeRef::Function{params,..} if matches!(&params[0],TypeRef::AssociatedProjection{base,..} if **base==owner))
        );
        assert_eq!(generic[1], Some(verum_vbc::types::TypeParamId(2)));
    }
}

fn projected_consumer(
    extra: &str,
    associated: &str,
    target: &str,
    actual: &str,
) -> (VbcModule, VbcCodegen) {
    let source = parse(&format!(
        r#"
module alpha.adapters;
type Payload is {{ value:Int }};
type Envelope<T> is {{ value:T }};
type Stage<T> is {{ value:T }};
type Wrapped<I,F> is {{ owner:I, callback:F }};
type Transform is protocol {{
 type {associated};
 fn transform<R,F:fn(Self.{associated})->R>(self,f:F)->Wrapped<Self,F> {{ Wrapped {{owner:self,callback:f}} }}
}};
implement<T> Transform for Stage<T> {{ type {associated}={target}; }}
implement<I,F> Wrapped<I,F> {{ fn finish<C>(self)->C {{ C.default() }} }}
{extra}
"#
    ));
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("alpha"));
    producer
        .collect_unit_declarations(&[&source])
        .expect("provider declarations");
    producer.resolve_pending_imports();
    producer
        .compile_pending_default_methods()
        .expect("defaults");
    producer
        .compile_items_into_state(&source)
        .expect("provider bodies");
    let provider = producer.finalize_module_from_state().expect("provider");
    let caller = parse(&format!(
        "module consumer; mount alpha.adapters.{{Stage,Payload}}; fn probe(stage:{actual})->Int {{ stage.transform(|x|x).finish<Int>() }}"
    ));
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    cg.import_functions(&producer.export_functions());
    cg.import_protocols(&producer.export_protocols());
    cg.import_bootstrap_nominal_dependencies(&[&caller], &[&provider])
        .expect("nominal boundary");
    cg.collect_unit_declarations(&[&caller])
        .expect("declarations");
    let module = cg.compile_function_bodies(&caller).expect("caller");
    (module, cg)
}
fn callable(module: &VbcModule) -> &verum_vbc::module::FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n.contains("$closure$"))
        })
        .expect("closure")
}
fn finish_witness(module: &VbcModule) -> List<TypeRef> {
    let mut pc = 0;
    while pc < module.bytecode.len() {
        if let verum_vbc::instruction::Instruction::CallG {
            func_id, type_args, ..
        } = verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).expect("decode")
        {
            if module
                .band_reference_name(func_id)
                .is_some_and(|n| n.ends_with("Wrapped.finish"))
            {
                return type_args.into();
            }
        }
    }
    panic!("finish witness")
}
#[test]
fn default_associated_callable_parameter_reaches_the_following_method() {
    let (module, _) = projected_consumer("", "Item", "T", "Stage<Payload>");
    let payload = nominal(&module, "Payload");
    assert_eq!(callable(&module).params[1].type_ref, payload);
    assert_eq!(callable(&module).return_type, payload);
    assert_eq!(
        finish_witness(&module)[1],
        TypeRef::Function {
            params: vec![payload.clone()],
            return_type: Box::new(payload),
            contexts: Default::default()
        }
    );
}
#[test]
fn arbitrary_nested_associated_binding_does_not_depend_on_iterator_names() {
    let (module, _) = projected_consumer("", "Argument", "Envelope<T>", "Stage<Payload>");
    let TypeRef::Concrete(base) = nominal(&module, "Envelope") else {
        panic!("envelope")
    };
    let expected = TypeRef::Instantiated {
        base,
        args: vec![nominal(&module, "Payload")],
    };
    assert_eq!(callable(&module).params[1].type_ref, expected);
    assert_eq!(callable(&module).return_type, expected);
    assert!(
        matches!(&finish_witness(&module)[1],TypeRef::Function{params,..} if params==&[expected])
    );
}
#[test]
fn ambiguous_associated_binding_cannot_certify_a_callable() {
    let extra =
        "type Other is protocol { type Item; }; implement<T> Other for Stage<T> { type Item=Int; }";
    let (module, _) = projected_consumer(extra, "Item", "T", "Stage<Payload>");
    assert!(
        finish_witness(&module)[1].is_generic(),
        "two incompatible Item declarations cannot select first"
    );
}
#[test]
fn malformed_owner_arity_cannot_partially_instantiate_projection() {
    let (module, _) = projected_consumer("", "Item", "T", "Stage<Payload,Int>");
    assert!(finish_witness(&module)[1].is_generic());
}

#[test]
fn recursive_associated_binding_remains_unknown() {
    let (module, _) = projected_consumer("", "Item", "Self.Item", "Stage<Payload>");
    assert!(finish_witness(&module)[1].is_generic());
}
