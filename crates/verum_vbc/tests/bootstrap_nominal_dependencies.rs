#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeRef};

fn source(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().expect("parse source")
}
fn producer(name: &str, text: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new(name))
        .compile_module(&source(text))
        .expect("producer")
}
fn import(text: &str, available: &[&VbcModule]) -> VbcModule {
    compile_boundary(text, available, true)
}
fn compile_boundary(text: &str, available: &[&VbcModule], carry_nominals: bool) -> VbcModule {
    let ast = source(text);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..400 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("foreign_pool_{i}"));
    }
    if carry_nominals {
        codegen
            .import_bootstrap_nominal_dependencies(&[&ast], available)
            .expect("import dependencies");
    }
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("collect source");
    codegen
        .compile_function_bodies(&ast)
        .expect("compile consumer")
}
fn descriptor<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some(name))
        .expect(name)
}
fn named_id(module: &VbcModule, name: &str) -> TypeId {
    descriptor(module, name).id
}

#[test]
fn relative_mounts_preserve_the_source_nominal_owner() {
    let owner = producer("core.handles", "module core.handles; public type Handle is { raw: Int };");
    let stranger = producer("unrelated", "module unrelated; public type Handle is { raw: Bool };");
    for (scope, mount) in [
        ("core.provider.file", "core.handles.Handle"),
        ("core.provider.file", "super.super.handles.Handle"),
        ("core.provider.file", "cog.handles.Handle"),
        ("core", ".handles.Handle"),
    ] {
        let ast = source(&format!(
            "module {scope}; mount {mount} as Owned; fn accept(x: Owned) -> Int {{ 7 }}"
        ));
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new(scope));
        codegen.import_bootstrap_nominal_dependencies(&[&ast], &[&stranger, &owner]).unwrap();
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        let module = codegen.compile_function_bodies(&ast).unwrap();
        let handle = descriptor(&module, "Handle");
        assert_eq!(module.strings.get(handle.origin_module.unwrap()), Some("core.handles"), "{mount}");
        let function = module.functions.iter().find(|f|
            module.strings.get(f.name).is_some_and(|name| name.ends_with(".accept"))).unwrap();
        assert_eq!(function.params[0].type_ref,
            TypeRef::Concrete(handle.id), "{mount}");
    }
}

#[test]
fn wildcard_mounts_preserve_exact_public_type_authority() {
    let owner = producer("core.handles", "module core.handles; public type Handle is { raw: Int };");
    let stranger = producer("unrelated", "module unrelated; public type Handle is { raw: Bool };");
    for mount in ["core.handles.*", "super.super.handles.*", "cog.handles.*"] {
        let module = import(&format!("module core.provider.file; mount {mount}; fn accept(x: Handle) -> Int {{ 7 }}"), &[&stranger, &owner]);
        let function = module.functions.iter().find(|f| module.strings.get(f.name).is_some_and(|n| n.ends_with(".accept"))).unwrap();
        let ty = function.semantic_parameter_types().unwrap()[0].as_ref().unwrap_or_else(|| panic!("{mount}: no declaration"));
        let declaration = module.get_type(ty.base_type_id().unwrap()).unwrap();
        assert_eq!(module.strings.get(declaration.origin_module.unwrap()), Some("core.handles"));
    }
}

#[test]
fn wildcard_ambiguity_and_private_names_never_become_ownership_evidence() {
    let left = producer("left", "module left; public type Handle is { raw: Int }; type Hidden is { raw: Int };");
    let right = producer("right", "module right; public type Handle is { raw: Bool };");
    for (mount, ty) in [("mount left.*; mount right.*;", "Handle"), ("mount left.*;", "Hidden")] {
        let module = import(&format!("module consumer; {mount} fn accept(x: {ty}) -> Int {{ 7 }}"), &[&left, &right]);
        let function = module.functions.iter().find(|f| module.strings.get(f.name).is_some_and(|n| n.ends_with(".accept"))).unwrap();
        assert!(function.semantic_parameter_types().unwrap()[0].is_none(), "{mount} {ty}");
    }
}

