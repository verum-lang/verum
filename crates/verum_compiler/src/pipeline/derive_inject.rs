//! `@derive(...)` turned into real `implement` blocks (T1018).
//!
//! The attribute was parsed, registered as a known attribute, and never
//! applied: `protocol.rs`'s `derive` dispatcher had ZERO callers, and the
//! subtree below it is DESCRIPTIVE — `derive_ord` returns
//! `DerivedMethod { body: DerivedBody::LexicographicCmp { .. } }`, and
//! `DerivedBody` has zero consumers outside its own file. So it produced a
//! structured description of a body that nothing turned into code.
//!
//! WIRING THAT DISPATCHER WOULD HAVE MADE THINGS WORSE. Registering a
//! `ProtocolImpl` makes `implements_protocol(T, "Ord")` true, which
//! silences the W0506 warning that detects the real defect — while no
//! `T.cmp` exists, so codegen still falls through to a POINTER comparison
//! and `a < b` stays allocation-order dice. The attribute would gain the
//! appearance of working and switch off the detector for the thing it did
//! not fix.
//!
//! So this GENERATES, and it generates SOURCE rather than AST or
//! bytecode: everything downstream — conformance checking, codegen, the
//! archive — then sees an ordinary `implement` block, and there is no
//! second spelling of what a derived `cmp` means. The chain uses the
//! standard library's own `Ordering.then`, so lexicographic order is not
//! reinvented either.
//!
//! Runs beside `inject_implicit_prelude_mount`, at the one point every
//! parse funnels through.

use verum_ast::Module;
use verum_ast::decl::{TypeDecl, TypeDeclBody};

/// A `@derive(P)` this pass cannot honour: the caller reports it rather
/// than letting the attribute go on looking effective.
pub(crate) struct UnsupportedDerive {
    pub type_name: String,
    pub protocol: String,
    pub reason: &'static str,
    pub span: verum_ast::span::Span,
}

/// The protocol an `@derive` argument names, whatever shape it takes.
fn protocol_name(expr: &verum_ast::Expr) -> Option<String> {
    use verum_ast::ty::PathSegment;
    use verum_ast::{ExprKind, LiteralKind};
    match &expr.kind {
        ExprKind::Path(path) => match path.segments.last() {
            Some(PathSegment::Name(id)) => Some(id.name.as_str().to_string()),
            _ => None,
        },
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(t) => Some(t.as_str().to_string()),
            _ => None,
        },
        ExprKind::Paren(inner) => protocol_name(inner),
        _ => None,
    }
}

/// The source of the `implement` block a derive stands for, or the reason
/// there is none.
fn derived_impl_source(td: &TypeDecl, protocol: &str) -> Result<String, &'static str> {
    let name = td.name.name.as_str();
    let TypeDeclBody::Record(fields) = &td.body else {
        return Err("only a record type can be derived from today");
    };
    if fields.is_empty() {
        return Err("the type has no fields to compare");
    }
    match protocol {
        "Ord" => {
            let mut chain = String::new();
            for (i, f) in fields.iter().enumerate() {
                let fname = f.name.name.as_str();
                let step = format!("self.{fname}.cmp(&other.{fname})");
                if i == 0 {
                    chain = step;
                } else {
                    // Lexicographic order, spelled with the stdlib's own
                    // combinator rather than a fresh one.
                    chain = format!("{chain}.then({step})");
                }
            }
            Ok(format!(
                "implement Ord for {name} {{\n    \
                 fn cmp(&self, other: &{name}) -> Ordering {{\n        \
                 {chain}\n    }}\n}}\n"
            ))
        }
        "Default" => {
            // `Default.default() -> Self` (core/base/protocols.vr:293)
            // is STATIC — no receiver — so the generated body is a
            // record literal whose every field asks its own type for
            // its default.  Recursion is the protocol's job, not this
            // generator's: `Defs { x: Int.default(), … }` works
            // whenever `Int` implements `Default`, and fails with the
            // ordinary "no method named `default`" when it does not,
            // which names the offending FIELD TYPE rather than the
            // derive.
            //
            // T1072: without this, `@derive(Default)` was accepted and
            // then reported W0507, and `T.default()` did not exist —
            // the one case of the fourteen documented derives where
            // the structural fallback covers nothing (`.clone()`,
            // `==`, `Map` keys all work with no derive at all).
            let inits: Vec<String> = fields
                .iter()
                .map(|f| {
                    let fname = f.name.name.as_str();
                    let fty = verum_ast::pretty::format_type(&f.ty);
                    format!("{fname}: {}.default()", fty.as_str())
                })
                .collect();
            Ok(format!(
                "implement Default for {name} {{\n    \
                 fn default() -> {name} {{\n        \
                 {name} {{ {} }}\n    }}\n}}\n",
                inits.join(", ")
            ))
        }
        _ => Err("no generator for this protocol yet"),
    }
}

/// Append the `implement` blocks that this module's `@derive` attributes
/// stand for. Returns the derives that could not be honoured.
pub(crate) fn inject_derived_impls(module: &mut Module) -> Vec<UnsupportedDerive> {
    let mut src = String::new();
    let mut unsupported = Vec::new();

    for item in module.items.iter() {
        let verum_ast::ItemKind::Type(td) = &item.kind else {
            continue;
        };
        for attr in td.attributes.iter() {
            if attr.name.as_str() != "derive" {
                continue;
            }
            let verum_common::Maybe::Some(args) = &attr.args else {
                continue;
            };
            for arg in args.iter() {
                let Some(proto) = protocol_name(arg) else {
                    continue;
                };
                match derived_impl_source(td, &proto) {
                    Ok(text) => src.push_str(&text),
                    Err(reason) => unsupported.push(UnsupportedDerive {
                        type_name: td.name.name.as_str().to_string(),
                        protocol: proto,
                        reason,
                        span: attr.span,
                    }),
                }
            }
        }
    }

    if !src.is_empty() {
        let parser = verum_fast_parser::FastParser::new();
        match parser.parse_module_str(&src, module.file_id) {
            Ok(generated) => {
                for it in generated.items.iter() {
                    module.items.push(it.clone());
                }
            }
            Err(_) => {
                // A generator that produces unparseable source is a defect
                // in THIS file, not in the author's program — say so
                // rather than dropping the derive silently, which is the
                // behaviour being replaced.
                unsupported.push(UnsupportedDerive {
                    type_name: "<generated>".to_string(),
                    protocol: "*".to_string(),
                    reason: "the generated implementation did not parse",
                    span: verum_ast::span::Span::default(),
                });
            }
        }
    }

    unsupported
}
