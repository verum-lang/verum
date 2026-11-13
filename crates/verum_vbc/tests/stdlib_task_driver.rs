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
        ItemKind::Module(_) => true,
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
"#;

#[test]
fn task_driver_preserves_the_pending_future_and_its_concrete_output() {
    let maybe = declarations(include_str!("../../../core/base/maybe.vr"), &["Maybe"], &[]);
    let poll = declarations(include_str!("../../../core/async/poll.vr"), &["Poll"], &[]);
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
        &["TaskState"],
        &["TaskState"],
    );
    executor
        .items
        .extend(Parser::new(PROBE).parse_module().unwrap().items);
    let sources = [&maybe, &poll, &waker, &future, &executor];
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("driver").with_validation());
    codegen.collect_unit_declarations(&sources).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = codegen.finalize_module_from_state().unwrap();
    assert!(module.resolve_external_bands().is_empty());
    module.resolve_protocol_dispatch();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for module in [module, decoded] {
        let drive = module
            .find_function_by_name("core.async.executor.drive")
            .unwrap();
        let premature = module
            .find_function_by_name("core.async.executor.premature")
            .unwrap();
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
    }
}
