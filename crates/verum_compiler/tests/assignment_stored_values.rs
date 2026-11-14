//! Checked assignments preserve declared storage across archive serialization.
use std::{fs, sync::Arc};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{VbcModule, archive::read_archive_from_file, interpreter::Interpreter};

fn produce(source: &str) -> Result<VbcModule, Text> {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), source).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core();
    match result {
        Ok(result) => {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            Ok(read_archive_from_file(&output)
                .unwrap()
                .load_module("core")
                .unwrap())
        }
        Err(error) => {
            assert!(!output.exists(), "rejected source published an archive");
            Err(format!("{error:#}").into())
        }
    }
}

fn execute(source: &str, expected: i64) {
    let module = produce(source).unwrap_or_else(|error| panic!("source:\n{source}\n{error}"));
    verum_vbc::validate::validate_module(&module).expect("strict producer validation");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize actual module"),
    )
    .expect("deserialize actual module");
    for (route, module) in [("produced", module), ("wire", wire)] {
        verum_vbc::validate::validate_module(&module).expect("strict execution validation");
        let entry = module
            .find_function_by_name("core.probe")
            .expect("actual probe declaration");
        assert!(
            !verum_vbc::stub_ranges::is_stub_id(entry.0),
            "probe is a forward stub"
        );
        let mut vm = Interpreter::new(Arc::new(module));
        vm.run_global_ctors().expect("actual global initialization");
        let result = vm
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error:?}"));
        assert_eq!(
            result.try_as_i64(),
            Some(expected),
            "{route}: receiver/body dispatch changed"
        );
    }
}

#[test]
fn automatic_user_cleanup_runs_its_declared_mutation() {
    execute(
        r#"
        module core;
        public module base {
            public module protocols {
                public type Drop is protocol {fn drop(&mut self);};
            }
        }
        mount core.base.protocols.Drop;
        type affine DeferredWrite is {target:&mut Int};
        implement Drop for DeferredWrite {
            fn drop(&mut self) {*self.target=73;}
        }
        fn replace_later(target:&mut Int) {
            let pending=DeferredWrite{target}; ()
        }
        public fn probe()->Int {
            let mut value=0;
            replace_later(&mut value);
            value
        }
    "#,
        73,
    );
}

#[test]
fn borrowed_variant_payloads_update_successive_fields() {
    execute(
        r#"
        module core;
        type Number is Value(Int64) | Missing;
        type Cursor is { first:Int64, second:Int64 };
        implement Cursor {
            fn update(&mut self, input:&Number) {
                let mut value:Int64=0_i64;
                match input { Value(n)=>{value=n;} Missing=>{} }
                self.first=value;
                self.second=value+1_i64;
            }
            fn total(&self)->Int {(self.first+self.second) as Int}
        }
        public fn probe()->Int {
            let input=Number.Value(37_i64);
            let mut cursor=Cursor {first:0_i64,second:0_i64};
            cursor.update(&input);
            cursor.total()
        }
    "#,
        75,
    );
}

#[test]
fn borrowed_scalar_assignment_keeps_the_value_independent_of_later_input_writes() {
    execute(
        r#"
        module core;
        public fn probe()->Int {
            let mut source:Int64=37_i64;
            let mut copied:Int64=0_i64;
            { let borrowed=&source; copied=borrowed; }
            source=99_i64;
            copied as Int
        }
    "#,
        37,
    );
}

#[test]
fn reference_assignment_preserves_the_selected_reference() {
    execute(
        r#"
        module core;
        public fn probe()->Int {
            let first:Int64=37_i64;
            let second:Int64=99_i64;
            let mut selected:&Int64=&first;
            selected=&second;
            (*selected) as Int
        }
    "#,
        99,
    );
}

#[test]
fn projected_assignment_copies_the_borrowed_value() {
    execute(
        r#"
        module core;
        type Pair is {first:Int64, second:Int64};
        implement Pair {
            fn set(&mut self, value:&Int64) {
                self.first=value;
                self.second=value;
            }
        }
        public fn probe()->Int {
            let mut input:Int64=37_i64;
            let mut pair=Pair {first:0_i64,second:0_i64};
            pair.set(&input);
            input=99_i64;
            (pair.first+pair.second) as Int
        }
    "#,
        74,
    );
}

#[test]
fn borrowed_record_assignment_copies_the_record_storage() {
    execute(
        r#"
        module core;
        type Cell is {value:Int};
        public fn probe()->Int {
            let mut source=Cell {value:37};
            let mut copied=Cell {value:0};
            { let borrowed=&source; copied=borrowed; }
            source.value=99;
            copied.value
        }
    "#,
        37,
    );
}

#[test]
fn returned_reference_assignment_evaluates_its_source_once() {
    execute(
        r#"
        module core;
        fn select(value:&Int64,calls:&mut Int)->&Int64 {
            *calls+=1;
            value
        }
        public fn probe()->Int {
            let source:Int64=37_i64;
            let mut calls=0;
            let mut copied:Int64=0_i64;
            copied=select(&source,&mut calls);
            (copied as Int)+calls
        }
    "#,
        38,
    );
}

