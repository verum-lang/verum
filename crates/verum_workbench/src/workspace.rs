//! Workspace paths and compare-and-swap edits shared by UI and agent tools.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use verum_common::Text;

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
        ensure!(
            Path::new(relative).extension().is_some_and(|e| e == "vr"),
            "Select a .vr source file"
        );
        let path = self.document_path(relative)?;
        ensure!(
            path.parent().is_some_and(Path::is_dir),
            "Parent directory does not exist"
        );
        Ok(path)
    }
    /// Shared path boundary for project text, directory browsing and Git views.
    pub fn document_path(&self, relative: &str) -> Result<PathBuf> {
        ensure!(!relative.is_empty(), "Expected a workspace-relative path");
        let relative = Path::new(relative);
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
        Ok(path)
    }
    pub(super) fn source_input(&self, relative: &str, draft: Option<Text>) -> Result<(PathBuf, Text)> {
        let path = self.path(relative)?;
        let source = match draft {
            Some(source) => source,
            None => Self::read_text(&path)?,
        };
        Ok((path, source))
    }
    pub fn read(&self, relative: &str) -> Result<Value> {
        Self::read_at(relative, &self.path(relative)?)
    }
    pub fn read_document(&self, relative: &str) -> Result<Value> {
        Self::read_at(relative, &self.document_path(relative)?)
    }
    fn read_at(relative: &str, path: &Path) -> Result<Value> {
        let source = Self::read_text(path)?;
        Ok(json!({"path":relative, "hash":digest(source.as_bytes()), "source":source}))
    }
    fn read_text(path: &Path) -> Result<Text> {
        ensure!(path.metadata()?.is_file(), "Select a regular text file");
        ensure!(
            path.metadata()?.len() <= SOURCE_LIMIT as u64,
            "Source exceeds editor limit"
        );
        // The standard I/O boundary requires a Vec; retain Text in the model.
        let mut bytes = vec![];
        std::fs::File::open(path)?
            .take(SOURCE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)?;
        let source = Text::from_utf8(bytes)?;
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds editor limit");
        ensure!(
            !source.contains("\0"),
            "Binary files cannot be edited as text"
        );
        Ok(source)
    }
    pub fn write(&self, relative: &str, source: &str, expected: Option<&str>) -> Result<Value> {
        self.path(relative)?;
        self.write_document(relative, source, expected)
    }
    pub fn write_document(
        &self,
        relative: &str,
        source: &str,
        expected: Option<&str>,
    ) -> Result<Value> {
        ensure!(source.len() <= SOURCE_LIMIT, "Source exceeds editor limit");
        ensure!(
            !source.contains('\0'),
            "Binary files cannot be edited as text"
        );
        let path = self.document_path(relative)?;
        ensure!(
            path.parent().is_some_and(Path::is_dir),
            "Parent directory does not exist"
        );
        if path.exists() {
            ensure!(path.is_file(), "Select a regular text file");
        }
        let actual = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = vec![];
                file.take(SOURCE_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= SOURCE_LIMIT, "Source exceeds editor limit");
                Some(digest(&bytes))
            }
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
        self.source_page(None, Self::MAX_SOURCE_PAGE)
    }

    pub(crate) const MAX_SOURCE_PAGE: usize = 2000;

    /// Names are workspace-relative UTF-8. A trailing slash on a directory
    /// gives its descendants their correct position among sibling file names.
    pub fn source_page(&self, after: Option<&str>, limit: usize) -> Result<Value> {
        ensure!(
            (1..=Self::MAX_SOURCE_PAGE).contains(&limit),
            "Source page limit must be 1..2000"
        );
        if let Some(cursor) = after {
            ensure!(
                crate::project_files::exposed(cursor)
                    && cursor
                        .split('/')
                        .all(|part| !part.is_empty() && part != ".")
                    && Path::new(cursor).extension().is_some_and(|ext| ext == "vr"),
                "Source cursor must be a workspace-relative .vr path"
            );
        }
        let disappeared = |e: &std::io::Error| {
            matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory)
        };
        let mut pending = vec![String::new()];
        let mut files = Vec::with_capacity(limit);
        let mut has_more = false;
        while let Some(key) = pending.pop() {
            let directory = key.is_empty() || key.ends_with('/');
            if let Some(cursor) = after
                && key.as_str() <= cursor
                && !(directory && cursor.starts_with(&key))
            {
                continue;
            }
            if !directory {
                if files.len() == limit {
                    has_more = true;
                    break;
                }
                files.push(key);
                continue;
            }
            let mut children = Vec::new();
            // Strip the sort-key suffix so symlink_metadata inspects the
            // directory entry itself on Unix.
            let path = self.root.join(key.trim_end_matches('/'));
            // A queued directory can be removed or replaced between reads.
            // Check its current entry before opening it. Other I/O failures
            // carry the directory name instead of returning an incomplete page.
            let label = if key.is_empty() { "." } else { key.as_str() };
            if !key.is_empty() {
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_dir() => {}
                    Ok(_) => continue,
                    Err(e) if disappeared(&e) => continue,
                    Err(e) => anyhow::bail!("Inspect source directory {label}: {e}"),
                }
            }
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(e) if !key.is_empty() && disappeared(&e) => continue,
                Err(e) => anyhow::bail!("Read source directory {label}: {e}"),
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) if disappeared(&e) => continue,
                    Err(e) => anyhow::bail!("Read entry in source directory {label}: {e}"),
                };
                let name = entry.file_name();
                if name.as_encoded_bytes().starts_with(b".") {
                    continue;
                }
                let ty = match entry.file_type() {
                    Ok(ty) => ty,
                    Err(e) if disappeared(&e) => continue,
                    Err(e) => {
                        anyhow::bail!("Inspect entry {name:?} in source directory {label}: {e}")
                    }
                };
                if !ty.is_dir()
                    && !(ty.is_file()
                        && Path::new(&name).extension().is_some_and(|ext| ext == "vr"))
                {
                    continue;
                }
                let name = name
                    .to_str()
                    .with_context(|| format!("Source path {name:?} in {label} must be UTF-8"))?;
                if !crate::project_files::exposed(name) {
                    continue;
                }
                if ty.is_dir() {
                    children.push(format!("{key}{name}/"));
                } else {
                    children.push(format!("{key}{name}"));
                }
            }
            children.sort_unstable_by(|a, b| b.cmp(a));
            pending.extend(children);
        }
        let next = if has_more {
            files.last().cloned()
        } else {
            None
        };
        let files: Vec<_> = files.into_iter().map(|path| json!({"path":path})).collect();
        Ok(json!({"root":self.root,"files":files,"next":next}))
    }
}
