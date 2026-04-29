//! Suppression baseline — adopt strict rules incrementally on
//! existing code without flooding the team with the legacy issue
//! count.
//!
//! Standard pattern from clippy / ESLint / Pyre / Pylint / Ruff:
//! snapshot the current N issues to a JSON file checked into the
//! repo; subsequent runs only fail on issues NOT in the baseline.
//! Fixed issues drop off automatically when the user re-runs
//! `--write-baseline`.
//!
//! Match policy is `(rule, file, line, message_hash)` with line
//! drift tolerance of ±5 — a TODO comment moved 2 lines doesn't
//! unfreeze the suppression. The message_hash is blake3 of the
//! diagnostic message body so a typo-fix in a rule's message
//! doesn't invalidate every entry corpus-wide.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::lint::LintIssue;

/// Bumped when the on-disk format changes in a breaking way.
const BASELINE_FORMAT_VERSION: u32 = 1;

/// Line-drift tolerance: an issue still matches its baseline entry
/// if the line has shifted by ≤ DRIFT.
const DRIFT: i64 = 5;

/// One baseline entry — a snapshot of an issue's identity at the
/// moment the baseline was written. `message_hash` lets us
/// distinguish cases where two issues land on the same (rule, file,
/// line) with different messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaselineEntry {
    rule: String,
    file: PathBuf,
    line: usize,
    message_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BaselineFile {
    version: u32,
    generated_at: String,
    issues: Vec<BaselineEntry>,
}

/// Loaded baseline ready for matching against live issues.
pub struct Baseline {
    entries: Vec<BaselineEntry>,
}

impl Baseline {
    /// Load from `path`. Returns `None` for missing file (first
    /// run) or unparseable file (forward-compat: a future
    /// schema_version is treated as no-baseline rather than an
    /// error so we don't break old binaries on new repos).
    pub fn load(path: &Path) -> Option<Self> {
        let raw = std::fs::read(path).ok()?;
        let parsed: BaselineFile = serde_json::from_slice(&raw).ok()?;
        if parsed.version != BASELINE_FORMAT_VERSION {
            return None;
        }
        Some(Baseline {
            entries: parsed.issues,
        })
    }

    /// True when this issue is suppressed by the baseline. Match
    /// policy: rule + file + message_hash exact + line within ±DRIFT.
    pub fn suppresses(&self, issue: &LintIssue) -> bool {
        let hash = message_hash(&issue.message);
        for e in &self.entries {
            if e.rule == issue.rule
                && e.file == issue.file
                && e.message_hash == hash
                && (e.line as i64 - issue.line as i64).abs() <= DRIFT
            {
                return true;
            }
        }
        false
    }

