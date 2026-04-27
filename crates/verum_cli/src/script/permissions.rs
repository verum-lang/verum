//! Permission-scope parser and runtime guard (P3.1).
//!
//! Verum scripts declare what they're allowed to do via Deno-style
//! permission scopes:
//!
//! ```text
//! permissions = ["net=api.example.com:443", "fs:read=./data", "time"]
//! ```
//!
//! This module turns those strings into a typed [`Permission`] and a
//! [`PermissionSet`] that can be queried at runtime by intrinsic
//! handlers ("am I allowed to open this socket?"). Parse failures
//! surface as [`ParseError`]; runtime denials surface as
//! [`DeniedReason`] with the granted-set context attached so the user
//! can see *why* the check failed.
//!
//! # Grammar
//!
//! ```text
//! scope         = scope_kind , [ "=" , scope_targets ]
//! scope_kind    = "fs:read" | "fs:write" | "net" | "env" | "run" | "ffi"
//!               | "time"    | "random"
//! scope_targets = target , { "," , target }     (* non-empty, no whitespace *)
//! target        = any non-comma, non-whitespace UTF-8 sequence
//! ```
//!
//! `time` and `random` accept blanket form only.
//!
//! # Matching
//!
//! Per kind:
//!
//! | kind        | target form                    | match rule                                      |
//! |-------------|--------------------------------|-------------------------------------------------|
//! | `fs:read`   | path prefix                    | request path starts with target path            |
//! | `fs:write`  | path prefix                    | request path starts with target path            |
//! | `net`       | `host`, `host:port`, `:port`   | host equality; port equality if grant has one   |
//! | `env`       | env-var name                   | literal equality                                |
//! | `run`       | program name                   | literal equality                                |
//! | `ffi`       | library name                   | literal equality                                |
//! | `time`      | (no targets)                   | always granted if scope present                 |
//! | `random`    | (no targets)                   | always granted if scope present                 |
//!
//! Multiple grants of the same kind are unioned: a request matches if
//! *any* grant for that kind allows it.

use std::fmt;
use std::path::{Path, PathBuf};

/// Eight permission kinds, mirroring the documented frontmatter grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    FsRead,
    FsWrite,
    Net,
    Env,
    Run,
    Ffi,
    Time,
    Random,
}

impl PermissionKind {
    /// Stringify back to the canonical wire format. Round-trip:
    /// `parse_kind(kind.as_str()) == Some(kind)`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FsRead => "fs:read",
            Self::FsWrite => "fs:write",
            Self::Net => "net",
            Self::Env => "env",
            Self::Run => "run",
            Self::Ffi => "ffi",
            Self::Time => "time",
            Self::Random => "random",
        }
    }

    /// `true` for kinds that have no `=value` form (blanket-only).
    fn is_blanket_only(&self) -> bool {
        matches!(self, Self::Time | Self::Random)
    }
}

impl fmt::Display for PermissionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn parse_kind(s: &str) -> Option<PermissionKind> {
    Some(match s {
        "fs:read" => PermissionKind::FsRead,
        "fs:write" => PermissionKind::FsWrite,
        "net" => PermissionKind::Net,
        "env" => PermissionKind::Env,
        "run" => PermissionKind::Run,
        "ffi" => PermissionKind::Ffi,
        "time" => PermissionKind::Time,
        "random" => PermissionKind::Random,
        _ => return None,
    })
}

/// Either blanket access or a non-empty list of targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionScope {
    /// Bare scope (no `=`) — grants blanket access for the kind.
    Any,
    /// Comma-separated targets. Always non-empty by construction.
    Targets(Vec<String>),
}

/// One parsed permission grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission {
    pub kind: PermissionKind,
    pub scope: PermissionScope,
}

/// A bundle of permission grants. Cheap to clone — internally a `Vec`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    grants: Vec<Permission>,
}

