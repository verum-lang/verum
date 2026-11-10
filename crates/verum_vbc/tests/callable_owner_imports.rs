//! T2310: implementation binders survive declaration interfaces and wire import.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

const TYPE_API: &str = "module api; public type Cell<T> is { public marker:Int }; implement<T> Cell<T> { public fn make()->Int{42} }";
const VALUE_API: &str = "module api; public type Cell<N: meta Int> is { public marker:Int }; implement<N: meta Int> Cell<N> { public fn count()->Int{N} }";
fn import(api: &str, mounted: &str, mutate: impl Fn(&mut VbcModule), allowed: bool) {
    import_with_producer(api, mounted, mutate, allowed, false);
}
fn import_with_producer(
    api: &str,
    mounted: &str,
    mutate: impl Fn(&mut VbcModule),
    allowed: bool,
    compiled: bool,
) {
    let ast = Parser::new(api).parse_module().expect("source grammar");
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("api"));
    let original = if compiled {
        producer.compile_module(&ast)
    } else {
        producer.collect_owned_declaration_interface(&[("api", &ast)])
    }
    .expect("actual source declaration producer");
    let functions = producer.export_functions();
    let mut errors = Vec::new();
    for wire in [false, true] {
        let mut archive = original.clone();
        mutate(&mut archive);
        let result = (|| -> Result<_, String> {
            let archive = if wire {
                deserialize_module(&serialize_module(&archive).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())?
            } else {
                archive
            };
            let source = format!("module consumer; mount {mounted}; public fn probe()->Int{{42}}");
            let consumer = Parser::new(&source).parse_module().unwrap();
            let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
            cg.import_functions(&functions);
            cg.import_owned_bootstrap_nominal_dependencies_with_resolver(
                &[("consumer", &consumer)],
                &[&archive],
                |name| Some(name.into()),
            )
            .and_then(|_| cg.collect_owned_declaration_interface(&[("consumer", &consumer)]))
            .map_err(|error| error.to_string())
        })();
        if allowed {
            if let Err(error) = result {
                errors.push(format!("wire={wire}: {error}"));
            }
        } else {
            match result {
                Ok(_) => errors.push(format!("wire={wire}: missing/ambiguous authority accepted")),
                Err(error) => assert!(
                    format!("{error}").contains("source signature"),
                    "wrong refusal {error}"
                ),
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
#[test]
fn static_type_owner() {
    import(TYPE_API, "api.Cell", |_| (), true)
}
#[test]
fn static_value_owner() {
    import(VALUE_API, "api.Cell", |_| (), true)
}
#[test]
fn ordinary_argument_is_not_a_receiver() {
    import(
        &TYPE_API.replace("make()", "make(value:T)"),
        "api.Cell",
        |_| (),
        true,
    )
}
#[test]
fn receiver_shaped_ordinary_argument_is_static() {
    import(
        &TYPE_API.replace("make()", "make(value:Cell<T>)"),
        "api.Cell",
        |_| (),
        true,
    )
}
#[test]
fn method_local_shadow_keeps_both_slots() {
    import(
        &TYPE_API.replace("make()", "make<T>(value:T)"),
        "api.Cell",
        |_| (),
        true,
    )
}
#[test]
fn renamed_reordered_mixed_implementation_roles() {
    import(
        "module api; public type Cell<T,N:meta Int> is {public marker:Int}; implement<N:meta Int,U> Cell<U,N>{public fn count()->Int{N}}",
        "api.Cell",
        |_| (),
        true,
    )
}
#[test]
fn instance_receiver_roles_remain_declared() {
    import(
        &TYPE_API.replace("make()", "make(&self)"),
        "api.Cell",
        |_| (),
        true,
    )
}
#[test]
fn default_instance_method_keeps_its_source_receiver() {
    import(
        "module api; public type Measure is protocol {fn get(&self)->Int{42}}; public type Cell<T> is {public marker:Int}; implement<T> Measure for Cell<T>{}",
        "api.Cell",
        |_| (),
        true,
    )
}
// The MappedIter declaration and this implementation signature are the actual
// core/base/iterator.vr:24,1498 case; surrounding protocol declarations supply
// a minimal source interface, not a claim about the full stdlib producer.
const MAPPED: &str = "module api; public type Iterator is protocol {type Item;}; public type ExactSizeIterator is protocol extends Iterator {fn len(&self)->Int;}; public type MappedIter<I,F> is {iter:I,f:F}; implement<I:ExactSizeIterator,B,F:fn(I.Item)->B> ExactSizeIterator for MappedIter<I,F>{fn len(&self)->Int{self.iter.len()}}";
#[test]
fn implementation_bound_owns_a_slot_absent_from_receiver() {
    import(MAPPED, "api.MappedIter", |_| (), true)
}
fn erase_signature(module: &mut VbcModule) {
    for f in &mut module.functions {
        if !f.type_params.is_empty() {
            f.source_signature = None;
        }
    }
}
#[test]
fn absent_static_signature_refuses() {
    import(TYPE_API, "api.Cell", erase_signature, false)
}
#[test]
fn absent_instance_signature_refuses() {
    import(
        &TYPE_API.replace("make()", "make(&self)"),
        "api.Cell",
        erase_signature,
        false,
    )
}
#[test]
fn erased_static_signature_cannot_borrow_an_argument_as_receiver() {
    import(
        &TYPE_API.replace("make()", "make(value:Cell<T>)"),
        "api.Cell",
        erase_signature,
        false,
    )
}
#[test]
fn absent_static_implementation_refuses() {
    import(
        TYPE_API,
        "api.Cell",
        |m| {
            for t in &mut m.types {
                t.source_inherent_implementations.clear();
            }
        },
        false,
    )
}
#[test]
fn absent_extra_slot_implementation_refuses() {
    import(
        MAPPED,
        "api.MappedIter",
        |m| {
            for t in &mut m.types {
                for p in &mut t.protocols {
                    p.source_implementation = None;
                }
            }
        },
        false,
    )
}
#[test]
fn declaration_and_executable_rows_share_one_generic_parameter_owner() {
    const SOURCE: &str = r#"module api;
        public type Convert<T> is protocol { fn make(v:T) -> Int; };
        public type Cell<T> is { marker:Int };
        implement<T> Convert<T> for Cell<T> { fn make(value:T) -> Int { 42 } }
    "#;
    import(SOURCE, "api.Cell", |module| {
        let parent = module.types.iter_mut().find(|ty| !ty.protocols.is_empty()).unwrap();
        let mut declaration = parent.protocols[0].clone();
        declaration.methods.clear();
        parent.protocols.push(declaration);
    }, true);
}

#[test]
fn ambiguous_static_implementation_refuses() {
    import(
        TYPE_API,
        "api.Cell",
        |m| {
            for t in &mut m.types {
                if let Some(first) = t.source_inherent_implementations.first() {
                    let owner = first.origin_module().into();
                    let mut decl = first.declaration().unwrap();
                    decl.is_unsafe = !decl.is_unsafe;
                    t.source_inherent_implementations.push(
                        verum_ast::source_implementation::AssociatedImplementationSource::new(
                            owner, &decl,
                        )
                        .unwrap(),
                    );
                    break;
                }
            }
        },
        false,
    )
}
#[test]
fn foreign_implementation_owner_refuses() {
    import(
        TYPE_API,
        "api.Cell",
        |m| {
            for t in &mut m.types {
                for c in &mut t.source_inherent_implementations {
                    *c = verum_ast::source_implementation::AssociatedImplementationSource::new(
                        "foreign".into(),
                        &c.declaration().unwrap(),
                    )
                    .unwrap();
                }
            }
        },
        false,
    )
}
#[test]
fn altered_selected_signature_refuses() {
    import(
        TYPE_API,
        "api.Cell",
        |m| {
            for f in &mut m.functions {
                if let Some(source) = &mut f.source_signature {
                    source.name.name = "different".into();
                }
            }
        },
        false,
    )
}

#[test]
fn erased_receiver_carrier_cannot_be_completed_from_implementation() {
    import(
        &TYPE_API.replace("make()", "make(&self)"),
        "api.Cell",
        |m| {
            for f in &mut m.functions {
                if !f.type_params.is_empty() {
                    f.semantic_params = Some(
                        [Some(verum_vbc::types::TypeRef::Concrete(
                            verum_vbc::types::TypeId::UNIT,
                        ))]
                        .into_iter()
                        .collect(),
                    );
                }
            }
        },
        false,
    )
}
#[test]
fn malformed_owner_slot_refuses() {
    import(
        TYPE_API,
        "api.Cell",
        |m| {
            for f in &mut m.functions {
                if let Some(slot) = f.type_params.first_mut() {
                    slot.id = verum_vbc::types::TypeParamId(17);
                }
            }
        },
        false,
    )
}

#[test]
fn receiver_removed_from_selected_signature_refuses() {
    import(
        &TYPE_API.replace("make()", "make(&self)"),
        "api.Cell",
        |module| {
            for function in &mut module.functions {
                if let Some(source) = &mut function.source_signature {
                    let donor = Parser::new("fn replacement(value: &Cell<T>)->Int{42}")
                        .parse_module()
                        .unwrap();
                    let verum_ast::ItemKind::Function(replacement) = &donor.items[0].kind else {
                        panic!("function source");
                    };
                    source.params[0] = replacement.params[0].clone();
                }
            }
        },
        false,
    );
}
#[test]
fn compiled_static_type_source_retains_owner_authority() {
    import_with_producer(TYPE_API, "api.Cell", |_| (), true, true);
}
#[test]
fn compiled_static_value_source_retains_owner_authority() {
    import_with_producer(VALUE_API, "api.Cell", |_| (), true, true);
}
#[test]
fn compiled_instance_source_retains_receiver_authority() {
    import_with_producer(
        &TYPE_API.replace("make()", "make(&self)"),
        "api.Cell",
        |_| (),
        true,
        true,
    );
}
