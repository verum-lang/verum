//! Named imports establish lexical scope before signatures, in every source order.
#![cfg(feature = "codegen")]

use std::fs;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
};

fn graph(mount: &str, before: bool, entry_consumer: bool) -> Result<VbcModule, CodegenError> {
    let scratch = tempfile::tempdir().unwrap();
    let core = scratch.path().join("core");
    fs::create_dir_all(core.join("sync")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module sync;").unwrap();
    fs::write(
        core.join("sync/mod.vr"),
        "module core.sync; public module provider; public module consumer; public module other;",
    )
    .unwrap();
    fs::write(
        core.join("sync/provider.vr"),
        r#"
        module core.sync.provider;
        public type Outcome<T, E> is Ok(T) | Err(E);
        public type Poison<T> is { guard: T };
        public type LockResult<T> is Outcome<T, Poison<T>>;
    "#,
    )
    .unwrap();
    fs::write(
        core.join("sync/other.vr"),
        r#"
        module core.sync.other;
        public type LockResult<T> is { different: T };
    "#,
    )
    .unwrap();
    let declarations = r#"
        public type Holder is { value: LockResult<Int> };
        public fn echo(value: LockResult<Int>) -> LockResult<Int> { value }
        fn main() -> Int { 41 }
    "#;
    let consumer = if before {
        format!("{mount}\n{declarations}")
    } else {
        format!("{declarations}\n{mount}")
    };
    let consumer = format!("module core.sync.consumer;\n{consumer}");
    fs::write(core.join("sync/consumer.vr"), &consumer).unwrap();
    let root = "module core.root; mount core.sync.consumer.{Holder, echo}; fn main() -> Int { 41 }";
    fs::write(core.join("root.vr"), root).unwrap();
    let (source, owner, path) = if entry_consumer {
        (
            consumer.as_str(),
            "core.sync.consumer",
            core.join("sync/consumer.vr"),
        )
    } else {
        (root, "core.root", core.join("root.vr"))
    };
    let ast = Parser::new(source).parse_module().expect("fixture grammar");
    VbcCodegen::with_config(CodegenConfig::new(owner).with_validation()).compile_module_with_mounts(
        &ast,
        path.to_str().unwrap(),
        core.to_str().unwrap(),
    )
}

fn alias_owner_survives_wire(module: VbcModule) {
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let declared = |name, owner| {
        module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some(name)
                    && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
            })
            .unwrap()
    };
    let alias = declared("LockResult", "core.sync.provider");
    let outcome = declared("Outcome", "core.sync.provider");
    let poison = declared("Poison", "core.sync.provider");
    let parameter = TypeRef::Generic(alias.type_params[0].id);
    assert_eq!(
        alias.alias_target,
        Some(TypeRef::Instantiated {
            base: outcome.id,
            args: vec![
                parameter.clone(),
                TypeRef::Instantiated {
                    base: poison.id,
                    args: vec![parameter],
                }
            ],
        })
    );
    let holder = declared("Holder", "core.sync.consumer");
    assert_eq!(
        holder.fields[0].declaration_type,
        Some(TypeRef::Instantiated {
            base: alias.id,
            args: vec![TypeRef::Concrete(TypeId::INT)],
        })
    );
}

#[test]
fn generic_alias_mount_order_is_independent_of_visibility_path_and_entry() {
    for before in [true, false] {
        for public in ["", "public "] {
            for path in ["super.provider.LockResult", "core.sync.provider.LockResult"] {
                for entry_consumer in [true, false] {
                    let mount = format!("{public}mount {path};");
                    let module = graph(&mount, before, entry_consumer).unwrap_or_else(|error| {
                        panic!("before={before}, entry_consumer={entry_consumer}, {mount}: {error}")
                    });
                    alias_owner_survives_wire(module);
                }
            }
        }
    }
}

#[test]
fn grouped_renames_are_published_before_signatures() {
    for before in [true, false] {
        alias_owner_survives_wire(
            graph(
                "mount super.provider.{Outcome, LockResult as LockResult};",
                before,
                false,
            )
            .unwrap(),
        );
    }
}

#[test]
fn repeated_identical_mounts_are_idempotent() {
    alias_owner_survives_wire(
        graph(
            "mount super.provider.LockResult; public mount core.sync.provider.LockResult;",
            false,
            false,
        )
        .unwrap(),
    );
}

#[test]
fn missing_late_mount_cannot_capture_another_modules_bare_name() {
    for before in [true, false] {
        let error = graph(
            "mount super.provider.Outcome; mount super.missing.LockResult;",
            before,
            false,
        )
        .expect_err("a loaded namesake does not satisfy the selected missing path");
        assert!(error.to_string().contains("LockResult"), "{error}");
    }
}

#[test]
fn conflicting_mounts_refuse_both_orders() {
    for before in [true, false] {
        for reverse in [true, false] {
            let (first, second) = if reverse {
                ("other", "provider")
            } else {
                ("provider", "other")
            };
            let error = graph(
                &format!("mount super.{first}.LockResult; mount super.{second}.LockResult;"),
                before,
                false,
            )
            .expect_err("source order must not choose a conflicting nominal");
            assert!(error.to_string().contains("LockResult"), "{error}");
        }
    }
}

fn compile_inline(source: &str) -> Result<VbcModule, CodegenError> {
    let ast = Parser::new(source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new("main").with_validation()).compile_module(&ast)
}

#[test]
fn late_inline_import_uses_its_actual_lexical_owner() {
    let module = compile_inline(
        r#"
        module provider { public type Token is { value: Int }; }
        module consumer {
            type Holder is { token: Token };
            mount super.provider.Token;
        }
        fn main() -> Int { 41 }
    "#,
    )
    .unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let token = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Token")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("provider")
        })
        .unwrap();
    let holder = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Holder"))
        .unwrap();
    assert_eq!(
        holder.fields[0].declaration_type,
        Some(TypeRef::Concrete(token.id))
    );
}

#[test]
fn inactive_late_mount_cannot_supply_a_type() {
    let error = compile_inline(
        r#"
        module provider { public type Token is { value: Int }; }
        module consumer {
            type Holder is { token: Token };
            @cfg(any())
            mount super.provider.Token;
        }
    "#,
    )
    .expect_err("inactive mount grants no declaration authority");
    assert!(error.to_string().contains("Token"), "{error}");
}
