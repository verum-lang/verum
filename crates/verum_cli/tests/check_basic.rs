//! `verum check` end-to-end exit-code + diagnostic contract.
//!
//! `verum check` runs lex → parse → type-check on the project,
//! skipping VBC/LLVM codegen. The CLI contract this test pins down:
//!
//!  * Clean project → exit 0, no diagnostic-formatted output on stderr.
//!  * Parse error → exit non-zero, stderr identifies the offending file.
//!  * Unknown identifier (post-parse type-check) → exit non-zero,
//!  stderr surfaces the failing name.
//!
//! Pre-commit hooks, CI gates, and IDE "save and check" workflows rely
//! on the exit code being a binary signal — locking it down is the
//! same hardening contract `fmt --check` follows.
//!
//! Test isolation: each fixture is built in `$TMPDIR/verum_check_*`
//! with a dedicated PID-suffixed name to avoid cross-test contamination
//! when run in parallel.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_check_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[cog]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    std::fs::write(dir.join("src").join("main.vr"), body).expect("main.vr");
    dir
}

#[test]
fn check_exits_zero_on_valid_program() {
    // Smallest valid Verum program: an empty main(). Lex + parse +
    // type-check all succeed, no diagnostics.
    let dir = make_fixture("valid", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        out.status.success(),
        "verum check should exit 0 on a valid program. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_parse_error() {
    // Unbalanced braces — the lexer accepts the bytes, the parser
    // fails at end-of-input expecting `}`.
    let dir = make_fixture("parse_err", "fn main() { let x = ;\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on a parse error. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Diagnostic should mention the offending file — accept either the
    // full basename `main.vr` or the bare module name `main` (the
    // location formatter renders `at <module>:<line>:<col>` which
    // identifies the file via its module name).
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("main.vr") || combined.contains("main:"),
        "parse error diagnostic should reference main.vr or main:; got: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_unknown_identifier() {
    // Reference an undeclared name. Parse succeeds; type-check fails.
    // This pins down that the type-check phase actually runs (not just
    // a lex-only fast path).
    let dir = make_fixture(
        "unknown_id",
        "fn main() { let x = some_undeclared_name; }\n",
    );
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on an unknown identifier. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_does_not_emit_codegen_artifacts() {
    // `verum check` must not emit a binary or VBC bundle. Pre-existing
    // contract: the fast-path is type-only, so the build directory
    // should be absent (or at least without compiled artefacts).
    let dir = make_fixture("no_artifacts", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(out.status.success());

    // No binary in any of the conventional output locations.
    for candidate in &["target/debug/no_artifacts", "build/no_artifacts"] {
        let p = dir.join(candidate);
        assert!(
            !p.exists(),
            "verum check should NOT produce a compiled binary at {p:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// Source comments describe VCS expectations only to the VCS runner. Ordinary
// checking reports the actual selected phase, including in editor/agent jobs.
#[test]
fn source_directives_cannot_skip_or_invert_parse_failures() {
    for (index, header) in [
        "// @skip: fixture",
        "// @expect: errors",
        "// @test: parse-fail",
        "// @test: typecheck-fail",
    ]
    .iter()
    .enumerate()
    {
        let dir = make_fixture(
            &format!("directive_parse_{index}"),
            &format!("{header}\nfn main() {{ let x = ;\n"),
        );
        let out = Command::new(binary())
            .args(["check", "src/main.vr", "--parse-only"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "{header} suppressed a parse error: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn negative_test_comments_do_not_invert_a_successful_parse() {
    let dir = make_fixture(
        "directive_valid",
        "// @test: parse-fail\n// @expect: errors\nfn main() {}\n",
    );
    let out = Command::new(binary())
        .args(["check", "src/main.vr", "--parse-only"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "comments changed a successful parse: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_the_explicit_parse_option_can_omit_type_checking() {
    let dir = make_fixture(
        "directive_phase",
        "// @test: parse-pass\nfn main() { let x = undeclared_check_sentinel; }\n",
    );
    let checked = Command::new(binary())
        .args(["check", "src/main.vr"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        !checked.status.success(),
        "source comments weakened checking: {} {}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("undeclared_check_sentinel"),
        "expected the actual type-check diagnostic: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let parsed = Command::new(binary())
        .args(["check", "src/main.vr", "--parse-only"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        parsed.status.success(),
        "explicit parse-only mode must retain its documented meaning"
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn write_source_core(root: &Path, modules: &[(&str, &str)]) -> PathBuf {
    let core = root.join("core");
    std::fs::create_dir_all(core.join("base")).unwrap();
    let mut declarations =
        String::from("module core; public module base; public module prelude {}");
    for (name, source) in modules {
        declarations.push_str(&format!(" public module {name};"));
        std::fs::write(core.join(format!("{name}.vr")), source).unwrap();
    }
    std::fs::write(core.join("mod.vr"), declarations).unwrap();
    std::fs::write(
        core.join("base/mod.vr"),
        "module core.base; public module protocols;",
    )
    .unwrap();
    std::fs::write(
        core.join("base/protocols.vr"),
        "module core.base.protocols;",
    )
    .unwrap();
    core
}

fn check_with_source_core(core: &Path, entry: &Path, source: &str) -> Output {
    let mut command = Command::new(binary());
    command.arg("check");
    if entry.is_dir() {
        std::fs::write(entry.join("src/main.vr"), source).unwrap();
        command.current_dir(entry);
    } else {
        std::fs::write(entry, source).unwrap();
        command.arg(entry).current_dir(entry.parent().unwrap());
    }
    command
        .env("VERUM_STDLIB_PATH", core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .output()
        .unwrap()
}

#[test]
fn glob_imports_preserve_generic_declarations_when_arguments_are_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let core = write_source_core(root, &[(
        "source",
        "module core.source;
         public type Parcel<T> is { public item: T };
         implement<T> Parcel<T> {
             public fn fetch(&self)->T {self.item}
             fn hidden(&self)->T {self.item}
         }
         public type Secret<T> is { public value: T };
         type Internal<T> is { public value: T };",
    )]);
    std::fs::write(
        core.join("mod.vr"),
        "module core; public module base; public module source;
         public module prelude {
             public mount super.source.{Parcel, Parcel as Package};
             mount super.source.Secret;
         }",
    )
    .unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("verum.toml"), "[cog]\nname='fixture'\nversion='0.1.0'\n").unwrap();
    let cases = [
        (
            "",
            "fn take(a: Package<Atom>, b: Parcel<Atom>)->Int {a.fetch() + b.fetch()}",
            None,
        ),
        (
            "mount core.prelude.*;",
            "fn take(a: Package<Atom>, b: Parcel<Atom>)->Int {a.fetch() + b.fetch()}",
            None,
        ),
        (
            "mount core.prelude.{Package as Delivery};",
            "fn take(a: Delivery<Atom>)->Int {a.fetch()}",
            None,
        ),
        (
            "",
            "type Parcel<T> is { public local: T };
             fn take(a: Parcel<Atom>, b: Package<Atom>)->Int {a.local + b.fetch()}",
            None,
        ),
        (
            "",
            "type Parcel<T> is { public local: T };
             fn wrong(a: Parcel<Atom>)->Int {a.fetch()}",
            Some("no method named `fetch`"),
        ),
        ("", "fn wrong(a: Package<Atom>)->Bool {a.fetch()}", Some("Type mismatch: expected 'Bool', found 'Int'")),
        ("", "fn wrong(a: Package<Atom>)->Int {a.hidden()}", Some("no method named `hidden`")),
        (
            "mount core.prelude.Missing;",
            "fn wrong(a: Missing<Atom>)->Int {0}",
            Some("Missing"),
        ),
        (
            "mount core.prelude.Secret;",
            "fn wrong(a: Secret<Atom>)->Int {0}",
            Some("cannot find `Secret` in module `core.prelude`"),
        ),
        (
            "",
            "fn wrong(a: core.source.Internal<Atom>)->Int {a.value}",
            Some("visibility error: 'Internal' is Private"),
        ),
        (
            "",
            "fn wrong(a: Package<Atom, Bool>)->Int {0}",
            Some("type `core.source.Parcel` takes 1 type argument(s), but 2 were supplied"),
        ),
        (
            "",
            "fn wrong(a: Parcel<Package<Atom>>)->Int {a.fetch()}",
            Some("Type mismatch: expected 'Int'"),
        ),
    ];
    for (mount, body, expected) in cases {
        let source = format!("module fixture.main; {mount} type Atom is Int; {body} fn main() {{}}");
        let out = check_with_source_core(&core, &project, &source);
        let log = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.success(), expected.is_none(), "{source}\n{log}");
        if let Some(needle) = expected {
            assert!(log.contains(needle), "{source}\n{log}");
        }
    }
}

#[test]
fn foreign_private_source_types_are_not_ambient_bindings() {
    let root = tempfile::tempdir().unwrap();
    let core = write_source_core(root.path(), &[(
        "source",
        "module core.source;
         type Hidden<T> is { public body: T };
         type PrivateCount is Int;
         type PrivateChoice is Stored(Int) | Reset;
         public(cog) type CogCount is Int;
         public type Carton<T> is { public item: T };
         public fn make(value: Int)->Hidden<Int> { Hidden { body: value } }
         public fn open(value: Hidden<Int>)->Int { value.body }",
    )]);
    std::fs::write(core.join("mod.vr"),
        "module core; public module base; public module source;
         public module prelude { public mount super.source.Carton as Published; }",
    ).unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("verum.toml"), "[cog]\nname='fixture'\nversion='0.1.0'\n").unwrap();
    for (body, diagnostic) in [
        ("mount core.source.{make, open}; fn value()->Int { open(make(7)) }", None),
        ("fn read(x: Published<Int>)->Int { x.item }", None),
        ("type Hidden<T> is { public local: T }; fn read(x: Hidden<Int>)->Int { x.local }", None),
        ("fn leak(x: Hidden<Int>)->Int { x.body }", Some("type not found: Hidden")),
        ("fn leak(x: PrivateCount)->Int { x }", Some("type not found: PrivateCount")),
        ("fn leak(x: PrivateChoice)->Int { 0 }", Some("type not found: PrivateChoice")),
        ("fn leak(x: CogCount)->Int { x }", Some("type not found: CogCount")),
        ("fn leak(x: core.source.Hidden<Int>)->Int { x.body }", Some("visibility error: 'Hidden' is Private")),
        ("mount core.source.Hidden; fn name()->Int { 1 }", Some("cannot find `Hidden`")),
        ("mount core.source.{make, open}; fn wrong()->Bool { open(make(7)) }", Some("expected 'Bool', found 'Int'")),
    ] {
        let source = format!("module fixture.main; {body} fn main() {{}}");
        let result = check_with_source_core(&core, &project, &source);
        let log = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.success(), diagnostic.is_none(), "{source}\n{log}");
        if let Some(expected) = diagnostic {
            assert!(log.contains(expected), "{source}\n{log}");
        }
    }
}

#[test]
fn hidden_source_case_names_require_their_declaring_scope() {
    let root = tempfile::tempdir().unwrap();
    let core = write_source_core(root.path(), &[(
        "ledger",
        "module core.ledger;
         type Receipt is Issued(Int) | Blank | Annotated { n:Int };
         public type Entry is Line(Int) | Empty | Detail { n:Int };
         public fn issue(n:Int)->Receipt { Issued(n) }
         public fn number(receipt:Receipt)->Int {
             match receipt { Issued(n)=>n, Annotated {n}=>n, _=>0 }
         }",
    )]);
    let project = root.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("verum.toml"), "[cog]\nname='fixture'\nversion='0.1.0'\n").unwrap();
    for (declarations, body, error) in [
        ("", "let n=number(issue(7));", None),
        ("mount core.ledger.Entry as Visible;", "let x=Visible.Line(7);", None),
        ("mount core.ledger.Entry;", "let f=core.ledger.Entry.Line; let x=f(7);", None),
        ("mount core.ledger.Entry;", "let x=core.ledger.Entry.Detail { n:7 };", None),
        ("mount core.ledger.Entry as Visible;", "let x:Visible=Visible.Detail { n:7 };", None),
        ("type Local is Issued(Bool) | Here;", "let x=Local.Issued(true);", None),
        ("", "let x=Issued(7);", Some("Issued")),
        ("", "let x=core.ledger.Receipt.Issued(7);", Some("Private")),
        ("", "let x=core.ledger.Receipt.Issued; let y=x(7);", Some("Receipt")),
        ("", "let x=Blank;", Some("Blank")),
        ("", "let x=Annotated { n:7 };", Some("Annotated")),
        ("", "let x=core.ledger.Receipt.Annotated { n:7 };", Some("Private")),
        ("", "let n=number(core.ledger.Receipt.Annotated { n:7 });", Some("Private")),
        ("fn invalid(x:Blank)->Int {0}", "", Some("Blank")),
        ("", "let n=number(Blank);", Some("Blank")),
        ("", "let n=number(Issued(7));", Some("Issued")),
        ("", "let r=issue(7); let v=match r { Issued(n)=>n, _=>0 };", Some("Issued")),
        ("", "let r=issue(7); let v=match r { core.ledger.Receipt.Issued(n)=>n, _=>0 };", Some("Issued")),
        ("", "let r=issue(7); let v=match r { Annotated {n}=>n, _=>0 };", Some("Annotated")),
        ("", "let r=issue(7); let v=match &r { Issued(n)=>0, _=>1 };", Some("Issued")),
        ("mount core.ledger.Entry as Visible;", "let x=Visible.Line(true);", Some("expected 'Int', found 'Bool'")),
        ("mount core.ledger.Entry;", "let x=core.ledger.Entry.Detail { n:true };", Some("expected 'Int', found 'Bool'")),
    ] {
        let source = format!(
            "module fixture.main; mount core.ledger.{{issue, number}}; {declarations} fn main() {{ {body} }}"
        );
        let result = check_with_source_core(&core, &project, &source);
        let log = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.success(), error.is_none(), "{source}\n{log}");
        if let Some(error) = error {
            assert!(log.lines().filter(|line| line.starts_with("error<")).any(|line| line.contains(error)),
                "{source}\n{log}");
        }
    }
}

#[test]
fn recursive_protocol_bounds_use_the_selected_source_parameters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let entry = root.join("main.vr");
    let core = write_source_core(
        root,
        &[
            ("source", ""),
            ("foreign", "module core.foreign; public type Feed is { value: Int };"),
        ],
    );
    let feed = "public type Feed<T> is protocol {
        type Item;
        type Output: Pipeline<Item = Self.Item>;
        fn produce(self) -> Self.Output;
    };";
    for (application, diagnostic) in [
        ("Feed<Int, Output = U>", None),
        ("Feed<Output = U>", Some("missing type argument")),
        ("Feed<3, Output = U>", Some("requires a type argument")),
        ("Missing<Output = U>", Some("type not found: Missing")),
        ("Foreign<Output = U>", Some("core.foreign.Feed")),
    ] {
        let pipeline = format!(
            "public type Pipeline is protocol {{
            type Item;
            fn flatten<U: Pipeline>(self) -> Int where Self.Item: {application} {{ 7 }}
        }};"
        );
        for declarations in [format!("{feed}\n{pipeline}"), format!("{pipeline}\n{feed}")] {
            std::fs::write(
                core.join("source.vr"),
                format!("module core.source; mount core.foreign.Feed as Foreign;\n{declarations}"),
            )
            .unwrap();
            let out = check_with_source_core(&core, &entry, "mount core.source.Feed; fn main() {}");
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(out.status.success(), diagnostic.is_none(), "{application}: {log}");
            if let Some(diagnostic) = diagnostic {
                assert!(log.contains(diagnostic), "{application}: {log}");
            }
        }
    }
}

#[test]
fn protocol_signatures_resolve_the_entry_files_source_mounts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let core = write_source_core(
        root,
        &[(
            "source_provider",
            "module core.source_provider; public type EntryTag is Ready | Busy; type PrivateTag is Hidden;",
        )],
    );
    let project = root.join("app");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("verum.toml"),
        "[cog]\nname = \"entry_scope\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    )
    .unwrap();

    let standalone = root.join("standalone.vr");
    let entry = project.join("src/main.vr");
    for name in ["EntryTag", "core.source_provider.EntryTag"] {
        let source = format!(
            "mount core.source_provider.{{EntryTag, Ready}};\n\
             type Provider is protocol {{ fn tag(&self) -> {name}; }};\n\
             type Peer is {{ count: Int }};\n\
             implement Provider for Peer {{ fn tag(&self) -> {name} {{ Ready }} }}\n\
             fn main() {{}}"
        );
        for path in [&standalone, &entry] {
            let output = check_with_source_core(&core, path, &source);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let hidden = check_with_source_core(
        &core,
        &standalone,
        "mount core.source_provider.EntryTag; type Provider is protocol { fn tag(&self) -> core.source_provider.PrivateTag; }; fn main() {}",
    );
    let error = String::from_utf8_lossy(&hidden.stderr);
    assert!(!hidden.status.success());
    assert!(
        error.contains("PrivateTag") && error.contains("visibility error"),
        "{error}"
    );
}

#[test]
fn inherent_signatures_resolve_types_in_their_defining_source_module() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let events = r#"
        module core.events;
        public type Event<T> is { public content: T };
        public type Closed is { public bytes: Int };
    "#;
    let worker = r#"
        module core.worker;
        mount core.events.Event as ImportedEvent;
        public type Outcome<T> is { public sample: T };
        public type Failure is Closed;
        public type Worker<T> is { public value: T };
        implement<T> Worker<T> {
            public fn failure(&self) -> Failure { Closed }
            public fn local(&self, x: T) -> Outcome<T> { Outcome { sample: x } }
            public fn foreign(&self, x: T) -> ImportedEvent<T> { ImportedEvent { content: x } }
        }
    "#;
    let user = r#"
        mount core.events.Closed;
        mount core.worker.Worker;
        type Outcome<T> is { mistaken: T };
        type ImportedEvent<T> is { mistaken: T };
        type ConsumerOnly<T> is { content: T };
        fn main() {
            let worker = Worker { value: 17 };
            let failure = worker.failure();
            assert_eq(worker.local(31).sample, 31);
            assert_eq(worker.foreign(37).content, 37);
        }
    "#;
    let core = write_source_core(root, &[("events", events), ("worker", worker)]);
    let project = root.join("app");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("verum.toml"),
        "[cog]\nname = \"signature_scope\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    )
    .unwrap();
    let entry = project.join("src/main.vr");
    for path in [&project, &entry] {
        let checked = check_with_source_core(&core, path, user);
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }

    let invalid_worker = worker.replace("-> ImportedEvent<T>", "-> ConsumerOnly<T>");
    let worker_path = core.join("worker.vr");
    std::fs::write(&worker_path, invalid_worker).unwrap();
    for path in [&project, &entry] {
        let missing = check_with_source_core(&core, path, user);
        let error = String::from_utf8_lossy(&missing.stderr);
        assert!(!missing.status.success());
        assert!(
            error.contains("type not found: ConsumerOnly")
                && error.contains(worker_path.to_str().unwrap()),
            "{error}"
        );
    }
}

#[test]
fn inherent_method_conditions_cover_signatures_and_calls() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let core = write_source_core(root, &[("model", "module core.model;")]);
    let header = r#"
        module core.model;
        public type Ordered is protocol { fn rank(&self) -> Int; };
        public type Key is { public number: Int };
        public type Unordered is { public number: Int };
        implement Ordered for Key { fn rank(&self) -> Int { self.number } }
        public type Table<K: Ordered, V> is { public key: K, public value: V };
    "#;
    let methods = r#"
        public fn flip(self) -> Table<V,K> where V: Ordered { Table { key: self.value, value: self.key } }
        public fn exchange(pair: Table<K,V>) -> Table<V,K> where V: Ordered { Table { key: pair.value, value: pair.key } }
        public fn replace<W: Ordered>(&self, value: W) -> Table<W,K> { Table { key: value, value: self.key } }
        public fn marker(&self) -> Int where V: Ordered { 31 }
        public fn stamp(pair: Table<K,V>) -> Int where V: Ordered { 31 }
    "#;
    let entry = root.join("main.vr");
    let check = |members: &str, value: &str, body: &str| {
        std::fs::write(
            core.join("model.vr"),
            format!("{header}\nimplement<K: Ordered,V> Table<K,V> {{ {members} }}"),
        )
        .unwrap();
        let user = format!(
            "mount core.model.{{Key, Unordered, Table}}; fn main() {{\n\
             let pair = Table {{ key: Key {{ number: 1 }}, value: {value} {{ number: 2 }} }};\n\
             {body} }}"
        );
        check_with_source_core(&core, &entry, &user)
    };
    for body in [
        "assert_eq(pair.flip().key.number, 2);",
        "assert_eq(Table.exchange(pair).key.number, 2);",
        "assert_eq(pair.replace(Key { number: 8 }).key.number, 8);",
        "assert_eq(Table<Key,Key>.stamp(pair), 31);",
    ] {
        let output = check(methods, "Key", body);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for body in [
        "assert_eq(pair.marker(), 31);",
        "assert_eq(Table.stamp(pair), 31);",
        "assert_eq(Table<Key,Unordered>.stamp(pair), 31);",
    ] {
        let output = check(methods, "Unordered", body);
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "accepted {body} for an unordered value"
        );
        assert!(diagnostics.contains("Ordered"), "{diagnostics}");
    }
    // An adjacent method cannot borrow the gated member's premise.
    let missing = format!(
        "{methods}\npublic fn unchecked(self) -> Table<V,K> {{ Table {{ key: self.value, value: self.key }} }}"
    );
    let output = check(&missing, "Key", "assert_eq(pair.marker(), 31);");
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        diagnostics.contains("Ordered") && diagnostics.contains("unchecked"),
        "{diagnostics}"
    );
}

#[test]
fn providers_authorize_imported_calls_in_their_lexical_scope() {
    let directory = tempfile::tempdir().unwrap();
    let core = write_source_core(directory.path(), &[]);
    let project = directory.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("verum.toml"), "[cog]\nname='fixture'\nversion='0.1.0'\n").unwrap();
    std::fs::write(
        project.join("src/service.vr"),
        "module fixture.service;
         public context RepositoryGate { fn read() -> Int; }
         public type Device is { public value: Int };
         implement Device { public fn read(&self) -> Int { self.value } }
         public fn snapshot() -> Int using [RepositoryGate] { RepositoryGate.read() }",
    ).unwrap();
    let cases = [
        ("fn main() {provide RepositoryGate = Device {value: 7}; let n = fetch();}", None),
        ("fn main() {provide RepositoryGate = Device {value: 1} in {let n = fetch();}}", None),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            provide RepositoryGate = Device {value: 2} in {let n = fetch();} let m = fetch();}", None),
        ("fn forward() -> Int using [RepositoryGate] {fetch()}
          fn main() {provide RepositoryGate = Device {value: 7}; let n = forward();}", None),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            let call = fetch; let n = call();}", None),
        ("fn main() {let n = fetch();}", Some("error<E613>")),
        ("fn main() {provide RepositoryGate = Device {value: 1} in {let n = fetch();} let m = fetch();}", Some("error<E613>")),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            provide RepositoryGate = Device {value: 2};}", Some("already provided")),
        ("fn bad(backend: Device with [Read]) {provide RepositoryGate = backend; let n = fetch();}
          fn main() {}", Some("error<E411>")),
        ("type Limited is Device with [Read]; fn bad(backend: &Limited) {
            provide RepositoryGate = backend in {let n = fetch();}} fn main() {}", Some("error<E411>")),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            let limited = RepositoryGate.attenuate(Capability.ReadOnly);
            provide RepositoryGate = limited in {let n = fetch();}}", Some("error<E411>")),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            let limited = RepositoryGate.attenuate(Capability.ReadOnly); let n = fetch();}", Some("error<E0306>")),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            let limited = RepositoryGate.attenuate(Capability.ReadOnly);
            provide RepositoryGate = RepositoryGate in {let n = fetch();}}", Some("error<E411>")),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            let limited = RepositoryGate.attenuate(Capability.Admin);
            provide RepositoryGate = limited in {let n = fetch();} let m = fetch();}", None),
        ("fn main() {provide RepositoryGate = Device {value: 1};
            provide RepositoryGate = Device {value: 2} in {
                let limited = RepositoryGate.attenuate(Capability.ReadOnly);
            } let n = fetch();}", None),
    ];
    std::fs::write(project.join("src/consumer.vr"), "module fixture.consumer;
        mount fixture.service.{RepositoryGate as Backend, snapshot};
        public fn relay()->Int using [Backend] {snapshot()}").unwrap();
    for (body, accepted) in [
        ("mount fixture.service.{RepositoryGate as Backend, Device, snapshot};
          fn main() {provide Backend = Device {value: 7}; let n = snapshot();}", true),
        ("mount fixture.service.{RepositoryGate as Backend, Device}; mount fixture.consumer.relay;
          fn main() {provide Backend = Device {value: 7}; let n = relay();}", true),
        ("mount fixture.service.{RepositoryGate as Backend, Device, snapshot};
          context RepositoryGate {fn read()->Int;}
          fn main() {provide RepositoryGate = Device {value: 7}; let n = snapshot();}", false),
    ] {
        let output = check_with_source_core(&core, &project, &format!("module fixture.main; {body}"));
        let errors = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), accepted, "{body}: {errors}");
        if !accepted {
            assert!(errors.contains("error<E613>") && errors.contains("RepositoryGate"), "{errors}");
        }
    }
    for (body, expected) in cases {
        let source = format!("module fixture.main; mount fixture.service.{{RepositoryGate, Device, snapshot as fetch}}; {body}");
        let output = check_with_source_core(&core, &project, &source);
        let errors = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), expected.is_none(), "{body}: {errors}");
        if let Some(expected) = expected {
            assert!(errors.contains(expected), "{body}: {errors}");
            assert!(!errors.contains("error<E0308>"), "{body}: {errors}");
        }
    }
}
