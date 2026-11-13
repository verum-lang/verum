//! Native allocation and source constructors retain their own callable witnesses.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn wrapper_static_initializer_and_generic_forwarder_receive_witnesses() {
    for wrapper in ["Shared", "Heap"] {
        let source = format!(
            r#"
module wrapper_witness;
type Payload is {{ value: Int }};
type {wrapper}<T> is {{ value: T }};
implement<T> {wrapper}<T> {{
    fn new(value: T) -> {wrapper}<T> {{ {wrapper} {{ value: value }} }}
}}
static ROOT: {wrapper}<Payload> = {wrapper}.new(Payload {{ value: 42 }});
fn forward<T>(value: T) -> {wrapper}<T> {{ {wrapper}.new(value) }}
fn probe() -> Int {{
    let local = forward<Payload>(Payload {{ value: 17 }});
    ROOT.value.value + local.value.value
}}
"#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        for bootstrap in [false, true] {
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("wrapper_witness"));
            let module = if bootstrap {
                codegen.collect_unit_declarations(&[&ast]).unwrap();
                codegen
                    .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                    .unwrap();
                codegen.finalize_module_from_state().unwrap()
            } else {
                codegen.compile_module(&ast).unwrap()
            };
            let module = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&module).unwrap(),
            )
            .unwrap();
            let new = module.find_function_by_name(&format!("{wrapper}.new")).unwrap();
            let forward = module.find_function_by_name("wrapper_witness.forward").unwrap();
            let descriptor = module.get_function(forward).unwrap();
            let start = descriptor.bytecode_offset as usize;
            let end = start + descriptor.bytecode_length as usize;
            let body = verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap();
            assert!(body.iter().any(|op| matches!(op, verum_vbc::Instruction::CallG {
                func_id, ..
            } if *func_id == new.0)), "{wrapper}, bootstrap={bootstrap}: {body:?}");
            let probe = module
                .find_function_by_name("wrapper_witness.probe")
                .unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter.run_global_ctors().unwrap();
            assert_eq!(
                interpreter.execute_function(probe).unwrap().as_i64(),
                59,
                "{wrapper}, bootstrap={bootstrap}"
            );
        }
    }
}

#[test]
fn dereferenced_arguments_forward_the_declared_protocol_target() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, types::TypeRef};

    let source = r#"module core.base.protocols;
        type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
        type Other is protocol { type Target; };
        type Project<A, B> is { context: A, item: B };
        implement<A, B> Other for Project<A, B> { type Target = A; }
        implement<A, B> Deref for Project<A, B> {
            type Target = B;
            fn deref(&self) -> &B { &self.item }
        }
        type Measure is protocol { fn measure(&self) -> Int; };
        type Payload is { value: Int };
        implement Measure for Payload { fn measure(&self) -> Int { self.value } }
    "#;
    for (async_fn, wait) in [("", ""), ("async ", ".await")] {
        let source = format!(r#"{source}
            {async_fn}fn inner<X, Y: Measure>(stream: &mut X, value: &Y) -> Int {{ value.measure() }}
            {async_fn}fn forward<T, S: Measure>(mut stream: T, app: Project<Int, S>) -> Int {{
                inner(&mut stream, &*app){wait}
            }}
            {async_fn}fn probe() -> Int {{
                forward(7, Project {{ context: 12, item: Payload {{ value: 37 }} }}){wait}
            }}
        "#);
        let ast = Parser::new(&source).parse_module().unwrap();
        for bootstrap in [false, true] {
            let mut compiler = VbcCodegen::with_config(CodegenConfig::new("core.base.protocols"));
            let compiled = if bootstrap {
                compiler.collect_unit_declarations(&[&ast]).unwrap();
                compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
                compiler.finalize_module_from_state().unwrap()
            } else {
                compiler.compile_module(&ast).unwrap()
            };
            let module = verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
            ).unwrap();
            let inner = module.find_function_by_name("core.base.protocols.inner").unwrap();
            let forward = module.find_function_by_name("core.base.protocols.forward").unwrap();
            let function = module.get_function(forward).unwrap();
            let start = function.bytecode_offset as usize;
            let end = start + function.bytecode_length as usize;
            let instructions = decode_instructions(&module.bytecode[start..end]).unwrap();
            let expected: Vec<_> = function.type_params.iter().map(|param| TypeRef::Generic(param.id)).collect();
            assert!(instructions.iter().any(|instruction| matches!(instruction,
                Instruction::CallG { func_id, type_args, .. } if *func_id == inner.0 && type_args.as_slice() == expected
            )), "bootstrap={bootstrap}: {instructions:?}");
            let probe = module.find_function_by_name("core.base.protocols.probe").unwrap();
            let result = Interpreter::new(Arc::new(module)).execute_function(probe).unwrap();
            assert_eq!(result.as_i64(), 37, "bootstrap={bootstrap}");
        }
    }
}

