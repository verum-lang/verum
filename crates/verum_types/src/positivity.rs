//! Strict-positivity check for user-declared inductive types.
//!
//! VUVA §7.3 K-Pos / Coquand & Paulin 1990: an inductive type `T` is
//! well-formed only when every recursive occurrence of `T` in any
//! constructor's argument types appears strictly positively. Berardi
//! 1998 establishes that admitting non-positive recursion in a system
//! with even minimal impredicativity yields a derivation of `False`;
//! the kernel therefore must reject every constructor whose argument
//! type contains `T` to the LEFT of an arrow.
//!
//! VUVA #150 (audit-derived task): C2 V1 shipped the kernel-side
//! `verum_kernel::InductiveRegistry::register` + `check_strict_
//! positivity` walker, but they had ZERO call sites in the type-
//! checker dispatch — user types `type Bad is Wrap(Bad -> A);`
//! compiled cleanly, with Berardi's paradox reachable through user
//! code. This module hosts the AST-level walker that fires at user-
//! type-decl registration time, parallel to the kernel's CoreTerm-
//! level walker. Both are needed because:
//!
//! - The AST walker runs BEFORE elaboration (so an ill-formed
//!   declaration is rejected with a useful span pointing at the
//!   offending constructor argument).
//! - The kernel walker runs AT elaboration (so any path that
//!   bypasses the AST walker — direct CoreTerm construction by
//!   tactics, derive macros, etc. — still gets the same guarantee).
//!
//! Discipline mirrors `verum_kernel::check_strict_positivity`: walk
//! every type-tree position, forbid `target` in any Function/Pi
//! domain, descend into return-types, sub-tuples, sub-records,
//! generic-arg lists, and so on.

use crate::ty::{InductiveConstructor, Type};
use verum_common::List;

/// Outcome of the positivity check on a user-declared inductive.
/// Carries breadcrumb information so the diagnostic can pinpoint the
/// offending constructor + argument index without a debugger.
#[derive(Debug, Clone)]
pub struct PositivityViolation {
    /// Name of the inductive type being declared.
    pub type_name: String,
    /// Name of the offending constructor.
    pub constructor: String,
    /// Argument index (0-based) within the constructor.
    pub arg_index: usize,
    /// Human-readable position breadcrumb (e.g. "left of an arrow",
    /// "inside Tuple element 2 → left of an arrow").
    pub position: String,
}

impl std::fmt::Display for PositivityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<E_POSITIVITY_VIOLATION> strict positivity violation in inductive '{}': constructor '{}' arg #{} has '{}' in {}",
            self.type_name, self.constructor, self.arg_index, self.type_name, self.position,
        )
    }
}

/// Run the strict-positivity check on every constructor of an
/// inductive declaration. Returns the FIRST violation found (we don't
/// continue past the first error to keep diagnostic output crisp;
/// the user fixes one and re-runs).
pub fn check_user_inductive(
    type_name: &str,
    constructors: &List<InductiveConstructor>,
) -> Result<(), PositivityViolation> {
    for ctor in constructors.iter() {
        for (i, arg_ty) in ctor.args.iter().enumerate() {
            let mut breadcrumb = format!("constructor '{}' arg #{}", ctor.name.as_str(), i);
            check_strictly_positive(type_name, arg_ty, &mut breadcrumb).map_err(|pos| {
                PositivityViolation {
                    type_name: type_name.to_string(),
                    constructor: ctor.name.as_str().to_string(),
                    arg_index: i,
                    position: pos,
                }
            })?;
        }
    }
    Ok(())
}

/// True iff `target` appears anywhere inside `ty` — used to detect
/// the negative position of a Function. Conservative: false-negatives
/// are acceptable (would under-report); false-positives are not
/// (would over-flag legitimate code).
fn name_appears_in(target: &str, ty: &Type) -> bool {
    match ty {
        Type::Inductive { name, params, indices, .. } => {
            if name.as_str() == target {
                return true;
            }
            params.iter().any(|(_, t)| name_appears_in(target, t))
                || indices.iter().any(|(_, t)| name_appears_in(target, t))
        }
        Type::Generic { name, args } => {
            if name.as_str() == target {
                return true;
            }
            args.iter().any(|a| name_appears_in(target, a))
        }
        Type::Named { path, args } => {
            // The placeholder a recursive type-decl registers under
            // `Type::Named { path: <self> }` BEFORE its variant body
            // is processed — so inside the variant body, recursive
            // `Bad` references resolve to `Type::Named { path: Bad
            // }`. The positivity check must recognise this shape
            // alongside Inductive / Generic.
            let name_matches = path.segments.last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str() == target),
                    _ => None,
                })
                .unwrap_or(false);
            if name_matches {
                return true;
            }
            args.iter().any(|a| name_appears_in(target, a))
        }
        Type::Function { params, return_type, .. } => {
            params.iter().any(|p| name_appears_in(target, p))
                || name_appears_in(target, return_type)
        }
        Type::Tuple(types) => types.iter().any(|t| name_appears_in(target, t)),
        Type::Array { element, .. } => name_appears_in(target, element),
        Type::Slice { element } => name_appears_in(target, element),
        Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } => name_appears_in(target, inner),
        Type::Record(fields) => fields.values().any(|t| name_appears_in(target, t)),
        Type::Variant(variants) => variants.values().any(|t| name_appears_in(target, t)),
        Type::Pi { param_type, return_type, .. } => {
            name_appears_in(target, param_type) || name_appears_in(target, return_type)
        }
        // User-named types are referenced by `Inductive` / `Generic`
        // (handled above) or `TypeAlias` — for the latter we
        // conservatively do NOT recurse into the alias body since
        // it lives in a different module's scope at registration
        // time.
        _ => false,
    }
}

