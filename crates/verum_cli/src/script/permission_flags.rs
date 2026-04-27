//! CLI permission flags + frontmatter merger (P3.3).
//!
//! Three permission sources, merged into one [`PermissionSet`] for the
//! script run:
//!
//! 1. **Frontmatter `permissions = [...]`** — what the script *declares*
//!    it needs. Source of truth for the script's intent.
//! 2. **Frontmatter `[run].default-permissions = [...]`** — what the
//!    script's manifest grants when the user invokes it without flags.
//!    Folded in alongside the above (both are explicit author-side
//!    declarations).
//! 3. **CLI `--allow <scope>`** — caller-side grants, additive over
//!    whatever the frontmatter declared. Repeatable.
//!
//! Plus two override switches:
//!
//! - `--allow-all` — bypass; grant blanket access for every kind.
//! - `--deny-all`  — bypass the other direction; empty set regardless
//!   of what the frontmatter or `--allow` flags say. Useful for forcing
//!   an unprivileged test run.
//!
//! `--allow-all` and `--deny-all` are mutually exclusive.
//!
//! # Precedence
//!
//! ```text
//!     deny_all       →  PermissionSet::empty()
//!     allow_all      →  blanket grant of every kind
//!     otherwise      →  union(frontmatter, run.default-permissions, --allow flags)
//! ```
//!
//! Union order is preserved (frontmatter first, then run defaults, then
//! CLI flags). [`PermissionSet::check`] is union-permissive — a request
//! matches if *any* grant authorises it — so order is informational
//! only (used in OutOfScope diagnostics to show authoring sequence).

use crate::script::frontmatter::Frontmatter;
use crate::script::permissions::{
    ParseError, Permission, PermissionKind, PermissionScope, PermissionSet,
};

/// CLI surface — derived directly via clap so subcommands can splat
/// `#[clap(flatten)]` and pick up every flag.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct PermissionFlags {
    /// Additional permission scope to grant the script. Repeat for
    /// multiple. Format: `<kind>[=<targets>]`. See [`crate::script::permissions`]
    /// for the grammar.
    #[clap(long = "allow", value_name = "SCOPE")]
    pub allow: Vec<String>,

    /// Grant every permission kind unconditionally. Equivalent to
    /// `--allow fs:read --allow fs:write --allow net --allow env
    /// --allow run --allow ffi --allow time --allow random`.
    /// Mutually exclusive with `--deny-all`.
    #[clap(long = "allow-all", default_value_t = false)]
    pub allow_all: bool,

    /// Drop every permission, including those declared in the
    /// frontmatter. Useful for sandbox-mode testing. Mutually exclusive
    /// with `--allow-all`.
    #[clap(long = "deny-all", default_value_t = false)]
    pub deny_all: bool,
}