fn canonical_allocation_declarations() -> verum_ast::Module {
    use verum_ast::{ItemKind, TypeKind, decl::{ImplItemKind, ImplKind}};

    // Only signatures of the intercepted .new functions are needed here; source
    // declaration collection keeps their actual stdlib owner and entire type roster.
    let mut declarations = Parser::new(include_str!("../../../core/base/memory.vr")).parse_module().unwrap();
    declarations.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(ty) => matches!(ty.name.as_str(), "Heap" | "Shared" | "SharedInner"),
        ItemKind::Impl(implementation) => {
            let ImplKind::Inherent(owner) = &implementation.kind else { return false };
            let TypeKind::Generic { base, .. } = &owner.kind else { return false };
            let TypeKind::Path(path) = &base.kind else { return false };
            if !path.as_ident().is_some_and(|name| matches!(name.as_str(), "Heap" | "Shared")) {
                return false;
            }
            implementation.items.retain(|member| matches!(&member.kind,
                ImplItemKind::Function(function) if function.name.as_str() == "new"));
            !implementation.items.is_empty()
        }
        _ => false,
    });
    declarations
}

#[test]
fn canonical_allocation_supplies_the_nested_receiver_constructor_result() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, types::TypeRef};

    let declarations = canonical_allocation_declarations();
    let source = r#"
        module allocation_client;
        mount core.base.memory.{Heap, Shared};
        type Receipt is { code: Int, count: Int };
        type Ticket<T> is { number: Int };
        implement<T> Ticket<T> {
            fn new(number: Int) -> Ticket<T> { Ticket { number } }
        }
        fn make_shared(number: Int) -> Shared<Ticket<Receipt>> {
            let cell: Shared<Ticket<Receipt>> = Shared.new(Ticket.new(number));
            cell
        }
        fn make_heap(number: Int) -> Heap<Ticket<Receipt>> {
            let cell: Heap<Ticket<Receipt>> = Heap.new(Ticket.new(number));
            cell
        }
    "#;
    let consumer = Parser::new(source).parse_module().unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("allocation_client"));
    cg.collect_unit_declarations(&[&declarations, &consumer]).unwrap();
    cg.resolve_pending_imports();
    cg.compile_unit_items(&[&consumer], ItemFailurePolicy::Strict).unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    let ticket = module.find_function_by_name("Ticket.new").unwrap();
    let receipt = module.types.iter().find(|ty| module.strings.get(ty.name)
        .is_some_and(|name| name.rsplit('.').next() == Some("Receipt"))).unwrap();
    for name in ["allocation_client.make_heap", "allocation_client.make_shared"] {
        let entry = module.find_function_by_name(name).unwrap();
        let function = module.get_function(entry).unwrap();
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let body = decode_instructions(&module.bytecode[start..end]).unwrap();
        assert!(body.iter().any(|op| matches!(op,
            Instruction::CallG { func_id, type_args, .. }
                if *func_id == ticket.0 && type_args.as_slice() == [TypeRef::Concrete(receipt.id)]
        )), "{name}: {body:?}");
    }
}

fn list_dispatch_declarations() -> verum_ast::Module {
    use verum_ast::{ItemKind, TypeKind, decl::{ImplItemKind, ImplKind}};

    let mut owner = Parser::new(include_str!("../../../core/collections/list.vr")).parse_module().unwrap();
    owner.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(declaration) => declaration.name.as_str() == "List",
        ItemKind::Impl(implementation) => {
            let ImplKind::Inherent(ty) = &implementation.kind else { return false };
            let TypeKind::Generic { base, .. } = &ty.kind else { return false };
            let TypeKind::Path(path) = &base.kind else { return false };
            if !path.as_ident().is_some_and(|name| name.as_str() == "List") { return false; }
            implementation.items.retain(|member| matches!(&member.kind,
                ImplItemKind::Function(method) if matches!(method.name.as_str(), "new" | "push" | "pop")));
            !implementation.items.is_empty()
        }
        _ => false,
    });
    owner
}

