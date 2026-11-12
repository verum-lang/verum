#![cfg(feature = "codegen")]
//! Root-qualified constructors retain their declared owners before and after serialization.
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const ROOT: &str = r#"
module cog;
public type State is | Cold | Warm;
implement State {
    public fn marker(&self) -> Int {
        match self { State.Cold => 11, State.Warm => 17 }
    }
}
public type Mode is | idle | active;
implement Mode {
    public fn marker(&self) -> Int {
        match self { Mode.idle => 19, Mode.active => 23 }
    }
}
public type Parcel<T> is | Empty | Full(T);
type Phase is State;
public fn supplied(value: Int = 29) -> Int { value }
public fn identity<T>(value: T) -> T { value }
fn run() -> Int {
    let first = cog.State.Cold;
    if first.marker() != 11 { return 1; }
    if cog.State.Warm.marker() != 17 { return 2; }
    if cog.Phase.Cold.marker() != 11 { return 5; }
    if cog.supplied() != 29 { return 6; }
    if cog.identity(31) != 31 { return 7; }
    if cog.Mode.idle.marker() != 19 { return 8; }
    if cog.Mode.active.marker() != 23 { return 9; }
    let foreign = cog.foreign.State.Cold(37);
    let payload = match foreign { cog.foreign.State.Cold(value) => value, _ => 0 };
    if payload != 37 { return 3; }
    let empty: Parcel<Int> = cog.Parcel.Empty;
    let full: Parcel<Int> = cog.Parcel.Full(73);
    match (empty, full) {
        (Parcel.Empty, Parcel.Full(value)) => value,
        _ => 4,
    }
}
"#;
const FOREIGN: &str = "module cog.foreign; public type State is | Cold(Int) | Hot;";

#[test]
fn root_variants_and_same_leaf_foreign_variants_keep_their_carriers() {
    for reverse in [false, true] {
        let root = Parser::new(ROOT).parse_module().unwrap();
        let foreign = Parser::new(FOREIGN).parse_module().unwrap();
        let units = if reverse {
            [&foreign, &root]
        } else {
            [&root, &foreign]
        };
        let mut generator = VbcCodegen::with_config(CodegenConfig::new("cog").with_validation());
        generator.collect_unit_declarations(&units).unwrap();
        generator
            .compile_unit_items(&units, ItemFailurePolicy::Strict)
            .unwrap();
        let module = generator.finalize_module_from_state().unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        let decoded = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
        for module in [module, decoded] {
            let entry = module.find_function_by_name("cog.run").unwrap();
            let result = Interpreter::new(module.into())
                .execute_function(entry)
                .unwrap();
            assert_eq!(result.try_as_i64(), Some(73), "reverse={reverse}");
        }
    }
}

#[test]
fn missing_rooted_owner_cannot_select_a_foreign_variant() {
    let root = Parser::new("module cog; fn run() { let value = cog.absent.State.Cold(7); }")
        .parse_module()
        .unwrap();
    let foreign = Parser::new(FOREIGN).parse_module().unwrap();
    let units = [&root, &foreign];
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("cog").with_validation());
    generator.collect_unit_declarations(&units).unwrap();
    assert!(
        generator
            .compile_unit_items(&units, ItemFailurePolicy::Strict)
            .is_err()
    );
}

#[test]
fn rooted_values_cannot_borrow_a_missing_or_private_owner() {
    for root in ["cog", "core"] {
        for expression in [
            "cog.absent.State.Cold",
            "cog.absent.State.Cold()",
            "cog.hidden.State.Cold",
            "cog.hidden.State.Cold()",
        ] {
            let source = format!("module {root}; fn run() {{ let value = {expression}; }}");
            let root_module = Parser::new(&source).parse_module().unwrap();
            let foreign = Parser::new(&format!(
                "module {root}.foreign; public type State is | Cold | Hot;"
            ))
            .parse_module()
            .unwrap();
            let hidden = Parser::new(&format!(
                "module {root}.hidden; type State is | Cold | Hot;"
            ))
            .parse_module()
            .unwrap();
            let units = [&root_module, &foreign, &hidden];
            let mut generator = VbcCodegen::with_config(CodegenConfig::new(root).with_validation());
            generator.collect_unit_declarations(&units).unwrap();
            let error = match generator.compile_unit_items(&units, ItemFailurePolicy::Strict) {
                Err(error) => error,
                Ok(()) => panic!(
                    "{source}: rooted lookup must not select another owner or ignore visibility"
                ),
            };
            assert!(error.to_string().contains("State"), "{source}: {error}");
        }
    }
}
