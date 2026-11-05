#![cfg(feature = "codegen")]
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "verum-loader-owner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn write(&self, p: &str, s: &str) {
        let p = self.0.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn probe(provider_header: bool, consumer_header: bool, missing: bool) {
    let f = Fixture::new();
    f.write("core/mod.vr", "module core;");
    // This is a complete independent namespace fixture, not a filtered stdlib.
    f.write("core/prelude/mod.vr", "module core.prelude;");
    f.write(
        "core/origin/token.vr",
        &format!(
            "{} public type Parcel is {{ value:Int }};",
            if provider_header {
                "module core.origin.token;"
            } else {
                ""
            }
        ),
    );
    f.write("core/bridges/mod.vr", "public mount core.bridges.convert;");
    f.write(
        "core/bridges/convert.vr",
        &format!(
            "{} mount core.origin.token.{}; public fn relay(p:Parcel)->Parcel{{p}}",
            if consumer_header {
                "module core.bridges.convert;"
            } else {
                ""
            },
            if missing { "Absent" } else { "Parcel" }
        ),
    );
    let source = "module fixture; mount core.bridges.convert; public fn probe()->Int{42}";
    f.write("main.vr", source);
    let mut cg = VbcCodegen::with_config(
        CodegenConfig::new("fixture")
            .with_validation()
            .with_strict_codegen(),
    );
    let result = cg.compile_module_with_mounts(
        &Parser::new(source).parse_module().unwrap(),
        f.0.join("main.vr").to_str().unwrap(),
        f.0.join("core").to_str().unwrap(),
    );
    if missing {
        assert!(result.is_err(), "unavailable import was admitted");
        return;
    }
    let module = result.unwrap();
    assert!(
        cg.export_functions()
            .contains_key("core.bridges.convert.relay"),
        "loader lost exact file owner"
    );
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let parcel = module
            .types
            .iter()
            .find(|t| module.strings.get(t.name) == Some("Parcel"))
            .unwrap();
        assert_eq!(
            parcel.origin_module.and_then(|id| module.strings.get(id)),
            Some("core.origin.token")
        );
        let entry = module.find_function_by_name("fixture.probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(42)
        );
    }
}
#[test]
fn both_explicit_headers() {
    probe(true, true, false)
}
#[test]
fn provider_has_no_header() {
    probe(false, true, false)
}
#[test]
fn consumer_has_no_header() {
    probe(true, false, false)
}
#[test]
fn both_files_without_headers() {
    probe(false, false, false)
}
#[test]
fn missing_import_with_headers_refuses() {
    probe(true, true, true)
}
#[test]
fn missing_import_without_headers_refuses() {
    probe(false, false, true)
}

fn graph(files: &[(&str, &str)], mounts: &str) -> (verum_vbc::VbcModule, VbcCodegen) {
    let f = Fixture::new();
    f.write("core/mod.vr", "module core;");
    f.write("core/prelude/mod.vr", "module core.prelude;");
    for (p, s) in files {
        f.write(&format!("core/{p}"), s);
    }
    let source = format!("module fixture; {mounts} public fn probe()->Int{{42}}");
    f.write("main.vr", &source);
    let mut cg = VbcCodegen::with_config(
        CodegenConfig::new("fixture")
            .with_validation()
            .with_strict_codegen(),
    );
    let module = cg
        .compile_module_with_mounts(
            &Parser::new(&source).parse_module().unwrap(),
            f.0.join("main.vr").to_str().unwrap(),
            f.0.join("core").to_str().unwrap(),
        )
        .unwrap();
    (module, cg)
}
#[test]
fn headerless_directory_module_keeps_directory_owner() {
    let (m, cg) = graph(
        &[
            ("origin/mod.vr", "public type Parcel is {value:Int};"),
            (
                "bridge.vr",
                "mount core.origin.Parcel; public fn relay(p:Parcel)->Parcel{p}",
            ),
        ],
        "mount core.bridge;",
    );
    assert!(cg.export_functions().contains_key("core.bridge.relay"));
    let p = m
        .types
        .iter()
        .find(|t| m.strings.get(t.name) == Some("Parcel"))
        .unwrap();
    assert_eq!(
        p.origin_module.and_then(|id| m.strings.get(id)),
        Some("core.origin")
    );
}
#[test]
fn headerless_super_imports_keep_two_nested_same_leaf_owners() {
    let files = [
        ("north/token.vr", "public type Parcel is {value:Int};"),
        ("south/token.vr", "public type Parcel is {flag:Bool};"),
        (
            "north/bridge.vr",
            "mount super.token.Parcel; public fn relay(p:Parcel)->Parcel{p}",
        ),
        (
            "south/bridge.vr",
            "mount super.token.Parcel; public fn relay(p:Parcel)->Parcel{p}",
        ),
    ];
    for reverse in [false, true] {
        let mounts = if reverse {
            "mount core.south.bridge as South; mount core.north.bridge as North;"
        } else {
            "mount core.north.bridge as North; mount core.south.bridge as South;"
        };
        let (m, cg) = graph(&files, mounts);
        let north = m
            .types
            .iter()
            .find(|t| {
                m.strings.get(t.name) == Some("Parcel")
                    && t.origin_module.and_then(|id| m.strings.get(id)) == Some("core.north.token")
            })
            .unwrap();
        let south = m
            .types
            .iter()
            .find(|t| {
                m.strings.get(t.name) == Some("Parcel")
                    && t.origin_module.and_then(|id| m.strings.get(id)) == Some("core.south.token")
            })
            .unwrap();
        assert_ne!(north.id, south.id);
        let exported = cg.export_functions();
        let n = exported.get("core.north.bridge.relay").unwrap();
        let s = exported.get("core.south.bridge.relay").unwrap();
        assert_ne!(n.id, s.id);
        for m in [
            m.clone(),
            verum_vbc::deserialize::deserialize_module(
                &verum_vbc::serialize::serialize_module(&m).unwrap(),
            )
            .unwrap(),
        ] {
            verum_vbc::validate::validate_module(&m).unwrap();
            let id = m.find_function_by_name("fixture.probe").unwrap();
            assert_eq!(
                Interpreter::new(Arc::new(m))
                    .execute_function(id)
                    .unwrap()
                    .try_as_i64(),
                Some(42)
            );
        }
    }
}
#[test]
fn headerless_self_import_resolves_own_declared_type() {
    let (m, cg) = graph(
        &[(
            "leaf.vr",
            "public type Parcel is {value:Int}; mount self.Parcel as OwnParcel; public fn relay(p:OwnParcel)->Parcel{p}",
        )],
        "mount core.leaf;",
    );
    assert!(cg.export_functions().contains_key("core.leaf.relay"));
    let p = m
        .types
        .iter()
        .find(|t| m.strings.get(t.name) == Some("Parcel"))
        .unwrap();
    assert_eq!(
        p.origin_module.and_then(|id| m.strings.get(id)),
        Some("core.leaf")
    );
}
#[test]
fn explicit_header_remains_authoritative_when_file_stem_differs() {
    let (m, cg) = graph(
        &[(
            "family/carrier.vr",
            "module renamed; public type Parcel is {value:Int}; public fn relay(p:Parcel)->Parcel{p}",
        )],
        "mount core.family.*;",
    );
    assert!(
        cg.export_functions()
            .contains_key("core.family.renamed.relay")
    );
    let p = m
        .types
        .iter()
        .find(|t| m.strings.get(t.name) == Some("Parcel"))
        .unwrap();
    assert_eq!(
        p.origin_module.and_then(|id| m.strings.get(id)),
        Some("core.family.renamed")
    );
}

