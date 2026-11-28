//! A synchronization type's name grants no native layout or method semantics.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn native_semaphore_methods_use_the_declared_fields_and_bodies() {
    let source = r#"
        type Semaphore is { label: Int, limit: Int };
        implement Semaphore {
            fn new(label:Int, limit:Int)->Semaphore { Semaphore { label:label, limit:limit } }
            fn available_permits(&self)->Int { self.limit - self.label }
            fn release(&mut self) { self.limit += self.label; }
        }
        fn main()->Int {
            let mut value = Semaphore.new(7, 73);
            if value.available_permits() != 66 { return 0; }
            value.release();
            value.available_permits()
        }
    "#;
    let module = source_image::module(source);
    let entry = module.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(module.clone().into()).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    source_image::run_module(module, |_| {});
}
