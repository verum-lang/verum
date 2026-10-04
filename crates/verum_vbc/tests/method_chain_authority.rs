#![cfg(feature = "codegen")]

use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::{FunctionDescriptor, VbcModule};
use verum_vbc::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};

const ENV: &str = r#"
module chain;
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Guard<T> is { value: T };
implement<T> Deref for Guard<T> { type Target = T; fn deref(&self) -> &T { &self.value } }
type Source<T> is { value: T };
type Items<T> is { value: T };
type Mapped<I, F> is { iter: I, f: F };
implement<T> Source<T> { fn iter(&self) -> Items<T> { Items { value: self.value } } }
implement<T> Items<T> { fn map<F>(self, f: F) -> Mapped<Self, F> { Mapped { iter: self, f: f } } }
implement<I, F> Mapped<I, F> { fn collect<C>(self) -> C { C.from_iter(self) } }
"#;

fn compile(source: &str) -> (VbcCodegen, VbcModule) {
    let ast = Parser::new(source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("chain"));
    let module = codegen.compile_module(&ast).expect("compile");
    (codegen, module)
}

fn function<'a>(module: &'a VbcModule, name: &str) -> &'a FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
        })
        .expect(name)
}

fn type_id(module: &VbcModule, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|t| {
            module
                .get_string(t.name)
                .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
        })
        .expect(name)
        .id
}

fn calls(module: &VbcModule, name: &str) -> List<(Text, List<TypeRef>)> {
    let f = function(module, name);
    let mut pc = f.bytecode_offset as usize;
    let end = pc + f.bytecode_length as usize;
    let mut result = List::new();
    while pc < end {
        let (name, args): (&str, List<TypeRef>) =
            match decode_instruction(&module.bytecode, &mut pc).expect("decode") {
                Instruction::CallG {
                    func_id, type_args, ..
                } => (
                    module
                        .get_string(module.functions[func_id as usize].name)
                        .unwrap(),
                    type_args.into_iter().collect(),
                ),
                Instruction::Call { func_id, .. } => (
                    module
                        .get_string(module.functions[func_id as usize].name)
                        .unwrap(),
                    List::new(),
                ),
                Instruction::CallM { method_id, .. } => (
                    module
                        .get_string(verum_vbc::types::StringId(method_id))
                        .unwrap(),
                    List::new(),
                ),
                _ => continue,
            };
        result.push((Text::from(name), args));
    }
    result
}

#[test]
fn implicit_deref_chain_preserves_the_same_method_owners_as_explicit_deref() {
    let (_, module) = compile(&format!(
        "{ENV}\n{}",
        r#"
fn direct(g: Source<Int>) -> Int { g.iter().map(|x| x).collect<Int>() }
fn implicit(g: Guard<Source<Int>>) -> Int { g.iter().map(|x| x).collect<Int>() }
fn explicit(g: Guard<Source<Int>>) -> Int { (*g).iter().map(|x| x).collect<Int>() }
"#
    ));
    let implicit = calls(&module, "implicit");
    let explicit = calls(&module, "explicit");
    assert_eq!(implicit, explicit);
    assert!(
        implicit.iter().any(|(name, _)| name.ends_with("Items.map")),
        "{implicit:?}"
    );
    let direct = calls(&module, "direct");
    assert_eq!(
        implicit
            .iter()
            .filter(|(name, _)| !name.ends_with("Guard.deref"))
            .cloned()
            .collect::<List<_>>(),
        direct
    );
}

#[test]
fn chain_receiver_instantiation_reaches_the_following_method_witness() {
    let (_, module) = compile(&format!(
        "{ENV}\nfn probe(g: Guard<Source<Int>>) -> Int {{ g.iter().map(|x| x).collect<Int>() }}"
    ));
    let calls = calls(&module, "probe");
    let args = &calls
        .iter()
        .find(|(name, _)| name.ends_with("Mapped.collect"))
        .expect("collect")
        .1;
    assert_eq!(
        args.first(),
        Some(&TypeRef::Instantiated {
            base: type_id(&module, "Items"),
            args: vec![TypeRef::Concrete(TypeId::INT)]
        }),
        "{calls:?}"
    );
}

#[test]
fn nested_self_registration_keeps_the_owners_generic_arguments() {
    let (codegen, _) = compile(ENV);
    let functions = codegen.export_functions();
    let info = functions.get("Items.map").expect("map");
    assert_eq!(
        info.return_type_name.as_deref(),
        Some("Mapped<Items<T>, F>")
    );
}

#[test]
fn nested_self_descriptor_keeps_the_same_owner_and_parameter_identity() {
    let (_, module) = compile(ENV);
    let map = function(&module, "Items.map");
    let owner = TypeRef::Instantiated {
        base: type_id(&module, "Items"),
        args: vec![TypeRef::Generic(TypeParamId(0))],
    };
    assert_eq!(map.params[0].type_ref, owner);
    assert_eq!(
        map.return_type,
        TypeRef::Instantiated {
            base: type_id(&module, "Mapped"),
            args: vec![owner, TypeRef::Generic(TypeParamId(1))]
        }
    );
}

