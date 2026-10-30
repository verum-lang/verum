//! Workspace paths and compare-and-swap edits shared by UI and agent tools.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};
use verum_common::{List, Text};

pub const SOURCE_LIMIT: usize = 512 * 1024;
pub fn digest(bytes: &[u8]) -> Text {
    hex::encode(Sha256::digest(bytes)).into()
}

pub fn private_dir(root: &Path, components: &[&str]) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for name in components {
        path.push(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "Studio storage must be a regular directory"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&path)?;
            }
            Err(e) => return Err(e.into()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(path)
}

pub struct Workspace {
    pub root: PathBuf,
}
impl Workspace {
    pub fn open(root: &Path) -> Result<Self> {
        let root = root.canonicalize().context("Open workspace directory")?;
        ensure!(root.is_dir(), "Workspace must be a directory");
        Ok(Self { root })
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        let relative = Path::new(relative);
        ensure!(
            relative.extension().is_some_and(|e| e == "vr"),
            "Select a .vr source file"
        );
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "Path must stay inside the workspace"
        );
        let mut path = self.root.clone();
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy();
            ensure!(
                !name.starts_with('.'),
                "Hidden workspace paths are not exposed"
            );
            path.push(component);
            match std::fs::symlink_metadata(&path) {
                Ok(meta) => ensure!(
                    !meta.file_type().is_symlink(),
                    "Symlink paths are not exposed"
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(
            path.parent().is_some_and(Path::is_dir),
            "Parent directory does not exist"
        );
        Ok(path)
    }
    pub fn read(&self, relative: &str) -> Result<Value> {
        let path = self.path(relative)?;
        ensure!(
            path.metadata()?.len() <= SOURCE_LIMIT as u64,
            "Source exceeds editor limit"
        );
        let source = std::fs::read_to_string(path)?;
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds editor limit");
        Ok(json!({"path":relative, "hash":digest(source.as_bytes()), "source":source}))
    }
    pub fn write(&self, relative: &str, source: &str, expected: Option<&str>) -> Result<Value> {
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds editor limit");
        let path = self.path(relative)?;
        let actual = match std::fs::read(&path) {
            Ok(bytes) => Some(digest(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        ensure!(
            actual.as_deref() == expected,
            "File changed on disk; reload and review before saving"
        );
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("Missing parent")?)?;
        temp.write_all(source.as_bytes())?;
        if path.exists() {
            temp.as_file()
                .set_permissions(path.metadata()?.permissions())?;
        }
        temp.as_file().sync_all()?;
        // Serializes workbench edits; external writers are detected on the next read/save.
        if actual.is_none() {
            temp.persist_noclobber(&path)?;
        } else {
            temp.persist(&path)?;
        }
        Ok(json!({"path":relative,"source":source,"hash":digest(source.as_bytes())}))
    }
    pub fn list(&self) -> Result<Value> {
        let mut files = List::new();
        self.walk(&self.root, &mut files, 0)?;
        files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        Ok(json!({"root":self.root, "files":files}))
    }
    fn walk(&self, directory: &Path, files: &mut List<Value>, depth: usize) -> Result<()> {
        if depth > 16 {
            return Ok(());
        }
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules" | "llvm")
            {
                continue;
            }
            let ty = entry.file_type()?;
            if ty.is_symlink() {
                continue;
            }
            if ty.is_dir() {
                self.walk(&entry.path(), files, depth + 1)?;
            } else if entry.path().extension().is_some_and(|e| e == "vr") {
                if files.len() == 2000 {
                    bail!("Workspace exceeds 2000 files; open a smaller project");
                }
                let path: Text = entry
                    .path()
                    .strip_prefix(&self.root)?
                    .to_string_lossy()
                    .replace('\\', "/")
                    .into();
                files.push(json!({"path":path}));
            }
        }
        Ok(())
    }
}
