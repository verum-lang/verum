//! Per-file digest cache for `verum lint`.
//!
//! On a CI run that re-lints an unchanged repository, every file
//! parse and AST walk is wasted work — the diagnostics are
//! deterministic, so the result for *(content_hash, config_hash)*
//! is stable. This module persists that result on disk so the
//! second run becomes O(files) cache lookups instead of O(files)
//! parses.
//!
//! ## Layout
//!
//! ```text
//! target/lint-cache/
//!   <config-hash>/
//!     <source-hash>.json     ← Vec<CachedIssue> for one file
//!   index.json               ← top-level metadata (version, last-cleanup)
//! ```
//!
//! `<config-hash>` folds the serialised `LintConfig` and the
//! verum_cli crate version. Any change to severity_map / presets /
//! custom rules / verum_cli itself produces a new directory; old
//! directories become unreachable and are cleaned up by the GC step
//! (see [`gc`]). `<source-hash>` is `blake3(content)` and is the only
//! thing the per-file lookup keys on.
//!
//! ## Why JSON, not bincode
//!
//! The cache entries are tiny (a few KB tops) and reading them is
//! not the hot path — it's the *avoidance* of running passes that
//! makes this fast. JSON keeps the cache human-inspectable, which
//! is worth more than a 2-3× decode speedup for files this size.

use std::path::{Path, PathBuf};

use blake3::Hasher;
use serde::{Deserialize, Serialize};

use super::lint::{LintConfig, LintIssue, LintLevel};
use verum_common::Text;

/// Bumped when the on-disk format changes in a breaking way.
const CACHE_FORMAT_VERSION: u32 = 1;

/// Per-issue cache record. Mirrors `LintIssue` but uses `String`
/// for the rule name so it survives a serialisation round-trip;
/// the `'static str` invariant is restored on load via `Box::leak`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedIssue {
    rule: String,
    level: LintLevel,
    file: PathBuf,
    line: usize,
    column: usize,
    message: String,
    suggestion: Option<String>,
    fixable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    issues: Vec<CachedIssue>,
}

/// Resolved cache configuration for one lint run. Built once at the
/// start of the run; passed by reference into the parallel walker.
pub struct LintCache {
    /// `target/lint-cache/<config-hash>/`. Per-file lookups append
    /// `<source-hash>.json` to this path.
    bucket: PathBuf,
    /// Whether reads + writes are honoured; `false` means the cache
    /// is bypassed (forces every file to re-lint).
    enabled: bool,
}

impl LintCache {
    /// Construct a cache rooted at `target/lint-cache/`. The
    /// `enabled` flag is the kill switch read from the CLI
    /// (`--no-cache`) or `[lint.cache].enabled = false`.
    pub fn new(target_dir: &Path, config: &LintConfig, enabled: bool) -> Self {
        let bucket = target_dir
            .join("lint-cache")
            .join(config_hash(config));
        if enabled {
            // Best-effort directory creation. If the FS rejects us
            // we'll just miss every cache entry, which is correct
            // behaviour but not fast.
            let _ = std::fs::create_dir_all(&bucket);
        }
        LintCache { bucket, enabled }
    }

    /// Build a "cache disabled" handle. Callers that don't have a
    /// resolved config (or are running in single-file mode) use
    /// this to keep the lookup API uniform.
    pub fn disabled() -> Self {
        LintCache {
            bucket: PathBuf::new(),
            enabled: false,
        }
    }

    /// Compute the path the cache *would* use for a given source
    /// hash, regardless of whether the entry exists.
    fn entry_path(&self, source_hash: &str) -> PathBuf {
        self.bucket.join(format!("{source_hash}.json"))
    }