    /// Persist the *current* set of issues to `path` as the new
    /// baseline. Issues that don't appear in this set drop off the
    /// baseline automatically — fixed bugs don't accumulate as
    /// dead suppressions.
    pub fn write(path: &Path, issues: &[LintIssue]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut entries: Vec<BaselineEntry> = issues
            .iter()
            .map(|i| BaselineEntry {
                rule: i.rule.to_string(),
                file: i.file.clone(),
                line: i.line,
                message_hash: message_hash(&i.message),
            })
            .collect();
        // Stable sort so two identical corpora produce
        // byte-identical baseline files (deterministic CI diffs).
        entries.sort_by(|a, b| {
            (a.file.to_string_lossy().to_string(), a.line, a.rule.clone(), a.message_hash.clone())
                .cmp(&(
                    b.file.to_string_lossy().to_string(),
                    b.line,
                    b.rule.clone(),
                    b.message_hash.clone(),
                ))
        });

        let file = BaselineFile {
            version: BASELINE_FORMAT_VERSION,
            generated_at: now_rfc3339(),
            issues: entries,
        };
        let serialised = serde_json::to_vec_pretty(&file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        // Atomic write: temp + rename.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &serialised)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Default location for the baseline file when `--baseline` is
/// passed without a path.
pub fn default_path() -> PathBuf {
    PathBuf::from(".verum").join("lint-baseline.json")
}

fn message_hash(message: &str) -> String {
    blake3::hash(message.as_bytes()).to_hex().to_string()
}

/// Best-effort RFC3339 timestamp. We don't depend on chrono — a
/// raw unix-time string is enough for the "when was this baseline
/// generated" debugging signal.
fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_issue(rule: &'static str, file: &str, line: usize, msg: &str) -> LintIssue {
        LintIssue {
            rule,
            level: super::super::lint::LintLevel::Warning,
            file: PathBuf::from(file),
            line,
            column: 1,
            message: msg.to_string(),
            suggestion: None,
            fixable: false,
        }
    }

    #[test]
    fn round_trip_write_load_suppresses_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baseline.json");
        let issues = vec![make_issue("todo-in-code", "src/a.vr", 10, "TODO comment in code")];
        Baseline::write(&path, &issues).expect("write");
        let baseline = Baseline::load(&path).expect("load");
        assert!(baseline.suppresses(&issues[0]));
    }

    #[test]
    fn line_drift_within_5_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baseline.json");
        let original = make_issue("todo-in-code", "src/a.vr", 10, "TODO comment");
        Baseline::write(&path, &[original.clone()]).expect("write");
        let baseline = Baseline::load(&path).expect("load");

        let drifted = make_issue("todo-in-code", "src/a.vr", 13, "TODO comment");
        assert!(baseline.suppresses(&drifted), "±3 drift should be tolerated");

        let too_far = make_issue("todo-in-code", "src/a.vr", 20, "TODO comment");
        assert!(!baseline.suppresses(&too_far), "±10 drift should NOT match");
    }

    #[test]
    fn different_message_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baseline.json");
        let original = make_issue("todo-in-code", "src/a.vr", 10, "TODO: foo");
        Baseline::write(&path, &[original.clone()]).expect("write");
        let baseline = Baseline::load(&path).expect("load");

        // Same rule + file + line but different message → not the
        // same issue. Author edited the comment; baseline is now
        // stale for this entry.
        let edited = make_issue("todo-in-code", "src/a.vr", 10, "TODO: bar");
        assert!(!baseline.suppresses(&edited));
    }

    #[test]
    fn missing_file_returns_none_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert!(Baseline::load(&path).is_none());
    }

    #[test]
    fn deterministic_ordering() {
        // Two identical issue sets must produce byte-identical
        // baselines so commits don't churn on cosmetic re-orderings.
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let p1 = dir1.path().join("a.json");
        let p2 = dir2.path().join("a.json");
        let issues = vec![
            make_issue("z-rule", "src/b.vr", 5, "msg"),
            make_issue("a-rule", "src/a.vr", 10, "msg"),
            make_issue("a-rule", "src/a.vr", 5, "msg"),
        ];
        Baseline::write(&p1, &issues).expect("w1");
        // Reverse order on second write — both must produce the same file.
        let mut reversed = issues.clone();
        reversed.reverse();
        Baseline::write(&p2, &reversed).expect("w2");
        let bytes1 = std::fs::read(&p1).unwrap();
        let bytes2 = std::fs::read(&p2).unwrap();
        // Generated_at differs (could be a few seconds apart) so
        // ignore that line. Compare everything else.
        let s1: String = String::from_utf8(bytes1).unwrap();
        let s2: String = String::from_utf8(bytes2).unwrap();
        let strip = |s: String| -> String {
            s.lines()
                .filter(|l| !l.trim_start().starts_with("\"generated_at\""))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip(s1), strip(s2));
    }

    #[test]
    fn empty_message_hash_stable() {
        // Just confirms blake3 is deterministic.
        assert_eq!(message_hash("hello"), message_hash("hello"));
        assert_ne!(message_hash("hello"), message_hash("world"));
    }
}
