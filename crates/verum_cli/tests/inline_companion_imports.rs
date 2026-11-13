//! Check a source file through the same public command used by an editor or a user.
use std::process::Command;
use verum_common::Text;

const FAMILY: &str = r#"module factory.family;
public type Parcel<T> is { public item: T };
public module Parcel {
    mount super.{Batch as Shipment};
    public fn wrap<T>(item: T) -> Parcel<T> { Parcel { item: item } }
    public fn pair<T>(item: T) -> Shipment<T> { Shipment { item: item } }
    fn hidden() -> Int { 0 }
}
public type Batch<T> is { public item: T };
"#;

fn check(family: &str, consumer: &str) -> (bool, Text) {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    let core = root.path().join("core");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module prelude {}").unwrap();
    std::fs::write(
        root.path().join("verum.toml"),
        "[cog]\nname = \"factory\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src.join("family.vr"), family).unwrap();
    std::fs::write(
        src.join("main.vr"),
        format!("module factory.main;\n{consumer}"),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_verum"))
        .current_dir(root.path())
        .env("VERUM_STDLIB_PATH", &core)
        .args(["check", "src/main.vr"])
        .output()
        .unwrap();
    (
        result.status.success(),
        Text::from_utf8_lossy(&result.stderr),
    )
}

#[test]
fn inline_companion_glob_keeps_nominal_results_and_polymorphic_calls() {
    for source in [
        "mount cog.family.Parcel; fn main() -> Int { Parcel.wrap(73).item }",
        "mount cog.family.Parcel.*; fn main() -> Int { let b: Bool = pair(true).item; pair(73).item }",
        "mount cog.family.{Parcel as Factory}; fn main() -> Int { Factory.pair(73).item }",
        "mount cog.family.Parcel; fn typed() -> Parcel<Int> { Parcel.wrap(73) }",
        "mount cog.family.{Parcel as Factory}; fn typed() -> Factory<Int> { Factory.wrap(73) }",
        "mount cog.family.{Parcel as Factory}; type Sample is { public n:Int }; fn typed(x: Sample) -> Sample { Factory.wrap<Sample>(x).item }",
    ] {
        let (admitted, errors) = check(FAMILY, source);
        assert!(admitted, "{source}\n{errors}");
    }
}

#[test]
fn inline_companion_glob_preserves_foreign_nominals_and_private_boundaries() {
    for (source, code, name) in [
        (
            "mount cog.family.Parcel.*; type Batch<T> is { public item: T }; fn wrong() -> Batch<Int> { pair(73) }",
            "E400",
            "expected 'factory.main.Batch<Int>', found 'factory.family.Batch<Int>'",
        ),
        (
            "mount cog.family.Parcel.*; fn main() -> Int { hidden() }",
            "E100",
            "hidden",
        ),
        (
            "mount cog.family.{Parcel}; fn main() -> Int { Parcel.hidden() }",
            "E202",
            "hidden",
        ),
        (
            "mount cog.family.Parcel.*; fn bad(value: Shipment<Int>) -> Int { value.item }",
            "E101",
            "Shipment",
        ),
    ] {
        let (admitted, errors) = check(FAMILY, source);
        assert!(
            !admitted && errors.contains(code) && errors.contains(name),
            "{source}\n{errors}"
        );
    }
}

#[test]
fn public_type_and_private_companion_grant_separate_visibility() {
    let family = FAMILY.replace("public module Parcel", "module Parcel");
    let source = "mount cog.family.Parcel; fn item(x: Parcel<Int>) -> Int { x.item }";
    let (admitted, diagnostics) = check(&family, source);
    assert!(admitted, "{diagnostics}");
    for source in [
        "mount cog.family.Parcel; fn bad() -> Int { Parcel.wrap(73).item }",
        "mount cog.family.Parcel.*; fn bad() -> Int { wrap(73).item }",
    ] {
        let (admitted, diagnostics) = check(&family, source);
        assert!(
            !admitted && diagnostics.contains("Parcel"),
            "{source}\n{diagnostics}"
        );
    }
}
