//! Real List source bodies and the ordinary interpreter method dispatch surface.
#![cfg(feature = "codegen")]
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
fn is_list(ty: &verum_ast::Type) -> bool {
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(p) => p.as_ident().is_some_and(|i| i.name == "List"),
        verum_ast::ty::TypeKind::Generic { base, .. } => is_list(base),
        _ => false,
    }
}
fn execute(source: &str) -> i64 {
    execute_with_method_names(source, true)
}

fn execute_with_method_names(source: &str, rename_methods: bool) -> i64 {
    try_execute_with_method_names(source, rename_methods)
        .expect("actual source methods")
        .as_i64()
}

fn try_execute_with_method_names(
    source: &str,
    rename_methods: bool,
) -> verum_vbc::interpreter::InterpreterResult<verum_vbc::Value> {
    let mut ast = Parser::new(source).parse_module().expect("caller grammar");
    let mut core = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module()
        .expect("core grammar");
    core.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(t) => t.name.name == "List",
        ItemKind::Function(f) => f.name.name == "list_max_len",
        ItemKind::Impl(i) if matches!(&i.kind, ImplKind::Inherent(t) if is_list(t)) => {
            // clear/truncate retain their real call to the original pop body.
            let original_pop = i
                .items
                .iter()
                .find(|item| {
                    matches!(&item.kind,
                ImplItemKind::Function(function) if function.name.name == "pop")
                })
                .cloned();
            i.items.retain_mut(|m| {
                let ImplItemKind::Function(f) = &mut m.kind else {
                    return false;
                };
                if [
                    "with_capacity",
                    "try_with_capacity",
                    "clear",
                    "truncate",
                    "get",
                    "set",
                    "push",
                    "pop",
                    "insert",
                    "remove",
                    "swap",
                    "swap_remove",
                ]
                .contains(&f.name.name.as_str())
                {
                    // Keep the actual source body while avoiding native method
                    // intercepts: these tests exercise the source implementation.
                    if rename_methods {
                        f.name.name = format!("checked_{}", f.name.name).into();
                    }
                    true
                } else {
                    [
                        "new",
                        "len",
                        "capacity",
                        "reserve",
                        "shrink_to_fit",
                        "resize_buffer",
                        "try_resize_buffer",
                        "free_buffer",
                        "grow",
                        "next_cap",
                        "reverse",
                    ]
                    .contains(&f.name.name.as_str())
                }
            });
            if let Some(original_pop) = original_pop.filter(|_| rename_methods) {
                i.items.push(original_pop);
            }
            true
        }
        _ => false,
    });
    ast.items.extend(core.items);
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .expect("memory grammar");
    memory.items.retain(|item| matches!(&item.kind, ItemKind::Function(f) if ["list_storage_read", "list_storage_write", "list_storage_move", "list_storage_resize"].contains(&f.name.name.as_str())));
    ast.items.extend(memory.items);
    let mut primitives = Parser::new(include_str!("../../../core/base/primitives.vr"))
        .parse_module()
        .expect("primitive grammar");
    primitives.items.retain_mut(|item| {
        let ItemKind::Impl(decl) = &mut item.kind else { return false; };
        if !matches!(&decl.kind, ImplKind::Inherent(ty) if matches!(&ty.kind,
            verum_ast::ty::TypeKind::Int)) {
            return false;
        }
        decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(function) if function.name.name == "checked_mul"));
        true
    });
    ast.items.extend(primitives.items);
    let mut arithmetic = Parser::new(include_str!("../../../core/intrinsics/arithmetic.vr"))
        .parse_module()
        .expect("arithmetic grammar");
    arithmetic.items.retain(|item| matches!(&item.kind, ItemKind::Function(function) if function.name.name == "checked_mul"));
    ast.items.extend(arithmetic.items);
    let mut allocation = Parser::new(include_str!("../../../core/mem/allocator.vr"))
        .parse_module()
        .expect("allocator grammar");
    allocation.items.retain(
        |item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "AllocError"),
    );
    ast.items.extend(allocation.items);
    let result_owner =
        Parser::new("module core.base.result; public type Result<T,E> is Ok(T) | Err(E);")
            .parse_module()
            .expect("canonical Result source owner");
    let mut maybe_owner =
        Parser::new("module core.base.maybe; public type Maybe<T> is None | Some(T);")
            .parse_module()
            .expect("canonical Maybe source owner");
    let mut maybe_methods = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module()
        .expect("actual Maybe source");
    maybe_methods.items.retain_mut(|item| {
        let ItemKind::Impl(decl) = &mut item.kind else {
            return false;
        };
        if !matches!(&decl.kind, ImplKind::Inherent(_)) {
            return false;
        }
        decl.items.retain(|item| {
            matches!(&item.kind, ImplItemKind::Function(function)
            if function.name.name == "is_some")
        });
        !decl.items.is_empty()
    });
    maybe_owner.items.extend(maybe_methods.items);

    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    codegen.register_stdlib_constants();
    codegen.register_stdlib_intrinsics();
    codegen
        .collect_unit_declarations(&[&maybe_owner, &result_owner, &ast])
        .expect("declarations");
    codegen
        .compile_unit_items(
            &[&maybe_owner, &result_owner, &ast],
            ItemFailurePolicy::Strict,
        )
        .expect("source bodies");
    let mut vbc = codegen.finalize_module().expect("source VBC");
    vbc.resolve_protocol_dispatch();
    for function in &mut vbc.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &vbc.functions {
        let body = &vbc.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        verum_vbc::mono::discover_call_instantiations(
            &vbc,
            &verum_vbc::bytecode::decode_instructions(body).unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .expect("source call discovery");
    }
    let vbc = verum_vbc::mono::monomorphize_minimal(vbc, &graph)
        .expect("source mono")
        .module;
    let bytes = verum_vbc::serialize::serialize_module(&vbc).expect("source wire");
    let vbc = verum_vbc::deserialize::deserialize_module(&bytes).expect("source roundtrip");
    assert!(vbc.header.version_minor >= 22);
    let entry = vbc
        .functions
        .iter()
        .find(|f| vbc.get_string(f.name) == Some("probe"))
        .unwrap()
        .id;
    verum_vbc::interpreter::Interpreter::new(verum_common::Shared::new(vbc).into_arc())
        .execute_function(entry)
}

