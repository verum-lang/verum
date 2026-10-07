//! T1615: declared slice methods and existing contextual List behavior.
//! Fixed/inferred array capacity identity remains tracked in T1620.
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("caller grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for (source, methods) in [
        (
            include_str!("../../../core/collections/list.vr"),
            &["push", "reserve"][..],
        ),
        (
            include_str!("../../../core/collections/slice.vr"),
            &["swap", "reverse", "len"][..],
        ),
    ] {
        let core = Parser::new(source)
            .parse_module()
            .expect("actual core grammar");
        for mut item in core.items {
            match &mut item.kind {
                ItemKind::Type(decl) if decl.name.name == "List" => {
                    checker
                        .register_type_declaration(decl)
                        .expect("List declaration");
                }
                ItemKind::Impl(decl)
                    if matches!(&decl.kind, ImplKind::Inherent(ty)
                    if matches!(&ty.kind, TypeKind::Slice(_)) || matches!(&ty.kind, TypeKind::Generic {base,..}
                        if matches!(&base.kind, TypeKind::Path(p) if p.as_ident().is_some_and(|i| i.name == "List")))) =>
                {
                    decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(f) if methods.contains(&f.name.name.as_str())));
                    checker
                        .register_impl_block(decl)
                        .expect("actual method signatures");
                }
                _ => (),
            }
        }
    }
    for item in &ast.items {
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("caller signature");
        }
    }
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|error| format!("{error:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}").into()),
    );
    errors
}

#[test]
fn fixed_array_and_borrowed_array_use_declared_slice_methods() {
    for ty in [
        "[Int;3]",
        "&mut [Int;3]",
        "&mut [Int]",
        "&checked mut [Int;3]",
    ] {
        let source = format!("fn probe(mut values:{ty}) {{values.swap(0,1);values.reverse();}}");
        let actual = errors(&source);
        assert!(actual.is_empty(), "{source}: {actual:?}");
    }
}

#[test]
fn explicit_list_keeps_its_own_capacity_operations() {
    let actual = errors("fn probe(mut values:List<Int>) {values.push(4);values.reserve(10);}");
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn literal_with_list_annotation_keeps_its_own_methods() {
    let actual = errors(
        "fn probe() {let mut values:List<Int> = [11,37,99];values.push(4);values.reserve(10);}",
    );
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn inferred_binding_of_contextual_list_result_keeps_its_own_methods() {
    let actual =
        errors("fn create()->List<Int> {[]} fn probe() {let mut values=create();values.push(7);}");
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn contextual_list_argument_keeps_its_capacity_methods() {
    let actual = errors(
        "fn grow(mut values:List<Int>) {values.push(7);} fn probe() {grow([]);grow([11,37,99]);}",
    );
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn inferred_empty_literal_keeps_its_existing_capacity_surface() {
    // T1620 tracks the inconsistent fixed-array type of this existing pattern.
    let actual = errors("fn probe() {let mut values=[];values.push(7);}");
    assert!(actual.is_empty(), "{actual:?}");
}
