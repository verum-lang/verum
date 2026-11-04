#![cfg(feature = "codegen")]
//! Check the real shell accessor's carrier; this is a signature gate, not file IO execution.
use std::path::PathBuf;
use verum_ast::{
    ItemKind, Module, Type,
    decl::{FunctionDecl, ImplItemKind, ImplKind},
    ty::{GenericArg, TypeKind},
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    types::{Mutability, TypeRef},
};

fn source(relative: &str) -> Module {
    let core = std::env::var_os("VERUM_TEST_SHELL_CORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../core"));
    Parser::new(&std::fs::read_to_string(core.join(relative)).unwrap())
        .parse_module()
        .unwrap()
}
fn nominal(ty: &Type) -> &str {
    let TypeKind::Path(path) = &ty.kind else {
        panic!("expected nominal: {ty:?}")
    };
    path.as_ident().unwrap().name.as_str()
}
fn application(ty: &Type) -> (&str, &Type) {
    let TypeKind::Generic { base, args } = &ty.kind else {
        panic!("expected application: {ty:?}")
    };
    assert_eq!(args.len(), 1);
    let GenericArg::Type(arg) = &args[0] else {
        panic!("expected type argument")
    };
    (nominal(base), arg)
}
fn method(module: &Module, owner: &str, name: &str) -> FunctionDecl {
    module
        .items
        .iter()
        .find_map(|item| {
            let ItemKind::Impl(decl) = &item.kind else {
                return None;
            };
            let ImplKind::Inherent(ty) = &decl.kind else {
                return None;
            };
            let target = match &ty.kind {
                TypeKind::Generic { base, .. } => nominal(base),
                _ => nominal(ty),
            };
            if target != owner {
                return None;
            }
            decl.items.iter().find_map(|item| match &item.kind {
                ImplItemKind::Function(f) if f.name.name.as_str() == name => Some(f.clone()),
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("missing actual {owner}.{name}"))
}
fn type_only(mut module: Module, name: &str) -> Module {
    module
        .items
        .retain(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.name.as_str()==name));
    assert_eq!(module.items.len(), 1);
    module
}
fn mutable_payload(ty: &Type) -> &Type {
    let TypeKind::Reference { mutable, inner } = &ty.kind else {
        panic!("expected reference: {ty:?}")
    };
    assert!(*mutable, "accessor must retain mutable access");
    inner
}
#[test]
fn actual_temp_file_result_matches_maybe_as_mut() {
    let accessor = method(&source("shell/resources.vr"), "TempFile", "file");
    let delegate = method(&source("base/maybe.vr"), "Maybe", "as_mut");
    let (carrier, payload) = application(accessor.return_type.as_ref().unwrap());
    let (delegate_carrier, delegate_payload) = application(delegate.return_type.as_ref().unwrap());
    assert_eq!(carrier, delegate_carrier);
    assert_eq!(carrier, "Maybe");
    assert_eq!(nominal(mutable_payload(payload)), "File");
    assert_eq!(nominal(mutable_payload(delegate_payload)), "T");
}
fn interface(return_type: Type) -> Result<verum_vbc::module::VbcModule, String> {
    // Reuse the actual declarations and accessor return AST. This probe only
    // isolates declaration/wire identity; it does not replace the resource implementation.
    let maybe = type_only(source("base/maybe.vr"), "Maybe");
    let file = type_only(source("io/file.vr"), "File");
    let mut probe=Parser::new("mount core.base.maybe.Maybe; mount core.io.file.File; public fn resource_result() -> () {}").parse_module().unwrap();
    let ItemKind::Function(function) = &mut probe.items.last_mut().unwrap().kind else {
        panic!()
    };
    function.return_type = verum_common::Maybe::Some(return_type);
    VbcCodegen::with_config(CodegenConfig::new("fixture"))
        .collect_owned_declaration_interface(&[
            ("core.base.maybe", &maybe),
            ("core.io.file", &file),
            ("fixture.resources", &probe),
        ])
        .map_err(|error| error.to_string())
}
#[test]
fn actual_temp_file_result_preserves_owner_and_mutability_on_wire() {
    let accessor = method(&source("shell/resources.vr"), "TempFile", "file");
    let module = interface(accessor.return_type.unwrap()).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let id = |owner: &str, name: &str| {
            module
                .types
                .iter()
                .find(|ty| {
                    module.get_string(ty.name) == Some(name)
                        && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                })
                .unwrap()
                .id
        };
        let result = &module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|name| name.ends_with("resource_result"))
            })
            .unwrap()
            .return_type;
        let TypeRef::Instantiated { base, args } = result else {
            panic!("expected Maybe application: {result:?}")
        };
        assert_eq!(*base, id("core.base.maybe", "Maybe"));
        assert_eq!(args.len(), 1);
        let TypeRef::Reference {
            inner, mutability, ..
        } = &args[0]
        else {
            panic!("expected borrowed File")
        };
        assert_eq!(*mutability, Mutability::Mutable);
        assert_eq!(**inner, TypeRef::Concrete(id("core.io.file", "File")));
    }
}
#[test]
fn undeclared_container_is_not_an_implicit_maybe_alias() {
    let module = Parser::new("public fn unknown() -> MissingContainerFixture<&mut File> {}")
        .parse_module()
        .unwrap();
    let ItemKind::Function(f) = &module.items[0].kind else {
        panic!()
    };
    let error = interface(f.return_type.as_ref().unwrap().clone()).unwrap_err();
    assert!(
        error.contains("unresolved nominal type 'MissingContainerFixture'"),
        "{error}"
    );
}
