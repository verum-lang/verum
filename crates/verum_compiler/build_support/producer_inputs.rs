//! Portable declared inputs of a Cargo producer, without running Cargo inside
//! its build script. Local normal/build dependencies include all target and
//! optional-feature branches; registry/git edges come from the resolved lock.
//!
//! Every local package file is an input, including build scripts and resources.
//! Additional workspace-relative resources may be declared in a package's
//! `metadata.verum.producer-inputs` array (relative to that package). This is a
//! source/configuration identity, not a claim about external tool binaries,
//! environment overrides or hermetic execution of arbitrary build scripts.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};
use toml::Value;
use verum_common::{List, OrderedMap, OrderedSet, Text};

#[derive(Default)]
pub(super) struct Inputs {
    pub files: OrderedSet<PathBuf>,
    pub directories: OrderedSet<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PackageId {
    name: Text,
    version: Text,
    source: Option<Text>,
}

#[derive(Clone)]
struct LocalPackage {
    manifest: PathBuf,
    value: Value,
    workspace: Value,
    workspace_root: PathBuf,
}

struct LockedPackage {
    dependencies: List<Text>,
}

fn invalid(message: impl Into<Text>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into().to_string())
}

fn string<'a>(value: &'a Value, key: &str) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("expected string `{key}` in Cargo input")))
}

/// Normalize declared paths lexically. A dependency outside the frozen checkout
/// has no portable source identity here and must not silently retain a cache hit.
fn relative(base: &Path, path: &str) -> io::Result<PathBuf> {
    if Path::new(path).is_absolute() {
        return Err(invalid(format!(
            "absolute producer input is not portable: {path}"
        )));
    }
    let mut normalized = base.to_owned();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => return Err(invalid(format!("producer input escapes checkout: {path}"))),
        }
    }
    Ok(normalized)
}

struct Graph<'a> {
    root: &'a Path,
    workspace: Value,
    locked: OrderedMap<PackageId, LockedPackage>,
    by_name: OrderedMap<Text, List<PackageId>>,
    manifests: OrderedMap<PackageId, LocalPackage>,
    pending: List<PackageId>,
    visited: OrderedSet<PackageId>,
    inputs: Inputs,
}