/// Failure mode for [`Permission::parse`] / [`PermissionSet::from_strings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Scope string was empty or whitespace-only.
    Empty,
    /// Kind prefix wasn't recognised.
    UnknownKind(String),
    /// Kind requires blanket form but got `=value`.
    BlanketOnlyAcceptsNoValue(PermissionKind),
    /// `=` present but target list empty (`net=`).
    EmptyTargetList(PermissionKind),
    /// Comma-separator produced an empty piece (`net=a,,b`).
    EmptyTarget(PermissionKind),
    /// Target contains whitespace.
    WhitespaceInTarget(PermissionKind, String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("permission scope is empty"),
            Self::UnknownKind(k) => write!(f, "unknown permission kind {k:?}"),
            Self::BlanketOnlyAcceptsNoValue(k) => {
                write!(f, "permission {k} grants blanket access only — drop the `=value`")
            }
            Self::EmptyTargetList(k) => write!(f, "permission {k}= requires at least one target"),
            Self::EmptyTarget(k) => write!(f, "permission {k} has an empty target between commas"),
            Self::WhitespaceInTarget(k, t) => {
                write!(f, "permission {k} target {t:?} contains whitespace; use commas to separate")
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl Permission {
    /// Parse a single scope string into a typed permission. Whitespace
    /// surrounding the kind is trimmed; whitespace inside targets is
    /// rejected (the grammar uses `,` as the only separator).
    pub fn parse(scope_str: &str) -> Result<Self, ParseError> {
        let s = scope_str.trim();
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let (kind_str, targets_str) = match s.split_once('=') {
            Some((k, t)) => (k.trim(), Some(t)),
            None => (s, None),
        };
        let kind =
            parse_kind(kind_str).ok_or_else(|| ParseError::UnknownKind(kind_str.to_string()))?;

        match (kind.is_blanket_only(), targets_str) {
            (true, Some(_)) => Err(ParseError::BlanketOnlyAcceptsNoValue(kind)),
            (_, None) => Ok(Self {
                kind,
                scope: PermissionScope::Any,
            }),
            (false, Some(t)) => {
                if t.is_empty() {
                    return Err(ParseError::EmptyTargetList(kind));
                }
                let mut targets = Vec::with_capacity(2);
                for piece in t.split(',') {
                    if piece.is_empty() {
                        return Err(ParseError::EmptyTarget(kind));
                    }
                    if piece.chars().any(|c| c.is_whitespace()) {
                        return Err(ParseError::WhitespaceInTarget(kind, piece.to_string()));
                    }
                    targets.push(piece.to_string());
                }
                Ok(Self {
                    kind,
                    scope: PermissionScope::Targets(targets),
                })
            }
        }
    }

    /// Match a runtime request against this single grant. Returns `true`
    /// only if the kind matches AND the scope authorises the request.
    pub fn matches(&self, req: &PermissionRequest<'_>) -> bool {
        if self.kind != req.kind() {
            return false;
        }
        match &self.scope {
            PermissionScope::Any => true,
            PermissionScope::Targets(targets) => {
                targets.iter().any(|t| match (self.kind, req) {
                    (PermissionKind::FsRead, PermissionRequest::FsRead(p))
                    | (PermissionKind::FsWrite, PermissionRequest::FsWrite(p)) => {
                        path_prefix_matches(p, t)
                    }
                    (PermissionKind::Net, PermissionRequest::Net { host, port }) => {
                        net_target_matches(t, host, *port)
                    }
                    (PermissionKind::Env, PermissionRequest::Env(name)) => t == name,
                    (PermissionKind::Run, PermissionRequest::Run(name)) => t == name,
                    (PermissionKind::Ffi, PermissionRequest::Ffi(name)) => t == name,
                    // Time / Random have no targets — handled above by Any.
                    _ => false,
                })
            }
        }
    }
}

/// Runtime request asked of a [`PermissionSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRequest<'a> {
    FsRead(&'a Path),
    FsWrite(&'a Path),
    Net { host: &'a str, port: Option<u16> },
    Env(&'a str),
    Run(&'a str),
    Ffi(&'a str),
    Time,
    Random,
}

impl PermissionRequest<'_> {
    /// Kind discriminant for matching against grants.
    pub fn kind(&self) -> PermissionKind {
        match self {
            Self::FsRead(_) => PermissionKind::FsRead,
            Self::FsWrite(_) => PermissionKind::FsWrite,
            Self::Net { .. } => PermissionKind::Net,
            Self::Env(_) => PermissionKind::Env,
            Self::Run(_) => PermissionKind::Run,
            Self::Ffi(_) => PermissionKind::Ffi,
            Self::Time => PermissionKind::Time,
            Self::Random => PermissionKind::Random,
        }
    }

    /// Render the request as a human-readable string for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Self::FsRead(p) => format!("fs:read {}", p.display()),
            Self::FsWrite(p) => format!("fs:write {}", p.display()),
            Self::Net { host, port: Some(p) } => format!("net {host}:{p}"),
            Self::Net { host, port: None } => format!("net {host}"),
            Self::Env(n) => format!("env {n}"),
            Self::Run(n) => format!("run {n}"),
            Self::Ffi(n) => format!("ffi {n}"),
            Self::Time => "time".to_string(),
            Self::Random => "random".to_string(),
        }
    }
}

/// Reason a [`PermissionSet::check`] returned an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeniedReason {
    /// No grant of the requested kind was present in the set.
    NotGranted { kind: PermissionKind },
    /// At least one grant of the kind exists but none authorise the
    /// specific request. The granted-target snapshot helps the user
    /// see what the script actually has.
    OutOfScope {
        kind: PermissionKind,
        requested: String,
        granted: Vec<String>,
    },
}

impl fmt::Display for DeniedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGranted { kind } => write!(
                f,
                "permission {kind} not granted (script frontmatter must declare it)"
            ),
            Self::OutOfScope {
                kind,
                requested,
                granted,
            } => write!(
                f,
                "permission {kind} granted but does not cover {requested:?}; granted: [{}]",
                granted.join(", ")
            ),
        }
    }
}

