//! Refinement Reflection — User Functions as SMT Axioms.
//!

//! When a Verum function `f` is **pure** and **total**, its definition
//! `f(x₁, …, xₙ) = body` can be safely reflected into the SMT context
//! as a universally-quantified equality:
//!

//! ```text
//!  ∀ x₁ … xₙ. f(x₁, …, xₙ) = ⟦body⟧
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
//!  (all of which would let `f(x)` denote different values on
//!  different calls).
//! * **Total** — terminate on every input; do not call `panic`,
//!  `unreachable`, or partial primitives.
//! * **Closed** — body uses only its formal parameters and other
//!  already-reflected functions.
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

//! ## Why it matters
//!

//! Refinement reflection is what makes SMT-backed proofs over
//! arbitrary user data structures feasible at scale: without it,
//! every helper function is opaque to Z3 and the user has to
//! rewrite manually. With it, the proof search can chain
//! unfoldings of user-defined functions automatically — the
//! mechanism that scales refinement types from trivial integer
//! bounds to domain-rich stdlib and downstream-project proof
//! corpora.

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

/// A reflected-function entry that the closure pass removed from the
/// emitted SMT-LIB block, together with the symbol that forced the
/// removal.
///
/// Surfaced to the caller (verify CLI / verification pipeline) so a
/// *loud* diagnostic can name the skipped function and the undeclared
/// symbol. Skipping the one open entry keeps the rest of the module's
/// reflection intact — before this gate a single such entry made Z3's
/// `from_string` reject the whole block, silently disabling *all*
/// refinement reflection for the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionDrop {
    /// Name of the function that could not be reflected soundly.
    pub name: Text,
    /// The first body symbol that is neither an SMT-LIB builtin, one of
    /// the function's formal parameters, nor another reflected function.
    pub missing_symbol: Text,
}

