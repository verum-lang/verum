#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    types::{CbgrTier, Mutability, TypeParamId, TypeRef},
};

const ALLOC: &str = r#"
    type Alloc<const SIZE: Int> is { top: Int };
    implement<const SIZE: Int> Alloc<SIZE> {
        fn new() -> Self { Self { top: 0 } }
        fn capacity(&self) -> Int { SIZE }
    }
"#;

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn declaration<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some(name))
        .unwrap()
}

fn execute(module: VbcModule) -> i64 {
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name == "main" || name.ends_with(".main"))
        })
        .unwrap()
        .id;
    verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .unwrap()
        .try_as_i64()
        .unwrap()
}

#[test]
fn closed_alias_arguments_survive_bindings_and_wire() {
    for value in ["1024", "(512 + 512)", "(-64 * -16)"] {
        for selected in ["Tiny", "Small"] {
            let module = compile(&format!(
                "{ALLOC} type Tiny is Alloc<{value}>; type Small is Tiny; fn main()->Int {{ let a={selected}.new(); a.capacity() }}"
            ));
            for module in [module.clone(), roundtrip(&module)] {
                assert_eq!(declaration(&module, "Alloc").type_params.len(), 1);
                assert_eq!(
                    declaration(&module, "Tiny").alias_target,
                    Some(TypeRef::Instantiated {
                        base: declaration(&module, "Alloc").id,
                        args: vec![TypeRef::ConstValue(1024)],
                    })
                );
                assert_eq!(execute(module), 1024);
            }
        }
    }
}

#[test]
fn borrowed_alias_receivers_retain_const_witnesses() {
    let module = compile(&format!(
        r#"{ALLOC}
        type Tiny is Alloc<1024>;
        type Borrowed is &Tiny;
        fn direct(value: &Tiny) -> Int {{ value.capacity() }}
        fn aliased(value: Borrowed) -> Int {{ value.capacity() }}
        fn main() -> Int {{
            let value = Tiny.new();
            let borrowed = &value;
            direct(&value) + aliased(borrowed)
        }}
    "#
    ));
    for module in [module.clone(), roundtrip(&module)] {
        assert_eq!(execute(module), 2048);
    }
}

#[test]
fn generic_const_aliases_keep_declared_parameters() {
    for parameter in ["const N: Int", "N: meta Int"] {
        let module = compile(&format!(
            "{ALLOC} type Sized<{parameter}> is Alloc<N>; type Tiny is Sized<1024>; fn main()->Int {{ let a=Tiny.new(); a.capacity() }}"
        ));
        for module in [module.clone(), roundtrip(&module)] {
            let alias = declaration(&module, "Sized");
            assert_eq!(alias.type_params.len(), 1);
            assert_eq!(module.get_string(alias.type_params[0].name), Some("N"));
            assert_eq!(
                alias.alias_target,
                Some(TypeRef::Instantiated {
                    base: declaration(&module, "Alloc").id,
                    args: vec![TypeRef::Generic(TypeParamId(0))],
                })
            );
            assert_eq!(execute(module), 1024);
        }
    }
}

#[test]
fn reference_alias_expansion_preserves_access_tiers() {
    let module = compile(&format!(
        r#"{ALLOC}
        type Tiny is Alloc<1024>;
        type Managed is &Tiny;
        type Checked is &checked mut Tiny;
        type Unsafe is &unsafe Tiny;
        fn managed(value: Managed) -> Int {{ value.capacity() }}
        fn inspect_checked(value: Checked) -> Int {{ value.capacity() }}
        fn unchecked(value: Unsafe) -> Int {{ value.capacity() }}
    "#
    ));
    for module in [module.clone(), roundtrip(&module)] {
        for (alias, tier, mutability) in [
            ("Managed", CbgrTier::Tier0, Mutability::Immutable),
            ("Checked", CbgrTier::Tier1, Mutability::Mutable),
            ("Unsafe", CbgrTier::Tier2, Mutability::Immutable),
        ] {
            assert_eq!(
                declaration(&module, alias).alias_target,
                Some(TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(declaration(&module, "Tiny").id)),
                    tier,
                    mutability,
                })
            );
        }
    }
}

