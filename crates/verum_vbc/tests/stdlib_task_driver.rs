//! Poll the stdlib's executor state before and after serializing its bytecode.
#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module, TypeKind, decl::ImplKind};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn declarations(source: &str, names: &[&str], inherent: &[&str]) -> Module {
    let mut module = Parser::new(source).parse_module().unwrap();
    module.items.retain(|item| match &item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Type(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Protocol(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Function(decl) => names.contains(&decl.name.as_str()),
        ItemKind::Impl(decl) => {
            let ImplKind::Inherent(ty) = &decl.kind else { return false };
            let ty = match &ty.kind { TypeKind::Generic { base, .. } => base, _ => ty };
            matches!(&ty.kind, TypeKind::Path(path) if path.as_ident().is_some_and(|name| inherent.contains(&name.as_str())))
        }
        _ => false,
    });
    module
}

fn compiled_and_archived(units: &[&Module]) -> [verum_vbc::module::VbcModule; 2] {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("driver").with_validation());
    codegen.collect_unit_declarations(units).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(units, ItemFailurePolicy::Strict).unwrap();
    let mut module = codegen.finalize_module_from_state().unwrap();
    assert!(module.resolve_external_bands().is_empty());
    module.resolve_protocol_dispatch();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let archive = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&archive).unwrap();
    [module, archive]
}