#[test]
fn packed_and_slot_source_methods_preserve_values_and_capacity() {
    for ty in ["Byte", "Int"] {
        let source = format!(
            r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            values.shrink_to_fit(); values.reserve(3);
            values.checked_push(0); values.checked_push(255); values.checked_push(42);
            values.checked_insert(1,17);
            let removed=values.checked_remove(1);
            values.checked_set(0,7); values.checked_swap(0,2);
            let popped=match values.checked_pop() {{Maybe.Some(x)=>x,Maybe.None=>10000}};
            values.shrink_to_fit(); values.reserve(17);
            let first=match values.checked_get(0) {{Maybe.Some(x)=>x,Maybe.None=>10000}};
            let second=match values.checked_get(1) {{Maybe.Some(x)=>x,Maybe.None=>10000}};
            removed+popped+first+second
        }}"#
        );
        assert_eq!(execute(&source), 321, "{ty}");
    }
}
#[test]
fn source_moves_record_values_as_slots() {
    assert_eq!(
        execute(
            r#"type Record is {x:Int,y:Int,z:Int}; fn probe()->Int {
        let mut values:List<Record> = List<Record>.new();
        values.checked_push(Record{x:0,y:11,z:0});
        values.checked_insert(0,Record{x:0,y:37,z:0});
        let removed=values.checked_swap_remove(0);
        let last=values.checked_remove(0);
        values.shrink_to_fit(); values.reserve(4);
        values.checked_push(Record{x:0,y:7,z:0});
        let result=match values.checked_pop() {Maybe.Some(x)=>x.y,Maybe.None=>10000};
        removed.y+last.y+result
    }"#
        ),
        55
    );
}
#[test]
fn source_empty_pop_and_swap_remove_last_do_not_read_empty_storage() {
    for ty in ["Byte", "Int"] {
        assert_eq!(
            execute(&format!(
                r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            let empty=match values.checked_pop() {{Maybe.None=>1,Maybe.Some(x)=>10000}};
            values.checked_push(255);
            let last=values.checked_swap_remove(0);
            values.shrink_to_fit();
            let again=match values.checked_pop() {{Maybe.None=>2,Maybe.Some(x)=>10000}};
            empty+last+again
        }}"#
            )),
            258,
            "{ty}"
        );
    }
}

