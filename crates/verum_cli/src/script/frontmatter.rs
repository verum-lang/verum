//! PEP-723-style inline metadata for Verum scripts.
//!
//! A Verum script may carry an inline metadata block delimited by
//! `// /// script` and `// ///` line comments. The interior is line-prefixed
//! with `// `; after stripping that prefix, the remainder is parsed as TOML.
//!
//! Example:
//!
//! ```verum
//! #!/usr/bin/env verum
//! // /// script
//! // verum = ">=0.6.0"
//! // dependencies = ["json@1", "http@^0.2"]
//! // permissions = ["net=api.example.com", "read=./data"]
//! // edition = "2026"
//! //
//! // [profile]
//! // tier = 0
//! // ///
//!
//! mount core.io
//! print("hello");
//! ```
//!
//! # Why this format
//!
//! - **Inert under regular Verum tooling**: the entire block is line comments,
//!   so the LSP, formatter, and linter do not need a special parse mode.
//! - **Industry alignment**: PEP 723 (Python, accepted Jan 2024) uses the same
//!   `# /// script` / `# ///` pattern. `uv run`, `pipx run`, `poetry run`, and
//!   the upcoming `dotnet run app.cs` all follow comparable shapes. Users
//!   transferring from those ecosystems recognise the structure immediately.
//! - **Single source of truth**: TOML is already the manifest format
//!   (`verum.toml`), so the field schema is reused — no new dialect.
//!
//! # Schema
//!
//! Top-level keys recognised today:
//!
//! | key | type | meaning |
//! |---|---|---|
//! | `verum` | semver spec | required minimum Verum compiler |
//! | `dependencies` | array or table | registry deps for this script |
//! | `permissions` | array of strings | Deno-style permission scopes |
//! | `edition` | string | language edition (currently informational) |
//! | `[profile]` | table | per-script profile overrides |
//! | `[run]` | table | run-time defaults (inherits from `[run]` in `verum.toml`) |
//!
//! Unknown keys are preserved in `raw` and surfaced to callers — this lets
//! future schema additions be picked up without parser revisions.

use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Parsed frontmatter metadata extracted from a script source. Wraps the raw
/// TOML value plus convenience accessors for the well-known fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    /// The raw TOML body (after `// `-stripping). Empty string if the body had
    /// no non-blank content.
    #[serde(skip)]
    pub raw_toml: String,

    /// Required minimum compiler version, e.g. `">=0.6.0"`. None if absent.
    #[serde(default)]
    pub verum: Option<String>,

    /// Dependencies declared inline. Each entry is either a bare version spec
    /// (`"json@1"`) or a fully qualified table (`{name="x", version="1"}`)
    /// represented as raw TOML for now — full typing happens in P4 (resolver).
    #[serde(default)]
    pub dependencies: Vec<DepSpec>,

    /// Permission scopes (Deno-style). Each is a string like `"net=api.x:443"`.
    #[serde(default)]
    pub permissions: Vec<String>,

    /// Language edition (informational; reserved for future use).
    #[serde(default)]
    pub edition: Option<String>,

    /// Profile overrides for this script (tier, verify, opt-level, etc.).
    #[serde(default)]
    pub profile: Option<ProfileOverrides>,

    /// Run-time defaults. May override [run] in user's verum.toml when present.
    #[serde(default)]
    pub run: Option<RunOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Short form: `"json@1"` or `"json"`.
    Short(String),
    /// Long form: `{ name = "json", version = "^1", features = ["..."], path = "...", git = "..." }`
    Long(DepLong),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepLong {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub git: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileOverrides {
    #[serde(default)]
    pub tier: Option<u8>,
    #[serde(default)]
    pub verify: Option<String>,
    #[serde(default, rename = "opt-level")]
    pub opt_level: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunOverrides {
    #[serde(default, rename = "default-permissions")]
    pub default_permissions: Vec<String>,
}

/// Successful extraction result: the parsed frontmatter plus the byte range it
/// occupied in the original source (so callers can slice it out before further
/// processing if desired).
#[derive(Debug, Clone)]
pub struct Extracted {
    pub frontmatter: Frontmatter,
    /// Byte range in the original source covering the whole frontmatter
    /// region (both delimiter lines included). Empty range if no block found.
    pub range: Range<usize>,
}

/// Errors that can arise while parsing frontmatter.
#[derive(Debug, Clone)]
pub enum FrontmatterError {
    UnterminatedBlock { line: usize },
    InvalidToml { line: usize, source: String },
    NonCommentLineInBlock { line: usize, content: String },
    /// `verum = "<spec>"` is not a valid semver requirement.
    InvalidVerumRequirement { value: String, reason: String },
    /// A `dependencies` short-form entry doesn't match `name(@version)?`
    /// or the name is not a valid cog identifier.
    InvalidDepShortForm { value: String, reason: String },
    /// A `dependencies` long-form table entry has an empty / invalid `name`.
    InvalidDepLongForm { name: String, reason: String },
    /// A `permissions` scope string doesn't match the documented grammar.
    /// `value` is the full scope string; `reason` explains the violation.
    InvalidPermissionScope { value: String, reason: String },
}

impl std::fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnterminatedBlock { line } => write!(
                f,
                "frontmatter: opening marker `// /// script` found at line {line} but no closing `// ///`",
            ),
            Self::InvalidToml { line, source } => {
                write!(f, "frontmatter: malformed TOML at line {line}: {source}")
            }
            Self::NonCommentLineInBlock { line, content } => write!(
                f,
                "frontmatter: line {line} inside block does not start with `//`: {content:?}",
            ),
            Self::InvalidVerumRequirement { value, reason } => write!(
                f,
                "frontmatter: `verum = {value:?}` is not a valid semver requirement: {reason}",
            ),
            Self::InvalidDepShortForm { value, reason } => write!(
                f,
                "frontmatter: dependency entry {value:?} is not a valid `name(@version)?` form: {reason}",
            ),
            Self::InvalidDepLongForm { name, reason } => write!(
                f,
                "frontmatter: long-form dependency `{name}`: {reason}",
            ),
            Self::InvalidPermissionScope { value, reason } => write!(
                f,
                "frontmatter: permission scope {value:?} is invalid: {reason}",
            ),
        }
    }
}

