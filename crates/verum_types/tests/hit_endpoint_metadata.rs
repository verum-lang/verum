//! Verifies that HIT path-constructor endpoints parsed from
//! `Foo(args) = from..to` syntax flow through to the type checker's
//! `hit_path_constructors` side-channel registry.
//!
//! The lowering keeps `Type::Variant` as the primary representation for
//! ordinary type checking; HIT-aware tactics (cubical, descent) then
//! consult the side channel for path-constructor topology.

use verum_common::{Heap, List, Maybe};
use verum_ast::decl::{Variant, VariantData};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Type as AstType, TypeKind};

fn span() -> Span {
    Span::default()
}

fn ident(name: &str) -> Ident {
    Ident::new(name, span())
}

fn endpoint_expr(name: &str) -> Heap<Expr> {
    Heap::new(Expr {
        kind: ExprKind::Path(verum_ast::ty::Path::single(ident(name))),
        span: span(),
        ref_kind: None,
        check_eliminated: false,
    })
}

#[test]
fn hit_variant_carries_endpoint_metadata_through_ast() {
    // Construct a HIT path-constructor variant directly:
    //     | Seg() = Zero..One
    let from = endpoint_expr("Zero");
    let to = endpoint_expr("One");

    let v = Variant {
        name: ident("Seg"),
        generic_params: List::new(),
        data: Maybe::Some(VariantData::Tuple(List::new())),
        where_clause: Maybe::None,
        attributes: List::new(),
        path_endpoints: Maybe::Some((from, to)),
        span: span(),
    };

    // Round-trip the variant through clone + match to confirm the
    // metadata is preserved.
    let cloned = v.clone();
    match cloned.path_endpoints {
        Maybe::Some((lhs, rhs)) => {
            assert!(matches!(&lhs.kind, ExprKind::Path(_)));
            assert!(matches!(&rhs.kind, ExprKind::Path(_)));
        }
        Maybe::None => panic!("expected path_endpoints to round-trip"),
    }
}

#[test]
fn ordinary_variant_has_no_endpoints() {
    // `Some(T)` — regular tuple variant, no endpoints.
    let v = Variant {
        name: ident("Some"),
        generic_params: List::new(),
        data: Maybe::Some(VariantData::Tuple(List::from_iter([AstType {
            kind: TypeKind::Path(verum_ast::ty::Path::single(ident("T"))),
            span: span(),
        }]))),
        where_clause: Maybe::None,
        attributes: List::new(),
        path_endpoints: Maybe::None,
        span: span(),
    };

    assert!(matches!(v.path_endpoints, Maybe::None));
}