#[test]
fn rejected_resize_and_heap_allocation_failure_preserve_the_owner() {
    for (ty, width) in [("Byte", 1), ("Int", 8)] {
        // The interpreter allocator refuses this request before asking the OS
        // for memory. It is inside the wire/storage capacity domain.
        let oom = verum_common::layout::MAX_ALLOCATION_SIZE / width + 1;
        assert_eq!(
            execute(&format!(
                r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.with_capacity(4);
            values.checked_push(37);
            let old_pointer=values.ptr as Int; let old_capacity=values.capacity();
            let a=unsafe {{list_storage_resize(&mut values,0)}};
            let b=unsafe {{list_storage_resize(&mut values,-1)}};
            let c=unsafe {{list_storage_resize(&mut values,{oom})}};
            assert(!a, "below length succeeded"); assert(!b, "negative succeeded"); assert(!c, "OOM succeeded");
            assert(values.len()==1, "length changed"); assert(values.capacity()==old_capacity, "capacity changed");
            assert((values.ptr as Int)==old_pointer, "pointer changed");
            unsafe {{list_storage_read(&values,0)}}
        }}"#
            )),
            37,
            "{ty}"
        );
    }
}

#[test]
fn fallible_source_resize_preserves_owner_and_reports_the_declared_error() {
    let capacity = verum_common::layout::MAX_ALLOCATION_SIZE / 8 + 1;
    assert_eq!(
        execute(&format!(
            r#"fn probe()->Int {{
        let mut values:List<Int> = List<Int>.with_capacity(4);
        values.checked_push(37);
        let pointer=values.ptr as Int;
        let outcome=match values.try_resize_buffer({capacity}) {{
            Result.Err(AllocError.OutOfMemory{{requested}})=>1,
            _=>10000,
        }};
        assert((values.ptr as Int)==pointer && values.capacity()==4 && values.len()==1);
        let grown=match values.try_resize_buffer(8) {{Result.Ok(())=>1,_=>10000}};
        outcome+grown+unsafe{{list_storage_read(&values,0)}}
    }}"#
        )),
        39
    );
}

#[test]
fn source_constructors_use_the_owner_storage_instead_of_element_layout() {
    for ty in ["Byte", "Int"] {
        assert_eq!(
            execute(&format!(
                r#"fn probe()->Int {{
            let mut a:List<{ty}> = List<{ty}>.checked_with_capacity(3);
            a.checked_push(255);
            let attempt: Result<List<{ty}>, AllocError> = List<{ty}>.checked_try_with_capacity(2);
            let mut b:List<{ty}> = match attempt {{Result.Ok(xs)=>xs,_=>List.new()}};
            b.checked_push(42);
            let first=match a.checked_get(0) {{Maybe.Some(x)=>x,_=>10000}};
            let second=match b.checked_get(0) {{Maybe.Some(x)=>x,_=>10000}};
            (first as Int)+(second as Int)
        }}"#
            )),
            297,
            "{ty}"
        );
    }
}

#[test]
fn source_clear_truncate_release_and_regrow_keep_owner_valid() {
    for ty in ["Byte", "Int"] {
        assert_eq!(
            execute(&format!(
                r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            values.checked_push(255); values.checked_push(42);
            values.checked_truncate(1); values.checked_clear(); values.shrink_to_fit();
            assert(values.len()==0 && values.capacity()==0);
            values.reserve(4); values.checked_push(37);
            match values.checked_get(0) {{Maybe.Some(x)=>x as Int,_=>10000}}
        }}"#
            )),
            37,
            "{ty}"
        );
    }
}

#[test]
fn ordinary_byte_methods_preserve_shrink_regrow_remove_insert() {
    for ty in ["Byte", "Int"] {
        let source = format!(
            r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            values.push(255 as {ty}); values.push(42 as {ty});
            values.shrink_to_fit(); values.reserve(17);
            values.swap(0,1);
            let removed=values.remove(1);
            values.insert(1,removed);
            let result=(values[0] as Int)*1000+(values[1] as Int);
            values.clear(); values.shrink_to_fit(); values.reserve(2);
            values.push(37 as {ty});
            result+(values[0] as Int)+(values.len()*100000)
        }}"#
        );
        assert_eq!(execute_with_method_names(&source, false), 142292, "{ty}");
    }
}