    /// Try to load cached issues for this source hash. Returns
    /// `None` if the cache is disabled, the file doesn't exist,
    /// the JSON is malformed, or the format version doesn't match.
    pub fn load(&self, source_hash: &str) -> Option<Vec<LintIssue>> {
        if !self.enabled {
            return None;
        }
        let path = self.entry_path(source_hash);
        let raw = std::fs::read(&path).ok()?;
        let parsed: CacheFile = serde_json::from_slice(&raw).ok()?;
        if parsed.version != CACHE_FORMAT_VERSION {
            return None;
        }
        Some(parsed.issues.into_iter().map(into_lint_issue).collect())
    }

    /// Persist this set of issues under the given source hash.
    /// Failures are silent — a write error means the next run
    /// re-lints the file, which is correct behaviour.
    pub fn store(&self, source_hash: &str, issues: &[LintIssue]) {
        if !self.enabled {
            return;
        }
        let entry = CacheFile {
            version: CACHE_FORMAT_VERSION,
            issues: issues.iter().map(from_lint_issue).collect(),
        };
        let payload = match serde_json::to_vec(&entry) {
            Ok(b) => b,
            Err(_) => return,
        };
        let path = self.entry_path(source_hash);
        // Atomic write: write to a sibling temp file, then rename.
        // Avoids a partially-written cache entry being read.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, &payload).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Clean every cache bucket that does *not* match the current
    /// config hash. Run once per CLI invocation that owns the cache
    /// — doing so on every read would amplify I/O in the common
    /// hit case.
    pub fn gc(&self) {
        if !self.enabled {
            return;
        }
        let parent = match self.bucket.parent() {
            Some(p) => p,
            None => return,
        };
        let our_dir = match self.bucket.file_name() {
            Some(n) => n.to_owned(),
            None => return,
        };
        let entries = match std::fs::read_dir(parent) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name == our_dir {
                continue;
            }
            // Only touch directories we created — anything else
            // (a stray file, a lockfile from another tool) stays
            // put.
            if entry.path().is_dir() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Hash the file content. Used as the key inside one config bucket.
pub fn source_hash(content: &str) -> String {
    let hash = blake3::hash(content.as_bytes());
    hash.to_hex().to_string()
}

/// Hash the resolved config + the verum_cli crate version. Two runs
/// with different presets / severity maps / custom rules produce
/// different config hashes, so they never share cache entries.
fn config_hash(config: &LintConfig) -> String {
    let mut hasher = Hasher::new();
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"\n");
    if let Some(extends) = &config.extends {
        hasher.update(b"extends=");
        hasher.update(extends.as_bytes());
        hasher.update(b"\n");
    }
    let mut sev: Vec<(&str, LintLevel)> = config
        .severity_map
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    sev.sort_by_key(|(k, _)| *k);
    for (rule, lvl) in sev {
        hasher.update(b"sev=");
        hasher.update(rule.as_bytes());
        hasher.update(b"=");
        hasher.update(lvl.as_str().as_bytes());
        hasher.update(b"\n");
    }
    let mut sets: [(&str, Vec<&String>); 4] = [
        ("disabled", config.disabled.iter().collect()),
        ("denied", config.denied.iter().collect()),
        ("allowed", config.allowed.iter().collect()),
        ("warned", config.warned.iter().collect()),
    ];
    for (label, list) in &mut sets {
        list.sort();
        hasher.update(label.as_bytes());
        for n in list {
            hasher.update(b"=");
            hasher.update(n.as_bytes());
        }
        hasher.update(b"\n");
    }
    let mut overrides: Vec<&(String, super::lint::FileOverride)> =
        config.per_file_overrides.iter().collect();
    overrides.sort_by(|a, b| a.0.cmp(&b.0));
    for (pat, ovr) in overrides {
        hasher.update(b"override=");
        hasher.update(pat.as_bytes());
        for slot in [&ovr.allow, &ovr.deny, &ovr.warn, &ovr.disable] {
            for r in slot {
                hasher.update(b":");
                hasher.update(r.as_bytes());
            }
        }
        hasher.update(b"\n");
    }
    for rule in &config.custom_rules {
        hasher.update(b"custom=");
        hasher.update(rule.name.as_bytes());
        hasher.update(b"|");
        hasher.update(rule.pattern.as_bytes());
        hasher.update(b"|");
        hasher.update(rule.level.as_str().as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

fn from_lint_issue(issue: &LintIssue) -> CachedIssue {
    CachedIssue {
        rule: issue.rule.to_string(),
        level: issue.level,
        file: issue.file.clone(),
        line: issue.line,
        column: issue.column,
        message: issue.message.clone(),
        suggestion: issue.suggestion.as_ref().map(|t| t.as_str().to_string()),
        fixable: issue.fixable,
    }
}

fn into_lint_issue(cached: CachedIssue) -> LintIssue {
    // The rule name on `LintIssue` is `&'static str` — built-in
    // rules use string literals, custom rules already leak their
    // names. Cache loads need the same lifetime, so we leak here
    // too. The leak is bounded by the number of distinct rule
    // names a single process ever loads from cache (~tens), which
    // is small.
    let rule_static: &'static str = Box::leak(cached.rule.into_boxed_str());
    LintIssue {
        rule: rule_static,
        level: cached.level,
        file: cached.file,
        line: cached.line,
        column: cached.column,
        message: cached.message,
        suggestion: cached.suggestion.map(Text::from),
        fixable: cached.fixable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn empty_config() -> LintConfig {
        LintConfig {
            extends: None,
            disabled: HashSet::new(),
            denied: HashSet::new(),
            allowed: HashSet::new(),
            warned: HashSet::new(),
            severity_map: HashMap::new(),
            rules: HashMap::new(),
            per_file_overrides: Vec::new(),
            profiles: HashMap::new(),
            custom_rules: Vec::new(),
        }
    }

    #[test]
    fn config_hash_changes_with_severity_map() {
        let mut c1 = empty_config();
        let h_empty = config_hash(&c1);
        c1.severity_map.insert("todo-in-code".into(), LintLevel::Error);
        let h_changed = config_hash(&c1);
        assert_ne!(h_empty, h_changed, "severity change must reshape the hash");
    }

    #[test]
    fn config_hash_is_order_independent() {
        let mut c1 = empty_config();
        c1.severity_map.insert("a".into(), LintLevel::Warning);
        c1.severity_map.insert("b".into(), LintLevel::Error);
        let h1 = config_hash(&c1);

        let mut c2 = empty_config();
        c2.severity_map.insert("b".into(), LintLevel::Error);
        c2.severity_map.insert("a".into(), LintLevel::Warning);
        let h2 = config_hash(&c2);
        assert_eq!(h1, h2, "hash must not depend on insertion order");
    }

    #[test]
    fn source_hash_differs_per_content() {
        let a = source_hash("fn main() { let x = 1; }");
        let b = source_hash("fn main() { let x = 2; }");
        assert_ne!(a, b);
    }

    #[test]
    fn store_and_load_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg = empty_config();
        let cache = LintCache::new(tmp.path(), &cfg, true);
        let sh = source_hash("fn main() {}");
        let issues = vec![LintIssue {
            rule: "todo-in-code",
            level: LintLevel::Warning,
            file: PathBuf::from("a.vr"),
            line: 1,
            column: 5,
            message: "TODO".to_string(),
            suggestion: None,
            fixable: false,
        }];
        cache.store(&sh, &issues);
        let loaded = cache.load(&sh).expect("cache hit expected");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].rule, "todo-in-code");
        assert_eq!(loaded[0].line, 1);
    }

    #[test]
    fn disabled_cache_returns_none_and_does_not_persist() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg = empty_config();
        let cache = LintCache::new(tmp.path(), &cfg, false);
        let sh = source_hash("fn main() {}");
        cache.store(&sh, &[]);
        assert!(cache.load(&sh).is_none());
    }
}