#[test]
fn explicit_mount_and_local_declaration_override_wildcard_ambiguity() {
    let left = producer("left", "module left; public type Handle is { raw: Int };");
    let right = producer("right", "module right; public type Handle is { raw: Bool };");
    for (declaration, owner) in [("mount left.Handle;", "left"), ("type Handle is { raw: Text };", "consumer")] {
        let module = import(&format!("module consumer; mount left.*; mount right.*; {declaration} fn accept(x: Handle) -> Int {{ 7 }}"), &[&left, &right]);
        let function = module.functions.iter().find(|f| module.strings.get(f.name).is_some_and(|n| n.ends_with(".accept"))).unwrap();
        let ty = function.semantic_parameter_types().unwrap()[0].as_ref().unwrap();
        let declaration = module.get_type(ty.base_type_id().unwrap()).unwrap();
        assert_eq!(module.strings.get(declaration.origin_module.unwrap()), Some(owner));
    }
}

#[test]
fn nested_nominal_function_signature_survives_bootstrap_import() {
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type Batch<T> is { head: T }; type Unused is { x: Int };",
    );
    let text = "module consumer; mount alpha.{Handle, Batch}; type Local is { value: Int }; fn accept(x: Batch<Handle<Local>>) -> Int { 7 }";
    // The previous bootstrap boundary carried layouts and names only. The
    // exact same public source then records PTR in both nominal positions.
    let before = compile_boundary(text, &[&dependency], false);
    let before_function = before
        .functions
        .iter()
        .find(|f| {
            before
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("accept"))
        })
        .unwrap();
    assert!(matches!(&before_function.params[0].type_ref,
        TypeRef::Instantiated { base, args } if *base == TypeId::PTR
            && matches!(&args[0], TypeRef::Instantiated { base, .. } if *base == TypeId::PTR)));
    let result = import(text, &[&dependency]);
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
        })
        .unwrap();
    assert_eq!(
        function.params[0].type_ref,
        TypeRef::Instantiated {
            base: named_id(&result, "Batch"),
            args: vec![TypeRef::Instantiated {
                base: named_id(&result, "Handle"),
                args: vec![TypeRef::Concrete(named_id(&result, "Local"))]
            }]
        }
    );
    assert!(
        !result
            .types
            .iter()
            .any(|ty| result.strings.get(ty.name) == Some("Unused"))
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Handle").fields[0].name),
        Some("item")
    );
}

#[test]
fn structural_dependency_closure_uses_source_ids_and_string_pool() {
    let dependency = producer(
        "alpha",
        "module alpha; type Leaf is { value: Int }; type Wrapper is { nested: Leaf }; type Other is { x: Int, y: Int };",
    );
    let result = import(
        "module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }",
        &[&dependency],
    );
    assert_eq!(
        descriptor(&result, "Wrapper").fields[0].type_ref,
        TypeRef::Concrete(named_id(&result, "Leaf"))
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Wrapper").fields[0].type_name),
        Some("Leaf")
    );
    assert_eq!(
        result
            .strings
            .get(descriptor(&result, "Leaf").fields[0].name),
        Some("value")
    );
}

#[test]
fn distinct_qualified_dependencies_remain_distinct_in_both_orders() {
    let alpha = producer("alpha", "module alpha; type Envelope is { left: Int };");
    let beta = producer(
        "beta",
        "module beta; type Envelope is { right: Text, extra: Int };",
    );
    for available in [&[&alpha, &beta][..], &[&beta, &alpha][..]] {
        let result = import(
            "module consumer; fn accept(a: alpha.Envelope, b: beta.Envelope) -> Int { 7 }",
            available,
        );
        let function = result
            .functions
            .iter()
            .find(|f| {
                result
                    .strings
                    .get(f.name)
                    .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
            })
            .unwrap();
        assert_ne!(function.params[0].type_ref, function.params[1].type_ref);
        let count = result
            .types
            .iter()
            .filter(|ty| {
                result
                    .strings
                    .get(ty.name)
                    .is_some_and(|name| name.ends_with("Envelope"))
            })
            .count();
        assert_eq!(count, 2);
    }
}

