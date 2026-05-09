//! Golden-file snapshot testing support (#61).
//!
//! When a test carries `@snapshot: <name>`, the runner compares captured stdout
//! against the content of `snapshots/<name>.snap` (relative to the spec file).
//! If `--update-snapshots` is passed, the `.snap` file is written/overwritten
//! with the actual output instead of failing.
//!
//! # Snapshot file format
//!
//! Plain UTF-8 text.  Leading/trailing whitespace is normalised before comparison
//! so that editors that add a trailing newline do not cause spurious failures.
//!
//! # Workflow
//!
//! 1. Write your test with `@snapshot: my_output`.
//! 2. Run once with `verum test --update-snapshots` — the `.snap` file is created.
//! 3. Commit the `.snap` file alongside the spec.
//! 4. CI runs without `--update-snapshots`; failures mean the output changed.

use std::path::{Path, PathBuf};

/// Result of a snapshot comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotResult {
    /// Actual output matched the stored snapshot.
    Match,
    /// No stored snapshot; one was created (only when `update` is true).
    Created,
    /// Stored snapshot was updated with new actual output.
    Updated,
    /// Actual output differed from the stored snapshot.
    Mismatch {
        /// Content of the stored snapshot file.
        expected: String,
        /// Actual captured output.
        actual: String,
    },
    /// Snapshot file does not exist and `update` is false.
    Missing { path: PathBuf },
}

/// Resolve the path of a snapshot file.
///
/// `spec_path` is the `.vr` file location; `name` is the `@snapshot:` value.
/// Snapshots live in a `snapshots/` directory next to the spec file.
pub fn snapshot_path(spec_path: &Path, name: &str) -> PathBuf {
    let dir = spec_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("snapshots");
    dir.join(format!("{name}.snap"))
}

/// Compare `actual` against the stored snapshot identified by `name`.
///
/// If `update` is `true`, the snapshot is written/overwritten with `actual`
/// rather than performing a comparison.
pub fn compare_or_update(
    spec_path: &Path,
    name: &str,
    actual: &str,
    update: bool,
) -> std::io::Result<SnapshotResult> {
    let path = snapshot_path(spec_path, name);
    let normalised_actual = actual.trim().to_string();

    if update {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        std::fs::write(&path, &normalised_actual)?;
        return Ok(if existed {
            SnapshotResult::Updated
        } else {
            SnapshotResult::Created
        });
    }

    if !path.exists() {
        return Ok(SnapshotResult::Missing { path });
    }

    let stored = std::fs::read_to_string(&path)?;
    let normalised_stored = stored.trim().to_string();

    if normalised_actual == normalised_stored {
        Ok(SnapshotResult::Match)
    } else {
        Ok(SnapshotResult::Mismatch {
            expected: normalised_stored,
            actual: normalised_actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn snapshot_path_is_next_to_spec() {
        let p = snapshot_path(Path::new("/a/b/spec.vr"), "my_output");
        assert_eq!(p, PathBuf::from("/a/b/snapshots/my_output.snap"));
    }

    #[test]
    fn snapshot_path_no_parent() {
        let p = snapshot_path(Path::new("spec.vr"), "x");
        assert_eq!(p, PathBuf::from("snapshots/x.snap"));
    }

    #[test]
    fn compare_match() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("t.vr");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).unwrap();
        std::fs::write(snap_dir.join("hello.snap"), "hello world").unwrap();

        let result = compare_or_update(&spec, "hello", "hello world\n", false).unwrap();
        assert_eq!(result, SnapshotResult::Match);
    }

    #[test]
    fn compare_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("t.vr");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).unwrap();
        std::fs::write(snap_dir.join("x.snap"), "old output").unwrap();

        let result = compare_or_update(&spec, "x", "new output", false).unwrap();
        assert!(matches!(result, SnapshotResult::Mismatch { .. }));
        if let SnapshotResult::Mismatch { expected, actual } = result {
            assert_eq!(expected, "old output");
            assert_eq!(actual, "new output");
        }
    }

    #[test]
    fn update_creates_snap_file() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("t.vr");

        let result = compare_or_update(&spec, "new_snap", "output text\n", true).unwrap();
        assert_eq!(result, SnapshotResult::Created);

        let content = std::fs::read_to_string(dir.path().join("snapshots/new_snap.snap")).unwrap();
        assert_eq!(content.trim(), "output text");
    }

    #[test]
    fn update_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("t.vr");
        let snap_dir = dir.path().join("snapshots");
        std::fs::create_dir_all(&snap_dir).unwrap();
        std::fs::write(snap_dir.join("s.snap"), "old").unwrap();

        let result = compare_or_update(&spec, "s", "new\n", true).unwrap();
        assert_eq!(result, SnapshotResult::Updated);
    }

    #[test]
    fn missing_snap_returns_missing() {
        let dir = tempfile::tempdir().unwrap();
        let spec = dir.path().join("t.vr");
        let result = compare_or_update(&spec, "absent", "anything", false).unwrap();
        assert!(matches!(result, SnapshotResult::Missing { .. }));
    }
}
