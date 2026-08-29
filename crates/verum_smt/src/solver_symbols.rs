//! The names Verum values carry inside a solver.
//!
//! Two translators put Verum expressions in front of a solver, and
//! they must agree on every symbol they both can emit:
//!
//! * [`crate::translate`] builds Z3 AST for the GOAL under proof;
//! * [`crate::expr_to_smtlib`] builds SMT-LIB text for the DEFINITIONS
//!   reflected out of pure functions.
//!
//! A goal mentioning `w.flag` and a reflected body mentioning `w.flag`
//! only meet if both spell that projection the same way, at the same
//! sort. When each translator carried its own `format!`, they did not:
//! the goal side named every field `field_w__flag` at sort `Int` while
//! the reflection side named it `Verum!proj!W!flag` at the field's
//! declared sort. Neither was "wrong" in isolation — they simply
//! described different symbols, so a definition could never reach the
//! goal that needed it, and a `Bool` field could not be a proposition
//! at all.
//!
//! This module is the single authority for those names. Two spellings
//! of one concept is not a style question here; it is the difference
//! between a proof that closes and one that cannot.

/// The uninterpreted sort standing for a named Verum type the solver
/// does not model — a user record, a protocol, a container reaching
/// the translator through its head.
///
/// Opaque **under its own name**, so two values of one type share a
/// sort and two different types never collide. The alternative that
/// was in the code — substituting `Int` — is worse than refusing:
/// over a list-shaped `Int`, `xs.len() > 0` is arithmetic that means
/// nothing, and a solver will happily "prove" it.
pub fn opaque_sort(type_name: &str) -> String {
    format!("Verum!{}", type_name)
}

/// The DISCRIMINANT symbol for one constructor of a sum type: "is this
/// value an `A`?".
///
/// A nullary constructor needs none — `(= k path_K.A)` says it, because
/// a nullary constructor IS a constant. One carrying a payload is not a
/// constant, so the test has to be a predicate on the scrutinee.
pub fn discriminant(type_name: &str, ctor: &str) -> String {
    format!("Verum!is!{}!{}", type_name, ctor)
}

/// The projection symbol for one PAYLOAD position of a constructor:
/// `match r { Ok(v) => … }` binds `v` to `payload("Result", "Ok", 0)`
/// applied to `r`.
///
/// Same device as a record field, and for the same reason: the solver
/// learns nothing about the payload's value, only that one scrutinee
/// projects to one payload — which is what an arm's body and a
/// hypothesis about the same value need in order to meet.
pub fn payload(type_name: &str, ctor: &str, index: usize) -> String {
    format!("Verum!payload!{}!{}!{}", type_name, ctor, index)
}

/// The projection symbol for a record field: `w.flag` becomes an
/// application of `projection("W", "flag")` to the receiver.
///
/// The solver learns nothing about the field's VALUE — only that one
/// receiver projects to one value. That is exactly what a reflected
/// body and a hypothesis about the same receiver need in order to
/// meet.
pub fn projection(type_name: &str, field: &str) -> String {
    format!("Verum!proj!{}!{}", type_name, field)
}

/// The symbol for a protocol or inherent method call:
/// `c.cond().holds()` becomes nested applications of
/// `method("Candidate", "cond")` and `method("CondFS", "holds")`.
pub fn method(type_name: &str, method_name: &str) -> String {
    format!("Verum!method!{}!{}", type_name, method_name)
}

/// Reading one element: `xs[i]` becomes an application of this symbol
/// to the container and the index.
///
/// Uninterpreted, and that is the whole content: the solver learns
/// nothing about the element's VALUE, only that one container and one
/// index name one value. Reflexivity follows, which sounds too small to
/// state until you see what its absence did. `translate_index` demanded
/// a Z3 array sort, an ordinary identifier is an `Int` constant, so
/// `Z3_mk_select` on it was refused and the WHOLE body failed to
/// translate — `result` unbound, nothing provable. Measured,
/// `xs[0] == xs[0]` did not verify.
///
/// The container is an ARGUMENT, not part of the name: different
/// containers and different indices are different applications, so
/// `xs[0] == xs[1]` stays unprovable — the control that says this is a
/// read and not a collapse.
///
/// The container's SORT is part of the name, because a container
/// reaches the solver with whatever sort it was declared at — `Int` for
/// a plain identifier, `Verum!Array` for a parameter `create_var` could
/// only make opaque — and one name with two signatures is a
/// redeclaration the moment both appear in one module's SMT text.
///
/// A base that really carries a Z3 array sort still gets the exact
/// `select`; this is the fallback for a sort that was never declared,
/// not a replacement for array theory.
pub fn index(base_sort: &str) -> String {
    format!("Verum!index!{}", base_sort)
}

/// True for a symbol this module minted. The reflection registry's
/// closure pass uses it: a projection is declared by the entry that
/// uses it, so it is never an "undeclared symbol" that would poison
/// the module's SMT block.
pub fn is_solver_symbol(token: &str) -> bool {
    token.starts_with("Verum!")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes are pinned because two translators depend on them
    /// byte for byte. A change here is a change to both.
    #[test]
    fn symbol_shapes_are_pinned() {
        assert_eq!(opaque_sort("Witness"), "Verum!Witness");
        assert_eq!(projection("Witness", "flag"), "Verum!proj!Witness!flag");
        assert_eq!(method("Candidate", "cond"), "Verum!method!Candidate!cond");
        assert_eq!(index("Int"), "Verum!index!Int");
    }

    /// Everything this module mints is recognisable as ours, and
    /// ordinary Verum identifiers are not.
    #[test]
    fn minted_symbols_are_recognisable() {
        assert!(is_solver_symbol(&opaque_sort("T")));
        assert!(is_solver_symbol(&projection("T", "f")));
        assert!(is_solver_symbol(&method("T", "m")));
        assert!(!is_solver_symbol("is_sorted"));
        assert!(!is_solver_symbol("path_Color.Red"));
    }

    /// Distinct members never collide: the separator is not a
    /// character a Verum identifier may contain, so `proj!T!a_b` and
    /// `proj!T_a!b` stay distinct.
    #[test]
    fn distinct_members_get_distinct_symbols() {
        assert_ne!(projection("T", "a_b"), projection("T_a", "b"));
        assert_ne!(projection("T", "f"), method("T", "f"));
    }
}