#[test]
fn declared_generic_shadows_a_foreign_nominal_with_the_same_name() {
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type T is { value: Int };",
    );
    let result = import(
        "module consumer; mount alpha.Handle; fn accept<T>(x: Handle<T>) -> T { x.item }",
        &[&dependency],
    );
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name == "accept" || name.ends_with(".accept"))
        })
        .unwrap();
    assert_eq!(
        function.params[0].type_ref,
        TypeRef::Instantiated {
            base: named_id(&result, "Handle"),
            args: vec![TypeRef::Generic(verum_vbc::types::TypeParamId(0))]
        }
    );
}

#[test]
fn missing_user_id_is_rejected_instead_of_reused_in_consumer_pool() {
    let mut dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let ty = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    ty.fields[0].type_ref = TypeRef::Concrete(TypeId(87654));
    let ast =
        source("module consumer; mount alpha.Wrapper; fn accept(value: Wrapper) -> Int { 1 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let error = codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .expect_err("unknown id");
    assert!(
        error.to_string().contains("unknown source TypeId 87654"),
        "{error}"
    );
}

#[test]
fn source_function_ids_are_never_copied_as_consumer_function_ids() {
    let mut dependency = producer(
        "alpha",
        "module alpha; type Wrapper is { value: Int }; fn foreign() -> Int { 1 }",
    );
    let foreign = dependency
        .functions
        .iter()
        .find(|f| {
            dependency
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("foreign"))
        })
        .unwrap()
        .id
        .0;
    let wrapper = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    wrapper.drop_fn = Some(foreign);
    wrapper.clone_fn = Some(foreign);
    let ast = source("module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    // Same raw number and bare name deliberately belong to a different owner.
    codegen.ctx_mut().register_function(
        "foreign".to_owned(),
        verum_vbc::codegen::context::FunctionInfo {
            callable_signature: None,
            id: verum_vbc::module::FunctionId(foreign),
            ..Default::default()
        },
    );
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let result = codegen.compile_function_bodies(&ast).unwrap();
    assert_eq!(descriptor(&result, "Wrapper").drop_fn, None);
    assert_eq!(descriptor(&result, "Wrapper").clone_fn, None);
}

#[test]
fn dependency_closure_keeps_methods_from_duplicate_declaration_carriers() {
    let mut dependency = producer("alpha", r#"
        module alpha;
        type Owner is { value: Int };
        type First is { value: Int };
        type Second is { value: Int };
        type Stranger is { value: Int };
        implement Owner {
            fn first(&self) -> First { First { value: self.value } }
            fn second(&self) -> Second { Second { value: self.value } }
        }
        implement Stranger { fn unrelated(&self) -> Int { self.value } }
    "#);
    let mut imported = descriptor(&dependency, "Owner").clone();
    imported.id = TypeId(70000);
    imported.name = dependency.strings.intern("alpha.Owner");
    for function in &mut dependency.functions {
        match dependency.strings.get(function.name) {
            Some(name) if name.ends_with("Owner.second") => {
                function.parent_type = Some(imported.id);
            }
            Some(name) if name.ends_with("Stranger.unrelated") => {
                // Unselected methods must not expand the dependency closure.
                function.return_type = TypeRef::Concrete(TypeId(87654));
            }
            _ => {}
        }
    }
    for imported_first in [false, true] {
        let mut dependency = dependency.clone();
        if imported_first { dependency.types.insert(0, imported.clone()); }
        else { dependency.types.push(imported.clone()); }
        let result = import(
            "module consumer; mount alpha.Owner; fn accept(x: Owner) -> Int { 7 }",
            &[&dependency],
        );
        descriptor(&result, "First");
        descriptor(&result, "Second");
        assert!(!result.types.iter().any(|ty| result.strings.get(ty.name) == Some("Stranger")));
    }
}

#[test]
fn method_signature_closure_and_registry_return_use_consumer_type_ids() {
    let dependency = producer(
        "alpha",
        "module alpha; type Hidden is { value: Int }; type Wrapper is { value: Int }; implement Wrapper { fn reveal(&self) -> Hidden { Hidden { value: self.value } } }",
    );
    let ast = source("module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    let original = dependency
        .functions
        .iter()
        .find(|f| {
            dependency
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("Wrapper.reveal"))
        })
        .unwrap();
    codegen.ctx_mut().register_function(
        "alpha.Wrapper.reveal".to_owned(),
        verum_vbc::codegen::context::FunctionInfo {
            callable_signature: None,
            id: verum_vbc::module::FunctionId(9123),
            return_type: Some(original.return_type.clone()),
            ..Default::default()
        },
    );
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&dependency])
        .unwrap();
    let carried = codegen.ctx_mut().functions["alpha.Wrapper.reveal"]
        .return_type
        .clone()
        .unwrap();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let result = codegen.compile_function_bodies(&ast).unwrap();
    assert_eq!(carried, TypeRef::Concrete(named_id(&result, "Hidden")));
}