impl std::error::Error for DeniedReason {}

impl PermissionSet {
    /// An empty permission set — every check fails with [`DeniedReason::NotGranted`].
    pub fn empty() -> Self {
        Self { grants: Vec::new() }
    }

    /// Build a set from raw scope strings. Stops at the first parse error.
    pub fn from_strings<I, S>(scopes: I) -> Result<Self, ParseError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut grants = Vec::new();
        for s in scopes {
            grants.push(Permission::parse(s.as_ref())?);
        }
        Ok(Self { grants })
    }

    /// Construct directly from already-parsed grants. Used by callers
    /// that want to merge multiple sources (frontmatter + CLI flags).
    pub fn from_grants(grants: Vec<Permission>) -> Self {
        Self { grants }
    }

    /// Number of grants in the set.
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    /// `true` iff there are no grants.
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// All grants of the given kind.
    pub fn grants_of(&self, kind: PermissionKind) -> impl Iterator<Item = &Permission> {
        self.grants.iter().filter(move |g| g.kind == kind)
    }

    /// Authorise a request against this set. Returns `Ok(())` if any
    /// grant matches; otherwise the most-specific `DeniedReason`.
    pub fn check(&self, req: &PermissionRequest<'_>) -> Result<(), DeniedReason> {
        let kind = req.kind();
        let mut saw_kind = false;
        let mut granted_targets = Vec::new();
        for grant in &self.grants {
            if grant.kind != kind {
                continue;
            }
            saw_kind = true;
            if grant.matches(req) {
                return Ok(());
            }
            if let PermissionScope::Targets(t) = &grant.scope {
                granted_targets.extend_from_slice(t);
            }
        }
        if !saw_kind {
            Err(DeniedReason::NotGranted { kind })
        } else {
            Err(DeniedReason::OutOfScope {
                kind,
                requested: req.describe(),
                granted: granted_targets,
            })
        }
    }

    /// Merge another set into this one. Order is preserved (`other` last).
    pub fn extend(&mut self, other: PermissionSet) {
        self.grants.extend(other.grants);
    }
}

/// Path-prefix match: `path` is authorised by `target` if, after
/// best-effort normalisation, `path` starts at or below `target`.
///
/// "Best effort" because we operate on lexical paths — we don't
/// canonicalise via the filesystem (which would refuse symlinks and
/// non-existent paths). For the purposes of permission checks, lexical
/// matching is sufficient: an attacker who controls the path argument
/// can already pick whatever lexical prefix they like.
fn path_prefix_matches(path: &Path, target: &str) -> bool {
    let t = PathBuf::from(target);
    let mut p_iter = path.components();
    for tc in t.components() {
        match p_iter.next() {
            Some(pc) if pc == tc => continue,
            _ => return false,
        }
    }
    true
}