#[test]
fn a_named_unique_allocation_survives_the_submitting_frame() {
    use verum_ast::ItemKind;
    use verum_common::ResourceDiscipline;
    use verum_vbc::types::{TypeId, TypeRef};

    let memory = canonical_allocation_declarations();
    let list = list_dispatch_declarations();
    let mut maybe = Parser::new(include_str!("../../../core/base/maybe.vr")).parse_module().unwrap();
    maybe.items.retain(|item| matches!(&item.kind, ItemKind::Module(_) | ItemKind::Type(_)));
    for annotation in ["", ": Heap<Receipt>"] {
        let source = format!(r#"
            module forwarding;
            mount core.base.memory.Heap;
            mount core.collections.list.List;
            mount core.base.maybe.Maybe;
            type Receipt is {{ base: Int }};
            fn enqueue(queue: &mut List<Heap<Receipt>>) {{
                let item{annotation} = Heap.new(Receipt {{ base: 37 }});
                queue.push(item);
            }}
            fn check() -> Int {{
                let mut queue: List<Heap<Receipt>> = List.new();
                enqueue(&mut queue);
                match queue.pop() {{
                    Maybe.Some(item) => (*item).base + 36,
                    Maybe.None => -1,
                }}
            }}
        "#);
        let client = Parser::new(&source).parse_module().unwrap();
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("forwarding"));
        // Native dispatch reads the real declared parameter/owner metadata.
        // Its built-in operation executes the canonical List and Heap carriers.
        cg.collect_unit_declarations(&[&memory, &list, &maybe, &client]).unwrap();
        cg.resolve_pending_imports();
        cg.compile_unit_items(&[&client], ItemFailurePolicy::Strict).unwrap();
        let archive = cg.finalize_module_from_state().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&archive).unwrap();
        let archive = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        let owner = TypeRef::Instantiated { base: TypeId::HEAP, args: vec![TypeRef::Concrete(TypeId::INT)] };
        assert_eq!(archive.resource_discipline(&owner), ResourceDiscipline::Affine);
        let entry = archive.find_function_by_name("forwarding.check").unwrap();
        let returned = Interpreter::new(Arc::new(archive)).execute_function(entry).unwrap();
        assert_eq!(returned.try_as_i64(), Some(73), "{annotation}");
    }
}

#[test]
fn canonical_raw_owner_recovery_retains_declared_call_witnesses() {
    use verum_ast::{
        decl::{ImplItemKind, ImplKind},
        ItemKind, TypeKind,
    };

    let mut memory = Parser::new(include_str!("../../../core/base/memory.vr"))
        .parse_module()
        .unwrap();
    memory.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(owner) => owner.name.as_str() == "Heap",
        ItemKind::Impl(implementation) => {
            let owner = match &implementation.kind {
                ImplKind::Inherent(owner) | ImplKind::Protocol { for_type: owner, .. } => owner,
            };
            let TypeKind::Generic { base, .. } = &owner.kind else { return false };
            let TypeKind::Path(path) = &base.kind else { return false };
            if !path.as_ident().is_some_and(|name| name.as_str() == "Heap") { return false; }
            match &implementation.kind {
                ImplKind::Inherent(_) => implementation.items.retain(|member| matches!(&member.kind,
                    ImplItemKind::Function(method) if matches!(method.name.as_str(), "new" | "from_raw" | "into_raw"))),
                ImplKind::Protocol { protocol, .. } if matches!(protocol.last_segment_name(), "Deref" | "DerefMut") => {},
                _ => return false,
            }
            !implementation.items.is_empty()
        }
        _ => false,
    });
    let mut protocols = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    protocols.items.retain(|item| matches!(&item.kind, ItemKind::Module(_))
        || matches!(&item.kind, ItemKind::Type(owner) if matches!(owner.name.as_str(), "Deref" | "DerefMut")));
    for pointer in ["&unsafe Receipt<T>", "&mut Receipt<T>", "&Receipt<T>"] {
        let source = r#"
        module raw_owner;
        mount core.base.memory.Heap;
        type Receipt<T> is { value: T };
        implement<T> Receipt<T> {
            fn width(&mut self) -> Int { let read = || T.size; read() as Int }
        }
        fn release<T>(raw: &unsafe Byte) -> Int {
            let mut owner = unsafe { Heap.from_raw(raw as $POINTER) };
            owner.width()
        }
        fn check() -> Int {
            let number = Heap.new(Receipt { value: 37 });
            let flag = Heap.new(Receipt { value: true });
            let number_raw = number.into_raw() as &unsafe Byte;
            let flag_raw = flag.into_raw() as &unsafe Byte;
            release<Int>(number_raw) * 10 + release<Bool>(flag_raw)
        }
    "#
        .replace("$POINTER", pointer);
        let client = Parser::new(&source).parse_module().unwrap();
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("raw_owner"));
        cg.collect_unit_declarations(&[&protocols, &memory, &client])
            .unwrap();
        cg.resolve_pending_imports();
        cg.compile_unit_items(&[&client], ItemFailurePolicy::Strict)
            .unwrap();
        let archive = cg.finalize_module_from_state().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&archive).unwrap();
        let archive = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        // Signature collection deliberately leaves the source implementation external.
        // Pin the ownership recovery's selected receiver before archive linking.
        let selected = archive.find_function_by_name("Receipt.width").unwrap();
        let entry = archive.find_function_by_name("raw_owner.release").unwrap();
        let function = archive.get_function(entry).unwrap();
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let body = verum_vbc::bytecode::decode_instructions(&archive.bytecode[start..end]).unwrap();
        let expected = verum_vbc::types::TypeRef::Generic(function.type_params[0].id);
        assert!(
            body.iter().any(|operation| matches!(operation,
                verum_vbc::Instruction::CallG { func_id, type_args, .. }
                    if *func_id == selected.0 && type_args.as_slice() == [expected.clone()]
            )),
            "pointer={pointer}: {body:?}"
        );
    }
}