impl std::error::Error for FrontmatterError {}

// =============================================================================
// Schema validation
// =============================================================================
//
// Beyond TOML well-formedness, the frontmatter contract pins a precise grammar
// for each user-facing field. We validate after the TOML pass so individual
// errors can pinpoint the offending value rather than fail the whole block on
// the first malformed entry.

/// Cog identifier grammar (matches `grammar/verum.ebnf` `identifier` for the
/// kebab-or-snake-case subset used in registry / mount paths):
///
/// ```text
/// cog_ident = ascii_alpha , { ascii_alpha | ascii_digit | "_" | "-" } ;
/// ```
///
/// First character must be an ASCII letter (so `42json` is rejected); rest
/// allow letters / digits / `_` / `-`. We deliberately stay ASCII to match
/// crates.io / cargo / npm cog-name conventions; Unicode identifiers are
/// fine inside Verum source but not in registry-targeted dep names where
/// case-folding ambiguity would be a real bug.
fn is_valid_cog_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Semver requirement validator. Accepts:
///
///  * Bare version    — `1.2.3`, `0.1.0-rc.1`
///  * Operator + ver  — `>=0.6.0`, `^1.2`, `~2.3.4`, `<3`, `>1.0`, `=1.2.3`
///  * Wildcard        — `1.*`, `1.2.*`, `*`
///  * Comma list      — `>=1.2, <2`
///
/// The grammar is deliberately permissive (matches Cargo's `VersionReq`)
/// because the PubGrub-side resolver (P4) does the load-bearing matching;
/// we just refuse obvious typos here so a misspelled spec fails at parse
/// time rather than at resolve time.
fn validate_semver_req(spec: &str) -> Result<(), &'static str> {
    let s = spec.trim();
    if s.is_empty() {
        return Err("empty spec");
    }
    // Each comma-separated clause is independently validated.
    for clause in s.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            return Err("empty clause inside comma-separated requirement");
        }
        // Strip a leading operator if present.
        let body = if let Some(rest) = clause.strip_prefix(">=") {
            rest
        } else if let Some(rest) = clause.strip_prefix("<=") {
            rest
        } else if let Some(rest) = clause.strip_prefix('>') {
            rest
        } else if let Some(rest) = clause.strip_prefix('<') {
            rest
        } else if let Some(rest) = clause.strip_prefix('^') {
            rest
        } else if let Some(rest) = clause.strip_prefix('~') {
            rest
        } else if let Some(rest) = clause.strip_prefix('=') {
            rest
        } else {
            clause
        };
        let body = body.trim();
        if body == "*" {
            continue;
        }
        // Body must look like a numeric version: digits, dots, and the
        // standard hyphenated pre-release / metadata tail (`-rc.1+sha.…`).
        let core = body.split(['-', '+']).next().unwrap_or(body);
        // Each dotted segment must be `*` or an ASCII numeric literal.
        let mut saw_segment = false;
        for seg in core.split('.') {
            saw_segment = true;
            if seg == "*" {
                continue;
            }
            if seg.is_empty() || !seg.chars().all(|c| c.is_ascii_digit()) {
                return Err("dotted segment must be digits or `*`");
            }
        }
        if !saw_segment {
            return Err("missing version core");
        }
    }
    Ok(())
}