#[test]
fn explicit_header_self_import_resolves_own_declared_type() {
    let (m, cg) = graph(
        &[(
            "leaf.vr",
            "module core.leaf; public type Parcel is {value:Int}; mount self.Parcel as OwnParcel; public fn relay(p:OwnParcel)->Parcel{p}",
        )],
        "mount core.leaf;",
    );
    assert!(cg.export_functions().contains_key("core.leaf.relay"));
    let p = m
        .types
        .iter()
        .find(|t| m.strings.get(t.name) == Some("Parcel"))
        .unwrap();
    assert_eq!(
        p.origin_module.and_then(|id| m.strings.get(id)),
        Some("core.leaf")
    );
}
#[test]
fn self_import_does_not_claim_a_missing_member() {
    let f = Fixture::new();
    f.write("core/mod.vr", "module core;");
    f.write("core/prelude/mod.vr", "module core.prelude;");
    f.write("core/leaf.vr","public type Parcel is {value:Int}; mount self.Missing as OwnParcel; public fn relay(p:OwnParcel)->Parcel{p}");
    let source = "module fixture; mount core.leaf; public fn probe()->Int{42}";
    f.write("main.vr", source);
    assert!(
        VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation())
            .compile_module_with_mounts(
                &Parser::new(source).parse_module().unwrap(),
                f.0.join("main.vr").to_str().unwrap(),
                f.0.join("core").to_str().unwrap()
            )
            .is_err()
    );
}

#[test]
fn headerless_file_owns_inline_glob_exports() {
    let (module, _) = graph(
        &[
            ("origin/token.vr", "public type Parcel is {value:Int};"),
            (
                "facade.vr",
                "public module exports { public mount core.origin.token.Parcel; }",
            ),
        ],
        "mount core.facade.exports.*; public fn carry(p:Parcel)->Parcel{p}",
    );
    let parcel = module
        .types
        .iter()
        .find(|t| {
            module.strings.get(t.name) == Some("Parcel")
                && t.origin_module.and_then(|id| module.strings.get(id))
                    == Some("core.origin.token")
        })
        .unwrap();
    let carry = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("fixture.carry"))
        .unwrap();
    assert_eq!(carry.params[0].type_ref.base_type_id(), Some(parcel.id));
    assert_eq!(carry.return_type.base_type_id(), Some(parcel.id));
}
#[test]
fn self_value_receiver_keeps_expression_semantics() {
    let source = "module fixture; type Wrapper is {value:Int}; implement Wrapper { public fn read(&self)->Int {self.value} } public fn probe()->Int{let value=Wrapper{value:42};value.read()}";
    let module = VbcCodegen::with_config(CodegenConfig::new("fixture").with_validation())
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module.find_function_by_name("fixture.probe").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(42)
        );
    }
}
