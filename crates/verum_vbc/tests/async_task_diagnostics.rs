//! Observe actual scheduler outcomes, including a failed detached child.
use std::sync::Arc;
use verum_common::runtime_diagnostics::{AsyncTaskStatus, MAX_ASYNC_TASK_ROWS};
use verum_vbc::{
    FunctionId, Instruction as I, Reg, RegRange, VbcModule, bytecode::encode_instruction,
    interpreter::Interpreter, module::FunctionDescriptor,
};

fn add(module: &mut VbcModule, name: &str, code: &[I]) -> FunctionId {
    let name = module.intern_string(name);
    let start = module.bytecode.len() as u32;
    for instruction in code {
        encode_instruction(instruction, &mut module.bytecode);
    }
    module.add_function(FunctionDescriptor {
        name,
        id: FunctionId(module.functions.len() as u32),
        bytecode_offset: start,
        bytecode_length: module.bytecode.len() as u32 - start,
        register_count: 4,
        ..Default::default()
    })
}

fn fixture(await_failure: bool) -> (Interpreter, FunctionId) {
    let mut module = VbcModule::new("task-diagnostics".into());
    let selector = module.intern_string("missing_task_method").0;
    let bad = add(
        &mut module,
        "child_failure",
        &[
            I::LoadI {
                dst: Reg(0),
                value: 17,
            },
            I::CallM {
                dst: Reg(1),
                receiver: Reg(0),
                method_id: selector,
                args: RegRange::new(Reg(0), 0),
            },
            I::Ret { value: Reg(1) },
        ],
    );
    let mut code = vec![
        I::Spawn {
            dst: Reg(1),
            func_id: bad.0,
            args: RegRange::new(Reg(0), 0),
        },
        I::AsyncYield,
    ];
    if await_failure {
        code.push(I::Await {
            dst: Reg(2),
            task: Reg(1),
        });
    }
    code.extend([
        I::LoadI {
            dst: Reg(0),
            value: 41,
        },
        I::Ret { value: Reg(0) },
    ]);
    let entry = add(&mut module, "parent", &code);
    (Interpreter::new(Arc::new(module)), entry)
}

#[test]
fn detached_failure_is_reported_without_failing_the_parent_and_await_keeps_its_cause() {
    let (mut vm, entry) = fixture(false);
    vm.state.tasks.enable_diagnostics(4).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 41);
    let report = vm.state.tasks.diagnostics_report().unwrap();
    assert!(report.validate());
    assert_eq!(report.rows.len(), 1);
    let row = &report.rows[0];
    assert_eq!(row.status, AsyncTaskStatus::Failed);
    assert_eq!(row.function.as_deref(), Some("child_failure"));
    assert_eq!(row.module.as_deref(), Some("task-diagnostics"));
    assert!(row.entered);
    assert!(
        row.error
            .as_deref()
            .unwrap()
            .contains("missing_task_method")
    );
    let (mut awaiting, entry) = fixture(true);
    awaiting.state.tasks.enable_diagnostics(4).unwrap();
    let error = awaiting.execute_function(entry).unwrap_err();
    assert_eq!(
        awaiting.state.tasks.diagnostics_report().unwrap().rows[0].error,
        row.error
    );
    assert!(error.to_string().contains("missing_task_method"));
}

#[test]
fn disabled_capture_and_reset_do_not_reuse_observations() {
    let (mut vm, entry) = fixture(false);
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 41);
    assert!(vm.state.tasks.diagnostics_report().is_none());
    vm.state.tasks.enable_diagnostics(0).unwrap();
    vm.execute_function(entry).unwrap();
    let report = vm.state.tasks.diagnostics_report().unwrap();
    assert!(report.validate());
    assert_eq!(report.omitted_tasks, 1);
    assert!(report.rows.is_empty());
    vm.reset();
    assert!(vm.state.tasks.diagnostics_report().is_none());
}

