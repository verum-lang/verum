use super::*;
use crate::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};
use verum_ast::{ItemKind, decl::ImplKind};
use verum_fast_parser::Parser;

fn fixture() -> Interpreter {
    let mut protocols = Parser::new(include_str!("../../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    protocols.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Clone")
    });
    let mut memory = Parser::new(include_str!("../../../../core/base/memory.vr"))
        .parse_module()
        .unwrap();
    memory.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(decl) => matches!(decl.name.name.as_str(), "Shared" | "SharedInner"),
        ItemKind::Impl(decl) => matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. } if protocol.to_string() == "Clone" && verum_ast::pretty::format_type(for_type) == "Shared<T>"),
        _ => false,
    });
    assert_eq!(
        memory.items.len(),
        4,
        "actual header, declarations and complete Clone impl"
    );
    let mut list = Parser::new(include_str!("../../../../core/collections/list.vr"))
        .parse_module()
        .unwrap();
    list.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "List")
    });
    let app = Parser::new(
        r#"
        module app;
        mount core.base.memory.Shared;
        mount core.collections.list.List;
        type Pair is { left: Shared<Int>, right: Shared<Int> };
        type Leaf is { value: Int };
        type Branch is { left: Leaf, right: Leaf };
        type Borrowed is { value: &Int };
        type Node is { children: List<Node> };
        type affine Ticket is { value: Int };
    "#,
    )
    .parse_module()
    .unwrap();
    let sources: List<_> = [&protocols, &memory, &list, &app].into_iter().collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen.collect_unit_declarations(&sources).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    crate::validate::validate_module(&module).unwrap();
    Interpreter::new(module.into())
}
fn owner(module: &VbcModule, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|d| {
            module
                .get_string(d.name)
                .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
        })
        .unwrap()
        .id
}
fn record(vm: &mut Interpreter, owner: TypeId, fields: &[Value]) -> Value {
    let object = vm.state.heap.alloc(owner, fields.len().max(1) * 8).unwrap();
    // SAFETY: this new exact allocation contains the provided initialized fields.
    unsafe {
        std::ptr::copy_nonoverlapping(fields.as_ptr(), object.data_ptr().cast(), fields.len());
    }
    Value::from_ptr(object.as_ptr().cast::<u8>())
}
fn named(vm: &mut Interpreter, name: &str, fields: &[Value]) -> Value {
    let id = owner(&vm.state.module, name);
    record(vm, id, fields)
}
fn field(value: Value, index: usize) -> Value {
    // SAFETY: each caller retains the exact live allocation and uses a declared slot.
    unsafe {
        value
            .as_ptr::<u8>()
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>()
            .add(index)
            .read_unaligned()
    }
}
fn free(vm: &mut Interpreter, value: Value) {
    assert!(vm.state.heap.contains(value.as_ptr()));
    // SAFETY: this control owns the exact object; fields are cleaned separately.
    unsafe {
        vm.state
            .heap
            .free(heap::Object::from_raw(value.as_ptr()).unwrap());
    }
}
fn plan(vm: &Interpreter, name: &str) -> CopyPlan {
    copy_plan::build(
        &vm.state.module,
        &TypeRef::Concrete(owner(&vm.state.module, name)),
    )
    .unwrap()
}
fn handles(vm: &mut Interpreter, count: u32, raw: bool) -> (Value, Value, *mut Value) {
    let inner_owner = owner(&vm.state.module, "SharedInner");
    let inner = record(
        vm,
        inner_owner,
        &[
            Value::from_i64(i64::from(count)),
            Value::from_i64(0),
            Value::from_i64(73),
        ],
    );
    let (address, slot) = if raw {
        let raw = super::super::ffi_extended::cbgr_user_allocate(&mut vm.state, 24, 8);
        assert_ne!(raw, 0);
        // SAFETY: both exact extents contain the same three initialized Value slots.
        unsafe {
            std::ptr::copy_nonoverlapping(
                inner.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE),
                raw as *mut u8,
                24,
            );
        }
        free(vm, inner);
        (Value::from_i64(raw), raw as *mut Value)
    } else {
        (inner, unsafe {
            inner
                .as_ptr::<u8>()
                .add(heap::OBJECT_HEADER_SIZE)
                .cast::<Value>()
        })
    };
    let shared = named(
        vm,
        "Shared",
        &[address, Value::from_i64(1), Value::from_i64(0)],
    );
    let pair = named(vm, "Pair", &[shared, shared]);
    (pair, shared, slot)
}

