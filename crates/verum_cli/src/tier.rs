//! Shared execution-tier resolver used by `run`, `test` and `bench`.
//!
//! Keeps the flag semantics identical across commands:
//!   * `--interp`          → Tier::Interpret
//!   * `--aot`             → Tier::Aot
//!   * `--tier <name>`     → resolved via `LanguageFeatureOverrides::tier`
//!   * otherwise           → caller-supplied default
//!
//! `--interp` and `--aot` are mutually exclusive in the clap definition;
//! the resolver additionally reports a clean error if the `--tier` value
//! is unknown, and lets each command pin its own default (e.g. `run` and
//! `bench` default to AOT; `check`-style commands may pick a different
//! default).

use crate::error::{CliError, Result};
use verum_common::Text;

/// Execution tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// VBC interpreter (Tier 0). In-process, full diagnostics, no LLVM.
    Interpret,
    /// AOT native compilation via LLVM (Tier 1).
    Aot,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Interpret => "interpret",
            Tier::Aot => "aot",
        }
    }
}

/// Context controlling where the tier came from — useful for commands
/// (like `bench`) that treat interpreter and AOT quite differently and
/// want to mention the choice in their output.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedTier {
    pub tier: Tier,
    /// `true` when the user explicitly passed a flag. `false` when we
    /// fell back to the caller-supplied default.
    pub explicit: bool,
}

/// Resolve execution tier from CLI inputs.
///
/// Precedence (highest → lowest):
///   1. `explicit_interp` / `explicit_aot` shortcut flags
///   2. `--tier <NAME>` long form (accepted values: `interpret`,
///      `interpreter`, `aot`)
///   3. `default`
///
/// Returns an error for unknown values or `"check"` (which is a build
/// mode, not an execution tier).
pub fn resolve(
    explicit_interp: bool,
    explicit_aot: bool,
    tier_override: Option<&Text>,
    default: Tier,
) -> Result<ResolvedTier> {
    if explicit_interp {
        return Ok(ResolvedTier { tier: Tier::Interpret, explicit: true });
    }
    if explicit_aot {
        return Ok(ResolvedTier { tier: Tier::Aot, explicit: true });
    }
    if let Some(name) = tier_override {
        let tier = match name.as_str() {
            "interpret" | "interpreter" => Tier::Interpret,
            "aot" => Tier::Aot,
            "check" => {
                return Err(CliError::InvalidArgument(
                    "--tier check is a build mode, not an execution tier".into(),
                ));
            }
            other => {
                return Err(CliError::InvalidArgument(format!(
                    "unknown tier `{}` (expected: interpret | aot)",
                    other
                )));
            }
        };
        return Ok(ResolvedTier { tier, explicit: true });
    }
    Ok(ResolvedTier { tier: default, explicit: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_wins_over_override() {
        let r = resolve(true, false, Some(&"aot".into()), Tier::Aot).unwrap();
        assert_eq!(r.tier, Tier::Interpret);
        assert!(r.explicit);
    }

    #[test]
    fn override_wins_over_default() {
        let r = resolve(false, false, Some(&"interpret".into()), Tier::Aot).unwrap();
        assert_eq!(r.tier, Tier::Interpret);
        assert!(r.explicit);
    }

    #[test]
    fn default_when_nothing_specified() {
        let r = resolve(false, false, None, Tier::Aot).unwrap();
        assert_eq!(r.tier, Tier::Aot);
        assert!(!r.explicit);
    }

    #[test]
    fn check_is_rejected() {
        let err = resolve(false, false, Some(&"check".into()), Tier::Aot);
        assert!(err.is_err());
    }

    #[test]
    fn unknown_tier_is_rejected() {
        let err = resolve(false, false, Some(&"jit".into()), Tier::Aot);
        assert!(err.is_err());
    }
}
