//! A type parameter's DEFAULT belongs to the parameter, not to its
//! bounds.
//!
//! The default was read only inside the branch taken when a `:` bounds
//! clause was present, so
//!
//! ```text
//! type Pair<A, B: Named = Text> is { a: A, b: B };   // parsed
//! type Pair<A, B = Text>        is { a: A, b: B };   // "unclosed '<'"
//! ```
//!
//! and the second spelling — the common one, and the one the stdlib
//! needs for `Result<T, E = Error>` — failed at the `=` with a delimiter
//! error that named the wrong thing entirely.
//!
//! Same shape as the qualified/bare pattern pair in `bind_pattern`: two
//! spellings of one construct, and the handling written on only one.
//!
//! Task: T0922.

use verum_ast::span::FileId;
use verum_ast::ty::GenericParamKind;
use verum_common::Maybe;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn parse(src: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(src, file_id);
    VerumParser::new()
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("parse failed for {src:?}: {e:?}"))
}

/// The `default` of the first generic parameter that has one.
fn first_default(module: &verum_ast::Module) -> Option<String> {
    for item in &module.items {
        if let verum_ast::ItemKind::Type(t) = &item.kind {
            for gp in t.generics.iter() {
                if let GenericParamKind::Type {
                    default: Maybe::Some(ty),
                    ..
                } = &gp.kind
                {
                    return Some(format!("{:?}", ty.kind));
                }
            }
        }
    }
    None
}

#[test]
fn a_default_without_bounds_parses_and_is_kept() {
    let m = parse("type Pair<A, B = Text> is { a: A, b: B };");
    let d = first_default(&m).expect("`B = Text` must record a default");
    assert!(d.contains("Text"), "the default must be Text, got {d}");
}

#[test]
fn a_default_after_bounds_still_parses_and_is_kept() {
    // The spelling that already worked, pinned so repairing its sibling
    // cannot take it away.
    let m = parse(
        "type Named is protocol { fn name(&self) -> Text; };\n\
         type Pair<A, B: Named = Text> is { a: A, b: B };",
    );
    let d = first_default(&m).expect("`B: Named = Text` must record a default");
    assert!(d.contains("Text"), "the default must be Text, got {d}");
}

#[test]
fn a_parameter_without_a_default_records_none() {
    // The control. A parser that recorded a default unconditionally would
    // pass both tests above.
    let m = parse("type Pair<A, B> is { a: A, b: B };");
    assert!(
        first_default(&m).is_none(),
        "neither parameter declared a default"
    );
}

#[test]
fn several_defaults_in_one_parameter_list() {
    let m = parse("type Triple<A, B = Text, C = Int> is { a: A, b: B, c: C };");
    let mut defaults: Vec<String> = Vec::new();
    for item in &m.items {
        if let verum_ast::ItemKind::Type(t) = &item.kind {
            for gp in t.generics.iter() {
                if let GenericParamKind::Type {
                    default: Maybe::Some(ty),
                    ..
                } = &gp.kind
                {
                    defaults.push(format!("{:?}", ty.kind));
                }
            }
        }
    }
    assert_eq!(defaults.len(), 2, "two parameters declared a default: {defaults:?}");
}

#[test]
fn a_default_on_a_function_type_parameter_parses() {
    // Generic parameters are one production, so the same spelling has to
    // work wherever it appears.
    let m = parse("fn take<A, B = Text>(a: A, b: B) -> A { a }");
    assert_eq!(m.items.len(), 1);
}
