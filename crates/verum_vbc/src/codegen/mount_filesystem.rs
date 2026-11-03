//! Filesystem identity for one source-mount resolution.
//!
//! Verum module paths are case-sensitive even on case-insensitive hosts. A
//! member named `Item` must not select `item.vr` before its facade's `mod.vr`.
//! Cache directory entries, canonical paths and glob walks for this resolution
//! only: subsequent compilations must see files created, removed or renamed by
//! an editor. Repeated mounts still pay for each filesystem lookup once.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub(super) struct MountFilesystem {
    canonical: HashMap<PathBuf, Option<String>>,
    directories: HashMap<PathBuf, Option<HashSet<OsString>>>,
    walks: HashMap<PathBuf, Arc<Vec<String>>>,
    memoize: bool,
}

impl Default for MountFilesystem {
    fn default() -> Self {
        Self {
            canonical: HashMap::new(),
            directories: HashMap::new(),
            walks: HashMap::new(),
            memoize: std::env::var_os("VERUM_NO_MOUNT_FS_MEMO").is_none(),
        }
    }
}

impl MountFilesystem {
    pub(super) fn canonicalize(&mut self, path: impl AsRef<Path>) -> Option<String> {
        let path = path.as_ref();
        if self.memoize
            && let Some(found) = self.canonical.get(path)
        {
            return found.clone();
        }
        let found = self.canonicalize_exact(path);
        if self.memoize {
            self.canonical.insert(path.to_owned(), found.clone());
        }
        found
    }

    fn canonicalize_exact(&mut self, path: &Path) -> Option<String> {
        let mut parent = if path.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().ok()?
        };
        for part in path.components() {
            if let Component::Normal(name) = part {
                let entries = if self.memoize {
                    self.directories
                        .entry(parent.clone())
                        .or_insert_with(|| read_names(&parent))
                } else {
                    &mut read_names(&parent)
                };
                if !entries.as_ref()?.contains(name) {
                    return None;
                }
            }
            // Keep `..` until canonicalization: popping lexically would change
            // its meaning after a symlink. Root and Windows prefixes are kept.
            parent.push(part.as_os_str());
        }
        std::fs::canonicalize(parent)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    }

    pub(super) fn walk_vr_files(&mut self, root: &Path) -> Arc<Vec<String>> {
        if self.memoize
            && let Some(found) = self.walks.get(root)
        {
            return Arc::clone(found);
        }
        let found = Arc::new(walk_vr_files(root));
        if self.memoize {
            self.walks.insert(root.to_owned(), Arc::clone(&found));
        }
        found
    }
}

fn read_names(path: &Path) -> Option<HashSet<OsString>> {
    std::fs::read_dir(path)
        .ok()?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .ok()
}

fn walk_vr_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("vr")
            {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_candidates_have_exact_case_on_every_host() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("model")).unwrap();
        let exact = root.path().join("model/item.vr");
        std::fs::write(&exact, "module model.item;").unwrap();
        for memoize in [true, false] {
            let mut fs = MountFilesystem {
                memoize,
                ..Default::default()
            };
            assert!(fs.canonicalize(&exact).is_some());
            assert!(fs.canonicalize(root.path().join("model/Item.vr")).is_none());
            assert!(fs.canonicalize(root.path().join("Model/item.vr")).is_none());
            assert!(
                fs.canonicalize(root.path().join("model/../model/item.vr"))
                    .is_some()
            );
        }
    }

    #[test]
    fn next_resolution_observes_created_and_removed_sources() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("new.vr");
        let mut first = MountFilesystem::default();
        assert!(first.canonicalize(&source).is_none());
        assert!(first.walk_vr_files(root.path()).is_empty());
        std::fs::write(&source, "module new;").unwrap();
        let mut second = MountFilesystem::default();
        assert!(second.canonicalize(&source).is_some());
        assert_eq!(second.walk_vr_files(root.path()).len(), 1);
        std::fs::remove_file(&source).unwrap();
        let mut third = MountFilesystem::default();
        assert!(third.canonicalize(&source).is_none());
        assert!(third.walk_vr_files(root.path()).is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn parent_segments_follow_symlink_identity() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("target/nested")).unwrap();
        std::os::unix::fs::symlink(root.path().join("target/nested"), root.path().join("link"))
            .unwrap();
        let target = root.path().join("target/item.vr");
        std::fs::write(&target, "module item;").unwrap();
        let mut fs = MountFilesystem::default();
        assert_eq!(
            fs.canonicalize(root.path().join("link/../item.vr")),
            fs.canonicalize(target)
        );
        assert!(
            fs.canonicalize(root.path().join("LINK/../item.vr"))
                .is_none()
        );
    }
}