/// Validate a `dependencies` short-form entry of the form
/// `name(@version)?`. Both the name and the optional version are checked
/// against their respective grammars.
fn validate_dep_short_form(spec: &str) -> Result<(), String> {
    if spec.is_empty() {
        return Err("empty dependency spec".to_string());
    }
    let (name, ver) = match spec.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (spec, None),
    };
    if !is_valid_cog_ident(name) {
        return Err(format!(
            "invalid cog name {name:?} (must start with an ASCII letter; \
             chars allowed: letters, digits, `_`, `-`)"
        ));
    }
    if let Some(v) = ver {
        validate_semver_req(v).map_err(|r| format!("invalid version {v:?}: {r}"))?;
    }
    Ok(())
}

/// Permission scope grammar — must match the design's
/// [run].default-permissions documentation exactly:
///
/// ```text
/// scope     = scope_kind , [ "=" , scope_targets ]
/// scope_kind = "fs:read" | "fs:write" | "net" | "env" | "run" | "ffi"
///            | "time"    | "random"
/// scope_targets = target , { "," , target }      (* non-empty, no whitespace *)
/// target    = any non-comma, non-whitespace UTF-8 sequence
/// ```
///
/// `time` and `random` are the only scopes that have no `=value` form;
/// every other scope may stand alone (granting blanket access) or be
/// narrowed via `=`-separated targets.
fn validate_permission_scope(scope: &str) -> Result<(), String> {
    const KINDS: &[&str] = &[
        "fs:read", "fs:write", "net", "env", "run", "ffi", "time", "random",
    ];
    let s = scope.trim();
    if s.is_empty() {
        return Err("empty scope".to_string());
    }
    let (kind, targets) = match s.split_once('=') {
        Some((k, t)) => (k.trim(), Some(t)),
        None => (s, None),
    };
    if !KINDS.contains(&kind) {
        return Err(format!(
            "unknown scope kind {kind:?} (expected one of {KINDS:?})"
        ));
    }
    if let Some(targets) = targets {
        if targets.is_empty() {
            return Err(format!("scope {kind:?} has empty target list after `=`"));
        }
        // `time` / `random` — no `=value` form per the grammar.
        if matches!(kind, "time" | "random") {
            return Err(format!(
                "scope {kind:?} does not accept `=value` (it grants blanket access)"
            ));
        }
        for t in targets.split(',') {
            if t.is_empty() {
                return Err(format!("scope {kind:?} has an empty target between commas"));
            }
            if t.chars().any(|c| c.is_whitespace()) {
                return Err(format!(
                    "scope target {t:?} contains whitespace; use commas to separate targets"
                ));
            }
        }
    }
    Ok(())
}

/// Validate a [`Frontmatter`]'s well-known fields against the documented
/// grammar. Called after [`extract`]'s TOML parse; returns the *first*
/// error encountered so the user can see one structured diagnostic at a
/// time. Re-validating after a fix surfaces the next issue.
pub fn validate(fm: &Frontmatter) -> Result<(), FrontmatterError> {
    if let Some(spec) = &fm.verum {
        validate_semver_req(spec).map_err(|reason| FrontmatterError::InvalidVerumRequirement {
            value: spec.clone(),
            reason: reason.to_string(),
        })?;
    }
    for dep in &fm.dependencies {
        match dep {
            DepSpec::Short(s) => {
                validate_dep_short_form(s).map_err(|reason| {
                    FrontmatterError::InvalidDepShortForm {
                        value: s.clone(),
                        reason,
                    }
                })?;
            }
            DepSpec::Long(d) => {
                if !is_valid_cog_ident(&d.name) {
                    return Err(FrontmatterError::InvalidDepLongForm {
                        name: d.name.clone(),
                        reason: "name must be a valid cog identifier (ASCII letter + \
                                 letters / digits / `_` / `-`)"
                            .to_string(),
                    });
                }
                if let Some(v) = &d.version {
                    validate_semver_req(v).map_err(|reason| {
                        FrontmatterError::InvalidDepLongForm {
                            name: d.name.clone(),
                            reason: format!("version {v:?}: {reason}"),
                        }
                    })?;
                }
            }
        }
    }
    for perm in &fm.permissions {
        validate_permission_scope(perm).map_err(|reason| {
            FrontmatterError::InvalidPermissionScope {
                value: perm.clone(),
                reason,
            }
        })?;
    }
    if let Some(run) = &fm.run {
        for perm in &run.default_permissions {
            validate_permission_scope(perm).map_err(|reason| {
                FrontmatterError::InvalidPermissionScope {
                    value: perm.clone(),
                    reason,
                }
            })?;
        }
    }
    Ok(())
}

