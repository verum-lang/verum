//! A method can return a borrow of the source field on which it was called.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter};

#[test]
fn returned_field_receivers_survive_temporary_register_reuse() {
    let source = r#"
        type Cell is { word: Int };
        type State is { counter: Cell, other: Cell };
        type Observer is { target: &Cell };
        type Editor is { target: &mut Cell };
        static mut EVALUATED: Int = 0;
        implement Cell {
            fn observe(&self) -> Observer { Observer { target: self } }
            fn edit(&mut self) -> Editor { Editor { target: self } }
        }
        fn location(state: &mut State) -> &mut State {
            unsafe { EVALUATED += 1; }
            state
        }
        fn read_number(x: Int) -> Int { x + 1 }
        fn read_after_call() -> Int {
            let state = State { counter: Cell { word: 73 }, other: Cell { word: 109 } };
            let observation = state.counter.observe();
            let a = read_number(17);
            let b = read_number(a);
            observation.target.word + b - 19
        }
        fn write_after_call() -> Int {
            let mut state = State { counter: Cell { word: 11 }, other: Cell { word: 109 } };
            let mut editor = location(&mut state).counter.edit();
            let noise = read_number(71);
            editor.target.word = noise + 1;
            if state.other.word == 109 && unsafe { EVALUATED } == 1 { state.counter.word } else { -1 }
        }
    "#;
    let source = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let decoded = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for module in [module, decoded] {
        for name in ["read_after_call", "write_after_call"] {
            let entry = module.find_function_by_name(name).unwrap();
            let mut vm = Interpreter::try_new_validated(module.clone().into()).unwrap();
            vm.run_global_ctors().unwrap();
            assert_eq!(
                vm.call(entry, &[]).unwrap().try_as_i64(),
                Some(73),
                "{name}"
            );
        }
    }
}

#[path = "fixtures/retained_mutex_receiver.rs"]
mod actual_mutex;

#[test]
fn a_mutex_guard_retains_its_field_after_the_lock_result_register_is_reused() {
    let source = actual_mutex::module();
    let wire = verum_vbc::serialize::serialize_module(&source).unwrap();
    let loaded = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    for image in [source, loaded] {
        let probe = image
            .find_function_by_name("core.sync.mutex.probe_retained_guard")
            .unwrap();
        let mut interpreter = Interpreter::try_new_validated(image.into()).unwrap();
        interpreter.run_global_ctors().unwrap();
        assert_eq!(interpreter.call(probe, &[]).unwrap().try_as_i64(), Some(1));
    }
}
