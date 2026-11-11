//! Mixed sum telescopes through the packaged compiler's module pipeline.
use std::{fs, process::Command};
use tempfile::TempDir;

fn check(body: &str, accepted: bool, diagnostic: &str) {
    let root = TempDir::new().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(
        core.join("mod.vr"),
        "module core; public module prelude {} public module indexed;",
    )
    .unwrap();
    fs::write(
        core.join("indexed.vr"),
        r#"
        @no_implicit_prelude
        module core.indexed;
        public type Cell<N: meta USize> is { public value: Int };
        public type Envelope<N: meta USize, T> is | Data(Cell<N>, T);
        public type Sized<N: meta USize {it == 4}> is | Item(Int);
        public type Projector<k: meta USize {k > 0}> is { public value: Int };
    "#,
    )
    .unwrap();
    let entry = root.path().join("main.vr");
    fs::write(
        &entry,
        format!(
            r#"
        @no_implicit_prelude
        module app.main;
        mount core.indexed.{{Cell, Envelope, Sized, Projector}};
        {body}
        fn main() {{}}
    "#
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("check")
        .arg(&entry)
        .current_dir(root.path())
        .env("VERUM_STDLIB_PATH", &core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .env("VERUM_NO_OBJECT_CACHE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), accepted, "{body}\n{messages}");
    if !accepted {
        assert!(messages.contains(diagnostic), "{messages}");
    }
}

#[test]
fn imported_named_domains_keep_their_declaring_subject() {
    check(
        r#"
        fn retain<n: meta USize {n > 0}>(value: Projector<n>) -> Projector<n> { value }
        implement<n: meta USize {n > 0}> Projector<n> {
            fn copy(&self) -> Projector<n> { Projector {value: self.value} }
        }
        fn concrete(value: Projector<4>) {}
    "#,
        true,
        "",
    );
    check(
        "fn refused(value: Projector<0>) {}",
        false,
        "declared value domain",
    );
    check(
        "fn refused<n: meta USize>(value: Projector<n>) {}",
        false,
        "declared value domain",
    );
}

#[test]
fn imported_sum_values_keep_their_indices_through_construction_and_matching() {
    check(
        r#"
        fn pack<T>(cell: Cell<4>, value: T) -> Envelope<4, T> { Envelope.Data(cell, value) }
        fn rebuild<T>(value: Envelope<4, T>) -> Envelope<4, T> {
            match value { Envelope.Data(cell, item) => pack(cell, item) }
        }
        fn constrained(value: Sized<4>) {}
    "#,
        true,
        "",
    );
}

#[test]
fn imported_sum_refuses_a_wrong_index_even_with_generic_payloads() {
    check(
        r#"
        fn pack<T>(cell: Cell<8>, value: T) -> Envelope<4, T> { Envelope.Data(cell, value) }
    "#,
        false,
        "exact value index",
    );
    check("fn refused(value: Sized<8>) {}", false, "it == 4");
}