#[test]
fn ordinary_swap_preserves_byte_owner_fields() {
    assert_eq!(
        execute_with_method_names(
            r#"fn probe()->Int {
        let mut values:List<Byte> = List<Byte>.new();
        values.push(255 as Byte); values.push(42 as Byte);
        values.shrink_to_fit(); values.reserve(17);
        let pointer=values.ptr as Int;
        values.swap(0,1);
        assert(values.len()==2,"swap must preserve length");
        assert(values.capacity()==19,"swap must preserve capacity");
        assert(values.ptr as Int==pointer,"swap must preserve backing");
        (values[0] as Int)*1000+(values[1] as Int)
    }"#,
            false
        ),
        42255
    );
}

#[test]
fn ordinary_reverse_preserves_packed_and_slot_owners() {
    for ty in ["Byte", "Int"] {
        assert_eq!(
            execute_with_method_names(
                &format!(
                    r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            values.shrink_to_fit(); values.reverse();
            values.push(0 as {ty}); values.push(255 as {ty}); values.push(42 as {ty});
            values.shrink_to_fit(); values.reserve(17);
            let pointer=values.ptr as Int;
            values.reverse();
            assert(values.len()==3,"reverse must preserve length");
            assert(values.capacity()==20,"reverse must preserve capacity");
            assert(values.ptr as Int==pointer,"reverse must preserve backing");
            (values[0] as Int)*1000000+(values[1] as Int)*1000+(values[2] as Int)
        }}"#
                ),
                false
            ),
            42255000,
            "{ty}"
        );
    }
}

#[test]
fn ordinary_reordering_preserves_record_handles_and_owner_metadata() {
    for resize in ["", "values.shrink_to_fit(); values.reserve(17);"] {
        let source = format!(
            r#"type Cell is {{x:Int,y:Int,z:Int}};
        fn probe()->Int {{
            let mut values:List<Cell> = List<Cell>.new();
            values.push(Cell{{x:1,y:11,z:111}});
            values.push(Cell{{x:2,y:37,z:222}});
            values.push(Cell{{x:3,y:99,z:333}});
            {resize}
            let pointer=values.ptr as Int;
            let capacity=values.capacity();
            values.swap(0,1);
            values.swap(1,1);
            values.reverse();
            assert(values.len()==3,"reordering must preserve length");
            assert(values.capacity()==capacity,"reordering must preserve capacity");
            assert(values.ptr as Int==pointer,"reordering must preserve backing");
            assert(values[0].x==3 && values[0].z==333,"whole last record");
            assert(values[1].x==1 && values[1].z==111,"whole first record");
            assert(values[2].x==2 && values[2].z==222,"whole middle record");
            values[0].y*1000000+values[1].y*1000+values[2].y
        }}"#
        );
        assert_eq!(
            execute_with_method_names(&source, false),
            99011037,
            "{resize}"
        );
    }
}

#[test]
fn ordinary_empty_reverse_preserves_null_owner() {
    for ty in ["Byte", "Int"] {
        let source = format!(
            r#"fn probe()->Int {{
            let mut values:List<{ty}> = List<{ty}>.new();
            values.shrink_to_fit();
            assert(values.ptr as Int==0,"released empty backing");
            values.reverse();
            assert(values.len()==0,"empty length");
            assert(values.capacity()==0,"empty capacity");
            assert(values.ptr as Int==0,"empty backing");
            37
        }}"#
        );
        assert_eq!(execute_with_method_names(&source, false), 37, "{ty}");
    }
}

#[test]
fn ordinary_reordering_rejects_malformed_owners_before_access() {
    for ty in ["Byte", "Int"] {
        for (setup, operation, diagnostic) in [
            (
                "values.cap=1;",
                "values.reverse();",
                "no live allocation extent",
            ),
            (
                "values.len=1;",
                "values.reverse();",
                "range exceeds capacity",
            ),
            (
                "values.len=2;",
                "values.swap(0,1);",
                "range exceeds capacity",
            ),
            ("", "values.swap(0,0);", "out of bounds"),
        ] {
            let source = format!(
                r#"fn probe()->Int {{
                let mut values:List<{ty}> = List<{ty}>.new();
                values.shrink_to_fit(); {setup} {operation} 0
            }}"#
            );
            let error = try_execute_with_method_names(&source, false)
                .expect_err("malformed owner or empty swap must be refused")
                .to_string();
            assert!(
                error.contains(diagnostic),
                "{ty}: {setup} {operation}: {error}"
            );
        }
    }
}