#[test]
fn missing_explicit_mount_does_not_import_a_same_leaf_ancestor() {
    let dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let result = import(
        "module consumer; mount alpha.child.Wrapper; fn accept(x: Wrapper) -> Int { 7 }",
        &[&dependency],
    );
    assert!(
        !result
            .types
            .iter()
            .any(|ty| result.strings.get(ty.name) == Some("Wrapper"))
    );
}

#[test]
fn mount_aliases_preserve_both_qualified_nominal_owners() {
    let alpha = producer("alpha", "module alpha; type Envelope is { left: Int };");
    let beta = producer("beta", "module beta; type Envelope is { right: Text };");
    let result = import(
        "module consumer; mount alpha.Envelope as A; mount beta.Envelope as B; fn accept(a: A, b: B) -> Int { 7 }",
        &[&alpha, &beta],
    );
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("accept"))
        })
        .unwrap();
    assert_ne!(function.params[0].type_ref, function.params[1].type_ref);
    assert!(!matches!(
        function.params[0].type_ref,
        TypeRef::Concrete(TypeId::PTR)
    ));
    assert!(!matches!(
        function.params[1].type_ref,
        TypeRef::Concrete(TypeId::PTR)
    ));
}

#[test]
fn promoted_names_and_originless_descriptors_keep_identity_on_reimport() {
    let mut dependency = producer("alpha", "module alpha; type Wrapper is { value: Int };");
    let promoted = dependency.strings.intern("alpha.Wrapper");
    let ty = dependency
        .types
        .iter_mut()
        .find(|ty| dependency.strings.get(ty.name) == Some("Wrapper"))
        .unwrap();
    ty.name = promoted;
    ty.origin_module = None;
    let source = "module consumer; mount alpha.Wrapper; fn accept(x: Wrapper) -> Int { 7 }";
    let first = import(source, &[&dependency]);
    let second = import(source, &[&first]);
    let wrapper = descriptor(&second, "Wrapper");
    assert_eq!(
        wrapper.origin_module.and_then(|id| second.strings.get(id)),
        Some("alpha")
    );
    let function = second
        .functions
        .iter()
        .find(|f| {
            second
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("accept"))
        })
        .unwrap();
    assert_eq!(function.params[0].type_ref, TypeRef::Concrete(wrapper.id));
}

#[test]
fn explicit_nested_hole_binds_the_imported_nominal_in_the_call_witness() {
    use verum_vbc::{bytecode::decode_instruction, instruction::Instruction};
    let dependency = producer(
        "alpha",
        "module alpha; type Handle<T> is { item: T }; type Batch<T> is { head: T };",
    );
    let text = r#"
module consumer;
mount alpha.{Handle, Batch};
type Local is { value: Int };
type Factory is { marker: Int };
implement Factory { fn make<C>(self) -> C { C.default() } }
fn probe(factory: Factory) -> Int {
    let result: Batch<Handle<Local>> = factory.make<Batch<_>>();
    7
}
"#;
    let result = import(text, &[&dependency]);
    let function = result
        .functions
        .iter()
        .find(|f| {
            result
                .strings
                .get(f.name)
                .is_some_and(|name| name.ends_with("probe"))
        })
        .unwrap();
    let mut pc = function.bytecode_offset as usize;
    let end = pc + function.bytecode_length as usize;
    let expected = TypeRef::Instantiated {
        base: named_id(&result, "Batch"),
        args: vec![TypeRef::Instantiated {
            base: named_id(&result, "Handle"),
            args: vec![TypeRef::Concrete(named_id(&result, "Local"))],
        }],
    };
    let mut witnesses = Vec::new();
    while pc < end {
        if let Instruction::CallG { type_args, .. } =
            decode_instruction(&result.bytecode, &mut pc).unwrap()
        {
            witnesses.push(type_args);
        }
    }
    assert_eq!(witnesses, vec![vec![expected]]);
}

