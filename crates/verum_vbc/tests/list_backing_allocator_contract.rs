//! T1517: allocator addresses and canonical List backing share one pointer contract.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction as I, MemSubOpcode as M, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

fn execute(module: VbcModule, id: FunctionId) -> Value {
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .expect("contract execution")
}
fn source(body: &str) -> Value {
    let source = format!("fn probe() -> Int {{ {body} }}");
    let ast = Parser::new(&source).parse_module().expect("source syntax");
    let module = VbcCodegen::with_config(CodegenConfig::new("contract"))
        .compile_module(&ast)
        .expect("compile source");
    let id = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("probe"))
        })
        .unwrap()
        .id;
    execute(module, id)
}
fn mem(sub_op: M, registers: &[u8]) -> I {
    I::MemExtended {
        sub_op: sub_op as u8,
        operands: registers.to_vec(),
    }
}
fn operations(ops: &[I]) -> Value {
    let mut module = VbcModule::new("backing".to_owned());
    for op in ops {
        verum_vbc::bytecode::encode_instruction(op, &mut module.bytecode);
    }
    let mut f = FunctionDescriptor::new(StringId::EMPTY);
    f.id = FunctionId(0);
    f.register_count = 12;
    f.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(f);
    execute(module, FunctionId(0))
}
#[test]
fn source_raw_allocation_accepts_integer_address_for_deallocation() {
    assert_eq!(source(r#"let p = @intrinsic("alloc", 8, 8); let address = p as Int; @intrinsic("dealloc", address as &unsafe Byte, 8, 8); 37"#).as_i64(),37);
}
#[test]
fn source_raw_allocation_accepts_integer_address_for_reallocation() {
    assert_eq!(source(r#"let p = @intrinsic("alloc", 8, 8); let address = p as Int; let next = @intrinsic("realloc", address as &unsafe Byte, 8, 16, 8); @intrinsic("dealloc", next, 16, 8); 37"#).as_i64(),37);
}
#[test]
fn source_null_pointer_deallocation_is_a_noop() {
    assert_eq!(
        source(r#"@intrinsic("dealloc", null_ptr(), 0, 8); 37"#).as_i64(),
        37
    );
}
#[test]
fn canonical_data_address_deallocation_leaves_heap_owned_backing_live() {
    assert_eq!(
        operations(&[
            I::NewList {
                dst: Reg(0),
                capacity_hint: 4
            },
            I::LoadI {
                dst: Reg(1),
                value: 37
            },
            I::ListPush {
                list: Reg(0),
                val: Reg(1)
            },
            I::GetF {
                dst: Reg(2),
                obj: Reg(0),
                field_idx: 2
            },
            I::LoadI {
                dst: Reg(3),
                value: 32
            },
            I::LoadI {
                dst: Reg(4),
                value: 8
            },
            mem(M::Dealloc, &[2, 3, 4]),
            mem(M::DerefValue, &[5, 2]),
            I::Ret { value: Reg(5) }
        ])
        .as_i64(),
        37
    );
}
#[test]
fn canonical_data_address_reallocation_preserves_first_element() {
    assert_eq!(
        operations(&[
            I::NewList {
                dst: Reg(0),
                capacity_hint: 4
            },
            I::LoadI {
                dst: Reg(1),
                value: 37
            },
            I::ListPush {
                list: Reg(0),
                val: Reg(1)
            },
            I::GetF {
                dst: Reg(2),
                obj: Reg(0),
                field_idx: 2
            },
            I::LoadI {
                dst: Reg(3),
                value: 32
            },
            I::LoadI {
                dst: Reg(4),
                value: 8
            },
            I::LoadI {
                dst: Reg(5),
                value: 64
            },
            mem(M::Realloc, &[6, 2, 3, 5, 4]),
            I::SetF {
                obj: Reg(0),
                field_idx: 2,
                value: Reg(6)
            },
            I::GetF {
                dst: Reg(7),
                obj: Reg(0),
                field_idx: 2
            },
            mem(M::DerefValue, &[8, 7]),
            I::Ret { value: Reg(8) }
        ])
        .as_i64(),
        37
    );
}
fn list_source(body: &str) -> Value {
    use verum_ast::ItemKind;
    use verum_ast::decl::{ImplItemKind, ImplKind};
    let mut ast = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module()
        .unwrap();
    let methods = [
        "new",
        "len",
        "capacity",
        "shrink_to_fit",
        "free_buffer",
        "reserve",
        "resize_buffer",
    ];
    let mut retained_impl = false;
    ast.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(t) => t.name.name=="List",
        ItemKind::Function(f) => f.name.name=="list_max_len",
        ItemKind::Impl(imp) => {
            if retained_impl || !matches!(imp.kind, ImplKind::Inherent(_)) { return false; }
            retained_impl=true;
            imp.items.retain(|method| matches!(&method.kind,ImplItemKind::Function(f) if methods.contains(&f.name.name.as_str())));
            !imp.items.is_empty()
        }
        _ => false,
    });
    let probe = format!("fn probe() -> Int {{ {body} }}");
    ast.items
        .extend(Parser::new(&probe).parse_module().unwrap().items);
    let module = VbcCodegen::with_config(CodegenConfig::new("core.collections.list"))
        .compile_module(&ast)
        .expect("compile actual List source subset");
    let id = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n.ends_with("probe"))
        })
        .unwrap()
        .id;
    execute(module, id)
}
#[test]
fn actual_list_source_shrinks_empty_byte_and_int_lists() {
    for ty in ["Byte", "Int"] {
        assert_eq!(
            list_source(&format!(
                "let mut xs=List<{ty}>.new(); xs.shrink_to_fit(); xs.capacity() + xs.len()"
            ))
            .as_i64(),
            0,
            "{ty}"
        );
    }
}

#[test]
fn actual_list_source_nonempty_shrink_preserves_values() {
    for ty in ["Byte", "Int"] {
        assert_eq!(list_source(&format!("let mut xs=List<{ty}>.new(); xs.push(37); xs.push(41); xs.shrink_to_fit(); if xs.capacity()==2 && xs.len()==2 {{ (xs[0] as Int)+(xs[1] as Int) }} else {{ 0 }}")).as_i64(),78,"{ty}");
    }
}

#[test]
fn source_raw_pointer_tag_allocation_still_deallocates() {
    assert_eq!(
        source(r#"let p = @intrinsic("alloc", 8, 8); @intrinsic("dealloc", p, 8, 8); 37"#).as_i64(),
        37
    );
}
