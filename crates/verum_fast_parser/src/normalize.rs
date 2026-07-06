//! Post-parse AST normalisation — module-level passes that need the
//! WHOLE module's declarations to classify individual items.
//!
//! # META-SPAN-ALIAS-1: `type X is Y;` single-bare-ident classification
//!
//! The unified `is` type-definition body is grammatically ambiguous for
//! a single bare identifier:
//!
//! ```text
//! type_definition_body = type_expr , [ type_refinement ] , ';'   (* alias  *)
//!                      | …
//!                      | variant_list , ';' ;                    (* enum   *)
//! variant_list         = [ '|' ] , variant , { '|' , variant } ;
//! ```
//!
//! `type Span is MetaSpan;` matches BOTH `type_expr ;` (alias to the
//! existing `MetaSpan` record) and `variant_list ;` (a fresh sum type
//! with one nullary variant *named* `MetaSpan`).  Both readings are
//! load-bearing in the standard library:
//!
//!   * alias idiom — `type Span is MetaSpan;`, `type IoError is
//!     StreamError;`, `type Port is UInt16;` (55+ sites in `core/`);
//!   * marker-enum idiom — `type SemaphoreError is Closed;` where
//!     `Closed` is a FRESH variant name (task #13 pinned this).
//!
//! A token-level parser cannot distinguish them — the decision needs a
//! type table.  Pre-fix the parser committed to the VARIANT reading for
//! every bare identifier, so every alias-intent declaration silently
//! became a bogus single-variant enum: `let s: Span = MetaSpan { … }`
//! then compiled the record literal as a VARIANT construction (payload
//! slots shifted by one) and every field read returned garbage.
//!
//! The resolution implemented here is **deferred classification at the
//! module level** — the earliest point where the whole module's
//! declarations and mounts are known, and upstream of EVERY consumer
//! (verum_types, VBC codegen, AOT, archive metadata, LSP), so all of
//! them see one consistent reading:
//!
//!   * the single bare variant's name resolves to a **known type** —
//!     declared in this module, explicitly mounted, or a well-known
//!     primitive/core name — ⇒ rewrite the body to
//!     `TypeDeclBody::Alias(<that type>)`;
//!   * otherwise ⇒ keep the single-variant enum (marker idiom).
//!
//! Explicit disambiguators always win and are never rewritten:
//! a leading pipe (`type X is | OnlyVariant;`) forces the enum reading
//! (the parser encodes that shape identically, but such a variant name
//! shadowing a known type is rejected by `looks-like-type` gate below
//! only for the *bare* form — the pipe form never reaches this pass:
//! see `parse_type_body`), payloads/attributes/generics on the variant
//! keep it an enum, and the `=` sigil (`type X = Y;`) is always an
//! alias at parse time.

use std::collections::HashSet;

use verum_ast::decl::{ItemKind, MountTree, MountTreeKind, TypeDeclBody};
use verum_ast::ty::{Path, PathSegment, Type, TypeKind};
use verum_ast::Module;
use verum_common::well_known_types::type_names;

/// Names that always denote types regardless of module contents —
/// the canonical primitive spellings plus the well-known core
/// wrapper/collection names re-exported by the implicit prelude.
fn is_well_known_type_name(name: &str) -> bool {
    if type_names::is_numeric_type(name) {
        return true;
    }
    matches!(
        name,
        "Text"
            | "Bool"
            | "Char"
            | "Unit"
            | "Never"
            | "Bytes"
            | "List"
            | "Map"
            | "Set"
            | "Deque"
            | "Array"
            | "Range"
            | "Maybe"
            | "Result"
            | "Heap"
            | "Shared"
            | "Weak"
    )
}

/// Collect the leaf (or aliased) names a mount tree brings into scope.
fn collect_mounted_names(tree: &MountTree, out: &mut HashSet<String>) {
    // An explicit `as alias` binds the alias name, whatever the path.
    if let verum_common::Maybe::Some(alias) = &tree.alias {
        out.insert(alias.name.to_string());
    }
    match &tree.kind {
        MountTreeKind::Path(path) => {
            if let Some(PathSegment::Name(ident)) = path.segments.last() {
                out.insert(ident.name.to_string());
            }
        }
        MountTreeKind::Nested { trees, .. } => {
            for t in trees.iter() {
                collect_mounted_names(t, out);
            }
        }
        // Glob mounts don't enumerate names; relative-file mounts bind
        // via their alias (handled above) or the file stem — the stem
        // form can't collide with a bare variant name meaningfully.
        _ => {}
    }
}

/// See module docs. Rewrites eligible single-bare-variant `TypeDecl`
/// bodies to `TypeDeclBody::Alias` in place, recursing into inline
/// module declarations (each inline module extends the visible-name
/// scope with its own declarations).
pub fn reclassify_single_variant_aliases(module: &mut Module) {
    let mut known: HashSet<String> = HashSet::new();
    collect_known_type_names(&module.items, &mut known);
    rewrite_items(&mut module.items, &known);
}

fn collect_known_type_names(
    items: &verum_common::List<verum_ast::Item>,
    known: &mut HashSet<String>,
) {
    for item in items.iter() {
        match &item.kind {
            ItemKind::Type(decl) => {
                known.insert(decl.name.name.to_string());
            }
            ItemKind::Protocol(decl) => {
                known.insert(decl.name.name.to_string());
            }
            ItemKind::Mount(mount) => {
                collect_mounted_names(&mount.tree, known);
            }
            ItemKind::Module(m) => {
                if let verum_common::Maybe::Some(inner) = &m.items {
                    collect_known_type_names(inner, known);
                }
            }
            _ => {}
        }
    }
}

fn rewrite_items(items: &mut verum_common::List<verum_ast::Item>, known: &HashSet<String>) {
    for item in items.iter_mut() {
        match &mut item.kind {
            ItemKind::Type(decl) => {
                let replacement: Option<Type> = match &decl.body {
                    TypeDeclBody::Variant(variants) if variants.len() == 1 => {
                        let v = match variants.first() {
                            Some(v) => v,
                            None => continue,
                        };
                        let is_bare_nullary = matches!(v.data, verum_common::Maybe::None)
                            && v.generic_params.is_empty()
                            && matches!(v.where_clause, verum_common::Maybe::None)
                            && v.attributes.is_empty()
                            && matches!(v.path_endpoints, verum_common::Maybe::None);
                        let names_known_type = {
                            let n = v.name.name.as_str();
                            n != decl.name.name.as_str()
                                && (known.contains(n) || is_well_known_type_name(n))
                        };
                        if is_bare_nullary && names_known_type {
                            let seg_span = v.name.span;
                            let segments: verum_common::List<PathSegment> =
                                vec![PathSegment::Name(v.name.clone())]
                                    .into_iter()
                                    .collect();
                            Some(Type::new(
                                TypeKind::Path(Path::new(segments, seg_span)),
                                seg_span,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(ty) = replacement {
                    decl.body = TypeDeclBody::Alias(ty);
                }
            }
            ItemKind::Module(m) => {
                if let verum_common::Maybe::Some(inner) = &mut m.items {
                    rewrite_items(inner, known);
                }
            }
            _ => {}
        }
    }
}
