//! Bind an ordinary project's registry dependencies to its installed, locked source.
use super::{CacheManager, CogSource, Lockfile};
use crate::config::{Dependency, Manifest, is_valid_cog_name};
use crate::error::{CliError, Result};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tempfile::TempDir;
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

/// Retain the returned private directory until compilation finishes. Every run
/// reads and verifies the locked archive again, including when fully offline.
pub(crate) fn register(
    session: &mut Session,
    project: &Path,
    manifest: &Manifest,
) -> Result<Option<TempDir>> {
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
    let mut staging = tempfile::Builder::new();
    staging.prefix("verum-installed-");
    #[cfg(unix)]
    staging.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let sources = staging.tempdir()?;
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
        let root = sources.path().join(&name);
        std::fs::create_dir(&root)?;
        cache.extract_locked_source(
            &name,
            package.version.as_str(),
            package.checksum.as_str(),
            &root,
        )?;
        // Parse only as data: a dependency must not install its own global solver configuration.
        let dependency: Manifest =
            toml::from_str(&std::fs::read_to_string(Manifest::manifest_path(&root))?)?;
        if dependency.cog.name != package.name
            || dependency.cog.version != package.version
            || !root.join("src").is_dir()
        {
            return Err(CliError::Registry(format!(
                "Installed source does not match locked cog {name} {version}"
            )));
        }
        pending.extend(requirements(&dependency));
        let module = name.replace('-', "_");
        if resolver.is_external_cog(&module)
            || module == manifest.cog.name.as_str().replace('-', "_")
        {
            return Err(CliError::Registry(format!(
                "Conflicting installed module name {module}"
            )));
        }
        resolver.register_cog(module, package.version.clone(), root);
    }
    session.set_cog_resolver(resolver);
    Ok(Some(sources))
}