impl Graph<'_> {
    fn file_kind(&self, path: &Path) -> io::Result<fs::FileType> {
        // A manifest may be reached directly through a dependency rather than
        // its parent's directory walk. Check each component in that case too.
        let mut current = self.root.to_owned();
        let mut kind = None;
        for component in path.components() {
            let Component::Normal(part) = component else {
                return Err(invalid(format!(
                    "unnormalized producer input: {}",
                    path.display()
                )));
            };
            current.push(part);
            let next = fs::symlink_metadata(&current)?.file_type();
            if next.is_symlink() {
                return Err(invalid(format!(
                    "symlinked producer input: {}",
                    current.display()
                )));
            }
            kind = Some(next);
        }
        kind.ok_or_else(|| invalid("empty producer input path"))
    }

    fn document(&mut self, path: &Path) -> io::Result<Value> {
        if !self.file_kind(path)?.is_file() {
            return Err(invalid(format!(
                "expected Cargo input file: {}",
                path.display()
            )));
        }
        self.inputs.files.insert(path.to_owned());
        toml::from_str(&fs::read_to_string(self.root.join(path))?)
            .map_err(|error| invalid(format!("invalid {}: {error}", path.display())))
    }

    fn input_tree(&mut self, path: &Path) -> io::Result<()> {
        let kind = self.file_kind(path)?;
        if kind.is_file() {
            self.inputs.files.insert(path.to_owned());
            return Ok(());
        }
        if !kind.is_dir() {
            return Err(invalid(format!(
                "unsupported producer input: {}",
                path.display()
            )));
        }
        let mut pending = List::from_iter([path.to_owned()]);
        while let Some(directory) = pending.pop() {
            if !self.inputs.directories.insert(directory.clone()) {
                continue;
            }
            for entry in fs::read_dir(self.root.join(&directory))? {
                let entry = entry?;
                let child = directory.join(entry.file_name());
                let kind = entry.file_type()?;
                // Cargo's workspace output and VCS metadata are not source
                // inputs. Other files, including non-Rust resources, are kept.
                if entry.file_name() == ".git" || child == Path::new("target") {
                    continue;
                }
                if kind.is_dir() {
                    pending.push(child);
                } else if kind.is_file() {
                    self.inputs.files.insert(child);
                } else {
                    return Err(invalid(format!(
                        "unsupported producer input: {}",
                        child.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn lock_id(&self, reference: &str) -> io::Result<PackageId> {
        let mut parts = reference.split_whitespace();
        let name = parts
            .next()
            .ok_or_else(|| invalid("empty Cargo.lock edge"))?;
        let version = parts.next();
        let source = parts.next();
        if parts.next().is_some() {
            return Err(invalid(format!("invalid Cargo.lock edge: {reference}")));
        }
        let source = match source {
            None => None,
            Some(value) => Some(
                value
                    .strip_prefix('(')
                    .and_then(|s| s.strip_suffix(')'))
                    .ok_or_else(|| invalid(format!("invalid lock source: {reference}")))?,
            ),
        };
        let candidates: List<_> = self
            .by_name
            .get(&Text::from(name))
            .into_iter()
            .flatten()
            .filter(|id| {
                version.is_none_or(|version| id.version == version)
                    && source.is_none_or(|source| id.source.as_deref() == Some(source))
            })
            .collect();
        match candidates.as_slice() {
            [id] => Ok((*id).clone()),
            _ => Err(invalid(format!(
                "unresolved or ambiguous Cargo.lock edge: {reference}"
            ))),
        }
    }

    fn workspace_for(&mut self, manifest: &Path, value: &Value) -> io::Result<(PathBuf, Value)> {
        let directory = manifest
            .parent()
            .ok_or_else(|| invalid("manifest has no directory"))?;
        if let Some(path) = value
            .get("package")
            .and_then(|package| package.get("workspace"))
        {
            let path = relative(
                directory,
                path.as_str()
                    .ok_or_else(|| invalid("invalid package workspace path"))?,
            )?;
            let workspace = self.document(&path.join("Cargo.toml"))?;
            if workspace.get("workspace").is_none() {
                return Err(invalid("declared workspace has no workspace table"));
            }
            return Ok((path, workspace));
        }
        let mut ancestor = Some(directory);
        while let Some(directory) = ancestor {
            let path = directory.join("Cargo.toml");
            let candidate = if path == manifest {
                Some(value.clone())
            } else if path == Path::new("Cargo.toml") {
                Some(self.workspace.clone())
            } else {
                match fs::symlink_metadata(self.root.join(&path)) {
                    Ok(_) => Some(self.document(&path)?),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error),
                }
            };
            if let Some(workspace) = candidate
                && workspace.get("workspace").is_some()
            {
                return Ok((directory.to_owned(), workspace));
            }
            ancestor = directory.parent();
        }
        Err(invalid(format!(
            "no declared workspace for {}",
            manifest.display()
        )))
    }

    fn local_identity(value: &Value, workspace: &Value) -> io::Result<PackageId> {
        let package = value
            .get("package")
            .ok_or_else(|| invalid("local manifest has no package"))?;
        let name = Text::from(string(package, "name")?);
        let version = match package.get("version") {
            Some(Value::String(version)) => Text::from(version.as_str()),
            Some(value) if value.get("workspace").and_then(Value::as_bool) == Some(true) => {
                workspace
                    .get("workspace")
                    .and_then(|value| value.get("package"))
                    .and_then(|value| value.get("version"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("missing inherited workspace package version"))?
                    .into()
            }
            _ => return Err(invalid("missing local package version")),
        };
        Ok(PackageId {
            name,
            version,
            source: None,
        })
    }

    fn register_local(
        &mut self,
        manifest: PathBuf,
        expected: Option<&PackageId>,
    ) -> io::Result<PackageId> {
        let value = self.document(&manifest)?;
        let (workspace_root, workspace) = self.workspace_for(&manifest, &value)?;
        let id = Self::local_identity(&value, &workspace)?;
        if !self.locked.contains_key(&id) || expected.is_some_and(|expected| expected != &id) {
            return Err(invalid(format!(
                "{} does not match its locked package: {id:?}",
                manifest.display()
            )));
        }
        if let Some(previous) = self.manifests.get(&id) {
            if previous.manifest != manifest {
                return Err(invalid(format!("multiple local sources for {id:?}")));
            }
        } else {
            self.manifests.insert(
                id.clone(),
                LocalPackage {
                    manifest,
                    value,
                    workspace,
                    workspace_root,
                },
            );
            self.pending.push(id.clone());
        }
        Ok(id)
    }

    fn register_patch(&mut self, id: &PackageId) -> io::Result<()> {
        let mut paths = OrderedSet::new();
        for section in ["patch", "replace"] {
            let Some(table) = self.workspace.get(section) else {
                continue;
            };
            let table = table
                .as_table()
                .ok_or_else(|| invalid(format!("invalid [{section}] table")))?;
            for (key, value) in table {
                let entries: List<_> = if section == "patch" {
                    value
                        .as_table()
                        .ok_or_else(|| invalid("invalid patch source table"))?
                        .iter()
                        .collect()
                } else {
                    List::from_iter([(key, value)])
                };
                for (alias, dependency) in entries {
                    let name = dependency
                        .get("package")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| alias.split(':').next().unwrap_or(alias));
                    if name == id.name
                        && let Some(path) = dependency.get("path").and_then(Value::as_str)
                    {
                        paths.insert(relative(Path::new(""), path)?.join("Cargo.toml"));
                    }
                }
            }
        }
        let mut matching = List::new();
        for path in paths {
            let value = self.document(&path)?;
            let (_, workspace) = self.workspace_for(&path, &value)?;
            if &Self::local_identity(&value, &workspace)? == id {
                matching.push(path);
            }
        }
        match matching.as_slice() {
            [path] => {
                self.register_local(path.clone(), Some(id))?;
                Ok(())
            }
            _ => Err(invalid(format!(
                "locked local package lacks one declared path source: {id:?}"
            ))),
        }
    }

    fn local_dependencies(&mut self, id: &PackageId, package: &LocalPackage) -> io::Result<()> {
        let value = &package.value;
        let directory = package
            .manifest
            .parent()
            .ok_or_else(|| invalid("manifest has no directory"))?;
        self.input_tree(directory)?;
        if let Some(resources) = value
            .get("package")
            .and_then(|p| p.get("metadata"))
            .and_then(|m| m.get("verum"))
            .and_then(|v| v.get("producer-inputs"))
        {
            for resource in resources
                .as_array()
                .ok_or_else(|| invalid("producer-inputs must be an array"))?
            {
                let resource = resource
                    .as_str()
                    .ok_or_else(|| invalid("producer input path must be a string"))?;
                self.input_tree(&relative(directory, resource)?)?;
            }
        }
        let mut scopes = List::from_iter([value]);
        if let Some(targets) = value.get("target") {
            scopes.extend(
                targets
                    .as_table()
                    .ok_or_else(|| invalid("invalid target table"))?
                    .values(),
            );
        }
        let locked = self
            .locked
            .get(id)
            .expect("registered package")
            .dependencies
            .iter()
            .map(|edge| self.lock_id(edge))
            .collect::<io::Result<List<_>>>()?;
        for scope in scopes {
            for kind in ["dependencies", "build-dependencies"] {
                let Some(dependencies) = scope.get(kind) else {
                    continue;
                };
                for (alias, declared) in dependencies
                    .as_table()
                    .ok_or_else(|| invalid("invalid dependency table"))?
                {
                    let inherited =
                        declared.get("workspace").and_then(Value::as_bool) == Some(true);
                    let dependency = if inherited {
                        package
                            .workspace
                            .get("workspace")
                            .and_then(|v| v.get("dependencies"))
                            .and_then(|v| v.get(alias))
                            .ok_or_else(|| {
                                invalid(format!("unknown workspace dependency: {alias}"))
                            })?
                            .clone()
                    } else {
                        declared.clone()
                    };
                    if !dependency.is_str() && !dependency.is_table() {
                        return Err(invalid(format!("invalid dependency: {alias}")));
                    }
                    let name = dependency
                        .get("package")
                        .and_then(Value::as_str)
                        .unwrap_or(alias);
                    let edges: List<_> = locked
                        .iter()
                        .filter(|edge| edge.name == name)
                        .cloned()
                        .collect();
                    if edges.is_empty() {
                        return Err(invalid(format!(
                            "dependency {name} of {} missing from Cargo.lock",
                            id.name
                        )));
                    }
                    if let Some(path) = dependency.get("path") {
                        let path = path
                            .as_str()
                            .ok_or_else(|| invalid("dependency path must be a string"))?;
                        let base = if inherited {
                            package.workspace_root.as_path()
                        } else {
                            directory
                        };
                        let local =
                            self.register_local(relative(base, path)?.join("Cargo.toml"), None)?;
                        if local.name != name || !edges.contains(&local) {
                            return Err(invalid(format!(
                                "path dependency {alias} does not match its lock edge"
                            )));
                        }
                    } else {
                        self.pending.extend(edges);
                    }
                }
            }
        }
        Ok(())
    }
}

pub(super) fn collect(root: &Path, producer: &Path) -> io::Result<Inputs> {
    let mut graph = Graph {
        root,
        workspace: Value::Table(Default::default()),
        locked: OrderedMap::new(),
        by_name: OrderedMap::new(),
        manifests: OrderedMap::new(),
        pending: List::new(),
        visited: OrderedSet::new(),
        inputs: Inputs::default(),
    };
    graph.workspace = graph.document(Path::new("Cargo.toml"))?;
    let lock = graph.document(Path::new("Cargo.lock"))?;
    if !matches!(lock.get("version").and_then(Value::as_integer), Some(3 | 4)) {
        return Err(invalid("unsupported Cargo.lock format version"));
    }
    for package in lock
        .get("package")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Cargo.lock has no package table"))?
    {
        let source = package
            .get("source")
            .map(|v| {
                v.as_str()
                    .map(Text::from)
                    .ok_or_else(|| invalid("invalid locked source"))
            })
            .transpose()?;
        if let Some(source) = &source {
            if source.starts_with("registry+") {
                let checksum = string(package, "checksum")?;
                if checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid("invalid registry source checksum"));
                }
            } else if !source.starts_with("git+")
                || !source
                    .as_str()
                    .rsplit_once('#')
                    .is_some_and(|(_, revision)| !revision.is_empty())
            {
                return Err(invalid(format!("unsupported locked source: {source}")));
            }
        }
        let id = PackageId {
            name: string(package, "name")?.into(),
            version: string(package, "version")?.into(),
            source,
        };
        let dependencies = match package.get("dependencies") {
            Some(value) => value
                .as_array()
                .ok_or_else(|| invalid("invalid locked dependencies"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(Text::from)
                        .ok_or_else(|| invalid("invalid locked dependency edge"))
                })
                .collect::<io::Result<List<_>>>()?,
            None => List::new(),
        };
        graph
            .by_name
            .entry(id.name.clone())
            .or_default()
            .push(id.clone());
        if graph
            .locked
            .insert(id.clone(), LockedPackage { dependencies })
            .is_some()
        {
            return Err(invalid("duplicate Cargo.lock package identity"));
        }
    }
    graph.document(Path::new("rust-toolchain.toml"))?;
    graph.input_tree(Path::new(".cargo"))?;
    for filename in [".cargo/config", ".cargo/config.toml"] {
        if root.join(filename).exists() {
            let configuration = graph.document(Path::new(filename))?;
            // These can replace a locked source with arbitrary mutable files.
            // Use declared path dependencies/patches so the graph can own them.
            for key in ["paths", "patch", "source"] {
                if configuration.get(key).is_some() {
                    return Err(invalid(format!(
                        "{filename}: `{key}` source overrides need a declared portable path dependency"
                    )));
                }
            }
        }
    }
    graph.register_local(producer.to_owned(), None)?;
    while let Some(id) = graph.pending.pop() {
        if !graph.visited.insert(id.clone()) {
            continue;
        }
        if id.source.is_none() {
            if !graph.manifests.contains_key(&id) {
                graph.register_patch(&id)?;
            }
            let package = graph
                .manifests
                .get(&id)
                .expect("registered local package")
                .clone();
            graph.local_dependencies(&id, &package)?;
        } else {
            for edge in &graph
                .locked
                .get(&id)
                .expect("resolved lock edge")
                .dependencies
            {
                graph.pending.push(graph.lock_id(edge)?);
            }
        }
    }
    Ok(graph.inputs)
}
