//! Refinement Reflection — User Functions as SMT Axioms.
//!
//! When a Verum function `f` is **pure** and **total**, its definition
//! `f(x₁, …, xₙ) = body` can be safely reflected into the SMT context
//! as a universally-quantified equality:
//!
//! ```text
//!     ∀ x₁ … xₙ.  f(x₁, …, xₙ) = ⟦body⟧
//! ```
//!
//! With this axiom installed, downstream proof obligations that
//! mention `f(...)` can be discharged by *unfolding* the call rather
//! than treating `f` as an uninterpreted symbol — the same technique
//! Liquid Haskell calls *refinement reflection*.
//!
//! ## Soundness gate
//!
//! Reflection is only sound for functions that are:
//!
//! * **Pure** — no `Mutates`, `IO`, `Async`, `Fallible` properties
//!   (all of which would let `f(x)` denote different values on
//!   different calls).
//! * **Total** — terminate on every input; do not call `panic`,
//!   `unreachable`, or partial primitives.
//! * **Closed** — body uses only its formal parameters and other
//!   already-reflected functions.
//!
//! The `Reflectability` predicate enforces these conditions before
//! a function is admitted to the registry.
//!
//! ## Registry
//!
//! [`RefinementReflectionRegistry`] holds the reflected definitions
//! keyed by qualified function name. Proof verifiers obtain a borrow
//! of the registry and call [`apply_to_solver`] before discharging
//! a goal — this asserts every relevant axiom into the Z3 context.
//!
//! ## Mathesis-readiness
//!
//! Refinement reflection is what makes SMT-backed proofs over
//! arbitrary user data structures feasible at scale: without it,
//! every helper function is opaque to Z3 and the user has to rewrite
//! manually. With it, the proof search can chain unfoldings of
//! user-defined functions automatically.

use verum_common::{List, Map, Text};

/// A single reflected function definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectedFunction {
    /// Fully-qualified function name (e.g., `"core.math.cubical.id_equiv"`).
    pub name: Text,
    /// Formal parameter names, in order.
    pub parameters: List<Text>,
    /// SMT-LIB rendering of the function body, with parameters
    /// referenced as bare identifiers.
    pub body_smtlib: Text,
    /// SMT-LIB sort of the function's return value (e.g., `"Int"`).
    pub return_sort: Text,
    /// SMT-LIB sorts of the parameters, in order.
    pub parameter_sorts: List<Text>,
}

impl ReflectedFunction {
    /// Render the universally-quantified axiom in SMT-LIB form:
    /// `(forall ((x₁ S₁) ... (xₙ Sₙ)) (= (f x₁ ... xₙ) body))`.
    pub fn to_smtlib_axiom(&self) -> Text {
        let mut out = String::with_capacity(64 + self.body_smtlib.as_str().len());
        if self.parameters.is_empty() {
            // Nullary functions reflect as a plain equality.
            out.push_str(&format!(
                "(assert (= ({}) {}))",
                self.name.as_str(),
                self.body_smtlib.as_str()
            ));
        } else {
            out.push_str("(assert (forall (");
            for (p, s) in self.parameters.iter().zip(self.parameter_sorts.iter()) {
                out.push_str(&format!("({} {}) ", p.as_str(), s.as_str()));
            }
            // remove trailing space
            if out.ends_with(' ') {
                out.pop();
            }
            out.push_str(") (= (");
            out.push_str(self.name.as_str());
            for p in self.parameters.iter() {
                out.push(' ');
                out.push_str(p.as_str());
            }
            out.push_str(") ");
            out.push_str(self.body_smtlib.as_str());
            out.push_str(")))");
        }
        Text::from(out)
    }

    /// Render the function declaration: `(declare-fun f (S₁ … Sₙ) S)`.
    pub fn to_smtlib_decl(&self) -> Text {
        let mut out = String::new();
        out.push_str("(declare-fun ");
        out.push_str(self.name.as_str());
        out.push_str(" (");
        for (i, s) in self.parameter_sorts.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(s.as_str());
        }
        out.push_str(") ");
        out.push_str(self.return_sort.as_str());
        out.push(')');
        Text::from(out)
    }
}

/// The registry of reflected user-function definitions.
///
/// Indexed by qualified function name for O(1) lookup. The verifier
/// applies the registry to its solver context once per proof goal,
/// so the cost of accumulating axioms is proportional to the number
/// of *distinct* reflected functions referenced, not the proof size.
#[derive(Debug, Default, Clone)]
pub struct RefinementReflectionRegistry {
    by_name: Map<Text, ReflectedFunction>,
}

