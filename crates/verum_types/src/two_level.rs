//! Two-Level Type Theory (2LTT) — fibrant vs strict universe layers.
//!
//! In a single-universe HoTT, every type lives in the *fibrant*
//! fragment: equality is the path type, transport computes through
//! the cubical normalizer, and uniqueness of identity proofs (UIP)
//! does not hold. This makes HoTT pleasant for proving theorems
//! about types-as-spaces but inconvenient for *meta*-reasoning
//! about types-as-data, where strict equality and decidable
//! comparison are needed.
//!
//! 2LTT (Voevodsky, 2013; Annenkov–Capriotti–Kraus 2017) introduces
//! a **second**, strict universe layer. Strict types satisfy UIP
//! and have decidable equality; fibrant types support the cubical
//! reductions. Crucially, the two layers can **interact** through
//! a constraint that fibrant terms remain fibrant under operations
//! that mix in strict data.
//!
//! ## Layers
//!
//! ```text
//!     UFib_n  ≼  UStrict_n
//! ```
//!
//! Every fibrant type is a strict type at the same level (the
//! inclusion `≼`), but not vice versa. Definitions can mix layers
//! provided the result fits the **least permissive** layer of all
//! contributing components.
//!
//! ## API
//!
//! The [`Layer`] enum names the two universes; [`UniverseLevel`] is
//! the standard naturals-augmented-with-variables level used
//! elsewhere in the type checker. [`StratifiedUniverse`] pairs a
//! Layer with a UniverseLevel.
//!
//! [`mix_layers`] computes the layer of a composite type built
//! from sub-types of mixed layers — the result is `Strict` if any
//! sub-type is `Strict`, otherwise `Fibrant`. This is the central
//! soundness rule: strictness is contagious upward through the
//! type former.

use std::cmp::Ordering;

use verum_common::Text;

/// The two universe layers of 2LTT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// **Fibrant** layer — supports HoTT/cubical operations:
    /// path types, transport, hcomp, univalence. UIP does not
    /// hold.
    Fibrant,
    /// **Strict** layer — UIP holds, equality is decidable, no
    /// path computation. Used for meta-level reasoning, syntactic
    /// data, and definitions that must commute with substitution
    /// strictly.
    Strict,
}

impl Layer {
    /// The fibrant layer is "more refined" than strict (every
    /// fibrant type is strict, not vice versa). For ordering we
    /// adopt `Fibrant ≼ Strict`, i.e. `Fibrant < Strict`, so the
    /// `mix` operation is `max`.
    pub fn mix(self, other: Layer) -> Layer {
        match (self, other) {
            (Layer::Strict, _) | (_, Layer::Strict) => Layer::Strict,
            _ => Layer::Fibrant,
        }
    }
}

impl Ord for Layer {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Layer::Fibrant, Layer::Fibrant) | (Layer::Strict, Layer::Strict) => {
                Ordering::Equal
            }
            (Layer::Fibrant, Layer::Strict) => Ordering::Less,
            (Layer::Strict, Layer::Fibrant) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Layer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::Fibrant => write!(f, "fib"),
            Layer::Strict => write!(f, "strict"),
        }
    }
}

/// A simplified universe level — concrete or symbolic. Distinct
/// from `crate::ty::UniverseLevel` to keep the 2LTT module
/// self-contained; conversion helpers may be added in a future
/// integration step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UniverseLevel {
    /// `Type_n` for some concrete natural n.
    Concrete(u32),
    /// `Type_α` for a symbolic level variable.
    Variable(Text),
}

impl UniverseLevel {
    pub fn zero() -> Self {
        Self::Concrete(0)
    }

    pub fn succ(&self) -> Self {
        match self {
            UniverseLevel::Concrete(n) => UniverseLevel::Concrete(n + 1),
            UniverseLevel::Variable(name) => {
                UniverseLevel::Variable(Text::from(format!("{}+1", name.as_str())))
            }
        }
    }
}

/// A stratified universe: a layer plus a level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StratifiedUniverse {
    pub layer: Layer,
    pub level: UniverseLevel,
}

impl StratifiedUniverse {
    pub fn fibrant(level: UniverseLevel) -> Self {
        Self {
            layer: Layer::Fibrant,
            level,
        }
    }

    pub fn strict(level: UniverseLevel) -> Self {
        Self {
            layer: Layer::Strict,
            level,
        }
    }

    /// `UFib_n  ≼  UStrict_n` — every fibrant type at level n is
    /// also a strict type at level n. Levels must agree.
    pub fn coerces_to(&self, other: &StratifiedUniverse) -> bool {
        if self.level != other.level {
            return false;
        }
        self.layer <= other.layer
    }
}

impl std::fmt::Display for StratifiedUniverse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.level {
            UniverseLevel::Concrete(n) => write!(f, "U_{}_{}", self.layer, n),
            UniverseLevel::Variable(name) => {
                write!(f, "U_{}_{}", self.layer, name.as_str())
            }
        }
    }
}

/// Compute the layer of a composite type built from sub-types of
/// the given layers. Returns `Strict` if any input is `Strict`,
/// otherwise `Fibrant`. This is the **strictness contagion** rule.
pub fn mix_layers(layers: &[Layer]) -> Layer {
    layers
        .iter()
        .copied()
        .fold(Layer::Fibrant, |acc, l| acc.mix(l))
}

