//! Tactic Metaprogramming — quote, splice, reflect.
//!
//! In a tactic metaprogramming system the user writes proof
//! procedures *in the host language* (here, Verum itself), inspects
//! and constructs proof terms as data, and elaborates the result
//! back into the type checker. Lean 4, Coq's Ltac2, and Agda's
//! reflection module all implement variants of this.
//!
//! ## The metalanguage
//!
//! ```text
//!     M ::= ⌜e⌝          (quote: lift expression e to data)
//!         | ▸M           (splice: lower data M back to expression)
//!         | reflect(g)   (reflect goal g into a data value)
//!         | custom(F)    (call user-defined elaborator F)
//!         | M₁ ; M₂      (sequence)
//!         | const(v)     (an opaque constant value)
//! ```
//!
//! ## Quote–splice duality
//!
//! `splice(quote(e)) ≡ e` — splicing a quoted expression yields it
//! back. The reverse `quote(splice(M))` only normalises when M is
//! already a quote — otherwise `quote` and `splice` remain stuck
//! on opaque metavalues, mirroring Lean 4's `Expr.quote` and
//! `Expr.unquote` behaviour.
//!
//! ## Custom elaborators
//!
//! `custom(F)` calls a registered elaborator `F: MetaTerm →
//! MetaTerm` to perform arbitrary transformations on quoted
//! expressions. Elaborators are registered in [`MetaContext`] by
//! name; this enables third-party tactic libraries without
//! recompiling the host.
//!
//! ## Status
//!
//! This module is the standalone evaluator core. Wiring `quote`
//! and `splice` to the actual Verum AST (via `expr_to_eqterm` and
//! its inverse) is a future integration step.

use std::collections::HashMap;

use verum_common::Text;

/// A metaprogramming term. Distinct from the object-level
/// `verum_ast::Expr` — meta terms describe *operations on*
/// expressions rather than expressions themselves.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaTerm {
    /// `⌜e⌝` — a quoted expression. The payload is an opaque
    /// representation (here a textual marker) standing in for the
    /// real `verum_ast::Expr` until the AST integration lands.
    Quote(Text),

    /// `▸M` — splice. Reduces to the underlying expression when M
    /// is a `Quote`, otherwise stays stuck.
    Splice(Box<MetaTerm>),

    /// `reflect(g)` — turn a goal description into a meta value
    /// that elaborators can inspect.
    Reflect(Text),

    /// `custom(name, arg)` — invoke the user-registered elaborator
    /// `name` with `arg`. The elaborator must be present in the
    /// `MetaContext` at evaluation time.
    Custom { name: Text, arg: Box<MetaTerm> },

    /// `M₁ ; M₂` — sequential composition; the result is M₂'s value
    /// after M₁ has been evaluated for its side effects.
    Seq(Box<MetaTerm>, Box<MetaTerm>),

    /// An opaque constant value (string-keyed). Used to represent
    /// elaborator outputs that can't be further reduced.
    Const(Text),
}

impl MetaTerm {
    pub fn quote(s: impl Into<Text>) -> Self {
        Self::Quote(s.into())
    }

    pub fn splice(m: MetaTerm) -> Self {
        Self::Splice(Box::new(m))
    }

    pub fn reflect(s: impl Into<Text>) -> Self {
        Self::Reflect(s.into())
    }

    pub fn custom(name: impl Into<Text>, arg: MetaTerm) -> Self {
        Self::Custom {
            name: name.into(),
            arg: Box::new(arg),
        }
    }

    pub fn seq(a: MetaTerm, b: MetaTerm) -> Self {
        Self::Seq(Box::new(a), Box::new(b))
    }

    pub fn cnst(s: impl Into<Text>) -> Self {
        Self::Const(s.into())
    }
}

/// A registered user elaborator. Takes one meta argument, returns
/// the rewritten meta term.
pub type Elaborator = std::sync::Arc<dyn Fn(&MetaTerm) -> MetaTerm + Send + Sync>;

/// Evaluation context for metaprogramming. Carries the elaborator
/// registry plus a small cache of reflected goals.
#[derive(Clone, Default)]
pub struct MetaContext {
    elaborators: HashMap<Text, Elaborator>,
    reflected_goals: HashMap<Text, MetaTerm>,
}

