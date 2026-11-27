//! A list allocated by native storage must run its selected source destructor.
#![cfg(target_os = "macos")]
#[path = "../../verum_vbc/tests/fixtures/canonical_drop.rs"]
mod canonical_drop;
#[path = "../../verum_vbc/tests/fixtures/list_storage.rs"]
mod list_storage;
#[path = "support/source_image.rs"]
mod source_image;

fn check_list_lifecycle(module: verum_vbc::VbcModule) {
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for image in [module, wire.clone()] {
        let entry = image.find_function_by_name("main").unwrap();
        let mut vm =
            verum_vbc::interpreter::Interpreter::try_new_validated(std::sync::Arc::new(image))
                .unwrap();
        vm.run_global_ctors().expect("initialize the source statics");
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
    source_image::run_module(wire, |_| {});
}

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
    check_list_lifecycle(list_storage::source_module(source, false));
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

#[test]
fn iteration_reads_the_initialized_slots_after_source_list_growth() {
    let module = list_storage::source_module(
        r#"
        mount core.base.protocols.Drop;
        type affine Ticket is { digit: Int };
        static mut ORDER: Int = 0;
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { ORDER = ORDER * 10 + self.digit; } }
        }
        fn main() -> Int {
            let mut pairs: List<(Int, Int)> = List.with_capacity(1);
            let mut bytes: List<Byte> = List.with_capacity(1);
            let empty: List<(Int, Int)> = List.new();
            let mut count = 0;
            for (_, _) in empty { count = count + 1; }
            if count != 0 { return 1; }
            let mut i = 0;
            while i < 12 {
                pairs.push((i, i * 3 + 5));
                bytes.push((240 + i) as Byte);
                i = i + 1;
            }
            for (key, value) in pairs {
                if key != count || value != count * 3 + 5 { return 2; }
                count = count + 1;
            }
            if count != 12 { return 3; }
            count = 0;
            for byte in bytes {
                if byte as Int != 240 + count { return 4; }
                count = count + 1;
            }
            if count != 12 { return 5; }
            {
                let mut owners: List<(Int, Ticket)> = List.with_capacity(1);
                owners.push((1, Ticket { digit: 3 }));
                owners.push((2, Ticket { digit: 7 }));
                let mut digits = 0;
                for (_, ref ticket) in &owners { digits = digits * 10 + ticket.digit; }
                if digits != 37 { return 6; }
                if unsafe { ORDER != 0 } { return 7; }
            }
            unsafe { ORDER }
        }
        "#,
        false,
    );
    check_list_lifecycle(module);
}
