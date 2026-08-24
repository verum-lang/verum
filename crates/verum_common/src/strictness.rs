//! Strictness policy — ONE authority for "is silent degradation
//! allowed here?" (T0693).
//!
//! # Why this exists
//!
//! Nine independent `VERUM_STRICT_*` opt-in flags accumulated across
//! the compiler, each guarding a different silent-degradation site:
//! monomorphisation falling back to the unspecialised module,
//! signature drift shipping arity-collided forward declarations,
//! `[lenient] SKIP` swallowing a body that fails to compile,
//! const-zero call stubs standing in for an unresolved callee. Every
//! one of them defaulted to OFF, so the shipped compiler's default
//! behaviour was: degrade quietly and keep going.
//!
//! That default is what turns a language defect into a debugging
//! expedition — the program does not fail, it produces a plausible
//! wrong answer (an empty list, a zero, a `Unit`). The directive this
//! module implements inverts it: **strict is the default, lenient is
//! the explicit escape hatch**.
//!
//! # The policy
//!
//! One process-wide answer, resolved ONCE (first read wins, cached in
//! a `OnceLock` — the per-instruction `std::env::var` cost is what the
//! interpreter's env-flag cache exists to avoid), from this order:
//!
//!  1. `VERUM_LENIENT=1` — process-wide escape hatch (CLI `--lenient`
//!     sets it, so both channels agree by construction).
//!  2. `VERUM_STRICT=0` — the same escape spelled as a negation, for
//!     callers who already carry a strictness variable.
//!  3. Default: **strict**.
//!
//! Per-site `VERUM_STRICT_<SITE>=1` variables keep working as
//! *forcing* overrides: they turn one site strict even under a
//! process-wide lenient. This preserves every existing A/B recipe —
//! a bisect that sets `VERUM_STRICT_MONO=1` still measures exactly
//! that site — while removing the need to set them at all in normal
//! runs.
//!
//! # What a strict site must do
//!
//! Fail loudly, naming the site and what it refused to fabricate.
//! A strict site that silently succeeds is worse than a lenient one,
//! because it also removes the warning that used to be printed.

use std::sync::OnceLock;

/// A degradation site — one variant per class of silent fallback the
/// compiler is capable of. Adding a variant is how a newly-discovered
/// degradation joins the policy; the `env_suffix` is its historical
/// `VERUM_STRICT_<suffix>` spelling, kept so existing recipes and CI
/// jobs keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Site {
    /// Monomorphisation failed; the unspecialised module would ship.
    Mono,
    /// Arity-collided functions are forward-declared without bodies.
    Signatures,
    /// A codegen construct has no lowering and would emit a stub.
    Codegen,
    /// A function body failed to compile and would be skipped.
    LenientSkip,
    /// A callee did not resolve and a const-zero stub would stand in.
    UnresolvedCall,
    /// A definer (impl / method owner) could not be determined.
    Definers,
    /// A value could not be classified and a default would be used.
    Values,
    /// A function id could not be resolved to a descriptor.
    FnId,
    /// A record field could not be resolved.
    Fields,
    /// A mount target could not be verified.
    Mounts,
    /// A visibility rule could not be checked.
    Visibility,
}

impl Site {
    /// The `VERUM_STRICT_<suffix>` spelling this site has historically
    /// answered to.
    pub const fn env_suffix(self) -> &'static str {
        match self {
            Site::Mono => "MONO",
            Site::Signatures => "SIGNATURES",
            Site::Codegen => "CODEGEN",
            Site::LenientSkip => "LENIENT_SKIP",
            Site::UnresolvedCall => "UNRESOLVED_CALL",
            Site::Definers => "DEFINERS",
            Site::Values => "VALUES",
            Site::FnId => "FN_ID",
            Site::Fields => "FIELDS",
            Site::Mounts => "MOUNTS",
            Site::Visibility => "VISIBILITY",
        }
    }
}

/// Process-wide lenient escape, resolved once.
fn process_lenient() -> bool {
    static LENIENT: OnceLock<bool> = OnceLock::new();
    *LENIENT.get_or_init(|| {
        if std::env::var_os("VERUM_LENIENT").is_some_and(|v| v != "0") {
            return true;
        }
        matches!(std::env::var("VERUM_STRICT").as_deref(), Ok("0"))
    })
}

/// Is `site` strict for this process?
///
/// Strict by default; `VERUM_LENIENT=1` (or `VERUM_STRICT=0`) relaxes
/// every site; `VERUM_STRICT_<SITE>=1` forces one site strict again.
pub fn is_strict(site: Site) -> bool {
    if std::env::var_os(format!("VERUM_STRICT_{}", site.env_suffix())).is_some() {
        return true;
    }
    !process_lenient()
}

/// The escape-hatch sentence to append to a strict failure, so the
/// message that stops a build also says how to proceed deliberately.
pub const ESCAPE_HINT: &str =
    "re-run with --lenient (or VERUM_LENIENT=1) to allow the degraded \
     build, or fix the reported site";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_site_has_a_distinct_env_suffix() {
        let sites = [
            Site::Mono,
            Site::Signatures,
            Site::Codegen,
            Site::LenientSkip,
            Site::UnresolvedCall,
            Site::Definers,
            Site::Values,
            Site::FnId,
            Site::Fields,
            Site::Mounts,
            Site::Visibility,
        ];
        let mut seen: Vec<&str> = sites.iter().map(|s| s.env_suffix()).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "env suffixes must be distinct");
    }

    #[test]
    fn suffixes_are_upper_snake() {
        for s in [Site::Mono, Site::LenientSkip, Site::UnresolvedCall] {
            let sfx = s.env_suffix();
            assert!(
                sfx.chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_'),
                "{sfx} must be UPPER_SNAKE"
            );
        }
    }
}