impl MetaContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a custom elaborator under `name`. Subsequent
    /// `Custom { name, arg }` terms invoke it during evaluation.
    pub fn register_elaborator(&mut self, name: impl Into<Text>, f: Elaborator) {
        self.elaborators.insert(name.into(), f);
    }

    pub fn elaborator_count(&self) -> usize {
        self.elaborators.len()
    }

    /// Cache a reflected goal so subsequent `Reflect(name)` lookups
    /// return the same data without recomputing.
    pub fn cache_reflected(&mut self, name: impl Into<Text>, m: MetaTerm) {
        self.reflected_goals.insert(name.into(), m);
    }

    /// Evaluate a meta term to its normal form.
    ///
    /// Reduction rules:
    ///
    ///   * `splice(quote(e))   ↦ quote(e)`  (β: cancellation)
    ///   * `custom(name, arg)  ↦ F(arg)` if F is registered
    ///   * `reflect(g)         ↦ cached(g)` if cached
    ///   * `seq(M₁, M₂)        ↦ M₂'`     where M₂' = eval(M₂)
    ///                                     after eval(M₁) has been
    ///                                     invoked for its effects
    pub fn eval(&self, term: &MetaTerm) -> MetaTerm {
        match term {
            MetaTerm::Quote(_) | MetaTerm::Const(_) => term.clone(),

            // Splice cancellation: ▸⌜e⌝ ↦ ⌜e⌝
            MetaTerm::Splice(inner) => {
                let v = self.eval(inner);
                if matches!(v, MetaTerm::Quote(_)) {
                    v
                } else {
                    MetaTerm::Splice(Box::new(v))
                }
            }

            MetaTerm::Reflect(name) => self
                .reflected_goals
                .get(name)
                .cloned()
                .unwrap_or_else(|| term.clone()),

            MetaTerm::Custom { name, arg } => {
                let arg_v = self.eval(arg);
                if let Some(f) = self.elaborators.get(name) {
                    self.eval(&f(&arg_v))
                } else {
                    MetaTerm::Custom {
                        name: name.clone(),
                        arg: Box::new(arg_v),
                    }
                }
            }

            MetaTerm::Seq(a, b) => {
                let _ = self.eval(a);
                self.eval(b)
            }
        }
    }

    /// Convenience: quote then splice round-trip should be identity.
    /// Useful sanity-check for elaborator implementers.
    pub fn quote_splice_roundtrip(&self, payload: impl Into<Text>) -> MetaTerm {
        let p = payload.into();
        let q = MetaTerm::quote(p.clone());
        let sq = MetaTerm::splice(q);
        self.eval(&sq)
    }
}