#[test]
fn unconstrained_generic_assignment_requires_a_value_copy_contract() {
    let error = produce(
        r#"
        module core;
        fn replace<T>(destination:&mut T, source:&T) { *destination=source; }
    "#,
    )
    .expect_err("an arbitrary borrowed parameter cannot provide value-copy evidence");
    assert!(error.contains("E400"), "{error}");
}

#[test]
fn equal_record_layouts_do_not_authorize_reference_assignment() {
    let error = produce(
        r#"
        module core;
        type Packet is {value:Int};
        type Parcel is {value:Int};
        fn replace(destination:&mut Packet, source:&Parcel) { *destination=source; }
    "#,
    )
    .expect_err("a matching layout does not identify the declared type");
    assert!(error.contains("E400"), "{error}");
}


#[test]
fn copied_reference_payload_survives_its_intermediate_owner() {
    execute(
        r#"
        module core;
        type Sample is { ptr: &Int, total: Int };
        fn forward(value: Sample) -> Sample {
            let alias = &value;
            Sample { ptr: alias.ptr, total: alias.total }
        }
        public fn probe() -> Int {
            let n = 71;
            let copied = forward(Sample { ptr: &n, total: 2 });
            *copied.ptr + copied.total
        }
        "#,
        73,
    );
}


#[test]
fn borrowed_assignment_preserves_the_declared_alias_after_storage_and_return() {
    execute(
        r#"
        module core;
        type Packet is { value: Int };
        type Selected is Packet;
        implement Packet { fn read(&self) -> Int { self.value } }
        fn forward(value: &Packet) -> Selected {
            let mut copied: Selected = Packet { value: 0 };
            copied = value;
            copied.value += 2;
            copied
        }
        public fn probe() -> Int {
            let owner = Packet { value: 7 };
            let copied = forward(&owner);
            owner.read() * 10 + copied.read()
        }
        "#,
        79,
    );
}

#[test]
fn implicit_assignment_after_explicit_dereference_evaluates_the_call_once() {
    execute(
        r#"
        module core;
        type Packet is { value: Int };
        type Pointer is &Packet;
        fn choose(count: &mut Int, value: &Pointer) -> &Pointer { *count += 1; value }
        public fn probe() -> Int {
            let owner = Packet { value: 7 };
            let first = &owner;
            let mut count = 0;
            let mut copied: Packet = Packet { value: 0 };
            copied = *choose(&mut count, &first);
            copied.value = 9;
            count * 100 + owner.value * 10 + copied.value
        }
        "#,
        179,
    );
}


#[test]
fn assigned_callable_reference_keeps_its_declared_call_signature() {
    execute(
        r#"
        module core;
        fn increment(value: Int) -> Int { value + 1 }
        fn decrement(value: Int) -> Int { value - 1 }
        public fn probe() -> Int {
            let chosen: fn(Int) -> Int = increment;
            let mut selected: fn(Int) -> Int = decrement;
            selected = &chosen;
            selected(72)
        }
        "#,
        73,
    );
}

#[test]
fn projected_call_payload_survives_its_temporary_container() {
    execute(
        r#"
        module core;
        type Payload is { ptr:&Int, marker:Int };
        fn selected(source:&Payload,key:&Int)->Payload {
            Payload{ptr:source.ptr,marker:source.marker}
        }
        public fn probe()->Int {
            let value=73;
            let result={
                let container=Payload{ptr:&value,marker:2};
                let key=3;
                selected(&container,&key)
            };
            *result.ptr
        }
        "#,
        73,
    );
}

#[test]
fn a_mutating_call_cannot_publish_a_dangling_payload_archive() {
    let result = produce(
        r#"
        module core;
        type Payload is { ptr:&Int, marker:Int };
        fn replace(target:&mut Payload,source:&Int) {target.ptr=source;}
        public fn probe(input:&Int)->Payload {
            let mut container=Payload{ptr:input,marker:2};
            {let local=3;replace(&mut container,&local);}
            Payload{ptr:container.ptr,marker:container.marker}
        }
        "#,
    );
    let error = result.expect_err("a stored local loan must refuse archive publication");
    assert!(error.contains("does not live long enough"), "{error}");
}

#[test]
fn implicit_user_cleanup_cannot_publish_a_dangling_payload_archive() {
    let result = produce(
        r#"
        module core;
        public module base {
            public module protocols {
                public type Drop is protocol {fn drop(&mut self);};
            }
        }
        mount core.base.protocols.Drop;
        type Payload is {ptr:&Int,marker:Int};
        type affine DeferredWrite is {target:&mut Payload,source:&Int};
        implement Drop for DeferredWrite {
            fn drop(&mut self) {self.target.ptr=self.source;}
        }
        fn replace_later(target:&mut Payload,source:&Int) {
            let pending=DeferredWrite{target,source}; ()
        }
        public fn probe(input:&Int)->Payload {
            let mut container=Payload{ptr:input,marker:2};
            {let local=3;replace_later(&mut container,&local);}
            Payload{ptr:container.ptr,marker:container.marker}
        }
    "#,
    );
    let error = result.expect_err("user cleanup must retain the stored local loan");
    assert!(error.contains("does not live long enough"), "{error}");
}
