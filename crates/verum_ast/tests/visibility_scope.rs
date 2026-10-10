use verum_ast::{Ident, Path, PathSegment, Span, Visibility};
use verum_common::{List, Text};

fn scope(segments: &[&str]) -> Visibility {
    let span = Span::new(4, 19, verum_ast::span::FileId::new(7));
    Visibility::PublicIn(Path::new(
        segments
            .iter()
            .map(|segment| match *segment {
                "self" => PathSegment::SelfValue,
                "super" => PathSegment::Super,
                "cog" => PathSegment::Cog,
                "." => PathSegment::Relative,
                name => PathSegment::Name(Ident::new(name, span)),
            })
            .collect(),
        span,
    ))
}

fn resolved(segments: &[&str], owner: &str, expected: &str) {
    let Visibility::PublicIn(path) = scope(segments).resolve_declared_scope(owner).unwrap() else {
        panic!("restricted policy lost");
    };
    assert_eq!(path.span, Span::dummy());
    let names: List<Text> = path
        .segments
        .iter()
        .map(|part| {
            let PathSegment::Name(ident) = part else {
                panic!("noncanonical scope");
            };
            assert_eq!(ident.span, Span::dummy());
            ident.name.clone()
        })
        .collect();
    assert_eq!(names.join("."), expected);
}

#[test]
fn absolute_scopes_are_span_free_and_importer_independent() {
    resolved(&["other", "scope"], "app.owner", "other.scope");
    resolved(&["other", "scope"], "", "other.scope");
}
#[test]
fn self_scopes_use_the_declaring_module() {
    resolved(
        &["self", "scope"],
        "app.owner.inner",
        "app.owner.inner.scope",
    );
}
#[test]
fn relative_and_super_scopes_use_the_declaring_parent() {
    resolved(&[".", "scope"], "app.owner.inner", "app.owner.scope");
    resolved(&["super", "scope"], "app.owner.inner", "app.owner.scope");
    resolved(&["super", "super", "scope"], "app.owner.inner", "app.scope");
}
#[test]
fn cog_scopes_use_the_declared_cog_root() {
    resolved(&["cog", "scope"], "app.owner.inner", "app.scope");
}
#[test]
fn scope_root_escape_is_refused() {
    for segments in [&["super"][..], &["."][..], &["super", "super"][..]] {
        assert!(scope(segments).resolve_declared_scope("app").is_err());
    }
    assert!(
        scope(&["super", "super", "super"])
            .resolve_declared_scope("app.owner.inner")
            .is_err()
    );
}
#[test]
fn scope_requires_a_valid_owner_and_leading_markers() {
    for owner in ["", ".app", "app.", "app..owner"] {
        assert!(
            scope(&["self", "scope"])
                .resolve_declared_scope(owner)
                .is_err()
        );
    }
    for segments in [
        &["name", "super"][..],
        &["self", "cog"][..],
        &[][..],
        &[""][..],
    ] {
        assert!(scope(segments).resolve_declared_scope("app.owner").is_err());
    }
}
#[test]
fn non_path_policies_keep_their_declaration_semantics() {
    for policy in [
        Visibility::Private,
        Visibility::Public,
        Visibility::PublicCrate,
        Visibility::PublicSuper,
        Visibility::Internal,
        Visibility::Protected,
    ] {
        assert_eq!(policy.resolve_declared_scope("app.owner").unwrap(), policy);
    }
}
