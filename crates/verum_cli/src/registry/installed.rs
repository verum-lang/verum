//! Bind an ordinary project's registry dependencies to its installed, locked source.
use super::cache_manager::SourceSnapshot;
use super::{CacheManager, CogSource, Lockfile};
use crate::config::{Dependency, Manifest, is_valid_cog_name};
use crate::error::{CliError, Result};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use verum_compiler::session::Session;
use verum_modules::cog_resolver::CogResolver;

fn requirements(manifest: &Manifest) -> Vec<(String, String)> {
    manifest
        .dependencies
        .iter()
        .filter_map(|(name, dep)| {
            // The generated project manifest declares the bundled standard library.
            if name.as_str() == "stdlib" {
                return None;
            }
            let req = match dep {
                Dependency::Simple(version) => version.as_str(),
                Dependency::Detailed {
                    path: None,
                    git: None,
                    version,
                    optional,
                    ..
                } if !optional.unwrap_or(false) => version.as_ref().map_or("*", |v| v.as_str()),
                _ => return None,
            };
            Some((name.to_string(), req.to_string()))
        })
        .collect()
}

/// Retain the returned private snapshots until compilation finishes. Every run
/// reads and verifies the locked archive again, including when fully offline.
pub(crate) fn register(
    session: &mut Session,
    project: &Path,
    manifest: &Manifest,
) -> Result<Option<Vec<SourceSnapshot>>> {
    let mut pending = requirements(manifest);
    if pending.is_empty() {
        return Ok(None);
    }
    let lock = Lockfile::from_file(&Manifest::lockfile_path(project)).map_err(|e| {
        CliError::Registry(format!(
            "Cannot read installed dependencies for {}: {e}",
            manifest.cog.name
        ))
    })?;
    if lock.version != 1 || lock.root != manifest.cog.name {
        return Err(CliError::Registry(
            "Registry lockfile does not identify this project".into(),
        ));
    }
    let mut packages = HashMap::new();
    for package in &lock.packages {
        if packages.insert(package.name.as_str(), package).is_some() {
            return Err(CliError::Registry(format!(
                "Duplicate locked cog {}",
                package.name
            )));
        }
    }
    let cache = CacheManager::new(CacheManager::default_cache_dir()?)?;
    let mut sources = Vec::new();
    let mut resolver = CogResolver::new();
    let mut loaded = HashSet::new();
    while let Some((name, requirement)) = pending.pop() {
        let package = packages.get(name.as_str()).ok_or_else(|| {
            CliError::Registry(format!(
                "No installed lock entry for {name}; run verum package install"
            ))
        })?;
        let version = Version::parse(package.version.as_str())?;
        if !is_valid_cog_name(&name)
            || !VersionReq::parse(&requirement)?.matches(&version)
            || !matches!(&package.source, CogSource::Registry { registry, version }
                         if registry == &manifest.registry.index && version == &package.version)
        {
            return Err(CliError::Registry(format!(
                "Locked source or version does not match {name} {requirement}"
            )));
        }
        if !loaded.insert(name.clone()) {
            continue;
        }
        let source =
            cache.materialize_source(&name, package.version.as_str(), package.checksum.as_str())?;
        pending.extend(requirements(&source.manifest));
        let module = name.replace('-', "_");
        if resolver.is_external_cog(&module)
            || module == manifest.cog.name.as_str().replace('-', "_")
        {
            return Err(CliError::Registry(format!(
                "Conflicting installed module name {module}"
            )));
        }
        resolver.register_cog(module, package.version.clone(), source.path().to_path_buf());
        sources.push(source);
    }
    session.set_cog_resolver(resolver);
    Ok(Some(sources))
}