#[test]
fn self_receiver_descriptors_preserve_reference_tiers_and_generic_owner() {
    let (_, module) = compile(
        r#"
type Owner<T> is { value: T };
implement<T> Owner<T> {
    fn borrowed(&self) -> Int { 1 }
    fn checked_ref(&checked self) -> Int { 1 }
    fn unsafe_ref(&unsafe self) -> Int { 1 }
    fn mutable(&mut self) -> Int { 1 }
    fn identity(self) -> Self { self }
}
"#,
    );
    let owner = TypeRef::Instantiated {
        base: type_id(&module, "Owner"),
        args: vec![TypeRef::Generic(TypeParamId(0))],
    };
    for (name, tier, mutability) in [
        ("borrowed", CbgrTier::Tier0, Mutability::Immutable),
        ("checked_ref", CbgrTier::Tier1, Mutability::Immutable),
        ("unsafe_ref", CbgrTier::Tier2, Mutability::Immutable),
        ("mutable", CbgrTier::Tier0, Mutability::Mutable),
    ] {
        assert_eq!(
            function(&module, &format!("Owner.{name}")).params[0].type_ref,
            TypeRef::Reference {
                inner: Box::new(owner.clone()),
                tier,
                mutability
            },
            "{name}"
        );
    }
    assert_eq!(function(&module, "Owner.identity").return_type, owner);
}

#[test]
fn wrapper_own_method_wins_over_deref_target_in_chains() {
    let (_, module) = compile(&format!(
        "{ENV}\n{}",
        r#"
type OwnItems is { value: Int };
implement OwnItems { fn map(self) -> Int { self.value } }
implement<T> Guard<T> { fn iter(&self) -> OwnItems { OwnItems { value: 9 } } }
fn probe(g: Guard<Source<Int>>) -> Int { g.iter().map() }
"#
    ));
    let calls = calls(&module, "probe");
    assert!(
        calls.iter().any(|(name, _)| name.ends_with("Guard.iter")),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(|(name, _)| name.ends_with("OwnItems.map")),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(name, _)| name.ends_with(".deref") || name.ends_with("Source.iter")),
        "{calls:?}"
    );
}

#[test]
fn materialized_protocol_default_carries_instantiated_self_through_archive() {
    let (codegen, module) = compile(
        r#"
type Mapped<I, F> is { iter: I, f: F };
type Transform is protocol {
    fn map<F>(self, f: F) -> Mapped<Self, F> { Mapped { iter: self, f: f } }
};
type Items<T> is { value: T };
implement<T> Transform for Items<T> { }
"#,
    );
    let map = function(&module, "Items.map");
    let owner = TypeRef::Instantiated {
        base: type_id(&module, "Items"),
        args: vec![TypeRef::Generic(TypeParamId(0))],
    };
    assert_eq!(map.params[0].type_ref, owner);
    assert_eq!(
        map.return_type,
        TypeRef::Instantiated {
            base: type_id(&module, "Mapped"),
            args: vec![owner, TypeRef::Generic(TypeParamId(1))]
        }
    );
    assert_eq!(
        codegen.export_functions()["Items.map"]
            .return_type_name
            .as_deref(),
        Some("Mapped<Items<T>, F>")
    );
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize");
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).expect("deserialize");
    assert_eq!(
        function(&decoded, "Items.map").params[0].type_ref,
        map.params[0].type_ref
    );
    assert_eq!(function(&decoded, "Items.map").return_type, map.return_type);
}

#[test]
fn generic_self_body_is_forwarded_as_the_declared_owner_instantiation() {
    let (_, module) = compile(
        r#"
fn consume<I>(value: I) -> Int { 1 }
type Owner<T> is { value: T };
implement<T> Owner<T> { fn forward(self) -> Int { consume(self) } }
"#,
    );
    let calls = calls(&module, "Owner.forward");
    let args = &calls
        .iter()
        .find(|(name, _)| name.ends_with("consume"))
        .expect("consume")
        .1;
    assert_eq!(
        args.first(),
        Some(&TypeRef::Instantiated {
            base: type_id(&module, "Owner"),
            args: vec![TypeRef::Generic(TypeParamId(0))]
        }),
        "{calls:?}"
    );
}

#[test]
fn self_in_nested_return_uses_impl_scope_when_method_parameter_shadows_it() {
    let (_, module) = compile(
        r#"
type Pair<A, B> is { left: A, right: B };
type Owner<T> is { value: T };
implement<T> Owner<T> {
    fn pair<T>(self, other: T) -> Pair<Self, T> { Pair { left: self, right: other } }
}
"#,
    );
    let pair = function(&module, "Owner.pair");
    let owner = TypeRef::Instantiated {
        base: type_id(&module, "Owner"),
        args: vec![TypeRef::Generic(TypeParamId(0))],
    };
    assert_eq!(pair.params[0].type_ref, owner);
    assert_eq!(
        pair.return_type,
        TypeRef::Instantiated {
            base: type_id(&module, "Pair"),
            args: vec![owner, TypeRef::Generic(TypeParamId(0x8000))]
        }
    );
}

#[test]
fn explicit_constructor_instantiation_survives_a_method_chain() {
    let (_, module) = compile(
        r#"
type Factory<T> is { value: T };
implement<T> Factory<T> {
    fn new(value: T) -> Self { Self { value } }
    fn copy(self) -> Self { self }
    fn read(self) -> T { self.value }
}
fn probe() -> Int { Factory<Int>.new(7).copy().read() }
"#,
    );
    let calls = calls(&module, "probe");
    let args = &calls
        .iter()
        .find(|(name, _)| name.ends_with("Factory.read"))
        .expect("read")
        .1;
    assert_eq!(
        args.first(),
        Some(&TypeRef::Concrete(TypeId::INT)),
        "{calls:?}"
    );
}