/// The strict-positivity walker. Returns Err with a breadcrumb when
/// `target` appears in a forbidden position; Ok otherwise.
fn check_strictly_positive(
    target: &str,
    ty: &Type,
    breadcrumb: &mut String,
) -> Result<(), String> {
    match ty {
        Type::Function { params, return_type, .. } => {
            // Negative position: target must not appear in any param.
            for (i, p) in params.iter().enumerate() {
                if name_appears_in(target, p) {
                    return Err(format!(
                        "{} → param #{} (left of an arrow / negative position)",
                        breadcrumb, i,
                    ));
                }
            }
            // Codomain (return_type) must itself be strictly positive
            // in target — this catches `Bad -> (Bad -> A)` where the
            // inner arrow's domain is also a negative position.
            let saved = breadcrumb.clone();
            breadcrumb.push_str(" → return_type");
            check_strictly_positive(target, return_type, breadcrumb)?;
            *breadcrumb = saved;
            Ok(())
        }
        Type::Pi { param_type, return_type, .. } => {
            if name_appears_in(target, param_type) {
                return Err(format!(
                    "{} → Π-domain (left of an arrow / negative position)",
                    breadcrumb,
                ));
            }
            let saved = breadcrumb.clone();
            breadcrumb.push_str(" → Π-codomain");
            check_strictly_positive(target, return_type, breadcrumb)?;
            *breadcrumb = saved;
            Ok(())
        }
        Type::Inductive { params, indices, .. } => {
            for (i, (_, t)) in params.iter().enumerate() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → Inductive param #{}", i));
                check_strictly_positive(target, t, breadcrumb)?;
                *breadcrumb = saved;
            }
            for (i, (_, t)) in indices.iter().enumerate() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → Inductive index #{}", i));
                check_strictly_positive(target, t, breadcrumb)?;
                *breadcrumb = saved;
            }
            Ok(())
        }
        Type::Generic { args, .. } | Type::Named { args, .. } => {
            for (i, a) in args.iter().enumerate() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → typed arg #{}", i));
                check_strictly_positive(target, a, breadcrumb)?;
                *breadcrumb = saved;
            }
            Ok(())
        }
        Type::Tuple(types) => {
            for (i, t) in types.iter().enumerate() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → Tuple element #{}", i));
                check_strictly_positive(target, t, breadcrumb)?;
                *breadcrumb = saved;
            }
            Ok(())
        }
        Type::Array { element, .. } | Type::Slice { element } => {
            let saved = breadcrumb.clone();
            breadcrumb.push_str(" → Array/Slice element");
            check_strictly_positive(target, element, breadcrumb)?;
            *breadcrumb = saved;
            Ok(())
        }
        Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } => {
            let saved = breadcrumb.clone();
            breadcrumb.push_str(" → Reference target");
            check_strictly_positive(target, inner, breadcrumb)?;
            *breadcrumb = saved;
            Ok(())
        }
        Type::Record(fields) => {
            for (name, t) in fields.iter() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → Record field '{}'", name));
                check_strictly_positive(target, t, breadcrumb)?;
                *breadcrumb = saved;
            }
            Ok(())
        }
        Type::Variant(variants) => {
            for (name, t) in variants.iter() {
                let saved = breadcrumb.clone();
                breadcrumb.push_str(&format!(" → Variant '{}'", name));
                check_strictly_positive(target, t, breadcrumb)?;
                *breadcrumb = saved;
            }
            Ok(())
        }
        // Atoms / type vars / aliases / refinements: no nested types
        // to recurse into for positivity purposes.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use super::*;
    use crate::ty::{InductiveConstructor, Type};
    use verum_common::{List, Text};

    fn ind(name: &str, args: Vec<Type>) -> Type {
        // Use the simpler Generic form for tests — `Type::Inductive`
        // requires UniverseLevel and the full param/index split which
        // is overkill for verifying the walker's positivity discipline.
        Type::Generic {
            name: Text::from(name),
            args: List::from_iter(args),
        }
    }

    fn arrow(domain: Type, codomain: Type) -> Type {
        Type::Function {
            params: List::from_iter(vec![domain]),
            return_type: Box::new(codomain),
            contexts: None,
            type_params: List::new(),
            properties: None,
        }
    }

    #[test]
    fn nat_is_strictly_positive() {
        // Nat = Zero | Succ(Nat)
        let ctors = List::from_iter(vec![
            InductiveConstructor::unit("Zero".into(), ind("Nat", vec![])),
            InductiveConstructor::with_args(
                "Succ".into(),
                List::from_iter(vec![ind("Nat", vec![])]),
                ind("Nat", vec![]),
            ),
        ]);
        assert!(check_user_inductive("Nat", &ctors).is_ok());
    }

    #[test]
    fn berardi_witness_rejected() {
        // Bad = Wrap(Bad -> A) — non-positive recursion.
        let ctors = List::from_iter(vec![InductiveConstructor::with_args(
            "Wrap".into(),
            List::from_iter(vec![arrow(ind("Bad", vec![]), Type::Bool)]),
            ind("Bad", vec![]),
        )]);
        let result = check_user_inductive("Bad", &ctors);
        match result {
            Err(violation) => {
                assert_eq!(violation.type_name, "Bad");
                assert_eq!(violation.constructor, "Wrap");
                assert!(violation.position.contains("left of an arrow"));
            }
            Ok(_) => panic!("Berardi witness must be rejected"),
        }
    }

    #[test]
    fn second_order_non_positive_rejected() {
        // Bad2 = Wrap((Bad2 -> A) -> A)
        let inner = arrow(ind("Bad2", vec![]), Type::Bool);
        let outer = arrow(inner, Type::Bool);
        let ctors = List::from_iter(vec![InductiveConstructor::with_args(
            "Wrap".into(),
            List::from_iter(vec![outer]),
            ind("Bad2", vec![]),
        )]);
        assert!(check_user_inductive("Bad2", &ctors).is_err());
    }

    #[test]
    fn positive_codomain_admitted() {
        // Curried = Curry(Int -> Curried) — codomain is positive,
        // so this must be admitted.
        let arg = arrow(Type::Bool, ind("Curried", vec![]));
        let ctors = List::from_iter(vec![InductiveConstructor::with_args(
            "Curry".into(),
            List::from_iter(vec![arg]),
            ind("Curried", vec![]),
        )]);
        assert!(check_user_inductive("Curried", &ctors).is_ok());
    }

    #[test]
    fn named_form_berardi_witness_rejected() {
        // The user-decl placeholder registers `Bad` as
        // Type::Named { path: Bad, args: [] } before processing the
        // variant body. The walker must detect Type::Named entries
        // alongside Type::Generic / Type::Inductive — without this,
        // self-recursion via `fn(Bad) -> Bool` slips through.
        use verum_ast::span::Span;
        use verum_ast::ty::{Ident, Path, PathSegment};
        let bad_named = Type::Named {
            path: Path::new(
                List::from_iter(vec![PathSegment::Name(
                    Ident::new(Text::from("Bad"), Span::default()),
                )]),
                Span::default(),
            ),
            args: List::new(),
        };
        let arg = Type::Function {
            params: List::from_iter(vec![bad_named]),
            return_type: Box::new(Type::Bool),
            contexts: None,
            type_params: List::new(),
            properties: None,
        };
        let ctors = List::from_iter(vec![InductiveConstructor::with_args(
            "Wrap".into(),
            List::from_iter(vec![arg]),
            ind("Bad", vec![]),
        )]);
        let result = check_user_inductive("Bad", &ctors);
        match result {
            Err(violation) => {
                assert_eq!(violation.constructor, "Wrap");
                assert!(violation.position.contains("left of an arrow"));
            }
            Ok(_) => panic!("Type::Named-shaped Berardi must be rejected"),
        }
    }

    #[test]
    fn list_with_generic_arg_is_strictly_positive() {
        // List<A> = Nil | Cons(A, List<A>) — use Generic for both
        // the type-parameter `A` (modelled as a top-level Generic
        // with no args, since Type::Var requires a TypeVar fresh-id
        // we don't want to mint here) and the recursive `List<A>`.
        let var_a = ind("A", vec![]);
        let list_a = ind("List", vec![var_a.clone()]);
        let ctors = List::from_iter(vec![
            InductiveConstructor::unit("Nil".into(), list_a.clone()),
            InductiveConstructor::with_args(
                "Cons".into(),
                List::from_iter(vec![var_a, list_a.clone()]),
                list_a,
            ),
        ]);
        assert!(check_user_inductive("List", &ctors).is_ok());
    }
}
