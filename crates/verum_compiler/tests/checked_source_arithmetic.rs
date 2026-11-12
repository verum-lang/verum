//! Scalar operation bounds survive source imports, method selection and archives.
use std::sync::Arc;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{
    deserialize::deserialize_module, interpreter::Interpreter, serialize::serialize_module,
};

#[test]
fn checked_unsigned_methods_execute_full_width_values_and_overflow() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir(&core).unwrap();
    for (path, source) in [
        ("mod.vr", "module core; public module values; public module arithmetic; public module duration;"),
        ("values.vr", "module core.values; public type Maybe<T> is None | Some(T);"),
        ("arithmetic.vr", "module core.arithmetic; mount core.values.Maybe;
            public pure fn add(a: UInt64, b: UInt64) -> Maybe<UInt64> { @intrinsic(\"checked_add_u64\", a, b) }
            public pure fn sub(a: UInt64, b: UInt64) -> Maybe<UInt64> { @intrinsic(\"checked_sub_u64\", a, b) }
            public pure fn mul(a: UInt64, b: UInt64) -> Maybe<UInt64> { @intrinsic(\"checked_mul_u64\", a, b) }"),
        ("duration.vr", "module core.duration;
            mount core.values.Maybe; mount core.arithmetic.{add, sub, mul};
            implement UInt64 {
                public pure fn checked_add(self, rhs: UInt64) -> Maybe<UInt64> { add(self, rhs) }
            }
            type Duration is (UInt64);
            type PositiveDuration is Duration where |duration| duration.0 > 0;
            implement PositiveDuration {
                pure fn raw(&self) -> UInt64 { self.0 }
            }
            implement Duration {
                pure fn capped(&self, other: Duration) -> Duration {
                    let a: UInt64 = self.0; let b: UInt64 = other.0;
                    match a.checked_add(b) {
                        Maybe.Some(result) => Duration(result),
                        Maybe.None => Duration(0xFFFFFFFFFFFFFFFF_u64),
                    }
                }
            }
            public pure fn high_bit() -> Bool {
                match 0x7FFFFFFFFFFFFFFF_u64.checked_add(1_u64) {
                    Maybe.Some(result) => result == 0x8000000000000000_u64, _ => false,
                }
            }
            public pure fn maximum() -> Bool {
                match add(0xFFFFFFFFFFFFFFF0_u64, 15_u64) {
                    Maybe.Some(result) => result == 0xFFFFFFFFFFFFFFFF_u64, _ => false,
                }
            }
            public pure fn overflow() -> Bool {
                match add(0xFFFFFFFFFFFFFFFF_u64, 1_u64) { Maybe.None => true, _ => false }
            }
            public pure fn subtract() -> Bool {
                match sub(0xFFFFFFFFFFFFFFFF_u64, 1_u64) {
                    Maybe.Some(result) => result == 0xFFFFFFFFFFFFFFFE_u64, _ => false,
                }
            }
            public pure fn underflow() -> Bool {
                match sub(0_u64, 1_u64) { Maybe.None => true, _ => false }
            }
            public pure fn multiply() -> Bool {
                match mul(0x4000000000000000_u64, 2_u64) {
                    Maybe.Some(result) => result == 0x8000000000000000_u64, _ => false,
                }
            }
            public pure fn product_overflow() -> Bool {
                match mul(0x8000000000000000_u64, 2_u64) { Maybe.None => true, _ => false }
            }
            public pure fn capped() -> Bool {
                Duration(0xFFFFFFFFFFFFFFFF_u64).capped(Duration(1_u64)).0 == 0xFFFFFFFFFFFFFFFF_u64
            }
            public pure fn refined_carrier() -> Bool {
                let duration = Duration(73_u64) as PositiveDuration;
                duration.raw() == 73_u64
            }"),
    ] { std::fs::write(core.join(path), source).unwrap(); }
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let module = archive.load_module("core").unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let module = Arc::new(module);
        for name in [
            "high_bit",
            "maximum",
            "overflow",
            "subtract",
            "underflow",
            "multiply",
            "product_overflow",
            "capped",
            "refined_carrier",
        ] {
            let entry = module
                .find_function_by_name(&format!("core.duration.{name}"))
                .unwrap();
            let value = Interpreter::new(module.clone())
                .execute_function(entry)
                .unwrap();
            assert_eq!(value.try_as_bool(), Some(true), "{name}");
        }
    }
}