impl RefinementReflectionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a reflected function. Returns `Err` if a function
    /// with this name has already been registered with a *different*
    /// definition (re-registration of the same definition is OK).
    pub fn register(
        &mut self,
        f: ReflectedFunction,
    ) -> Result<(), ReflectionError> {
        if let Some(existing) = self.by_name.get(&f.name) {
            if existing != &f {
                return Err(ReflectionError::Conflict {
                    name: f.name.clone(),
                });
            }
            return Ok(());
        }
        self.by_name.insert(f.name.clone(), f);
        Ok(())
    }

    pub fn lookup(&self, name: &Text) -> Option<&ReflectedFunction> {
        self.by_name.get(name)
    }

    /// Iterate over every reflected function in the registry. Used by
    /// the translator to register callee signatures so the UF fallback
    /// emits `FuncDecl`s with the correct sort signature (Bool/Real/
    /// Int) and stays in sync with the SMT-LIB declaration block.
    pub fn iter(&self) -> impl Iterator<Item = &ReflectedFunction> {
        self.by_name.values()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Render every reflected function as an SMT-LIB declaration +
    /// axiom block, suitable for direct injection into a solver
    /// context. Declarations precede axioms so that forward
    /// references resolve.
    pub fn to_smtlib_block(&self) -> Text {
        let mut out = String::new();
        // Stable ordering so runs are deterministic for proof_stability.
        let mut names: Vec<&Text> = self.by_name.keys().collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));

        for n in &names {
            out.push_str(self.by_name[*n].to_smtlib_decl().as_str());
            out.push('\n');
        }
        for n in &names {
            out.push_str(self.by_name[*n].to_smtlib_axiom().as_str());
            out.push('\n');
        }
        Text::from(out)
    }

    /// Apply the entire registry to a solver via a callback.
    /// The callback receives one assertion string per axiom; the
    /// caller is responsible for parsing/asserting it into Z3.
    pub fn apply_to_solver<F>(&self, mut sink: F)
    where
        F: FnMut(&str),
    {
        let mut names: Vec<&Text> = self.by_name.keys().collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for n in &names {
            sink(self.by_name[*n].to_smtlib_decl().as_str());
        }
        for n in &names {
            sink(self.by_name[*n].to_smtlib_axiom().as_str());
        }
    }
}

/// Errors that can arise when reflecting a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionError {
    /// Two different definitions registered under the same name.
    Conflict { name: Text },
    /// Function flunked the soundness gate (impure / partial / open).
    NotReflectable { name: Text, reason: Text },
    /// Body could not be lowered to SMT (unsupported expression).
    UnsupportedBody { name: Text, reason: Text },
}

impl std::fmt::Display for ReflectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { name } => {
                write!(f, "refinement reflection conflict for `{}`", name.as_str())
            }
            Self::NotReflectable { name, reason } => write!(
                f,
                "function `{}` is not reflectable: {}",
                name.as_str(),
                reason.as_str()
            ),
            Self::UnsupportedBody { name, reason } => write!(
                f,
                "function `{}` body cannot be reflected: {}",
                name.as_str(),
                reason.as_str()
            ),
        }
    }
}

impl std::error::Error for ReflectionError {}