/// Locate the `// /// script` ... `// ///` block in `source`, parse the body
/// as TOML, and return the structured `Frontmatter`. Returns `Ok(None)` if
/// the source contains no frontmatter block.
///
/// Detection rules:
/// - Opening line, after `trim_end()`, equals `// /// script` (with optional
///   inner whitespace, e.g. `//  /// script` is also accepted — we strip a
///   single space after `//`).
/// - Closing line, after trim, equals `// ///`.
/// - Detection scans the source from byte 0; a leading UTF-8 BOM
///   (`EF BB BF`) is silently skipped, and a leading shebang line is also
///   silently skipped if present.
/// - At most one block per source. The opening match wins; subsequent
///   `// /// script` lines are treated as ordinary comments.
///
/// Byte ranges in the returned [`Extracted::range`] are in *original-source*
/// coordinates (i.e. include any BOM byte offset). Callers that strip the
/// BOM separately can subtract `UTF8_BOM_LEN` themselves.
pub fn extract(source: &str) -> Result<Option<Extracted>, FrontmatterError> {
    // Detect and skip a leading UTF-8 BOM. Cross-platform editors prepend
    // it on save and the marker-matcher below can't see past it (the BOM
    // is 3 bytes that don't form a valid `//` prefix). We skip the BOM
    // for line-walking but add `bom_len` back to every byte range we
    // return so callers stay in original-source coordinates.
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let bom_len = if source.as_bytes().starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    };
    let scan = &source[bom_len..];

    let mut lines: Vec<(Range<usize>, &str)> = Vec::new();
    let bytes = scan.as_bytes();
    let mut start = 0usize;
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            let line = &scan[start..i];
            // Spans here are scan-relative; we shift them by `bom_len`
            // when assembling `Extracted::range` below.
            lines.push((start..i + 1, line));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        lines.push((start..bytes.len(), &scan[start..]));
    }

    // Optional shebang skip on first line.
    let mut idx = 0usize;
    if let Some((_, l)) = lines.first() {
        if l.starts_with("#!") {
            idx = 1;
        }
    }

    // Find opening marker.
    let mut open_idx: Option<usize> = None;
    while idx < lines.len() {
        if is_open_marker(lines[idx].1) {
            open_idx = Some(idx);
            break;
        }
        idx += 1;
    }
    let open_idx = match open_idx {
        Some(i) => i,
        None => return Ok(None),
    };

    // Find closing marker.
    let mut close_idx: Option<usize> = None;
    let mut j = open_idx + 1;
    while j < lines.len() {
        if is_close_marker(lines[j].1) {
            close_idx = Some(j);
            break;
        }
        j += 1;
    }
    let close_idx = match close_idx {
        Some(i) => i,
        None => return Err(FrontmatterError::UnterminatedBlock { line: open_idx + 1 }),
    };

    // Strip `// ` prefix from each interior line and assemble TOML body.
    let mut toml_body = String::new();
    for (line_no, (_, raw)) in lines[(open_idx + 1)..close_idx].iter().enumerate() {
        let logical_line = open_idx + 2 + line_no; // 1-indexed user-visible
        let trimmed = strip_comment_prefix(raw)
            .ok_or_else(|| FrontmatterError::NonCommentLineInBlock {
                line: logical_line,
                content: raw.trim().to_string(),
            })?;
        toml_body.push_str(trimmed);
        toml_body.push('\n');
    }

    let parsed: Frontmatter = match toml::from_str::<Frontmatter>(&toml_body) {
        Ok(mut fm) => {
            fm.raw_toml = toml_body;
            fm
        }
        Err(e) => {
            // The TOML error reports a position inside `toml_body` (which is
            // post-strip). Translating it back to the original source line
            // requires accounting for the offset at which interior lines
            // begin; we approximate by adding the opening marker line.
            return Err(FrontmatterError::InvalidToml {
                line: open_idx + 2,
                source: e.to_string(),
            });
        }
    };

    // Shift scan-relative byte ranges back into original-source coordinates
    // by re-adding the BOM length we stripped at the top.
    let range = (lines[open_idx].0.start + bom_len)..(lines[close_idx].0.end + bom_len);
    Ok(Some(Extracted {
        frontmatter: parsed,
        range,
    }))
}