/// Net target match: a grant `target` matches a request `(host, port)`.
///
/// Target forms:
///   - `"host"`            — any port on this host
///   - `"host:port"`       — exact host + port
///   - `":port"`           — any host on this port
fn net_target_matches(target: &str, host: &str, port: Option<u16>) -> bool {
    if let Some(suffix) = target.strip_prefix(':') {
        // Bare port form.
        return suffix.parse::<u16>().ok() == port;
    }
    match target.rsplit_once(':') {
        // host:port form. `rsplit_once` so `[::1]:443` style addresses
        // (which we do not support yet) don't trip on internal `:`.
        Some((thost, tport)) if tport.parse::<u16>().is_ok() => {
            thost == host && tport.parse::<u16>().ok() == port
        }
        // Bare host form — port is a wildcard.
        _ => target == host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Permission {
        Permission::parse(s).expect("parse")
    }

    // ── parse ────────────────────────────────────────────────────────

    #[test]
    fn parse_blanket_kinds() {
        for kind in [
            "fs:read", "fs:write", "net", "env", "run", "ffi", "time", "random",
        ] {
            let perm = p(kind);
            assert!(matches!(perm.scope, PermissionScope::Any));
            assert_eq!(perm.kind.as_str(), kind);
        }
    }

    #[test]
    fn parse_with_targets() {
        let perm = p("net=api.example.com:443,fallback.example.com");
        assert_eq!(perm.kind, PermissionKind::Net);
        assert_eq!(
            perm.scope,
            PermissionScope::Targets(vec![
                "api.example.com:443".into(),
                "fallback.example.com".into(),
            ])
        );
    }

    #[test]
    fn parse_rejects_empty() {
        assert_eq!(Permission::parse(""), Err(ParseError::Empty));
        assert_eq!(Permission::parse("   "), Err(ParseError::Empty));
    }

    #[test]
    fn parse_rejects_unknown_kind() {
        assert!(matches!(
            Permission::parse("kernel=/dev/mem"),
            Err(ParseError::UnknownKind(k)) if k == "kernel"
        ));
    }

    #[test]
    fn parse_rejects_blanket_with_value() {
        assert_eq!(
            Permission::parse("time=1h"),
            Err(ParseError::BlanketOnlyAcceptsNoValue(PermissionKind::Time))
        );
        assert_eq!(
            Permission::parse("random=/dev/urandom"),
            Err(ParseError::BlanketOnlyAcceptsNoValue(PermissionKind::Random))
        );
    }

    #[test]
    fn parse_rejects_empty_target_list() {
        assert_eq!(
            Permission::parse("net="),
            Err(ParseError::EmptyTargetList(PermissionKind::Net))
        );
    }

    #[test]
    fn parse_rejects_empty_inner_target() {
        assert_eq!(
            Permission::parse("net=a,,b"),
            Err(ParseError::EmptyTarget(PermissionKind::Net))
        );
    }

    #[test]
    fn parse_rejects_whitespace_in_target() {
        assert!(matches!(
            Permission::parse("net=foo bar"),
            Err(ParseError::WhitespaceInTarget(PermissionKind::Net, _))
        ));
    }

    // ── matches: fs ──────────────────────────────────────────────────

    #[test]
    fn fs_read_blanket_matches_any_path() {
        let set = PermissionSet::from_strings(["fs:read"]).unwrap();
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("/etc/passwd")))
            .is_ok());
    }

    #[test]
    fn fs_read_target_prefix_match() {
        let set = PermissionSet::from_strings(["fs:read=./data"]).unwrap();
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("./data/file.txt")))
            .is_ok());
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("./data")))
            .is_ok());
        assert!(set
            .check(&PermissionRequest::FsRead(Path::new("./other/file")))
            .is_err());
    }

    #[test]
    fn fs_read_does_not_authorise_fs_write() {
        let set = PermissionSet::from_strings(["fs:read"]).unwrap();
        let denied = set
            .check(&PermissionRequest::FsWrite(Path::new("/x")))
            .unwrap_err();
        assert!(matches!(denied, DeniedReason::NotGranted { kind } if kind == PermissionKind::FsWrite));
    }

    // ── matches: net ─────────────────────────────────────────────────

    #[test]
    fn net_blanket_matches_any_endpoint() {
        let set = PermissionSet::from_strings(["net"]).unwrap();
        assert!(set
            .check(&PermissionRequest::Net {
                host: "example.com",
                port: Some(443),
            })
            .is_ok());
    }

    #[test]
    fn net_host_only_matches_any_port() {
        let set = PermissionSet::from_strings(["net=api.example.com"]).unwrap();
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: None,
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "other.example.com",
                port: Some(443),
            })
            .is_err());
    }

    #[test]
    fn net_host_port_matches_only_exact() {
        let set = PermissionSet::from_strings(["net=api.example.com:443"]).unwrap();
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(80),
            })
            .is_err());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: None,
            })
            .is_err());
    }

    #[test]
    fn net_bare_port_matches_any_host() {
        let set = PermissionSet::from_strings(["net=:443"]).unwrap();
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "any-host.x",
                port: Some(443),
            })
            .is_ok());
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(80),
            })
            .is_err());
    }

    // ── matches: env / run / ffi / time / random ──────────────────────

    #[test]
    fn env_run_ffi_target_literal_match() {
        let set = PermissionSet::from_strings(["env=PATH", "run=git", "ffi=libc"]).unwrap();
        assert!(set.check(&PermissionRequest::Env("PATH")).is_ok());
        assert!(set.check(&PermissionRequest::Env("HOME")).is_err());
        assert!(set.check(&PermissionRequest::Run("git")).is_ok());
        assert!(set.check(&PermissionRequest::Run("rm")).is_err());
        assert!(set.check(&PermissionRequest::Ffi("libc")).is_ok());
        assert!(set.check(&PermissionRequest::Ffi("openssl")).is_err());
    }

    #[test]
    fn time_random_blanket_match() {
        let set = PermissionSet::from_strings(["time", "random"]).unwrap();
        assert!(set.check(&PermissionRequest::Time).is_ok());
        assert!(set.check(&PermissionRequest::Random).is_ok());
    }

    // ── set behaviour ────────────────────────────────────────────────

    #[test]
    fn empty_set_denies_everything() {
        let set = PermissionSet::empty();
        assert!(matches!(
            set.check(&PermissionRequest::Net { host: "x", port: None }),
            Err(DeniedReason::NotGranted { .. })
        ));
    }

    #[test]
    fn multiple_grants_unioned() {
        let set = PermissionSet::from_strings([
            "net=api.example.com:443",
            "net=fallback.example.com",
        ])
        .unwrap();
        // First grant authorises:
        assert!(set
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
        // Second grant authorises:
        assert!(set
            .check(&PermissionRequest::Net {
                host: "fallback.example.com",
                port: Some(80),
            })
            .is_ok());
    }

    #[test]
    fn out_of_scope_reports_granted_targets() {
        let set = PermissionSet::from_strings([
            "net=api.example.com:443",
            "net=cdn.example.com",
        ])
        .unwrap();
        let err = set
            .check(&PermissionRequest::Net {
                host: "evil.example.com",
                port: Some(80),
            })
            .unwrap_err();
        match err {
            DeniedReason::OutOfScope { granted, requested, kind } => {
                assert_eq!(kind, PermissionKind::Net);
                assert!(requested.contains("evil.example.com"));
                assert!(granted.iter().any(|g| g == "api.example.com:443"));
                assert!(granted.iter().any(|g| g == "cdn.example.com"));
            }
            other => panic!("expected OutOfScope, got {other:?}"),
        }
    }

    #[test]
    fn extend_unions_grants() {
        let mut a = PermissionSet::from_strings(["net=api.x"]).unwrap();
        let b = PermissionSet::from_strings(["fs:read=./data"]).unwrap();
        a.extend(b);
        assert_eq!(a.len(), 2);
        assert!(a
            .check(&PermissionRequest::FsRead(Path::new("./data/x")))
            .is_ok());
    }

    #[test]
    fn from_grants_round_trips() {
        let grants = vec![p("net=api.x"), p("fs:read")];
        let set = PermissionSet::from_grants(grants.clone());
        assert_eq!(set.len(), 2);
        assert_eq!(
            set.grants_of(PermissionKind::Net).count(),
            1,
            "exactly one net grant"
        );
    }

    #[test]
    fn permission_kind_round_trip() {
        for kind_str in [
            "fs:read", "fs:write", "net", "env", "run", "ffi", "time", "random",
        ] {
            let k = parse_kind(kind_str).unwrap();
            assert_eq!(k.as_str(), kind_str);
            assert_eq!(format!("{k}"), kind_str);
        }
    }

    #[test]
    fn parse_error_display_carries_kind_and_value() {
        let err = Permission::parse("kernel=/x").unwrap_err();
        assert!(err.to_string().contains("kernel"));
        let err = Permission::parse("time=1h").unwrap_err();
        assert!(err.to_string().contains("time") && err.to_string().contains("blanket"));
    }
}
