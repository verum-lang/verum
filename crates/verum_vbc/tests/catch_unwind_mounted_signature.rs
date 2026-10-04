//! T1536: source-mounted canonical sums retain the IDs used by intrinsic ABI.
#![cfg(feature = "codegen")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::{self as heap, Interpreter};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeParamId, TypeRef};

struct Fixture(PathBuf);
impl Fixture {
    fn new(owner: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "verum-catch-signature-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let fixture = Self(root);
        fixture.write(
            "base/result.vr",
            &format!("module {owner}.result; public type Result<T, E> is Ok(T) | Err(E);"),
        );
        fixture.write(
            "base/maybe.vr",
            &format!("module {owner}.maybe; public type Maybe<T> is None | Some(T);"),
        );
        fixture.write(
            "base/ordering.vr",
            &format!("module {owner}.ordering; public type Ordering is Less | Equal | Greater;"),
        );
        fixture
    }

    fn write(&self, relative: &str, source: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create fixture directory");
        std::fs::write(path, source).expect("write fixture");
    }

    fn compile(&self, source: &str) -> Result<VbcModule, String> {
        self.write("intrinsics/control.vr", source);
        let path = self.0.join("intrinsics/control.vr");
        let ast = Parser::new(source).parse_module().expect("parse fixture");
        VbcCodegen::with_config(
            CodegenConfig::new(path.to_str().expect("source path")).with_validation(),
        )
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), self.0.to_str().unwrap())
        .map_err(|error| error.to_string())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove fixture");
    }
}

fn fence_source() -> &'static str {
    r#"
module core.intrinsics.control;
mount core.base.result.{Result};
mount core.base.maybe.{Maybe};
mount core.base.ordering.{Ordering};
type Location is { file: Text, line: Int, column: Int };
type PanicInfo is { message: Text, location: Maybe<Location> };
@intrinsic("catch_unwind")
fn fence<T>(f: fn() -> T) -> Result<T, PanicInfo> { @intrinsic("catch_unwind", f) }
fn probe() -> Result<Int, PanicInfo> { fence(|| 7) }
fn probe_panic() -> core.base.result.Result<Int, PanicInfo> { fence(|| { panic("mounted panic"); 0 }) }
fn compare() -> core.base.ordering.Ordering { Ordering.Less }
"#
}

#[test]
fn source_mounted_canonical_sum_ids_match_the_intrinsic_contract() {
    let fixture = Fixture::new("core.base");
    let module = fixture
        .compile(fence_source())
        .expect("canonical source mounts");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize source module");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("reload source module");
    let functions: verum_common::List<_> = module
        .functions
        .iter()
        .filter(|function| {
            module.get_string(function.name).is_some_and(|name| {
                name.ends_with(".fence")
                    || name.ends_with(".probe")
                    || name.ends_with(".probe_panic")
            })
        })
        .collect();
    assert_eq!(functions.len(), 3);
    for function in functions {
        if module
            .get_string(function.name)
            .is_some_and(|name| name.ends_with(".fence"))
        {
            assert_eq!(
                function.type_params.len(),
                1,
                "the declaration roster must survive body compilation"
            );
            assert_eq!(function.type_params[0].id, TypeParamId(0));
            assert_eq!(module.get_string(function.type_params[0].name), Some("T"));
            assert_eq!(function.explicit_type_param_ids, [Some(TypeParamId(0))]);
            assert!(
                matches!(&function.params[0].type_ref,
                TypeRef::Function { params, return_type, .. }
                if params.is_empty() && **return_type == TypeRef::Generic(TypeParamId(0))),
                "callable return must remain linked to declaration T: {:?}",
                function.params[0].type_ref
            );
        }
        assert!(
            matches!(&function.return_type, TypeRef::Instantiated { base, args } if *base == TypeId::RESULT && args.len() == 2),
            "{:?}",
            function.return_type
        );
    }
    let panic = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("PanicInfo"))
        .expect("panic descriptor");
    assert!(
        matches!(&panic.fields[1].type_ref, TypeRef::Instantiated { base, args } if *base == TypeId::MAYBE && args.len() == 1)
    );
    let panic_id = panic.id;
    let compare = module
        .functions
        .iter()
        .find(|function| {
            module.get_string(function.name) == Some("core.intrinsics.control.compare")
        })
        .expect("qualified Ordering return");
    assert_eq!(compare.return_type, TypeRef::Concrete(TypeId::ORDERING));
    let module = Arc::new(module);
    for (name, tag) in [("probe", 0), ("probe_panic", 1)] {
        let id = module
            .functions
            .iter()
            .find(|function| {
                module.get_string(function.name)
                    == Some(format!("core.intrinsics.control.{name}").as_str())
            })
            .expect("probe function")
            .id;
        let mut interpreter = Interpreter::new(module.clone());
        let result = interpreter
            .execute_function(id)
            .expect("source-mounted catch executes");
        // SAFETY: each probe returns a freshly allocated one-payload Result.
        unsafe {
            assert_eq!(
                heap::ObjectHeader::ref_or_stub(result.as_ptr()).type_id,
                TypeId::RESULT
            );
            assert_eq!(heap::variant_tag(result.as_ptr()), tag);
            let payload = *(result.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE + 8)
                as *const verum_vbc::value::Value);
            if tag == 0 {
                assert_eq!(payload.as_i64(), 7);
            } else {
                assert_eq!(
                    heap::ObjectHeader::ref_or_stub(payload.as_ptr()).type_id,
                    panic_id
                );
                let fields = payload.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE)
                    as *const verum_vbc::value::Value;
                let location = *fields.add(1);
                assert_eq!(
                    heap::ObjectHeader::ref_or_stub(location.as_ptr()).type_id,
                    TypeId::MAYBE
                );
                assert_eq!(heap::variant_tag(location.as_ptr()), 0);
            }
        }
    }
}

#[test]
fn foreign_same_leaf_result_is_not_the_intrinsic_result_contract() {
    let fixture = Fixture::new("core.base");
    fixture.write(
        "foreign/base/result.vr",
        "module foreign.base.result; public type Result<T, E> is Ok(T) | Err(E);",
    );
    let source = fence_source().replace("mount core.base.result", "mount foreign.base.result");
    let error = fixture
        .compile(&source)
        .expect_err("foreign Result must remain distinct");
    assert!(
        error.contains("catch_unwind declaration must return"),
        "{error}"
    );
}

#[test]
fn foreign_same_leaf_maybe_is_not_the_intrinsic_location_contract() {
    let fixture = Fixture::new("core.base");
    fixture.write(
        "foreign/base/maybe.vr",
        "module foreign.base.maybe; public type Maybe<T> is None | Some(T);",
    );
    let source = fence_source().replace("mount core.base.maybe", "mount foreign.base.maybe");
    let error = fixture
        .compile(&source)
        .expect_err("foreign Maybe must remain distinct");
    assert!(error.contains("error descriptor must declare"), "{error}");
}
