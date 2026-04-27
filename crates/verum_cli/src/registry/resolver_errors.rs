//! Structured resolver-error model (P4.3).
//!
//! The classic shape of a dependency-resolution failure has three
//! distinct cases — version conflict, no matching version, malformed
//! requirement — each with its own diagnostic dimensions:
//!
//!   - **VersionConflict** wants to surface the *requirement chain*:
//!     "package A wants foo ^1; package B wants foo ^2; the resolver
//!     picked A's ^1 first, then B forced ^2 in." Without the chain,
//!     the user sees "conflict on foo" and has to git-grep manually.
//!
//!   - **NoMatchingVersion** wants to surface what's actually
//!     *available*: a typo'd `^99` against a registry that goes up to
//!     `^2` is a fix-the-typo problem, not a missing-package problem.
//!     Showing the version list lets the user spot the mistake at a
//!     glance.
//!
//!   - **InvalidRequirement** wants to surface the *position* of the
//!     parse error, not just "invalid version requirement".
//!
//! This module collects all four into a single [`ResolverError`] enum
//! whose [`std::fmt::Display`] renders multi-line, indented diagnostic
//! output suitable for terminal consumption. A `From<ResolverError>`
//! into [`crate::error::CliError`] keeps the existing CLI-error path
//! working.

use crate::error::CliError;
use std::fmt;

/// Where a version requirement on `package` came from. Carried inside
/// [`ResolverError::VersionConflict`] so the user can see the full
/// chain that produced the disagreement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementOrigin {
    /// Cog that imposed this requirement (or `None` for the workspace
    /// root manifest, which is the user's project).
    pub requirer: Option<RequirerSpec>,
    /// The literal version-requirement string (e.g. `"^1.0"`,
    /// `">=2, <3"`).
    pub requirement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirerSpec {
    pub name: String,
    pub version: String,
}

/// Resolver-specific failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolverError {
    /// Two or more requirements on the same package can't all be
    /// satisfied — they pin distinct, mutually-exclusive versions.
    VersionConflict {
        /// The disputed package.
        package: String,
        /// Origin chain — every (requirer, requirement) pair that
        /// landed on `package` during resolution.
        requirements: Vec<RequirementOrigin>,
        /// Distinct versions the resolver already placed on `package`
        /// before it noticed the conflict.
        present_versions: Vec<String>,
    },
    /// `package` was constrained by `requirement` but no version on
    /// disk / in the registry matches.
    NoMatchingVersion {
        package: String,
        requirement: String,
        /// All versions currently known for `package`. Truncated to
        /// the most-recent `max_shown` when rendered.
        available: Vec<String>,
    },
    /// Version-requirement string didn't parse as semver.
    InvalidRequirement {
        input: String,
        reason: String,
        /// 0-based byte offset into `input` where the parse failed,
        /// when the underlying parser provided one. semver 1.x does
        /// not expose this, so it's typically `None`; the field is
        /// kept for forward compatibility with a future parser that
        /// does carry positional info.
        position: Option<usize>,
    },
    /// Dependency graph contains a cycle. The path is rendered in
    /// declaration order; a final `→ <first>` is appended at display
    /// time to make the cycle visually obvious.
    Cycle { path: Vec<String> },
}

impl ResolverError {
    /// Convenience: infer `present_versions` from the requirements'
    /// origin chain if the caller hasn't computed it separately.
    pub fn version_conflict(
        package: impl Into<String>,
        requirements: Vec<RequirementOrigin>,
        present_versions: Vec<String>,
    ) -> Self {
        Self::VersionConflict {
            package: package.into(),
            requirements,
            present_versions,
        }
    }

    pub fn no_matching_version(
        package: impl Into<String>,
        requirement: impl Into<String>,
        available: Vec<String>,
    ) -> Self {
        Self::NoMatchingVersion {
            package: package.into(),
            requirement: requirement.into(),
            available,
        }
    }

    pub fn invalid_requirement(input: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidRequirement {
            input: input.into(),
            reason: reason.into(),
            position: None,
        }
    }

    pub fn cycle(path: Vec<String>) -> Self {
        Self::Cycle { path }
    }
}

