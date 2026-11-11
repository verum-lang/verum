//! Exercise the packaged source loader and its early inherent-method path.
//! The small independent library is a language regression, not a stdlib bake.
use std::{fs, process::Command};
use tempfile::TempDir;
use verum_common::Text;

fn check(extra_producer: &str, extra_handle: &str, imports: &str, body: &str, result: &str) {
    check_payload(extra_producer, extra_handle, imports, body, result, r#"
        @no_implicit_prelude
        module core.returned;
        public type Receipt<T> is { data:T };
        implement<T> Receipt<T> { public fn take(self) -> T { self.data } }
        public fn receipt<T>(data:T) -> Receipt<T> { Receipt {data} }
    "#);
}

fn check_payload(
    extra_producer: &str,
    extra_handle: &str,
    imports: &str,
    body: &str,
    result: &str,
    returned: &str,
) {
    let root = TempDir::new().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    for (path, source) in [
        ("mod.vr", "module core; public module prelude {} public module producer; public module handle; public module owner; public module returned;"),
        ("producer.vr", r#"
            @no_implicit_prelude
            module core.producer;
            public type Produce is protocol { type Output; fn produce(self) -> Self.Output; };
            public type Ready<T> is { data:T };
            implement<T> Produce for Ready<T> { type Output = T; fn produce(self) -> T { self.data } }
            public fn ready<T>(data:T) -> Ready<T> { Ready {data} }
        "#),
        ("handle.vr", r#"
            @no_implicit_prelude
            module core.handle;
            mount core.producer.Produce;
            mount core.returned.{Receipt,receipt};
            public type Handle<T> is { data:T };
            implement<T> Produce for Handle<T> {
                type Output = Receipt<T>;
                fn produce(self) -> Receipt<T> { receipt(self.data) }
            }
            public fn handle<T>(data:T) -> Handle<T> { Handle {data} }
        "#),
        ("owner.vr", r#"
            @no_implicit_prelude
            module core.owner;
            mount core.producer.Produce;
            mount core.handle.{Handle,handle};
            public type Driver is { tag:Int };
            implement Driver {
                public fn spawn<F:Produce>(&self, value:F) -> Handle<F.Output> { handle(value.produce()) }
                public fn drive<F:Produce>(&self, value:F) -> F.Output { value.produce() }
            }
            public fn driver() -> Driver { Driver {tag:1} }
        "#),
        ("returned.vr", returned),
    ] {
        let extra = match path { "producer.vr" => extra_producer, "handle.vr" => extra_handle, _ => "" };
        fs::write(core.join(path), format!("{source}\n{extra}")).unwrap();
    }
    let entry = root.path().join("main.vr");
    fs::write(&entry, format!(r#"
        @no_implicit_prelude
        module app.main;
        mount core.owner.driver;
        mount core.producer.ready;
        {imports}
        fn main() -> {result} {{ let rt = driver(); {body} }}
    "#)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("check").arg(entry)
        .current_dir(root.path())
        .env("VERUM_STDLIB_PATH", core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .env("VERUM_NO_OBJECT_CACHE", "1")
        .env("NO_COLOR", "1")
        .output().unwrap();
    let diagnostics = format!("{}{}", Text::from_utf8_lossy(&output.stdout), Text::from_utf8_lossy(&output.stderr));
    if result == "Int" {
        assert!(output.status.success(), "{diagnostics}");
    } else {
        assert!(!output.status.success(), "foreign associated result was accepted: {diagnostics}");
        assert!(diagnostics.contains("error<E"), "{diagnostics}");
    }
}

#[test]
fn an_associated_sum_keeps_its_nominal_methods_across_modules() {
    let returned = r#"
        @no_implicit_prelude
        module core.returned;
        public type Receipt<T> is | Value(T);
        implement<T> Receipt<T> {
            public fn take(self) -> T { match self { Value(value) => value } }
        }
        public fn receipt<T>(data:T) -> Receipt<T> { Receipt.Value(data) }
    "#;
    for imports in ["", "mount core.returned.Receipt; mount core.handle.Handle;"] {
        check_payload(
            "", "", imports,
            "let h = rt.spawn(ready(42)); rt.drive(h).take()", "Int", returned,
        );
    }
}

#[test]
fn imported_and_transitive_outputs_keep_the_declaring_bound() {
    for (producer, handle) in [
        ("", ""),
        ("public type IntoProduce is protocol { type Output; fn finish(self)->Self.Output; }; implement<F:Produce> IntoProduce for F { type Output = F.Output; fn finish(self)->F.Output { self.produce() } }", ""),
        ("", "public type Other is protocol { type Output; }; implement<T> Other for Handle<T> { type Output = Bool; }"),
    ] {
        check(producer, handle, "", "let h = rt.spawn(ready(42)); rt.drive(h).take()", "Int");
        check(producer, handle,
            "mount core.handle.Handle; mount core.producer.Produce; mount core.returned.Receipt;",
            "let h:Handle<Int> = rt.spawn(ready(42)); rt.drive(h).take()", "Int");
    }
}

#[test]
fn an_unrelated_associated_result_cannot_satisfy_the_return_type() {
    check("", "public type Other is protocol { type Output; }; implement<T> Other for Handle<T> { type Output = Bool; }",
        "mount core.handle.Handle;", "let h:Handle<Int> = rt.spawn(ready(42)); rt.drive(h)", "Bool");
}