/// True for the fixed set of SMT-LIB operators / atoms that
/// [`crate::expr_to_smtlib`] can emit as application heads (see
/// `binop_to_smtlib` plus the `Unary` / `If` arms) together with numeric
/// and boolean literals. Every *other* symbol appearing in a reflected
/// body must be a formal parameter or another reflected function —
/// otherwise the emitted block references a symbol it never declares and
/// Z3 rejects the block wholesale.
fn is_smtlib_builtin_symbol(tok: &str) -> bool {
    // Numeric literal (int or float): the translator renders these via
    // `format!("{}", value)`, which always starts with an ASCII digit;
    // no SMT operator or Verum identifier does.
    if tok.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return true;
    }
    // Variant-path constants `path_K.A` are declared at the top of the
    // reflection block (see `to_smtlib_block`) and by the goal-side Z3-AST
    // translator, so they are always in scope. Treat them as declared so the
    // closure gate does not drop a body that dispatches on a variant — the
    // `.` in the token means the paren/whitespace split keeps it whole.
    if tok.starts_with("path_") {
        return true;
    }
    matches!(
        tok,
        "+" | "-" | "*" | "div" | "mod"
            | "=" | "<" | "<=" | ">" | ">="
            | "and" | "or" | "not" | "=>" | "ite" | "distinct"
            | "true" | "false"
    )
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
    pub fn register(&mut self, f: ReflectedFunction) -> Result<(), ReflectionError> {
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
        // Close the registry under its call graph and omit any open entry
        // (T0489): a body naming a symbol the block never declares would
        // make Z3 reject the *entire* block. Emitting only the closed
        // subset makes that poisoning structurally impossible — one bad
        // axiom degrades exactly one function, not the whole module.
        let (dropped, _drops) = self.dropped_entry_names();
        // Stable ordering so runs are deterministic for proof_stability.
        let mut names: Vec<&Text> = self
            .by_name
            .keys()
            .filter(|n| !dropped.contains(n.as_str()))
            .collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));

        // Declare the variant-path constants (`path_K.A`, sort Int) referenced
        // by the reflected bodies BEFORE the function declarations, so Z3's
        // `from_string` can resolve them. The block is injected before the
        // goal/axiom side does `Int::new_const("path_K.A")` (verify_cmd.rs
        // :1211-1220), so declaring here first means the later `new_const`
        // reuses the same Int symbol — no double-declaration. A BTreeSet keeps
        // the emission order deterministic for proof_stability.
        let mut path_consts: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for n in &names {
            for tok in self.by_name[*n]
                .body_smtlib
                .as_str()
                .split(|c: char| c == '(' || c == ')' || c.is_whitespace())
                .filter(|t| t.starts_with("path_"))
            {
                path_consts.insert(tok.to_string());
            }
        }
        for pc in &path_consts {
            out.push_str("(declare-const ");
            out.push_str(pc);
            out.push_str(" Int)\n");
        }

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
        // Same closure gate as `to_smtlib_block` (T0489): only entries
        // that are closed under the call graph are handed to the solver.
        let (dropped, _drops) = self.dropped_entry_names();
        let mut names: Vec<&Text> = self
            .by_name
            .keys()
            .filter(|n| !dropped.contains(n.as_str()))
            .collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for n in &names {
            sink(self.by_name[*n].to_smtlib_decl().as_str());
        }
        for n in &names {
            sink(self.by_name[*n].to_smtlib_axiom().as_str());
        }
    }

    /// Compute the entries that must be dropped to close the registry
    /// under its call graph, together with the first undeclared symbol
    /// that forced each drop. Pure — never mutates the registry, so an
    /// omitted function keeps its entry (and thus its sort signature via
    /// [`Self::iter`]) and simply stays an *opaque* symbol at use sites.
    ///
    /// A reflected function is *open* when its body names a symbol that is
    /// neither an SMT-LIB builtin, one of its own formal parameters, nor
    /// another **still-live** reflected function. Dropping one entry can
    /// open another (a caller of the dropped function), so the pass
    /// iterates to a fixpoint. Drops are returned in a deterministic
    /// order independent of map iteration order.
    fn dropped_entry_names(&self) -> (std::collections::BTreeSet<String>, Vec<ReflectionDrop>) {
        use std::collections::BTreeSet;
        let mut live: BTreeSet<String> =
            self.by_name.keys().map(|n| n.as_str().to_string()).collect();
        let mut drops: Vec<ReflectionDrop> = Vec::new();
        loop {
            let mut newly_dropped: Vec<(String, String)> = Vec::new();
            for f in self.by_name.values() {
                let name = f.name.as_str();
                if !live.contains(name) {
                    continue; // already dropped in an earlier pass
                }
                if let Some(sym) = Self::first_undeclared_symbol(f, &live) {
                    newly_dropped.push((name.to_string(), sym));
                }
            }
            if newly_dropped.is_empty() {
                break;
            }
            newly_dropped.sort();
            for (name, sym) in newly_dropped {
                live.remove(&name);
                drops.push(ReflectionDrop {
                    name: Text::from(name.as_str()),
                    missing_symbol: Text::from(sym.as_str()),
                });
            }
        }
        let dropped: BTreeSet<String> =
            drops.iter().map(|d| d.name.as_str().to_string()).collect();
        (dropped, drops)
    }

    /// First symbol in `f`'s body that is not an SMT-LIB builtin, not one
    /// of `f`'s formal parameters, and not a currently-live reflected
    /// function. `None` ⇒ `f` is closed under `live`.
    fn first_undeclared_symbol(
        f: &ReflectedFunction,
        live: &std::collections::BTreeSet<String>,
    ) -> Option<String> {
        // The translator emits no binders (no `forall`/`exists`/`let`),
        // so every whitespace/paren-delimited token is either an operator,
        // a literal, or an applied/referenced symbol — splitting on parens
        // and whitespace yields exactly the referenced-symbol set.
        for tok in f
            .body_smtlib
            .as_str()
            .split(|c: char| c == '(' || c == ')' || c.is_whitespace())
            .filter(|t| !t.is_empty())
        {
            if is_smtlib_builtin_symbol(tok) {
                continue;
            }
            if f.parameters.iter().any(|p| p.as_str() == tok) {
                continue;
            }
            if live.contains(tok) {
                continue;
            }
            return Some(tok.to_string());
        }
        None
    }

    /// The entries the SMT-LIB block will omit, for a loud caller-side
    /// diagnostic. Each names the skipped function and the undeclared
    /// symbol that forced the skip. Empty ⇒ the whole registry is closed
    /// and every reflected function is emitted.
    pub fn open_entry_drops(&self) -> Vec<ReflectionDrop> {
        self.dropped_entry_names().1
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

        assert!(matches!(result, Err(ReflectionError::Conflict { .. })));
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

    // ----------------------------------------------------------------
    // T0489 — one open axiom must not poison the whole block.
    // ----------------------------------------------------------------

    /// Body references `helper_unreflected`, which is neither a parameter
    /// nor another reflected function — the poison shape from the
    /// reproducer (a caller of a body-less / multi-statement function).
    fn uses_helper_reflected() -> ReflectedFunction {
        ReflectedFunction {
            name: Text::from("uses_helper"),
            parameters: List::from_iter([Text::from("x")]),
            body_smtlib: Text::from("(helper_unreflected x)"),
            return_sort: Text::from("Int"),
            parameter_sorts: List::from_iter([Text::from("Int")]),
        }
    }

    #[test]
    fn block_drops_open_entry_keeps_closed() {
        // add (closed) + uses_helper (open): the block must keep add's
        // decl+axiom and OMIT uses_helper entirely, so Z3 never sees the
        // undeclared `helper_unreflected` and the good axiom survives.
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(add_reflected()).unwrap();
        reg.register(uses_helper_reflected()).unwrap();

        let block = reg.to_smtlib_block();
        let s = block.as_str();
        assert!(s.contains("(declare-fun add"));
        assert!(s.contains("(= (add a b) (+ a b))"));
        assert!(!s.contains("uses_helper"));
        assert!(!s.contains("helper_unreflected"));
    }

    #[test]
    fn open_entry_drops_names_function_and_symbol() {
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(add_reflected()).unwrap();
        reg.register(uses_helper_reflected()).unwrap();

        let drops = reg.open_entry_drops();
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].name.as_str(), "uses_helper");
        assert_eq!(drops[0].missing_symbol.as_str(), "helper_unreflected");
    }

    #[test]
    fn closure_is_transitive_to_fixpoint() {
        // chain_a -> chain_b -> helper_unreflected(undeclared). Removing
        // chain_b opens chain_a, so BOTH must be dropped; add is untouched.
        let mut reg = RefinementReflectionRegistry::new();
        reg.register(add_reflected()).unwrap();
        let mk = |name: &str, body: &str| ReflectedFunction {
            name: Text::from(name),
            parameters: List::from_iter([Text::from("x")]),
            body_smtlib: Text::from(body),
            return_sort: Text::from("Int"),
            parameter_sorts: List::from_iter([Text::from("Int")]),
        };
        reg.register(mk("chain_a", "(chain_b x)")).unwrap();
        reg.register(mk("chain_b", "(helper_unreflected x)")).unwrap();

        assert_eq!(reg.open_entry_drops().len(), 2);
        let block = reg.to_smtlib_block();
        let s = block.as_str();
        assert!(s.contains("(declare-fun add"));
        assert!(!s.contains("chain_a"));
        assert!(!s.contains("chain_b"));
    }

    #[test]
    fn self_recursion_is_kept() {
        // A function may reference its OWN name (recursion): the name is
        // declared in the block, so it does not poison and must be kept.
        let mut reg = RefinementReflectionRegistry::new();
        let fact = ReflectedFunction {
            name: Text::from("fact"),
            parameters: List::from_iter([Text::from("n")]),
            body_smtlib: Text::from("(ite (= n 0) 1 (* n (fact (- n 1))))"),
            return_sort: Text::from("Int"),
            parameter_sorts: List::from_iter([Text::from("Int")]),
        };
        reg.register(fact).unwrap();
        assert!(reg.open_entry_drops().is_empty());
        assert!(reg.to_smtlib_block().as_str().contains("(declare-fun fact"));
    }
}