#[test]
fn late_shared_overflow_rolls_back_the_whole_unpublished_graph() {
    for raw in [false, true] {
        let mut vm = fixture();
        let (source, shared, slot) = handles(&mut vm, u32::MAX - 1, raw);
        let plan = plan(&vm, "Pair");
        assert!(
            plan.nodes
                .iter()
                .any(|n| matches!(n, CopyNode::Shared { .. }))
        );
        let before_allocs = vm.state.heap.stats().total_allocs;
        let before_frees = vm.state.heap.stats().total_frees;
        let error =
            execute(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap_err();
        assert!(
            error.to_string().contains("nonzero, non-overflowing"),
            "{error}"
        );
        assert_eq!(vm.state.heap.stats().total_allocs - before_allocs, 3);
        assert_eq!(vm.state.heap.stats().total_frees - before_frees, 3);
        // SAFETY: only copied wrappers were retired; the original allocation remains live.
        unsafe {
            assert_eq!(slot.read().try_as_i64(), Some(i64::from(u32::MAX - 1)));
            assert_eq!(slot.add(2).read().try_as_i64(), Some(73));
        }
        assert_eq!(field(source, 0), shared);
        assert_eq!(field(source, 1), shared);
    }
}

#[test]
fn repeated_shared_subgraphs_retain_independently_without_becoming_a_cycle() {
    for raw in [false, true] {
        let mut vm = fixture();
        let (source, original, slot) = handles(&mut vm, 2, raw);
        let plan = plan(&vm, "Pair");
        let copied = execute(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap();
        let first = field(copied, 0);
        let second = field(copied, 1);
        assert_ne!(first, original);
        assert_ne!(second, original);
        assert_ne!(first, second);
        assert_eq!(field(first, 0), field(original, 0));
        assert_eq!(field(second, 0), field(original, 0));
        // SAFETY: publication retains the same original allocation twice.
        unsafe {
            assert_eq!(slot.read().try_as_i64(), Some(4));
            assert_eq!(slot.add(2).read().try_as_i64(), Some(73));
        }
    }
}

#[test]
fn owned_fields_copy_independently_and_survive_original_storage_release() {
    let mut vm = fixture();
    let leaf = named(&mut vm, "Leaf", &[Value::from_i64(73)]);
    let source = named(&mut vm, "Branch", &[leaf, leaf]);
    let plan = plan(&vm, "Branch");
    let copied = execute(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap();
    let left = field(copied, 0);
    let right = field(copied, 1);
    assert_ne!(left, leaf);
    assert_ne!(right, leaf);
    assert_ne!(left, right);
    free(&mut vm, source);
    free(&mut vm, leaf);
    assert_eq!(field(left, 0).try_as_i64(), Some(73));
    assert_eq!(field(right, 0).try_as_i64(), Some(73));
}

#[test]
fn borrowed_fields_keep_the_exact_original_reference() {
    let mut vm = fixture();
    let leaf = named(&mut vm, "Leaf", &[Value::from_i64(73)]);
    let reference = vm
        .state
        .heap
        .project_object_field(
            leaf.as_ptr::<u8>() as usize,
            0,
            crate::value::Capabilities::READ_ONLY,
        )
        .unwrap();
    let source = named(&mut vm, "Borrowed", &[reference]);
    let plan = plan(&vm, "Borrowed");
    let copied = execute(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap();
    assert_eq!(field(copied, 0), reference);
    free(&mut vm, source);
    assert_eq!(
        cbgr_helpers::read_live_reference(&vm.state, field(copied, 0))
            .unwrap()
            .try_as_i64(),
        Some(73)
    );
}

#[test]
fn owning_cycle_and_budget_exhaustion_refuse_before_copying() {
    let mut vm = fixture();
    let node = named(&mut vm, "Node", &[Value::unit()]);
    let backing = record(&mut vm, TypeId::LIST, &[node]);
    let list = record(
        &mut vm,
        TypeId::LIST,
        &[Value::from_i64(1), Value::from_i64(1), backing],
    );
    // SAFETY: initialize the declared child slot before any traversal begins.
    unsafe {
        node.as_ptr::<u8>()
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>()
            .write(list);
    }
    let plan = plan(&vm, "Node");
    let before = vm.state.heap.stats().total_allocs;
    let error = execute(&mut vm.state, node, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
    let error = execute(
        &mut vm.state,
        node,
        &plan,
        CopyLimits {
            max_work: 1,
            ..copy_plan::DEFAULT_COPY_LIMITS
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("work budget"), "{error}");
    assert_eq!(vm.state.heap.stats().total_allocs, before);
}

#[test]
fn real_allocation_refusal_after_preparation_frees_only_new_storage() {
    let mut vm = fixture();
    let (source, _, slot) = handles(&mut vm, 2, false);
    let plan = plan(&vm, "Pair");
    let schedule = preflight(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap();
    let mut transaction = Transaction::new(4).unwrap();
    let before = vm.state.heap.stats().total_frees;
    for step in &schedule.steps {
        transaction.prepare(&mut vm.state, &step.operation).unwrap();
    }
    let error = transaction
        .managed(&mut vm.state, TypeId::LIST, usize::MAX)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains(CopyFailure::AllocationFailure.message()),
        "{error}"
    );
    transaction.abort(&mut vm.state).unwrap();
    assert_eq!(vm.state.heap.stats().total_frees - before, 3);
    // SAFETY: no retain occurs until the complete preparation phase succeeds.
    assert_eq!(unsafe { slot.read() }.try_as_i64(), Some(2));
}

#[test]
fn affine_and_callable_values_do_not_gain_identity_copy_authority() {
    let vm = fixture();
    assert!(
        copy_plan::build(
            &vm.state.module,
            &TypeRef::Concrete(owner(&vm.state.module, "Ticket"))
        )
        .is_err()
    );
    let source = Parser::new(
        "fn callback(value: Int)->Int{value} type Holder is { callback: fn(Int)->Int };",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    assert!(copy_plan::build(&module, &TypeRef::Concrete(owner(&module, "Holder"))).is_err());
}

#[test]
fn long_active_path_and_repeated_subtree_use_bounded_work_on_ordinary_stack() {
    let mut vm = fixture();
    let empty = record(
        &mut vm,
        TypeId::LIST,
        &[Value::from_i64(0), Value::from_i64(0), Value::from_i64(0)],
    );
    let mut child = named(&mut vm, "Node", &[empty]);
    for _ in 1..1500 {
        let backing = record(&mut vm, TypeId::LIST, &[child]);
        let list = record(
            &mut vm,
            TypeId::LIST,
            &[Value::from_i64(1), Value::from_i64(1), backing],
        );
        child = named(&mut vm, "Node", &[list]);
    }
    let backing = record(&mut vm, TypeId::LIST, &[child, child]);
    let list = record(
        &mut vm,
        TypeId::LIST,
        &[Value::from_i64(2), Value::from_i64(2), backing],
    );
    let source = named(&mut vm, "Node", &[list]);
    let plan = plan(&vm, "Node");
    let copied = execute(&mut vm.state, source, &plan, copy_plan::DEFAULT_COPY_LIMITS).unwrap();
    let branches = field(field(copied, 0), 2);
    let first = field(branches, 0);
    let second = field(branches, 1);
    assert_ne!(first, child);
    assert_ne!(first, second);
    for mut cursor in [first, second] {
        let mut count = 0;
        loop {
            count += 1;
            let list = field(cursor, 0);
            if field(list, 0).try_as_i64() == Some(0) {
                break;
            }
            assert_eq!(field(list, 0).try_as_i64(), Some(1));
            cursor = field(field(list, 2), 0);
        }
        assert_eq!(count, 1500);
    }
    assert_eq!(field(backing, 0), child);
    assert_eq!(field(backing, 1), child);
}

#[test]
fn workspace_refusal_uses_the_shared_allocation_failure_category() {
    let error = match Transaction::new(usize::MAX) {
        Ok(_) => panic!("impossible journal reservation succeeded"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains(CopyFailure::AllocationFailure.message()),
        "{error}"
    );
}
