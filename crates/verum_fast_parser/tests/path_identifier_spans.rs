//! T1548: path names retain the identifier token's byte span, not the next token.

use verum_ast::{FileId, Ident, Path, PathSegment, Span, Type, TypeKind};
use verum_fast_parser::VerumParser;

const FILE: FileId = FileId::new(37);

fn parse(source: &str) -> Type {
    VerumParser::new()
        .parse_type_str(source, FILE)
        .unwrap_or_else(|errors| panic!("{source:?}: {errors:?}"))
}

fn path(ty: &Type) -> &Path {
    match &ty.kind {
        TypeKind::Path(path) => path,
        other => panic!("expected a path, got {other:?}"),
    }
}

fn name(path: &Path, index: usize) -> &Ident {
    match &path.segments[index] {
        PathSegment::Name(name) => name,
        other => panic!("expected a name, got {other:?}"),
    }
}

fn assert_name(source: &str, ident: &Ident, spelling: &str, start: u32, end: u32) {
    assert_eq!(ident.name.as_str(), spelling);
    assert_eq!(
        ident.span,
        Span::new(start, end, FILE),
        "{source:?}: {spelling}"
    );
    assert_eq!(source.get(start as usize..end as usize), Some(spelling));
}

#[test]
fn bare_identifier_at_eof_and_after_whitespace_has_its_own_span() {
    for (source, spelling, start, end) in [("U", "U", 0, 1), ("  Имя  ", "Имя", 2, 8)] {
        let ty = parse(source);
        assert_name(source, name(path(&ty), 0), spelling, start, end);
        assert_eq!(ty.span, Span::new(start, end, FILE));
        assert_eq!(path(&ty).span, ty.span);
    }
}

#[test]
fn qualified_names_keep_individual_utf8_byte_offsets() {
    for (source, first, second, split, end) in [
        ("alpha.Item", "alpha", "Item", 5, 10),
        ("модуль.Имя", "модуль", "Имя", 12, 19),
    ] {
        let ty = parse(source);
        let parsed = path(&ty);
        assert_eq!(parsed.segments.len(), 2);
        assert_name(source, name(parsed, 0), first, 0, split);
        assert_name(source, name(parsed, 1), second, split + 1, end);
        assert_eq!(parsed.span, Span::new(0, end, FILE));
        assert_eq!(ty.span, parsed.span);
    }
}

#[test]
fn projection_names_are_tokens_while_path_and_type_cover_the_whole_projection() {
    for (source, root, associated, root_end, end) in [
        ("T.Item", "T", "Item", 1, 6),
        ("Тип.Имя", "Тип", "Имя", 6, 13),
    ] {
        let ty = parse(source);
        let TypeKind::Qualified {
            self_ty,
            assoc_name,
            ..
        } = &ty.kind
        else {
            panic!("expected a projection: {ty:?}");
        };
        assert_name(source, name(path(self_ty), 0), root, 0, root_end);
        assert_name(source, assoc_name, associated, root_end + 1, end);
        let full = Span::new(0, end, FILE);
        assert_eq!(ty.span, full);
        assert_eq!(self_ty.span, full);
        assert_eq!(path(self_ty).span, full);
    }
}

#[test]
fn contextual_keyword_continuations_are_literal_names_with_token_spans() {
    for (source, keyword, keyword_end, end) in [
        ("core.async.Item", "async", 10, 15),
        ("core.cog.Item", "cog", 8, 13),
        ("core.self.Item", "self", 9, 14),
    ] {
        let ty = parse(source);
        let parsed = path(&ty);
        assert_eq!(parsed.segments.len(), 3);
        assert_name(source, name(parsed, 0), "core", 0, 4);
        assert_name(source, name(parsed, 1), keyword, 5, keyword_end);
        assert_name(source, name(parsed, 2), "Item", keyword_end + 1, end);
    }
}