/// Failure mode for [`build_permission_set`]. Wraps the underlying
/// scope-parse error with a "where it came from" note so the user can
/// distinguish a typo in the frontmatter from a typo in `--allow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// Both `--allow-all` and `--deny-all` were passed.
    ConflictingOverrides,
    /// A scope from the frontmatter `permissions = [...]` array failed
    /// to parse. Should be caught by [`crate::script::frontmatter::validate`]
    /// upstream — kept here for defence-in-depth.
    InvalidFrontmatterScope { value: String, source: ParseError },
    /// A scope from the frontmatter `[run].default-permissions` array
    /// failed to parse.
    InvalidFrontmatterRunScope { value: String, source: ParseError },
    /// A scope passed via `--allow` failed to parse.
    InvalidCliScope { value: String, source: ParseError },
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConflictingOverrides => f.write_str(
                "--allow-all and --deny-all are mutually exclusive; pick one",
            ),
            Self::InvalidFrontmatterScope { value, source } => write!(
                f,
                "frontmatter `permissions` entry {value:?}: {source}"
            ),
            Self::InvalidFrontmatterRunScope { value, source } => write!(
                f,
                "frontmatter `[run].default-permissions` entry {value:?}: {source}"
            ),
            Self::InvalidCliScope { value, source } => {
                write!(f, "--allow {value:?}: {source}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// Compose the final [`PermissionSet`] for a script run from the three
/// declared sources. Stops at the first parse error.
///
/// `frontmatter == None` means the script had no inline metadata —
/// equivalent to a frontmatter with empty `permissions` and no `[run]`.
pub fn build_permission_set(
    frontmatter: Option<&Frontmatter>,
    flags: &PermissionFlags,
) -> Result<PermissionSet, BuildError> {
    if flags.allow_all && flags.deny_all {
        return Err(BuildError::ConflictingOverrides);
    }
    if flags.deny_all {
        return Ok(PermissionSet::empty());
    }
    if flags.allow_all {
        return Ok(blanket_set());
    }

    let mut grants: Vec<Permission> = Vec::new();
    if let Some(fm) = frontmatter {
        for raw in &fm.permissions {
            let perm = Permission::parse(raw).map_err(|source| {
                BuildError::InvalidFrontmatterScope {
                    value: raw.clone(),
                    source,
                }
            })?;
            grants.push(perm);
        }
        if let Some(run) = &fm.run {
            for raw in &run.default_permissions {
                let perm = Permission::parse(raw).map_err(|source| {
                    BuildError::InvalidFrontmatterRunScope {
                        value: raw.clone(),
                        source,
                    }
                })?;
                grants.push(perm);
            }
        }
    }
    for raw in &flags.allow {
        let perm = Permission::parse(raw).map_err(|source| BuildError::InvalidCliScope {
            value: raw.clone(),
            source,
        })?;
        grants.push(perm);
    }
    Ok(PermissionSet::from_grants(grants))
}

/// `--allow-all`: a blanket grant for every defined kind.
fn blanket_set() -> PermissionSet {
    let kinds = [
        PermissionKind::FsRead,
        PermissionKind::FsWrite,
        PermissionKind::Net,
        PermissionKind::Env,
        PermissionKind::Run,
        PermissionKind::Ffi,
        PermissionKind::Time,
        PermissionKind::Random,
    ];
    let grants = kinds
        .into_iter()
        .map(|kind| Permission {
            kind,
            scope: PermissionScope::Any,
        })
        .collect();
    PermissionSet::from_grants(grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::frontmatter::{Frontmatter, RunOverrides};
    use crate::script::permissions::PermissionRequest;
    use std::path::Path;

    fn fm_with(permissions: Vec<&str>, run_perms: Vec<&str>) -> Frontmatter {
        Frontmatter {
            raw_toml: String::new(),
            verum: None,
            dependencies: Vec::new(),
            permissions: permissions.into_iter().map(String::from).collect(),
            edition: None,
            profile: None,
            run: if run_perms.is_empty() {
                None
            } else {
                Some(RunOverrides {
                    default_permissions: run_perms.into_iter().map(String::from).collect(),
                })
            },
        }
    }

    #[test]
    fn empty_inputs_produce_empty_set() {
        let set = build_permission_set(None, &PermissionFlags::default()).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn frontmatter_permissions_passed_through() {
        let fm = fm_with(vec!["net=api.example.com:443"], vec![]);
        let set =
            build_permission_set(Some(&fm), &PermissionFlags::default()).unwrap();
        assert_eq!(set.len(), 1);
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
    }

    #[test]
    fn run_default_permissions_folded_in() {
        let fm = fm_with(vec![], vec!["fs:read=./data"]);
        let set =
            build_permission_set(Some(&fm), &PermissionFlags::default()).unwrap();
        assert_eq!(set.len(), 1);
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("./data/file")))
            .is_ok());
    }

    #[test]
    fn cli_flags_extend_frontmatter() {
        let fm = fm_with(vec!["net=api.example.com:443"], vec![]);
        let flags = PermissionFlags {
            allow: vec!["fs:read=./data".to_string()],
            ..Default::default()
        };
        let set = build_permission_set(Some(&fm), &flags).unwrap();
        assert_eq!(set.len(), 2);
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("./data/file")))
            .is_ok());
    }

    #[test]
    fn deny_all_overrides_everything() {
        let fm = fm_with(vec!["net"], vec!["fs:read=./data"]);
        let flags = PermissionFlags {
            allow: vec!["env=PATH".to_string()],
            deny_all: true,
            ..Default::default()
        };
        let set = build_permission_set(Some(&fm), &flags).unwrap();
        assert!(set.is_empty());
        assert!(set.check(&PermissionRequest::Time).is_err());
    }

    #[test]
    fn allow_all_grants_every_kind() {
        let flags = PermissionFlags {
            allow_all: true,
            ..Default::default()
        };
        let set = build_permission_set(None, &flags).unwrap();
        assert_eq!(set.len(), 8);
        for req in [
            PermissionRequest::FsRead(Path::new("/anything")),
            PermissionRequest::FsWrite(Path::new("/anything")),
            PermissionRequest::Net {
                host: "x",
                port: Some(1),
            },
            PermissionRequest::Env("X"),
            PermissionRequest::Run("rm"),
            PermissionRequest::Ffi("libc"),
            PermissionRequest::Time,
            PermissionRequest::Random,
        ] {
            assert!(set.check(&req).is_ok(), "allow-all denied {req:?}");
        }
    }

    #[test]
    fn allow_all_and_deny_all_conflict() {
        let flags = PermissionFlags {
            allow_all: true,
            deny_all: true,
            ..Default::default()
        };
        let err = build_permission_set(None, &flags).unwrap_err();
        assert_eq!(err, BuildError::ConflictingOverrides);
    }

    #[test]
    fn invalid_frontmatter_scope_surfaces_with_value() {
        let fm = fm_with(vec!["kernel=/dev/mem"], vec![]);
        let err = build_permission_set(Some(&fm), &PermissionFlags::default()).unwrap_err();
        match err {
            BuildError::InvalidFrontmatterScope { value, .. } => {
                assert_eq!(value, "kernel=/dev/mem");
            }
            other => panic!("expected InvalidFrontmatterScope, got {other:?}"),
        }
    }

    #[test]
    fn invalid_run_default_scope_distinguished_from_top_level() {
        let fm = fm_with(vec![], vec!["bogus=x"]);
        let err = build_permission_set(Some(&fm), &PermissionFlags::default()).unwrap_err();
        match err {
            BuildError::InvalidFrontmatterRunScope { value, .. } => {
                assert_eq!(value, "bogus=x");
            }
            other => panic!("expected InvalidFrontmatterRunScope, got {other:?}"),
        }
    }

    #[test]
    fn invalid_cli_scope_surfaces_with_value() {
        let flags = PermissionFlags {
            allow: vec!["nope".to_string()],
            ..Default::default()
        };
        let err = build_permission_set(None, &flags).unwrap_err();
        match err {
            BuildError::InvalidCliScope { value, .. } => assert_eq!(value, "nope"),
            other => panic!("expected InvalidCliScope, got {other:?}"),
        }
    }

    #[test]
    fn frontmatter_and_cli_concurrent_grants_unioned() {
        // Two grants of the same kind; check() should hit either via union.
        let fm = fm_with(vec!["net=api.example.com:443"], vec![]);
        let flags = PermissionFlags {
            allow: vec!["net=fallback.example.com".to_string()],
            ..Default::default()
        };
        let set = build_permission_set(Some(&fm), &flags).unwrap();
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "fallback.example.com",
                port: Some(80),
            })
            .is_ok());
        // A request matching neither grant must still fail.
        assert!(set
            .check(&PermissionRequest::Net {
                host: "evil.example.com",
                port: Some(80),
            })
            .is_err());
    }

    #[test]
    fn deny_all_short_circuits_invalid_input() {
        // deny_all wins even if frontmatter would have failed parsing.
        // This makes `--deny-all` a usable escape hatch for a bad script.
        let fm = fm_with(vec!["kernel=/dev/mem"], vec![]);
        let flags = PermissionFlags {
            deny_all: true,
            ..Default::default()
        };
        let set = build_permission_set(Some(&fm), &flags).unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn ordering_is_frontmatter_then_run_then_cli() {
        let fm = fm_with(vec!["env=A"], vec!["env=B"]);
        let flags = PermissionFlags {
            allow: vec!["env=C".to_string()],
            ..Default::default()
        };
        let set = build_permission_set(Some(&fm), &flags).unwrap();
        let env_grants: Vec<_> = set
            .grants_of(PermissionKind::Env)
            .filter_map(|p| match &p.scope {
                PermissionScope::Targets(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            env_grants,
            vec![vec!["A".to_string()], vec!["B".to_string()], vec!["C".to_string()]]
        );
    }
}