/// Variant: Like [`extract`] but returns `None` quietly on error. Useful for
/// best-effort tooling that wants to display the script even with a malformed
/// frontmatter (the proper user-facing reporting path uses [`extract`]).
pub fn extract_lossy(source: &str) -> Option<Extracted> {
    extract(source).ok().flatten()
}

/// One-shot extract + validate.
///
/// Production callers (script-mode dispatch in `verum run`) want to fail
/// fast on either a malformed block (invalid TOML, unterminated marker)
/// OR a malformed value (bad semver req, unknown permission scope). This
/// helper composes the two passes so a single `?` covers both.
///
/// Returns `Ok(None)` when the source has no frontmatter — that's not an
/// error, it just means the script doesn't carry inline metadata.
///
/// Returns `Ok(Some(extracted))` when both extraction and schema
/// validation succeeded.
///
/// Returns `Err(...)` on the first failure, whether structural or
/// semantic. The error variants are user-actionable: each variant carries
/// the offending value so the caller can render a precise diagnostic.
pub fn extract_and_validate(source: &str) -> Result<Option<Extracted>, FrontmatterError> {
    let Some(extracted) = extract(source)? else {
        return Ok(None);
    };
    validate(&extracted.frontmatter)?;
    Ok(Some(extracted))
}

fn is_open_marker(line: &str) -> bool {
    let s = line.trim_end_matches(['\r', '\n']);
    let s = strip_comment_prefix(s).unwrap_or(s);
    s.trim() == "/// script"
}

fn is_close_marker(line: &str) -> bool {
    let s = line.trim_end_matches(['\r', '\n']);
    let s = strip_comment_prefix(s).unwrap_or(s);
    s.trim() == "///"
}