#[test]
fn navigation_segments_keep_their_meaning() {
    let source = "super.super.Item";
    let ty = parse(source);
    let parsed = path(&ty);
    assert!(matches!(parsed.segments[0], PathSegment::Super));
    assert!(matches!(parsed.segments[1], PathSegment::Super));
    assert_name(source, name(parsed, 2), "Item", 12, 16);

    let ty = parse("cog.module.Item");
    assert!(matches!(path(&ty).segments[0], PathSegment::Cog));
    assert_name("cog.module.Item", name(path(&ty), 1), "module", 4, 10);
}

#[test]
fn rank2_parameter_and_return_paths_keep_their_own_source_offsets() {
    let source = "fn<R>(R, модуль.Имя) -> R.Item";
    let ty = parse(source);
    let TypeKind::Rank2Function {
        type_params,
        params,
        return_type,
        ..
    } = &ty.kind
    else {
        panic!("expected a rank-2 function: {ty:?}");
    };
    assert_eq!(type_params.len(), 1);
    assert_eq!(params.len(), 2);
    assert_name(source, name(path(&params[0]), 0), "R", 6, 7);
    assert_name(source, name(path(&params[1]), 0), "модуль", 9, 21);
    assert_name(source, name(path(&params[1]), 1), "Имя", 22, 28);
    let TypeKind::Qualified {
        self_ty,
        assoc_name,
        ..
    } = &return_type.kind
    else {
        panic!("expected a return projection: {return_type:?}");
    };
    assert_name(source, name(path(self_ty), 0), "R", 33, 34);
    assert_name(source, assoc_name, "Item", 35, 39);
    assert_eq!(return_type.span, Span::new(33, 39, FILE));
}

#[test]
fn array_const_expression_name_has_its_identifier_token_span() {
    for (source, spelling, end) in [("[T; N]", "N", 5), ("[T; Число]", "Число", 14)] {
        let ty = parse(source);
        let TypeKind::Array { size, .. } = &ty.kind else {
            panic!("expected an array: {ty:?}");
        };
        let size = size.as_ref().expect("array has a size expression");
        let verum_ast::expr::ExprKind::Path(parsed) = &size.kind else {
            panic!("expected a named const expression: {size:?}");
        };
        assert_name(source, name(parsed, 0), spelling, 4, end);
        assert_eq!(parsed.span, Span::new(4, end, FILE));
        assert_eq!(size.span, parsed.span);
    }
}

fn expression_path(source: &str) -> Path {
    let expr = VerumParser::new()
        .parse_expr_str(source, FILE)
        .unwrap_or_else(|errors| panic!("{source:?}: {errors:?}"));
    match expr.kind {
        verum_ast::expr::ExprKind::Path(path) => path,
        other => panic!("expected an expression path, got {other:?}"),
    }
}

#[test]
fn ordinary_expression_path_name_retains_utf8_token_span() {
    for (source, spelling, start, end) in [("N", "N", 0, 1), (" Число ", "Число", 1, 11)]
    {
        let parsed = expression_path(source);
        assert_name(source, name(&parsed, 0), spelling, start, end);
        assert_eq!(parsed.span, Span::new(start, end, FILE));
    }
}

#[test]
fn self_type_expression_path_is_a_name_with_its_token_span() {
    let source = " Self ";
    let parsed = expression_path(source);
    assert_name(source, name(&parsed, 0), "Self", 1, 5);
    assert_eq!(parsed.span, Span::new(1, 5, FILE));
}

#[test]
fn recovery_colon_paths_retain_each_identifier_span() {
    // `::` is diagnostic recovery syntax, not a new accepted Verum separator.
    for (source, first, second, first_end, end) in [
        ("Size::Count", "Size", "Count", 4, 11),
        ("Тип::Размер", "Тип", "Размер", 6, 20),
    ] {
        let parsed = expression_path(source);
        assert_eq!(parsed.segments.len(), 2);
        assert_name(source, name(&parsed, 1), second, first_end + 2, end);
        assert_name(source, name(&parsed, 0), first, 0, first_end);
        assert_eq!(parsed.span, Span::new(0, end, FILE));
    }
}