#[test]
fn completion_moves_the_output_once_and_returns_the_losing_value() {
    let result = declarations(include_str!("../../../core/base/result.vr"), &["Result"], &[]);
    let poll = declarations(include_str!("../../../core/async/poll.vr"), &["Poll"], &[]);
    let memory = declarations(include_str!("../../../core/base/memory.vr"), &["replace"], &[]);
    let mut task = declarations(
        include_str!("../../../core/async/task.vr"), &["JoinError", "TaskOutcome"], &["TaskOutcome"],
    );
    let probe = Parser::new(r#"
        mount core.base.memory.replace;
        mount core.base.result.Result;
        mount core.async.poll.Poll;
        type Receipt is { left: Int, right: Int };
        fn lifecycle(left: Int, right: Int) -> Int {
            let mut normal: TaskOutcome<Receipt> = TaskOutcome.Pending;
            match normal.take_result() { Poll.Pending => {}, _ => return -1 };
            match normal.publish(Receipt { left, right }) { Result.Ok(()) => {}, _ => return -2 };
            if !normal.is_finished() || normal.request_cancel() { return -3; }
            let sum = match normal.take_result() {
                Poll.Ready(Result.Ok(receipt)) => receipt.left + receipt.right,
                _ => return -4,
            };
            let mut cancelled: TaskOutcome<Receipt> = TaskOutcome.Pending;
            if !cancelled.request_cancel() || cancelled.request_cancel() { return -5; }
            match cancelled.take_result() { Poll.Pending => {}, _ => return -6 };
            match cancelled.publish(Receipt { left: 19, right: 23 }) {
                Result.Err(rejected) => if rejected.left + rejected.right != 42 { return -7; },
                _ => return -8,
            };
            if cancelled.is_finished() || cancelled.was_cancelled() { return -9; }
            cancelled.fail(JoinError.Cancelled);
            match cancelled.take_result() { Poll.Ready(Result.Err(JoinError.Cancelled)) => {}, _ => return -10 };
            if !cancelled.was_cancelled() || !cancelled.is_finished() { return -11; }
            let mut failed: TaskOutcome<Receipt> = TaskOutcome.Pending;
            failed.fail(JoinError.Panicked);
            match failed.take_result() { Poll.Ready(Result.Err(JoinError.Panicked)) => sum, _ => -12 }
        }
        fn twice() -> Poll<Result<Int,JoinError>> {
            let mut outcome: TaskOutcome<Int> = TaskOutcome.Pending;
            outcome.publish(73);
            outcome.take_result();
            outcome.take_result()
        }
    "#).parse_module().unwrap();
    task.items.extend(probe.items);
    for module in compiled_and_archived(&[&memory, &poll, &result, &task]) {
        let lifecycle = module.find_function_by_name("core.async.task.lifecycle").unwrap();
        let twice = module.find_function_by_name("core.async.task.twice").unwrap();
        let mut vm = Interpreter::new(module.into());
        for (left, right) in [(37, 36), (-9, 12)] {
            let args = [verum_vbc::Value::from_i64(left), verum_vbc::Value::from_i64(right)];
            assert_eq!(vm.execute_function_with_args(lifecycle, &args).unwrap().try_as_i64(), Some(left + right));
        }
        assert!(vm.execute_function(twice).unwrap_err().to_string()
            .contains("JoinHandle polled after completion"));
    }
}

const PROBE: &str = r#"
    mount core.base.maybe.Maybe;
    mount core.async.future.Future;
    mount core.async.poll.Poll;
    mount core.async.waker.{Context, noop_waker};
    type Receipt is { base: Int, increase: Int };
    type YieldReceipt is { base: Int, increase: Int, polls: Int };
    implement Future for YieldReceipt {
        type Output = Receipt;
        fn poll(&mut self, _cx: &mut Context) -> Poll<Receipt> {
            self.polls = self.polls + 1;
            match self.polls {
                1 => Poll.Pending,
                2 => Poll.Ready(Receipt { base: self.base, increase: self.increase }),
                _ => panic("future polled after completion"),
            }
        }
    }
    fn drive(base: Int, increase: Int) -> Int {
        let mut driver: TaskState<YieldReceipt> = TaskState.new(YieldReceipt { base, increase, polls: 0 });
        let waker = noop_waker();
        let mut cx = Context.from_waker(&waker);
        match driver.poll(&mut cx) { Poll.Pending => {}, _ => return -1 };
        match driver.poll(&mut cx) { Poll.Ready(()) => {}, _ => return -2 };
        match driver.poll(&mut cx) { Poll.Ready(()) => {}, _ => return -3 };
        let receipt: Receipt = driver.into_result();
        receipt.base + receipt.increase
    }
    fn premature() -> Receipt {
        let driver: TaskState<YieldReceipt> = TaskState.new(YieldReceipt { base: 1, increase: 2, polls: 0 });
        driver.into_result()
    }
    fn pending_wait() -> Int {
        let start = monotonic_nanos();
        let receipt = block_on(YieldReceipt { base: 37, increase: 36, polls: 0 });
        if receipt.base + receipt.increase != 73 { return -1; }
        (monotonic_nanos() - start) as Int
    }
"#;

#[test]
fn task_driver_preserves_the_pending_future_and_its_concrete_output() {
    let maybe = declarations(include_str!("../../../core/base/maybe.vr"), &["Maybe"], &[]);
    let poll = declarations(include_str!("../../../core/async/poll.vr"), &["Poll"], &[]);
    let clock = declarations(
        include_str!("../../../core/intrinsics/runtime/time.vr"), &["monotonic_nanos", "sleep_ns"], &[],
    );
    let waker = declarations(
        include_str!("../../../core/async/waker.vr"),
        &[
            "RawWakerVTable",
            "RawWaker",
            "Waker",
            "Context",
            "noop_clone",
            "noop_wake",
            "noop_wake_by_ref",
            "noop_drop",
            "noop_vtable",
            "noop_waker",
        ],
        &["RawWakerVTable", "RawWaker", "Context"],
    );
    let mut future = declarations(
        include_str!("../../../core/async/future.vr"),
        &["Future"],
        &[],
    );
    future.items.extend(
        Parser::new("mount core.async.poll.Poll; mount core.async.waker.Context;")
            .parse_module()
            .unwrap()
            .items,
    );
    let mut executor = declarations(
        include_str!("../../../core/async/executor.vr"),
        &["TaskState", "block_on"],
        &["TaskState"],
    );
    executor.items.extend(Parser::new("mount core.intrinsics.runtime.time.{monotonic_nanos, sleep_ns};")
        .parse_module().unwrap().items);
    executor
        .items
        .extend(Parser::new(PROBE).parse_module().unwrap().items);
    let sources = [&maybe, &poll, &waker, &future, &clock, &executor];
    for module in compiled_and_archived(&sources) {
        let drive = module
            .find_function_by_name("core.async.executor.drive")
            .unwrap();
        let premature = module
            .find_function_by_name("core.async.executor.premature")
            .unwrap();
        let pending_wait = module.find_function_by_name("core.async.executor.pending_wait").unwrap();
        let block_on = module.find_function_by_name("core.async.executor.block_on").unwrap();
        let descriptor = module.get_function(block_on).unwrap();
        let start = descriptor.bytecode_offset as usize;
        let body = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[start..start + descriptor.bytecode_length as usize],
        ).unwrap();
        assert!(body.iter().any(|op| matches!(op, verum_vbc::Instruction::TimeExtended { sub_op, .. }
            if *sub_op == verum_vbc::instruction::TimeSubOpcode::SleepNanos as u8)), "{body:?}");
        let mut vm = Interpreter::new(module.into());
        for (base, increase) in [(37, 36), (-9, 12)] {
            let arguments = [
                verum_vbc::Value::from_i64(base),
                verum_vbc::Value::from_i64(increase),
            ];
            let value = vm.execute_function_with_args(drive, &arguments).unwrap();
            assert_eq!(value.try_as_i64(), Some(base + increase));
        }
        let error = vm.execute_function(premature).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("task result requested before completion"),
            "{error}"
        );
        let elapsed = vm.execute_function(pending_wait).unwrap().try_as_i64().unwrap();
        assert!(elapsed >= 50_000, "50 microsecond poll backoff took {elapsed}ns");
    }
}