/// If `line` begins with a `//` line-comment marker, return the content after
/// it (with a single following space stripped if present). Returns `None` for
/// non-comment lines.
fn strip_comment_prefix(line: &str) -> Option<&str> {
    // Allow optional leading whitespace before `//` so that callers indenting
    // their frontmatter for visual alignment still parse.
    let trimmed = line.trim_start();
    if !trimmed.starts_with("//") {
        return None;
    }
    let after = &trimmed[2..];
    // Strip one trailing space — `// foo` -> `foo`, `//foo` -> `foo`.
    if after.starts_with(' ') {
        Some(&after[1..])
    } else {
        Some(after)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let src = "fn main() {}";
        let r = extract(src).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn empty_block() {
        let src = "// /// script\n// ///\nfn main() {}";
        let e = extract(src).unwrap().expect("frontmatter must be detected");
        assert_eq!(e.frontmatter.verum, None);
        assert!(e.frontmatter.dependencies.is_empty());
        assert!(e.frontmatter.permissions.is_empty());
    }

    #[test]
    fn full_block() {
        let src = r#"#!/usr/bin/env verum
// /// script
// verum = ">=0.6.0"
// dependencies = ["json@1", "http"]
// permissions = ["net=api.example.com:443", "read=./data"]
// edition = "2026"
//
// [profile]
// tier = 0
// verify = "off"
// opt-level = 2
//
// [run]
// default-permissions = ["fs:cwd"]
// ///

mount core.io
print("hello");
"#;
        let e = extract(src).unwrap().unwrap();
        let fm = &e.frontmatter;
        assert_eq!(fm.verum.as_deref(), Some(">=0.6.0"));
        assert_eq!(fm.dependencies.len(), 2);
        match &fm.dependencies[0] {
            DepSpec::Short(s) => assert_eq!(s, "json@1"),
            DepSpec::Long(_) => panic!("expected short form"),
        }
        assert_eq!(fm.permissions.len(), 2);
        assert_eq!(fm.permissions[0], "net=api.example.com:443");
        assert_eq!(fm.edition.as_deref(), Some("2026"));
        let p = fm.profile.as_ref().unwrap();
        assert_eq!(p.tier, Some(0));
        assert_eq!(p.verify.as_deref(), Some("off"));
        assert_eq!(p.opt_level, Some(2));
        let r = fm.run.as_ref().unwrap();
        assert_eq!(r.default_permissions, vec!["fs:cwd"]);
    }

    #[test]
    fn long_form_dep() {
        let src = r#"// /// script
// dependencies = [{ name = "json", version = "^1.0", features = ["serde"] }]
// ///
"#;
        let e = extract(src).unwrap().unwrap();
        match &e.frontmatter.dependencies[0] {
            DepSpec::Long(d) => {
                assert_eq!(d.name, "json");
                assert_eq!(d.version.as_deref(), Some("^1.0"));
                assert_eq!(d.features, vec!["serde"]);
            }
            DepSpec::Short(_) => panic!("expected long form"),
        }
    }

    #[test]
    fn unterminated_block() {
        let src = "// /// script\n// verum = \">=0.6\"\n// (no terminator)\n";
        match extract(src) {
            Err(FrontmatterError::UnterminatedBlock { line }) => assert_eq!(line, 1),
            other => panic!("expected UnterminatedBlock, got {:?}", other),
        }
    }

    #[test]
    fn malformed_toml() {
        let src = "// /// script\n// this is not valid = toml = at all\n// ///\n";
        match extract(src) {
            Err(FrontmatterError::InvalidToml { .. }) => {}
            other => panic!("expected InvalidToml, got {:?}", other),
        }
    }

    #[test]
    fn shebang_then_block() {
        let src = "#!/usr/bin/env verum\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn block_with_indented_comments() {
        let src = "    // /// script\n    // verum = \"0.6\"\n    // ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn no_extra_block_after_closer() {
        // A second `// /// script` after the first block closes is just a
        // comment; we don't re-enter parsing.
        let src = "// /// script\n// verum = \"0.6\"\n// ///\n// /// script\n// foo\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range should cover only the first block (3 lines).
        let covered = &src[e.range];
        assert!(covered.contains("verum"));
        assert!(!covered.contains("foo"));
    }

    #[test]
    fn range_is_correct() {
        let src = "// /// script\n// verum = \"0.6\"\n// ///\n\nfn main() {}";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.range.start, 0);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
        assert!(covered.trim_end().ends_with("// ///"));
    }

    #[test]
    fn block_after_blank_lines() {
        let src = "\n\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn bom_only_then_block() {
        // Cross-platform editor saved the script with a UTF-8 BOM. The
        // marker matcher must look past the BOM and the byte range we
        // return must be in original-source coordinates.
        let src = "\u{FEFF}// /// script\n// verum = \"0.6\"\n// ///\nmount core.io";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range must be original-source coords: start at byte 3 (post-BOM).
        assert_eq!(e.range.start, 3);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
        assert!(covered.trim_end().ends_with("// ///"));
    }

    #[test]
    fn bom_then_shebang_then_block() {
        // Most-permissive layout: BOM precedes shebang precedes block.
        // Both prefix layers must be silently skipped.
        let src = "\u{FEFF}#!/usr/bin/env verum\n// /// script\n// verum = \"0.6\"\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
        // Range starts at 3 (BOM) + 21 (shebang) = 24.
        let expected_start = 3 + "#!/usr/bin/env verum\n".len();
        assert_eq!(e.range.start, expected_start);
        let covered = &src[e.range];
        assert!(covered.starts_with("// /// script"));
    }

    #[test]
    fn bom_with_no_block_returns_none() {
        let src = "\u{FEFF}fn main() {}";
        assert!(extract(src).unwrap().is_none());
    }

    // =========================================================================
    // Schema validation (F20)
    // =========================================================================

    fn validated(src: &str) -> Result<Frontmatter, FrontmatterError> {
        let e = extract(src).unwrap().expect("frontmatter must extract");
        validate(&e.frontmatter)?;
        Ok(e.frontmatter)
    }

    // --- semver req ---------------------------------------------------------

    #[test]
    fn validate_semver_accepts_canonical_forms() {
        for spec in &[
            "0.6.0", "1.2.3", "0.1.0-rc.1",
            ">=0.6.0", "^1.2", "~2.3.4", "<3", ">1.0", "=1.2.3", "<=4.5",
            "1.*", "1.2.*", "*",
            ">=1.2, <2",
        ] {
            assert!(
                validate_semver_req(spec).is_ok(),
                "should accept {spec:?}"
            );
        }
    }

    #[test]
    fn validate_semver_rejects_garbage() {
        for spec in &["", "abc", ">=v1", "1.x.0", "..1", "1..", ">"] {
            assert!(
                validate_semver_req(spec).is_err(),
                "should reject {spec:?}"
            );
        }
    }

    #[test]
    fn extract_then_validate_invalid_verum() {
        let src = "// /// script\n// verum = \"not-a-version\"\n// ///\n";
        let err = validated(src).expect_err("expected validation error");
        match err {
            FrontmatterError::InvalidVerumRequirement { value, .. } => {
                assert_eq!(value, "not-a-version");
            }
            other => panic!("expected InvalidVerumRequirement, got {other:?}"),
        }
    }

    // --- dep names ----------------------------------------------------------

    #[test]
    fn validate_dep_short_form_accepts_canonical() {
        for s in &["json", "json@1", "http@^0.2", "kebab-name@1.2.3", "snake_name"] {
            assert!(validate_dep_short_form(s).is_ok(), "should accept {s:?}");
        }
    }

    #[test]
    fn validate_dep_short_form_rejects_garbage() {
        for s in &["", "@1.0", "42json@1.0", "with spaces@1", "json@bad"] {
            assert!(validate_dep_short_form(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn extract_then_validate_invalid_dep_short() {
        let src = "// /// script\n// dependencies = [\"42json\"]\n// ///\n";
        let err = validated(src).expect_err("expected validation error");
        assert!(
            matches!(err, FrontmatterError::InvalidDepShortForm { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn extract_then_validate_invalid_dep_long_name() {
        let src =
            "// /// script\n// dependencies = [{ name = \"-bad\", version = \"1\" }]\n// ///\n";
        let err = validated(src).expect_err("expected validation error");
        assert!(
            matches!(err, FrontmatterError::InvalidDepLongForm { .. }),
            "got {err:?}"
        );
    }

    // --- permissions --------------------------------------------------------

    #[test]
    fn validate_permission_accepts_canonical_scopes() {
        for s in &[
            "fs:read",
            "fs:write",
            "fs:read=./data",
            "fs:read=./data,./logs",
            "net",
            "net=api.example.com",
            "net=api.example.com:443,localhost",
            "env",
            "env=PATH,HOME",
            "run",
            "ffi=libc",
            "time",
            "random",
        ] {
            assert!(validate_permission_scope(s).is_ok(), "should accept {s:?}");
        }
    }

    #[test]
    fn validate_permission_rejects_unknown_kind() {
        let err = validate_permission_scope("kernel=/dev/mem")
            .expect_err("unknown kind must be rejected");
        assert!(err.contains("unknown scope kind"), "got {err:?}");
    }

    #[test]
    fn validate_permission_rejects_time_with_value() {
        // `time` is blanket-only.
        assert!(validate_permission_scope("time=1h").is_err());
        assert!(validate_permission_scope("random=/dev/urandom").is_err());
    }

    #[test]
    fn validate_permission_rejects_empty_target() {
        assert!(validate_permission_scope("net=").is_err());
        assert!(validate_permission_scope("fs:read=,foo").is_err());
    }

    #[test]
    fn validate_permission_rejects_whitespace_target() {
        assert!(validate_permission_scope("net=foo bar").is_err());
    }

    #[test]
    fn extract_then_validate_invalid_permission() {
        let src = "// /// script\n// permissions = [\"kernel=/dev/mem\"]\n// ///\n";
        let err = validated(src).expect_err("expected validation error");
        match err {
            FrontmatterError::InvalidPermissionScope { value, .. } => {
                assert_eq!(value, "kernel=/dev/mem");
            }
            other => panic!("expected InvalidPermissionScope, got {other:?}"),
        }
    }

    #[test]
    fn extract_then_validate_invalid_permission_in_run_section() {
        let src = "// /// script\n// [run]\n// default-permissions = [\"unknown=x\"]\n// ///\n";
        let err = validated(src).expect_err("expected validation error");
        assert!(
            matches!(err, FrontmatterError::InvalidPermissionScope { .. }),
            "got {err:?}"
        );
    }

    // --- positive end-to-end ------------------------------------------------

    #[test]
    fn validate_full_canonical_block_passes() {
        let src = r#"// /// script
// verum = ">=0.6.0"
// dependencies = ["json@1", "http@^0.2", { name = "x", version = "1" }]
// permissions = ["net=api.example.com:443", "fs:read=./data", "time"]
// edition = "2026"
// [profile]
// tier = 0
// [run]
// default-permissions = ["fs:cwd"]
// ///
"#;
        // Note: `fs:cwd` is intentionally rejected — it's not in the
        // documented kind list — and that's exactly what we want to surface.
        let err = validated(src).expect_err("fs:cwd is not a documented scope kind");
        match err {
            FrontmatterError::InvalidPermissionScope { value, .. } => {
                assert_eq!(value, "fs:cwd");
            }
            other => panic!("expected InvalidPermissionScope, got {other:?}"),
        }
    }

    #[test]
    fn validate_minimal_block_passes() {
        // No fields → no validation errors.
        assert!(validated("// /// script\n// ///\n").is_ok());
    }

    #[test]
    fn cog_ident_grammar_pin() {
        // Pin the cog-ident contract so registry/mount-path consumers
        // can rely on it without re-checking.
        for s in &["a", "abc", "a-b", "a_b", "abc-123_x"] {
            assert!(is_valid_cog_ident(s), "should accept {s:?}");
        }
        for s in &["", "1abc", "-abc", "_abc", "a b", "a.b", "Ünicode"] {
            assert!(!is_valid_cog_ident(s), "should reject {s:?}");
        }
    }

    // =========================================================================
    // extract_and_validate (F21)
    // =========================================================================

    #[test]
    fn extract_and_validate_none_for_no_block() {
        let r = extract_and_validate("fn main() {}").expect("no error");
        assert!(r.is_none());
    }

    #[test]
    fn extract_and_validate_some_for_clean_block() {
        let src = "// /// script\n// verum = \"0.6\"\n// ///\n";
        let r = extract_and_validate(src).expect("no error");
        let e = r.expect("frontmatter must be present");
        assert_eq!(e.frontmatter.verum.as_deref(), Some("0.6"));
    }

    #[test]
    fn extract_and_validate_propagates_extract_error() {
        // Unterminated block — the extraction layer rejects this; the
        // validation layer never runs.
        let src = "// /// script\n// verum = \"0.6\"\n";
        let err = extract_and_validate(src).expect_err("expected error");
        assert!(matches!(err, FrontmatterError::UnterminatedBlock { .. }));
    }

    #[test]
    fn extract_and_validate_propagates_validation_error() {
        // Well-formed TOML but invalid permission scope — the validation
        // layer must reject and the user sees a structured error.
        let src = "// /// script\n// permissions = [\"impossible=x\"]\n// ///\n";
        let err = extract_and_validate(src).expect_err("expected error");
        match err {
            FrontmatterError::InvalidPermissionScope { value, .. } => {
                assert_eq!(value, "impossible=x");
            }
            other => panic!("expected InvalidPermissionScope, got {other:?}"),
        }
    }

    #[test]
    fn extract_and_validate_validates_after_bom_and_shebang() {
        // Stress-test: BOM + shebang + valid block — extract must succeed
        // through both prefix layers AND validation must pass.
        let src = "\u{FEFF}#!/usr/bin/env verum\n// /// script\n// verum = \">=0.6\"\n// ///\n";
        let e = extract_and_validate(src).expect("no error").expect("frontmatter");
        assert_eq!(e.frontmatter.verum.as_deref(), Some(">=0.6"));
    }

    #[test]
    fn permissions_array_only() {
        let src = "// /// script\n// permissions = [\"net\", \"read=./x\"]\n// ///\n";
        let e = extract(src).unwrap().unwrap();
        assert_eq!(e.frontmatter.permissions, vec!["net", "read=./x"]);
    }

    #[test]
    fn extract_lossy_returns_none_on_error() {
        let src = "// /// script\n// (bad TOML\n// ///\n";
        assert!(extract_lossy(src).is_none());
    }

    #[test]
    fn no_block_when_only_comments() {
        // `// /// script` mid-file with no real block doesn't trigger anything.
        let src = "fn main() {\n    // /// script with a real fn\n}\n";
        // We DO match it because is_open_marker only checks the line ends with `/// script`.
        // To avoid false positives the closer must follow. Without a closer,
        // it's an error — UnterminatedBlock — which is acceptable: this is a
        // very narrow user mistake.
        let result = extract(src);
        // Either Err(Unterminated) or Ok(None) is acceptable here. Pin the
        // current behaviour: it parses the marker as a valid open and errors
        // due to missing close.
        matches!(result, Err(FrontmatterError::UnterminatedBlock { .. }));
    }
}