/// A 2LTT violation: a fibrant context expected a fibrant type but
/// received a strict one, or universe levels disagree under the
/// coercion rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerViolation {
    /// Strict type used in fibrant context (downward-violation).
    StrictInFibrantContext { universe: StratifiedUniverse },
    /// Universe levels disagree under attempted coercion.
    LevelMismatch {
        from: UniverseLevel,
        to: UniverseLevel,
    },
}

impl std::fmt::Display for LayerViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StrictInFibrantContext { universe } => write!(
                f,
                "2LTT: strict type `{}` cannot occupy fibrant context",
                universe
            ),
            Self::LevelMismatch { from, to } => write!(
                f,
                "2LTT: cannot coerce universe level {:?} to {:?}",
                from, to
            ),
        }
    }
}

impl std::error::Error for LayerViolation {}

/// Validate that a type at universe `actual` may be used in a
/// context expecting universe `expected`. Returns `Ok(())` when
/// `actual ≼ expected` (same layer, or fibrant flowing into strict
/// context). Returns `Err(LayerViolation)` otherwise.
pub fn check_layer_flow(
    actual: &StratifiedUniverse,
    expected: &StratifiedUniverse,
) -> Result<(), LayerViolation> {
    if actual.level != expected.level {
        return Err(LayerViolation::LevelMismatch {
            from: actual.level.clone(),
            to: expected.level.clone(),
        });
    }
    if actual.layer == Layer::Strict && expected.layer == Layer::Fibrant {
        return Err(LayerViolation::StrictInFibrantContext {
            universe: actual.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fibrant_less_than_strict() {
        assert!(Layer::Fibrant < Layer::Strict);
        assert!(Layer::Strict > Layer::Fibrant);
    }

    #[test]
    fn mix_two_fibrant_is_fibrant() {
        assert_eq!(Layer::Fibrant.mix(Layer::Fibrant), Layer::Fibrant);
    }

    #[test]
    fn mix_with_strict_yields_strict() {
        assert_eq!(Layer::Fibrant.mix(Layer::Strict), Layer::Strict);
        assert_eq!(Layer::Strict.mix(Layer::Fibrant), Layer::Strict);
        assert_eq!(Layer::Strict.mix(Layer::Strict), Layer::Strict);
    }

    #[test]
    fn mix_layers_respects_strictness_contagion() {
        let result = mix_layers(&[
            Layer::Fibrant,
            Layer::Fibrant,
            Layer::Strict,
            Layer::Fibrant,
        ]);
        assert_eq!(result, Layer::Strict);
    }

    #[test]
    fn mix_layers_empty_defaults_to_fibrant() {
        assert_eq!(mix_layers(&[]), Layer::Fibrant);
    }

    #[test]
    fn fibrant_universe_coerces_to_strict_at_same_level() {
        let fib = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        let strct = StratifiedUniverse::strict(UniverseLevel::Concrete(0));
        assert!(fib.coerces_to(&strct));
    }

    #[test]
    fn strict_does_not_coerce_to_fibrant() {
        let strct = StratifiedUniverse::strict(UniverseLevel::Concrete(0));
        let fib = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        assert!(!strct.coerces_to(&fib));
    }

    #[test]
    fn coercion_requires_matching_levels() {
        let fib0 = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        let strct1 = StratifiedUniverse::strict(UniverseLevel::Concrete(1));
        assert!(!fib0.coerces_to(&strct1));
    }

    #[test]
    fn check_layer_flow_permits_fibrant_to_strict() {
        let fib = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        let strct = StratifiedUniverse::strict(UniverseLevel::Concrete(0));
        assert!(check_layer_flow(&fib, &strct).is_ok());
    }

    #[test]
    fn check_layer_flow_rejects_strict_in_fibrant_context() {
        let strct = StratifiedUniverse::strict(UniverseLevel::Concrete(0));
        let fib = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        let err = check_layer_flow(&strct, &fib).unwrap_err();
        assert!(matches!(
            err,
            LayerViolation::StrictInFibrantContext { .. }
        ));
    }

    #[test]
    fn check_layer_flow_rejects_level_mismatch() {
        let fib0 = StratifiedUniverse::fibrant(UniverseLevel::Concrete(0));
        let fib1 = StratifiedUniverse::fibrant(UniverseLevel::Concrete(1));
        let err = check_layer_flow(&fib0, &fib1).unwrap_err();
        assert!(matches!(err, LayerViolation::LevelMismatch { .. }));
    }

    #[test]
    fn universe_level_succ_lifts_concrete() {
        let l = UniverseLevel::zero();
        assert_eq!(l.succ(), UniverseLevel::Concrete(1));
        assert_eq!(l.succ().succ(), UniverseLevel::Concrete(2));
    }

    #[test]
    fn universe_level_succ_annotates_variables() {
        let v = UniverseLevel::Variable(Text::from("u"));
        if let UniverseLevel::Variable(name) = v.succ() {
            assert_eq!(name.as_str(), "u+1");
        } else {
            panic!("expected variable level");
        }
    }

    #[test]
    fn stratified_display_includes_layer_and_level() {
        let s = StratifiedUniverse::fibrant(UniverseLevel::Concrete(2));
        assert_eq!(format!("{}", s), "U_fib_2");
        let t = StratifiedUniverse::strict(UniverseLevel::Concrete(0));
        assert_eq!(format!("{}", t), "U_strict_0");
    }

    #[test]
    fn same_layer_same_level_coerces() {
        let a = StratifiedUniverse::fibrant(UniverseLevel::Concrete(3));
        let b = StratifiedUniverse::fibrant(UniverseLevel::Concrete(3));
        assert!(a.coerces_to(&b));
    }
}