const AVAILABLE_TRUNC: usize = 8;
const REQUIREMENTS_TRUNC: usize = 12;

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionConflict {
                package,
                requirements,
                present_versions,
            } => {
                writeln!(f, "version conflict on `{package}`")?;
                if !present_versions.is_empty() {
                    writeln!(f, "  resolver placed: {}", present_versions.join(", "))?;
                }
                if requirements.is_empty() {
                    writeln!(
                        f,
                        "  no requirement origins recorded — resolver may have detected the conflict before tracing edges"
                    )?;
                } else {
                    writeln!(f, "  required by:")?;
                    let shown = requirements.len().min(REQUIREMENTS_TRUNC);
                    for origin in &requirements[..shown] {
                        match &origin.requirer {
                            Some(r) => writeln!(
                                f,
                                "    - {}@{} requires `{}`",
                                r.name, r.version, origin.requirement
                            )?,
                            None => writeln!(
                                f,
                                "    - <project root> requires `{}`",
                                origin.requirement
                            )?,
                        }
                    }
                    if requirements.len() > shown {
                        writeln!(
                            f,
                            "    ... ({} more not shown)",
                            requirements.len() - shown
                        )?;
                    }
                }
                write!(
                    f,
                    "  hint: relax one requirement, or pin a single version in `[dependencies]`"
                )
            }
            Self::NoMatchingVersion {
                package,
                requirement,
                available,
            } => {
                writeln!(
                    f,
                    "no version of `{package}` matches requirement `{requirement}`"
                )?;
                if available.is_empty() {
                    write!(f, "  no versions of `{package}` are known to the resolver")
                } else {
                    let shown = available.len().min(AVAILABLE_TRUNC);
                    write!(f, "  available: {}", available[..shown].join(", "))?;
                    if available.len() > shown {
                        write!(f, " ... ({} more)", available.len() - shown)?;
                    }
                    Ok(())
                }
            }
            Self::InvalidRequirement {
                input,
                reason,
                position,
            } => {
                write!(f, "invalid version requirement `{input}`: {reason}")?;
                if let Some(pos) = position {
                    write!(f, " (at byte offset {pos})")?;
                }
                Ok(())
            }
            Self::Cycle { path } => {
                if path.is_empty() {
                    return write!(f, "dependency cycle (no nodes recorded)");
                }
                write!(f, "dependency cycle: ")?;
                for name in path {
                    write!(f, "{name} → ")?;
                }
                // Close the loop visually.
                write!(f, "{}", path[0])
            }
        }
    }
}

impl std::error::Error for ResolverError {}