#[test]
fn capture_retains_only_recent_tasks_and_reads_current_state() {
    let (mut vm, _) = fixture(false);
    vm.state.tasks.enable_diagnostics(usize::MAX).unwrap();
    for i in 0..1000 {
        let id = vm.state.tasks.spawn(FunctionId(0));
        if i % 2 == 0 {
            vm.state.tasks.complete(id, verum_vbc::value::Value::nil());
        } else {
            vm.state.tasks.fail(id);
        }
    }
    let report = vm.state.tasks.diagnostics_report().unwrap();
    assert!(report.validate());
    assert_eq!(report.capacity, MAX_ASYNC_TASK_ROWS);
    assert_eq!(report.rows.len(), MAX_ASYNC_TASK_ROWS);
    assert_eq!(report.omitted_tasks, 1000 - MAX_ASYNC_TASK_ROWS as u64);
    assert_eq!(report.rows[0].task_id, report.omitted_tasks);
    assert_eq!(report.rows.last().unwrap().status, AsyncTaskStatus::Failed);
    vm.state.tasks.enable_diagnostics(1).unwrap();
    let id = vm
        .state
        .tasks
        .spawn_closure_deferred(verum_vbc::value::Value::nil());
    let row = vm.state.tasks.diagnostics_report().unwrap().rows[0].clone();
    assert_eq!(row.task_id, id.0);
    assert_eq!(
        row.function_id, None,
        "closure placeholder zero is not a real function target"
    );
    assert_eq!(row.status, AsyncTaskStatus::Pending);
}

#[test]
fn nested_tasks_record_their_own_entry_not_their_parents_helpers() {
    let mut module = VbcModule::new("nested".into());
    let leaf = add(
        &mut module,
        "leaf",
        &[
            I::LoadI {
                dst: Reg(0),
                value: 7,
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let parent = add(
        &mut module,
        "child_parent",
        &[
            I::Spawn {
                dst: Reg(1),
                func_id: leaf.0,
                args: RegRange::new(Reg(0), 0),
            },
            I::AsyncYield,
            I::LoadI {
                dst: Reg(0),
                value: 13,
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let entry = add(
        &mut module,
        "main",
        &[
            I::Spawn {
                dst: Reg(1),
                func_id: parent.0,
                args: RegRange::new(Reg(0), 0),
            },
            I::AsyncYield,
            I::LoadI {
                dst: Reg(0),
                value: 19,
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let mut vm = Interpreter::new(Arc::new(module));
    vm.state.tasks.enable_diagnostics(4).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 19);
    let report = vm.state.tasks.diagnostics_report().unwrap();
    assert!(report.validate());
    assert_eq!(
        report
            .rows
            .iter()
            .map(|r| r.function.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["child_parent", "leaf"]
    );
    assert!(
        report
            .rows
            .iter()
            .all(|r| r.status == AsyncTaskStatus::Completed)
    );
}

#[test]
fn closure_task_target_is_the_entered_callable_not_placeholder_function_zero() {
    let mut module = VbcModule::new("closures".into());
    let entry = add(
        &mut module,
        "main",
        &[
            I::AsyncYield,
            I::LoadI {
                dst: Reg(0),
                value: 3,
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let target = add(
        &mut module,
        "closure_body",
        &[
            I::LoadI {
                dst: Reg(0),
                value: 5,
            },
            I::Ret { value: Reg(0) },
        ],
    );
    let mut vm = Interpreter::new(Arc::new(module));
    vm.state.tasks.enable_diagnostics(4).unwrap();
    vm.state
        .tasks
        .spawn_closure_deferred(verum_vbc::value::Value::from_function(target));
    assert_eq!(vm.execute_function(entry).unwrap().as_i64(), 3);
    let report = vm.state.tasks.diagnostics_report().unwrap();
    assert!(report.validate());
    assert_eq!(report.rows[0].function_id, Some(target.0));
    assert_eq!(report.rows[0].function.as_deref(), Some("closure_body"));
    assert_eq!(report.rows[0].status, AsyncTaskStatus::Completed);
}
