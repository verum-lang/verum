//! The production interpreter entry shares capture, constructors and argv for
//! compiled/cache-loaded modules. No source-shaped substitute for execution.
use std::sync::Arc;
use verum_common::{CbgrViolationKind, List, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};
use verum_vbc::{
    bytecode::encode_instruction,
    instruction::{Instruction, Reg},
    module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcModule},
    types::{TypeId, TypeRef},
};

fn add_function(module: &mut VbcModule, name: &str, body: &[Instruction]) -> FunctionId {
    let mut function = FunctionDescriptor::new(module.intern_string(name));
    function.id = FunctionId(module.functions.len() as u32);
    function.bytecode_offset = module.bytecode.len() as u32;
    function.register_count = 4;
    function.return_type = TypeRef::Concrete(TypeId::INT);
    for instruction in body {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    let id = function.id;
    module.add_function(function);
    id
}

fn module(failing_ctor: bool) -> Arc<VbcModule> {
    let mut module = VbcModule::new("runtime_report".into());
    add_function(
        &mut module,
        "main",
        &[
            Instruction::LoadSmallI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    if failing_ctor {
        let ctor = add_function(
            &mut module,
            "__tls_init_observed",
            &[
                Instruction::LoadSmallI {
                    dst: Reg(0),
                    value: 7,
                },
                Instruction::Ref {
                    dst: Reg(1),
                    src: Reg(0),
                },
                Instruction::DropRef { src: Reg(0) },
                Instruction::Deref {
                    dst: Reg(2),
                    ref_reg: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
        );
        module.global_ctors.push((ctor, 0));
    }
    Arc::new(module)
}

#[test]
fn runtime_capture_preserves_constructor_failures_with_and_without_arguments() {
    for args in [List::new(), [Text::from("argument")].into_iter().collect()] {
        let mut session = Session::new(CompilerOptions {
            runtime_diagnostics: true,
            ..Default::default()
        });
        let result = CompilationPipeline::new(&mut session).run_compiled_vbc(module(true), args);
        assert!(result.unwrap_err().to_string().contains("global_ctors"));
        let report = session.take_runtime_report().unwrap();
        assert!(!report.truncated);
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].kind, CbgrViolationKind::GenerationMismatch);
        assert_eq!(
            report.events[0].function.as_deref(),
            Some("__tls_init_observed")
        );
        assert!(session.take_runtime_report().is_none());
    }
}

#[test]
fn new_requests_cannot_reuse_prior_observations() {
    let root = tempfile::tempdir().unwrap();
    let mut session = Session::new(CompilerOptions {
        input: root.path().join("missing.vr"),
        runtime_diagnostics: true,
        ..Default::default()
    });
    for count in [0, 2] {
        let args = (0..count).map(|_| Text::from("arg")).collect();
        CompilationPipeline::new(&mut session)
            .run_compiled_vbc(module(false), args)
            .unwrap();
        assert_eq!(session.take_exit_code(), Some(42));
        let report = session.take_runtime_report().unwrap();
        assert!(report.events.is_empty());
        assert!(!report.truncated);
    }
    // The shared application-entry contract requires zero parameters.
    // Rejection before dispatch must not retain a previous successful report.
    CompilationPipeline::new(&mut session)
        .run_compiled_vbc(module(false), List::new())
        .unwrap();
    let mut invalid_entry = Arc::unwrap_or_clone(module(false));
    invalid_entry.functions[0].params.push(ParamDescriptor {
        type_ref: TypeRef::Concrete(TypeId::LIST),
        ..Default::default()
    });
    let error = CompilationPipeline::new(&mut session)
        .run_compiled_vbc(Arc::new(invalid_entry), List::new())
        .unwrap_err();
    assert!(error.to_string().contains("No main function"));
    assert!(session.take_runtime_report().is_none());
    CompilationPipeline::new(&mut session)
        .run_compiled_vbc(module(false), List::new())
        .unwrap();
    assert!(
        CompilationPipeline::new(&mut session)
            .run_interpreter(List::new())
            .is_err()
    );
    assert!(session.take_runtime_report().is_none());
    CompilationPipeline::new(&mut session)
        .run_compiled_vbc(module(false), List::new())
        .unwrap();
    assert!(
        CompilationPipeline::new(&mut session)
            .run_for_test()
            .is_err()
    );
    assert!(session.take_runtime_report().is_none());
}

#[test]
fn actual_entry_retains_a_detached_task_failure_in_the_same_measurement_capture() {
    use verum_common::runtime_diagnostics::AsyncTaskStatus;
    use verum_vbc::RegRange;
    let mut module = VbcModule::new("detached".into());
    let selector = module.intern_string("missing_child_method").0;
    let child = add_function(&mut module, "child", &[
        Instruction::LoadSmallI { dst: Reg(0), value: 7 },
        Instruction::CallM { dst: Reg(1), receiver: Reg(0), method_id: selector, args: RegRange::new(Reg(0), 0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    add_function(&mut module, "main", &[
        Instruction::Spawn { dst: Reg(1), func_id: child.0, args: RegRange::new(Reg(0), 0) },
        Instruction::AsyncYield,
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);
    let mut session = Session::new(CompilerOptions { runtime_diagnostics: true, ..Default::default() });
    CompilationPipeline::new(&mut session).run_compiled_vbc(Arc::new(module), List::new()).unwrap();
    assert_eq!(session.take_exit_code(), Some(0));
    let report = session.take_runtime_measurements().unwrap();
    assert!(report.async_tasks.validate());
    assert_eq!(report.async_tasks.rows[0].status, AsyncTaskStatus::Failed);
    assert_eq!(report.async_tasks.rows[0].function.as_deref(), Some("child"));
    assert!(report.async_tasks.rows[0].error.as_deref().unwrap().contains("missing_child_method"));
    assert!(session.take_runtime_measurements().is_none());
}
