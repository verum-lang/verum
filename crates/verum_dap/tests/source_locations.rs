use serde_json::json;
use std::sync::Arc;
use verum_dap::{adapter::DebugAdapter, types::Request};
use verum_vbc::module::{FunctionDescriptor, FunctionId, SourceMap, SourceMapEntry, VbcModule};

fn source_module() -> VbcModule {
    let mut module = VbcModule::new("geometry".into());
    let mut functions = Vec::new();
    for (id, start) in [(0, 0), (1, 8)] {
        let name = module.intern_string(&format!("f{id}"));
        let mut function = FunctionDescriptor::new(name);
        function.id = FunctionId(id);
        function.bytecode_offset = start;
        function.bytecode_length = 8;
        functions.push(function);
    }
    module.functions = functions;
    let files = ["cell.vr", "grid/cell.vr", "src/main.vr"]
        .map(|path| module.intern_string(path))
        .to_vec();
    let entries = [(8, 1, 12, 4), (2, 0, 3, 1), (9, 1, 10, 3), (3, 2, 7, 2)]
        .map(|(bytecode_offset, file_idx, line, column)| SourceMapEntry {
            bytecode_offset,
            file_idx,
            line,
            column,
        })
        .to_vec();
    module.source_map = Some(SourceMap { files, entries });
    module
}

fn set_breakpoint(adapter: &mut DebugAdapter, path: &str, line: i64) -> serde_json::Value {
    let (response, _) = adapter.handle_request(&Request {
        seq: 2,
        command: "setBreakpoints".into(),
        arguments: Some(json!({
            "source": { "path": path }, "breakpoints": [{ "line": line }]
        })),
    });
    assert!(response.success);
    response.body.unwrap()["breakpoints"][0].clone()
}

#[test]
fn breakpoints_resolve_unambiguous_path_components_in_client_coordinates() {
    let mut adapter = DebugAdapter::new();
    adapter.session.module = Some(Arc::new(source_module()));
    let (response, _) = adapter.handle_request(&Request {
        seq: 1,
        command: "initialize".into(),
        arguments: Some(json!({
            "linesStartAt1": false, "columnsStartAt1": false
        })),
    });
    assert!(response.success);

    // The first entry in this unsorted map is on a later source line and at an earlier PC.
    let exact = set_breakpoint(&mut adapter, "grid/cell.vr", 9);
    assert_eq!(exact["verified"], true);
    assert_eq!(exact["line"], 9);
    assert!(adapter.session.check_breakpoint(FunctionId(1), 1).is_some());
    let snapped = set_breakpoint(&mut adapter, "/workspace/src/main.vr", 0);
    assert_eq!(snapped["verified"], true);
    assert_eq!(snapped["line"], 6);
    for (path, line) in [
        ("/workspace/grid/cell.vr", 9),
        ("mycell.vr", 2),
        ("src/main.vr", -1),
    ] {
        assert_eq!(set_breakpoint(&mut adapter, path, line)["verified"], false);
    }
    assert_eq!(
        adapter.session.lookup_source_location(FunctionId(1), 1),
        Some(("grid/cell.vr".into(), 9, 2))
    );
}

#[test]
fn unmapped_pcs_and_conflicting_or_invalid_entries_have_no_source() {
    let mut module = source_module();
    let second = FunctionId(1);
    assert_eq!(
        module.instruction_source(second, 0),
        Some(("grid/cell.vr", 12, 4))
    );
    assert!(module.instruction_source(second, 2).is_none());
    assert!(module.instruction_source(FunctionId(0), 8).is_none());

    let map = module.source_map.as_mut().unwrap();
    map.entries.push(SourceMapEntry {
        bytecode_offset: 8,
        file_idx: 2,
        line: 1,
        column: 1,
    });
    map.entries.push(SourceMapEntry {
        bytecode_offset: 10,
        file_idx: 90,
        line: 1,
        column: 1,
    });
    assert!(module.instruction_source(second, 0).is_none());
    assert!(module.instruction_source(second, 2).is_none());
}

#[test]
fn suspended_callers_report_the_call_even_when_the_continuation_has_its_own_line() {
    use verum_vbc::bytecode::encode_instruction;
    use verum_vbc::instruction::{Instruction, Reg, RegRange};
    use verum_vbc::interpreter::InterpreterState;

    let mut module = VbcModule::new("stack".into());
    let path = module.intern_string("app.vr");
    let mut caller = FunctionDescriptor::new(module.intern_string("caller"));
    let mut callee = FunctionDescriptor::new(module.intern_string("callee"));
    caller.id = FunctionId(0);
    callee.id = FunctionId(1);
    encode_instruction(&Instruction::Nop, &mut module.bytecode);
    let call = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(0), 0),
        },
        &mut module.bytecode,
    );
    let resume = module.bytecode.len() as u32;
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    caller.bytecode_length = module.bytecode.len() as u32;
    callee.bytecode_offset = caller.bytecode_length;
    callee.bytecode_length = encode_instruction(&Instruction::RetV, &mut module.bytecode) as u32;
    let entries =
        [(call, 3), (resume, 4), (callee.bytecode_offset, 9)].map(|(bytecode_offset, line)| {
            SourceMapEntry {
                bytecode_offset,
                file_idx: 0,
                line,
                column: 1,
            }
        });
    module.functions = vec![caller, callee];
    module.source_map = Some(SourceMap {
        files: vec![path],
        entries: entries.to_vec(),
    });
    assert!(
        module
            .source_before_resume(FunctionId(0), call + 1)
            .is_none()
    );
    let module = Arc::new(module);
    let mut state = InterpreterState::new(module.clone());
    state
        .call_stack
        .push_frame(FunctionId(0), 1, 0, Reg(0))
        .unwrap();
    state.set_pc(resume);
    state
        .call_stack
        .push_frame(FunctionId(1), 1, resume, Reg(0))
        .unwrap();

    let mut adapter = DebugAdapter::new();
    adapter.session.module = Some(module);
    adapter.interpreter = Some(state);
    let (response, _) = adapter.handle_request(&Request {
        seq: 1,
        command: "stackTrace".into(),
        arguments: Some(json!({ "threadId": 1 })),
    });
    assert!(response.success);
    let frames = &response.body.unwrap()["stackFrames"];
    assert_eq!(frames[0]["line"], 9);
    assert_eq!(frames[1]["line"], 3);
}