/// Soundness gate: returns `Ok` iff the function may be reflected.
///
/// Pure callers pass simple booleans for each property; in the full
/// integration these will be derived from the type checker's
/// `PropertySet` analysis.
pub fn is_reflectable(
    name: &Text,
    is_pure: bool,
    is_total: bool,
    body_uses_only_params_and_known: bool,
) -> Result<(), ReflectionError> {
    if !is_pure {
        return Err(ReflectionError::NotReflectable {
            name: name.clone(),
            reason: Text::from("function has side effects (Mutates/IO/Async/Fallible)"),
        });
    }
    if !is_total {
        return Err(ReflectionError::NotReflectable {
            name: name.clone(),
            reason: Text::from("function is not provably total (panics or non-terminating)"),
        });
    }
    if !body_uses_only_params_and_known {
        return Err(ReflectionError::NotReflectable {
            name: name.clone(),
            reason: Text::from(
                "body references symbols outside formal parameters or other reflected functions",
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn double_reflected() -> ReflectedFunction {
        ReflectedFunction {
            name: Text::from("double"),
            parameters: List::from_iter([Text::from("n")]),
            body_smtlib: Text::from("(* 2 n)"),
            return_sort: Text::from("Int"),
            parameter_sorts: List::from_iter([Text::from("Int")]),
        }
    }

    fn add_reflected() -> ReflectedFunction {
        ReflectedFunction {
            name: Text::from("add"),
            parameters: List::from_iter([Text::from("a"), Text::from("b")]),
            body_smtlib: Text::from("(+ a b)"),
            return_sort: Text::from("Int"),
            parameter_sorts: List::from_iter([Text::from("Int"), Text::from("Int")]),
        }
    }

    fn const_zero_reflected() -> ReflectedFunction {
        ReflectedFunction {
            name: Text::from("zero"),
            parameters: List::new(),
            body_smtlib: Text::from("0"),
            return_sort: Text::from("Int"),
            parameter_sorts: List::new(),
        }
    }

    #[test]
    fn axiom_unary_smt_form() {
        let f = double_reflected();
        let s = f.to_smtlib_axiom();
        assert_eq!(
            s.as_str(),
            "(assert (forall ((n Int)) (= (double n) (* 2 n))))"
        );
    }

    #[test]
    fn axiom_binary_smt_form() {
        let f = add_reflected();
        let s = f.to_smtlib_axiom();
        assert_eq!(
            s.as_str(),
            "(assert (forall ((a Int) (b Int)) (= (add a b) (+ a b))))"
        );
    }

    #[test]
    fn axiom_nullary_smt_form() {
        let f = const_zero_reflected();
        let s = f.to_smtlib_axiom();
        assert_eq!(s.as_str(), "(assert (= (zero) 0))");
    }

    #[test]
    fn declaration_form() {
        let f = add_reflected();
        let s = f.to_smtlib_decl();
        assert_eq!(s.as_str(), "(declare-fun add (Int Int) Int)");
    }

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(double_reflected()).unwrap();
        reg.register(add_reflected()).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.lookup(&Text::from("double")).is_some());
        assert!(reg.lookup(&Text::from("nope")).is_none());
    }

    #[test]
    fn registry_idempotent_reregister_same_def() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(double_reflected()).unwrap();
        // Re-registering the *same* definition is OK — supports
        // multi-pass type checking and incremental compilation.
        reg.register(double_reflected()).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_rejects_conflicting_definitions() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(double_reflected()).unwrap();

        let mut alt = double_reflected();
        alt.body_smtlib = Text::from("(+ n n)");
        let result = reg.register(alt);

        assert!(matches!(
            result,
            Err(ReflectionError::Conflict { .. })
        ));
    }

    #[test]
    fn registry_block_renders_decls_then_axioms() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(double_reflected()).unwrap();
        reg.register(add_reflected()).unwrap();

        let block = reg.to_smtlib_block();
        let s = block.as_str();

        // declare-fun lines come before assert lines (sorted alphabetically by name)
        let add_decl_pos = s.find("(declare-fun add").unwrap();
        let dbl_decl_pos = s.find("(declare-fun double").unwrap();
        let add_ax_pos = s.find("(assert (forall ((a Int)").unwrap();
        let dbl_ax_pos = s.find("(assert (forall ((n Int)").unwrap();

        assert!(add_decl_pos < add_ax_pos);
        assert!(dbl_decl_pos < dbl_ax_pos);
        assert!(add_decl_pos < dbl_decl_pos); // alphabetical
    }

    #[test]
    fn apply_to_solver_invokes_callback_per_assertion() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(double_reflected()).unwrap();
        reg.register(add_reflected()).unwrap();

        let mut received: Vec<String> = Vec::new();
        reg.apply_to_solver(|s| received.push(s.to_string()));

        // 2 decls + 2 axioms = 4 lines
        assert_eq!(received.len(), 4);
        assert!(received[0].starts_with("(declare-fun"));
        assert!(received[2].starts_with("(assert"));
    }

    #[test]
    fn reflectable_pure_total_closed_passes() {
        assert!(is_reflectable(&Text::from("f"), true, true, true).is_ok());
    }

    #[test]
    fn reflectable_impure_rejected() {
        let r = is_reflectable(&Text::from("f"), false, true, true);
        assert!(matches!(r, Err(ReflectionError::NotReflectable { .. })));
    }

    #[test]
    fn reflectable_partial_rejected() {
        let r = is_reflectable(&Text::from("f"), true, false, true);
        assert!(matches!(r, Err(ReflectionError::NotReflectable { .. })));
    }

    #[test]
    fn reflectable_open_rejected() {
        let r = is_reflectable(&Text::from("f"), true, true, false);
        assert!(matches!(r, Err(ReflectionError::NotReflectable { .. })));
    }
}