impl From<ResolverError> for CliError {
    fn from(e: ResolverError) -> Self {
        match e {
            ResolverError::VersionConflict {
                package,
                requirements,
                present_versions,
            } => {
                // `CliError::VersionConflict` is shape-pinned (package
                // + required + found) for legacy compatibility; we lossy-
                // map onto it. Callers that want the full chain should
                // print the `ResolverError` directly via `Display`
                // before converting.
                let required = requirements
                    .first()
                    .map(|r| r.requirement.clone())
                    .unwrap_or_default();
                let found = present_versions.first().cloned().unwrap_or_default();
                CliError::VersionConflict {
                    package,
                    required,
                    found,
                }
            }
            ResolverError::NoMatchingVersion { package, requirement, available } => {
                let suffix = if available.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; available: {}",
                        available
                            .iter()
                            .take(AVAILABLE_TRUNC)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                CliError::DependencyNotFound(format!(
                    "no version of `{package}` matches `{requirement}`{suffix}"
                ))
            }
            ResolverError::InvalidRequirement { input, reason, .. } => {
                CliError::InvalidArgument(format!("invalid version requirement `{input}`: {reason}"))
            }
            ResolverError::Cycle { path } => CliError::Custom(format!(
                "dependency cycle: {}",
                if path.is_empty() {
                    String::from("(empty)")
                } else {
                    let mut s = path.join(" → ");
                    s.push_str(" → ");
                    s.push_str(&path[0]);
                    s
                }
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(name: &str, version: &str, req: &str) -> RequirementOrigin {
        RequirementOrigin {
            requirer: Some(RequirerSpec {
                name: name.to_string(),
                version: version.to_string(),
            }),
            requirement: req.to_string(),
        }
    }

    fn root_origin(req: &str) -> RequirementOrigin {
        RequirementOrigin {
            requirer: None,
            requirement: req.to_string(),
        }
    }

    // ── Display: VersionConflict ─────────────────────────────────────

    #[test]
    fn version_conflict_lists_full_chain() {
        let err = ResolverError::version_conflict(
            "foo",
            vec![
                origin("a", "1.0.0", "^1"),
                origin("b", "2.0.0", "^2"),
            ],
            vec!["1.0.0".into(), "2.0.0".into()],
        );
        let s = err.to_string();
        assert!(s.contains("version conflict on `foo`"));
        assert!(s.contains("placed: 1.0.0, 2.0.0"));
        assert!(s.contains("a@1.0.0 requires `^1`"));
        assert!(s.contains("b@2.0.0 requires `^2`"));
        assert!(s.contains("hint:"));
    }

    #[test]
    fn version_conflict_truncates_long_chain() {
        let mut chain = Vec::new();
        for i in 0..20 {
            chain.push(origin(&format!("pkg{i}"), "1.0.0", "^1"));
        }
        let err = ResolverError::version_conflict("foo", chain, vec!["1.0.0".into()]);
        let s = err.to_string();
        assert!(s.contains("... (8 more not shown)"), "got {s}");
    }

    #[test]
    fn version_conflict_root_origin_renders_specially() {
        let err = ResolverError::version_conflict(
            "foo",
            vec![root_origin("^1.0")],
            vec!["1.0.0".into()],
        );
        let s = err.to_string();
        assert!(s.contains("<project root> requires `^1.0`"));
    }

    #[test]
    fn version_conflict_with_empty_origins_says_so() {
        let err = ResolverError::VersionConflict {
            package: "foo".into(),
            requirements: vec![],
            present_versions: vec!["1.0.0".into()],
        };
        let s = err.to_string();
        assert!(s.contains("no requirement origins recorded"));
    }

    // ── Display: NoMatchingVersion ───────────────────────────────────

    #[test]
    fn no_matching_version_lists_available() {
        let err = ResolverError::no_matching_version(
            "foo",
            "^99",
            vec!["1.0.0".into(), "1.1.0".into(), "2.0.0".into()],
        );
        let s = err.to_string();
        assert!(s.contains("no version of `foo` matches requirement `^99`"));
        assert!(s.contains("available: 1.0.0, 1.1.0, 2.0.0"));
    }

    #[test]
    fn no_matching_version_truncates_long_available_list() {
        let avail: Vec<String> = (0..20).map(|i| format!("0.{i}.0")).collect();
        let err = ResolverError::no_matching_version("foo", "^99", avail);
        let s = err.to_string();
        assert!(s.contains("(12 more)"), "got {s}");
    }

    #[test]
    fn no_matching_version_handles_empty_available() {
        let err = ResolverError::no_matching_version("foo", "^1", vec![]);
        let s = err.to_string();
        assert!(s.contains("no versions of `foo` are known"));
    }

    // ── Display: InvalidRequirement ──────────────────────────────────

    #[test]
    fn invalid_requirement_renders_input_and_reason() {
        let err = ResolverError::invalid_requirement("not-a-semver", "expected version");
        let s = err.to_string();
        assert!(s.contains("invalid version requirement `not-a-semver`"));
        assert!(s.contains("expected version"));
    }

    #[test]
    fn invalid_requirement_with_position_renders_offset() {
        let err = ResolverError::InvalidRequirement {
            input: "1.0.x".into(),
            reason: "unexpected character".into(),
            position: Some(4),
        };
        let s = err.to_string();
        assert!(s.contains("byte offset 4"));
    }

    // ── Display: Cycle ───────────────────────────────────────────────

    #[test]
    fn cycle_closes_the_loop_visually() {
        let err = ResolverError::cycle(vec!["a".into(), "b".into(), "c".into()]);
        let s = err.to_string();
        assert!(s.starts_with("dependency cycle: a → b → c → a"));
    }

    #[test]
    fn cycle_handles_empty_path() {
        let err = ResolverError::cycle(vec![]);
        let s = err.to_string();
        assert!(s.contains("no nodes recorded"));
    }

    // ── CliError conversion ──────────────────────────────────────────

    #[test]
    fn into_cli_error_preserves_legacy_shape() {
        let err = ResolverError::version_conflict(
            "foo",
            vec![origin("a", "1.0.0", "^1"), origin("b", "2.0.0", "^2")],
            vec!["1.0.0".into(), "2.0.0".into()],
        );
        let cli: CliError = err.into();
        match cli {
            CliError::VersionConflict { package, required, found } => {
                assert_eq!(package, "foo");
                assert_eq!(required, "^1");
                assert_eq!(found, "1.0.0");
            }
            other => panic!("expected VersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn into_cli_error_no_match_uses_dependency_not_found() {
        let err = ResolverError::no_matching_version(
            "foo",
            "^99",
            vec!["1.0.0".into(), "2.0.0".into()],
        );
        let cli: CliError = err.into();
        match cli {
            CliError::DependencyNotFound(s) => {
                assert!(s.contains("foo"));
                assert!(s.contains("^99"));
                assert!(s.contains("available"));
            }
            other => panic!("expected DependencyNotFound, got {other:?}"),
        }
    }

    #[test]
    fn into_cli_error_invalid_requirement_uses_invalid_argument() {
        let err = ResolverError::invalid_requirement("xx", "bad");
        let cli: CliError = err.into();
        assert!(matches!(cli, CliError::InvalidArgument(_)));
    }

    #[test]
    fn into_cli_error_cycle_uses_custom() {
        let err = ResolverError::cycle(vec!["a".into(), "b".into()]);
        let cli: CliError = err.into();
        match cli {
            CliError::Custom(s) => {
                assert!(s.contains("a → b → a"), "got {s}");
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }
}