#[test]
fn unresolved_and_overflowing_const_arguments_never_become_semantic_slots() {
    for argument in ["(missing + 1)", "(9223372036854775807 + 1)", "(1 / 0)"] {
        let module = compile(&format!("{ALLOC} type Invalid<T> is Alloc<{argument}>;"));
        for module in [module.clone(), roundtrip(&module)] {
            // Generic slot 0 exists, but an unresolved expression does not
            // establish that slot as an alias target's value witness.
            assert_eq!(declaration(&module, "Invalid").type_params.len(), 1);
            assert_eq!(declaration(&module, "Invalid").alias_target, None);
        }
    }
}

#[test]
fn selected_alias_keeps_its_owner_and_const_value() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(
        core.join("provider.vr"),
        r#"
 module core.provider;
 public type Alloc<const SIZE:Int> is {top:Int};
 implement<const SIZE:Int> Alloc<SIZE> {
 public fn new()->Self {Self{top:0}}
 public fn capacity(&self)->Int {SIZE}
 }
 public type Tiny is Alloc<1024>;
 "#,
    )
    .unwrap();
    let source = r#"
 module core.consumer;
 mount core.provider.Tiny as Selected;
 type Alloc<const SIZE:Int> is {wrong:Int};
 implement<const SIZE:Int> Alloc<SIZE> {
 fn new()->Self {Self{wrong:0}}
 fn capacity(&self)->Int {SIZE}
 }
 type Tiny is Alloc<17>;
 fn main()->Int {let selected=Selected.new(); selected.capacity()}
 "#;
    let path = core.join("consumer.vr");
    std::fs::write(&path, source).unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core.consumer"))
        .compile_module_with_mounts(
            &Parser::new(source).parse_module().unwrap(),
            path.to_str().unwrap(),
            core.to_str().unwrap(),
        )
        .unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("core.consumer.main").unwrap();
        assert_eq!(
            verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(1024)
        );
    }
}

#[test]
fn record_sum_and_alias_share_dense_slots_and_declared_constraints() {
    let module = compile(
        r#"
        type Marker is protocol {};
        type Value is { value: Int };
        implement Marker for Value {}
        type Store<'a, T: Marker = Value, const N: Int> is { value: T };
        type Choice<'a, T: Marker = Value, N: meta Int> is End | Item(T);
        type Alias<'a, T: Marker = Value, N: meta Int> is Store<'a, T, N>;
        type Handler<F: fn(Int) -> Int, N: meta Int> is { call: F };
    "#,
    );
    for module in [module.clone(), roundtrip(&module)] {
        let marker = declaration(&module, "Marker").id;
        let value = declaration(&module, "Value").id;
        for name in ["Store", "Choice", "Alias"] {
            let owner = declaration(&module, name);
            assert_eq!(owner.type_params.len(), 2, "{name}");
            assert_eq!(module.get_string(owner.type_params[0].name), Some("T"));
            assert_eq!(module.get_string(owner.type_params[1].name), Some("N"));
            assert_eq!(owner.type_params[0].id, TypeParamId(0));
            assert_eq!(owner.type_params[1].id, TypeParamId(1));
            assert_eq!(owner.type_params[0].default, Some(TypeRef::Concrete(value)));
            assert_eq!(
                owner.type_params[0].bounds.as_slice(),
                &[verum_vbc::types::ProtocolId(marker.0)]
            );
        }
        let handler = declaration(&module, "Handler");
        assert_eq!(handler.type_params.len(), 2);
        assert!(matches!(
            handler.type_params[0].type_bounds.as_slice(),
            [TypeRef::Function { .. }]
        ));
        assert_eq!(
            declaration(&module, "Alias").alias_target,
            Some(TypeRef::Instantiated {
                base: declaration(&module, "Store").id,
                args: vec![
                    TypeRef::Generic(TypeParamId(0)),
                    TypeRef::Generic(TypeParamId(1))
                ],
            })
        );
    }
}
