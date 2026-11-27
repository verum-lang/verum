//! A list allocated by native storage must run its selected source destructor.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/canonical_drop.rs"]
mod canonical_drop;
#[path = "../../verum_vbc/tests/fixtures/list_storage.rs"]
mod list_storage;
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn lists_release_their_elements_once_in_reverse_order_after_wire_reload() {
    let source = r#"
        mount core.base.protocols.Drop;
        static mut ORDER: Int = 0;
        type affine Ticket is { digit: Int };
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { ORDER = ORDER * 10 + self.digit; } }
        }
        fn main() -> Int {
            {
                let mut items: List<Ticket> = List.with_capacity(1);
                items.push(Ticket { digit: 3 });
                items.push(Ticket { digit: 7 });
            }
            unsafe { ORDER }
        }
    "#;
    let module = list_storage::source_module(source, false);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    for image in [module, wire.clone()] {
        let entry = image.find_function_by_name("main").unwrap();
        let mut vm =
            verum_vbc::interpreter::Interpreter::try_new_validated(std::sync::Arc::new(image))
                .unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
    source_image::run_module(wire, |_| {});
}

#[test]
fn a_record_with_a_bootstrap_list_retires_its_declared_elements() {
    let source = r#"
        mount core.base.protocols.Drop;
        type affine Ticket is { digit: Int };
        type Basket is { items: List<Ticket> };
        static mut ORDER: Int = 0;
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { ORDER = ORDER * 10 + self.digit; } }
        }
        fn main() -> Int {
            let numbers: List<Int> = [3, 7];
            let width = @intrinsic("list_storage_stride", &numbers);
            let second: Int = unsafe { @intrinsic("list_storage_read", &numbers, 1) };
            { let basket = Basket { items: [Ticket { digit: 3 }, Ticket { digit: 7 }] }; }
            if width == 8 && second == 7 { unsafe { ORDER } } else { 0 }
        }
    "#;
    let mut codegen = verum_vbc::codegen::VbcCodegen::new();
    canonical_drop::initialize(&mut codegen);
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let module = codegen.compile_additional_module(&ast).unwrap();
    assert!(module.get_type(verum_vbc::TypeId::LIST).is_none());
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    source_image::run_module(wire, |_| {});
}