impl std::fmt::Debug for MetaContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaContext")
            .field("elaborators", &self.elaborators.keys().collect::<Vec<_>>())
            .field("reflected_goals", &self.reflected_goals.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_evaluates_to_itself() {
        let ctx = MetaContext::new();
        let q = MetaTerm::quote("x + 1");
        assert_eq!(ctx.eval(&q), q);
    }

    #[test]
    fn const_evaluates_to_itself() {
        let ctx = MetaContext::new();
        let c = MetaTerm::cnst("zero");
        assert_eq!(ctx.eval(&c), c);
    }

    #[test]
    fn splice_quote_cancels() {
        let ctx = MetaContext::new();
        let inner = MetaTerm::quote("body");
        let term = MetaTerm::splice(inner.clone());
        assert_eq!(ctx.eval(&term), inner);
    }

    #[test]
    fn splice_const_stays_stuck() {
        let ctx = MetaContext::new();
        let inner = MetaTerm::cnst("opaque");
        let term = MetaTerm::splice(inner.clone());
        assert_eq!(ctx.eval(&term), MetaTerm::splice(inner));
    }

    #[test]
    fn quote_splice_roundtrip_is_identity() {
        let ctx = MetaContext::new();
        let r = ctx.quote_splice_roundtrip("body");
        assert_eq!(r, MetaTerm::quote("body"));
    }

    #[test]
    fn nested_splice_reduces_inner_first() {
        let ctx = MetaContext::new();
        // ▸▸⌜e⌝
        let nested = MetaTerm::splice(MetaTerm::splice(MetaTerm::quote("e")));
        // ▸⌜e⌝ ↦ ⌜e⌝, then ▸⌜e⌝ ↦ ⌜e⌝
        assert_eq!(ctx.eval(&nested), MetaTerm::quote("e"));
    }

    #[test]
    fn custom_elaborator_invoked_when_registered() {
        let mut ctx = MetaContext::new();
        let f: Elaborator = std::sync::Arc::new(|arg| match arg {
            MetaTerm::Quote(s) => {
                let upper = s.as_str().to_uppercase();
                MetaTerm::Quote(Text::from(upper))
            }
            other => other.clone(),
        });
        ctx.register_elaborator("upcase", f);

        let term = MetaTerm::custom("upcase", MetaTerm::quote("hello"));
        assert_eq!(ctx.eval(&term), MetaTerm::quote("HELLO"));
    }

    #[test]
    fn unregistered_custom_stays_stuck() {
        let ctx = MetaContext::new();
        let term = MetaTerm::custom("nonexistent", MetaTerm::quote("x"));
        // Arg is still evaluated; the custom call itself remains.
        let r = ctx.eval(&term);
        assert!(matches!(r, MetaTerm::Custom { .. }));
    }

    #[test]
    fn elaborator_chain_composes() {
        let mut ctx = MetaContext::new();
        let upcase: Elaborator = std::sync::Arc::new(|arg| match arg {
            MetaTerm::Quote(s) => MetaTerm::Quote(Text::from(s.as_str().to_uppercase())),
            other => other.clone(),
        });
        let suffix: Elaborator = std::sync::Arc::new(|arg| match arg {
            MetaTerm::Quote(s) => {
                MetaTerm::Quote(Text::from(format!("{}!", s.as_str())))
            }
            other => other.clone(),
        });
        ctx.register_elaborator("upcase", upcase);
        ctx.register_elaborator("bang", suffix);

        // bang(upcase(⌜hi⌝))  ↦ bang(⌜HI⌝) ↦ ⌜HI!⌝
        let term = MetaTerm::custom(
            "bang",
            MetaTerm::custom("upcase", MetaTerm::quote("hi")),
        );
        assert_eq!(ctx.eval(&term), MetaTerm::quote("HI!"));
    }

    #[test]
    fn reflect_returns_cached_goal() {
        let mut ctx = MetaContext::new();
        let body = MetaTerm::quote("forall x. P(x)");
        ctx.cache_reflected("main_goal", body.clone());
        assert_eq!(ctx.eval(&MetaTerm::reflect("main_goal")), body);
    }

    #[test]
    fn reflect_uncached_stays_stuck() {
        let ctx = MetaContext::new();
        let r = MetaTerm::reflect("missing");
        assert_eq!(ctx.eval(&r), r);
    }

    #[test]
    fn seq_evaluates_to_second_arm() {
        let ctx = MetaContext::new();
        let term = MetaTerm::seq(MetaTerm::quote("first"), MetaTerm::quote("second"));
        assert_eq!(ctx.eval(&term), MetaTerm::quote("second"));
    }

    #[test]
    fn elaborator_count_tracks_registrations() {
        let mut ctx = MetaContext::new();
        assert_eq!(ctx.elaborator_count(), 0);
        let f: Elaborator = std::sync::Arc::new(|x| x.clone());
        ctx.register_elaborator("a", f.clone());
        ctx.register_elaborator("b", f);
        assert_eq!(ctx.elaborator_count(), 2);
    }

    #[test]
    fn deep_splice_inside_custom_arg() {
        let mut ctx = MetaContext::new();
        let id: Elaborator = std::sync::Arc::new(|x| x.clone());
        ctx.register_elaborator("id", id);

        // id(▸⌜x⌝)  ↦ id(⌜x⌝) ↦ ⌜x⌝
        let term = MetaTerm::custom(
            "id",
            MetaTerm::splice(MetaTerm::quote("x")),
        );
        assert_eq!(ctx.eval(&term), MetaTerm::quote("x"));
    }

    #[test]
    fn elaborator_can_emit_constants() {
        let mut ctx = MetaContext::new();
        let to_const: Elaborator = std::sync::Arc::new(|_arg| MetaTerm::cnst("DONE"));
        ctx.register_elaborator("finalize", to_const);

        let r = ctx.eval(&MetaTerm::custom("finalize", MetaTerm::quote("anything")));
        assert_eq!(r, MetaTerm::cnst("DONE"));
    }
}