fn bundled_sum_patterns(
    pattern: &str,
    local_first: bool,
) -> verum_vbc::codegen::CodegenResult<VbcModule> {
    let provider = producer("crypto", "module crypto; public type Algorithm is Alpha | Extra | Shared;");
    let local = source(r#"
        module bundle.internal_alg;
        type Algorithm is Shared | Alpha | SiblingOnly;
        fn local_tag(a: Algorithm) -> Int { match a { Algorithm.Shared => 7, _ => 0 } }
    "#);
    let consumer = source(&format!(
        "module bundle.keys;
         mount crypto.{{Algorithm}};
         fn imported_tag(a: &Algorithm) -> Int {{ match a {{ Algorithm.{pattern} => 9, _ => 0 }} }}"
    ));
    let files = if local_first { [&local, &consumer] } else { [&consumer, &local] };
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen.import_bootstrap_nominal_dependencies(&files, &[&provider])?;
    codegen.collect_unit_declarations(&files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, verum_vbc::codegen::ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn sibling_sum_does_not_shadow_a_files_explicit_mount() {
    use verum_vbc::{Instruction, bytecode::decode_instructions};
    for local_first in [false, true] {
        for (variant, expected) in [("Shared", 2), ("Extra", 1)] {
            let module = bundled_sum_patterns(variant, local_first).unwrap();
            let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
            let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
            for (name, tag) in [
                ("bundle.keys.imported_tag", expected),
                ("bundle.internal_alg.local_tag", 0),
            ] {
                let id = module.find_function_by_name(name).unwrap();
                let function = module.get_function(id).unwrap();
                let start = function.bytecode_offset as usize;
                let end = start + function.bytecode_length as usize;
                let ops = decode_instructions(&module.bytecode[start..end]).unwrap();
                assert!(ops.iter().any(|op| matches!(op, Instruction::IsVar { tag: actual, .. } if *actual == tag)),
                    "{name}: expected tag {tag}, got {ops:?}");
            }
        }
    }
    assert!(bundled_sum_patterns("SiblingOnly", true).is_err());
}

fn awaited_bound_pattern(body: &str) -> verum_vbc::codegen::CodegenResult<VbcModule> {
    let sums = producer("package", "
        module package.sums;
        public type First<T> is Empty | Good(T);
        public type Second<T> is Good(T) | Empty;
        public type RemoteValue is Empty | Ready(Int);
        public type Remote is protocol { async fn fetch(&self) -> RemoteValue; };
    ");
    let ast = source(&format!(
        "module service.adapter;
         mount package.sums.{{First, Second, Remote}};
         type Endpoint is protocol {{ async fn invoke(&self) -> First<Int>; }};
         type Other<T> is protocol {{ async fn invoke(&self) -> Second<T>; }};
         type Adapter<R> is {{ inner: R }};
         {body}"
    ));
    let files = [&ast];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("service"));
    codegen.import_bootstrap_nominal_dependencies(&files, &[&sums])?;
    codegen.collect_unit_declarations(&files)?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&files, verum_vbc::codegen::ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}

#[test]
fn awaited_protocol_method_uses_the_receiver_binders_declared_result() {
    use verum_vbc::{Instruction, bytecode::decode_instructions};
    let module = awaited_bound_pattern("
        type RemoteValue is Ready(Int) | Empty;
        async fn remote<R: Remote>(item: R) -> Int {
            match item.fetch().await { Ready(n) => n, _ => 0 }
        }
        implement<R: Endpoint> Adapter<R> {
            async fn from_field(&self) -> Int {
                match self.inner.invoke().await { Good(n) => n, _ => 0 }
            }
            async fn shadow<R: Other<Int>>(&self, item: R) -> Int {
                match item.invoke().await { Good(n) => n, _ => 0 }
            }
        }
        async fn from_argument<R>(item: R) -> Int where type R: Endpoint {
            match item.invoke().await { Good(n) => n, _ => 0 }
        }
    ").unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for (name, tag) in [("from_field", 1), ("shadow", 0), ("from_argument", 1), ("remote", 1)] {
        let function = module.functions.iter().find(|f| module.strings.get(f.name)
            .is_some_and(|path| path.rsplit('.').next() == Some(name))).unwrap();
        let start = function.bytecode_offset as usize;
        let ops = decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize]).unwrap();
        assert!(ops.iter().any(|op| matches!(op, Instruction::IsVar { tag: actual, .. } if *actual == tag)),
            "{name}: {ops:?}");
    }
}

#[test]
fn awaited_protocol_result_rejects_an_absent_variant() {
    let err = awaited_bound_pattern("
        type Unrelated is Absent(Int) | Nope;
        implement<R: Endpoint> Adapter<R> {
            async fn handle(&self) -> Int {
                match self.inner.invoke().await { Absent(n) => n, _ => 0 }
            }
        }
    ").unwrap_err().to_string();
    assert!(err.contains("no declared tag") && err.contains("Absent") && err.contains("First"), "{err}");
}

#[test]
fn awaited_protocol_result_refuses_ambiguous_or_uninstantiated_bounds() {
    for (declaration, bounds, reason) in [
        ("type Unrelated is Good(Int) | None;", "Endpoint + Other<Int>", "ambiguous protocol method"),
        ("type Child is protocol extends Other<Int> {};", "Child", "no declared tag"),
    ] {
        let err = awaited_bound_pattern(&format!(
            "{declaration}
             async fn handle<R: {bounds}>(item: R) -> Int {{
                 match item.invoke().await {{ Good(n) => n, _ => 0 }}
             }}"
        )).unwrap_err().to_string();
        assert!(err.contains(reason), "{bounds}: {err}");
    }
}

#[test]
fn nested_record_pattern_tests_use_the_declared_field_owner() {
    use verum_vbc::instruction::Instruction;
    let compile = |inner: &str| {
        let ast = source(&format!("module test;
            type Phase is Idle | Busy;
            type Fault is Busy | Foreign | Delayed {{ phase: Phase }};
            type Packet<T> is {{ phase: T }};
            fn variant(x: Fault) -> Int {{ match x {{ Delayed {{ phase: {inner} }} => 1, _ => 0 }} }}
            fn record(x: Packet<Phase>) -> Int {{ match x {{ Packet {{ phase: {inner} }} => 1, _ => 0 }} }}
        "));
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("test"));
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen.compile_unit_items(&[&ast], verum_vbc::codegen::ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    };
    let module = compile("Busy").unwrap();
    for (name, expected) in [("test.variant", vec![2, 1]), ("test.record", vec![1])] {
        let function = module.get_function(module.find_function_by_name(name).unwrap()).unwrap();
        let start = function.bytecode_offset as usize;
        let body = &module.bytecode[start..start + function.bytecode_length as usize];
        let tags: Vec<_> = verum_vbc::bytecode::decode_instructions(body).unwrap()
            .into_iter().filter_map(|op| match op {
                Instruction::IsVar { tag, .. } => Some(tag),
                _ => None,
            }).collect();
        assert_eq!(tags, expected, "{name}");
    }
    compile("Idle").expect("member of the nested field only");
    assert!(compile("Foreign").is_err(), "an outer variant cannot belong to Phase");
}

#[test]
fn database_error_retry_policy_executes_against_the_declared_timeout_phase() {
    let mut error_source = include_str!("../../../core/database/common/error.vr").to_owned();
    error_source.push_str("
        fn check_retry_policy() -> Int {
            let statement = DbError.TimedOut { phase: TimeoutPhase.TpStatement };
            let connect = DbError.TimedOut { phase: TimeoutPhase.TpConnect };
            if statement.is_retryable() && !connect.is_retryable() { 42 } else { 0 }
        }
    ");
    let errors = source(&error_source);
    let maybe = source(include_str!("../../../core/base/maybe.vr"));
    let result = source(include_str!("../../../core/base/result.vr"));
    let text = source(include_str!("../../../core/text/text.vr"));
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.database.common"));
    codegen.collect_unit_declarations(&[&maybe, &result, &text, &errors]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&errors], verum_vbc::codegen::ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let entry = module.find_function_by_name("core.database.common.error.check_retry_policy").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry).unwrap();
    assert_eq!(value.try_as_i64(), Some(42));
}

#[test]
fn module_qualified_patterns_use_the_scrutinees_declaring_sum() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, codegen::ItemFailurePolicy};
    let columns = producer("wire", "
        module wire.columns;
        public type Row is { reserved: Int, value: Int };
        public type Cell is Null | Textual(Row) | Binary(Row) | Marked { reserved: Int, value: Int };
    ");
    let foreign = producer("wire", "
        module wire.another;
        public type Row is { value: Int, reserved: Int };
        public type Cell is Textual(Row) | Marked { value: Int, reserved: Int } | Null;
    ");
    let compile = |pattern: &str, named: &str| {
        let ast = source(&format!("
            module app.reader;
            mount wire.columns.{{Cell}};
            mount wire.another.Cell as OtherCell;
            fn named(cell: &wire.columns.Cell) -> Int {{
                match cell {{ {named} {{ value, .. }} => value, _ => 0 }}
            }}
            fn read(cell: &wire.columns.Cell) -> Int {{
                match cell {{
                    {pattern}(row) | wire.columns.Binary(row) => row.value,
                    _ => 0,
                }}
            }}"
        ));
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
        codegen.import_bootstrap_nominal_dependencies(&[&ast], &[&foreign, &columns])?;
        codegen.collect_unit_declarations(&[&ast])?;
        codegen.resolve_pending_imports();
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        codegen.finalize_module_from_state()
    };
    for pattern in ["wire.columns.Textual", "wire.columns.Cell.Textual"] {
        let module = compile(pattern, "wire.columns.Marked").unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let named = module.get_function(module.find_function_by_name("app.reader.named").unwrap()).unwrap();
        let start = named.bytecode_offset as usize;
        let instructions = decode_instructions(&module.bytecode[start..start + named.bytecode_length as usize]).unwrap();
        assert!(instructions.iter().any(|op| matches!(op, Instruction::IsVar { tag: 3, .. })), "{instructions:?}");
        assert!(instructions.iter().any(|op| matches!(op, Instruction::GetVariantDataRef { field: 1, .. })), "{instructions:?}");
        let id = module.find_function_by_name("app.reader.read").unwrap();
        let function = module.get_function(id).unwrap();
        let start = function.bytecode_offset as usize;
        let ops = decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize]).unwrap();
        let tags: Vec<_> = ops.iter().filter_map(|op| match op {
            Instruction::IsVar { tag, .. } => Some(*tag),
            _ => None,
        }).collect();
        assert_eq!(tags, [1, 2], "{pattern}: {ops:?}");
        assert!(ops.iter().any(|op| matches!(op, Instruction::GetF { field_idx: 1, .. })), "{pattern}: {ops:?}");
    }
    for path in ["wire.another.Textual", "wire.another.Cell.Textual", "wire.columns.Absent"] {
        let err = compile(path, "wire.columns.Marked").unwrap_err().to_string();
        assert!(err.contains("no declared tag") && err.contains("wire.columns.Cell"), "{path}: {err}");
    }
    let err = compile("wire.columns.Textual", "wire.another.Marked").unwrap_err().to_string();
    assert!(err.contains("no declared tag") && err.contains("Marked"), "{err}");
}
