//! Lossless projection of representable manifest dependencies for publication.
//! Local/Git source intent requires a separate contract; never erase it here.

use super::types::DependencySpec;
use crate::config::{Dependency, Manifest};
use crate::error::{CliError, Result};
use semver::VersionReq;
use verum_common::{Map, Text};

/// Preserve declared options and refuse dependencies the registry cannot locate.
/// Explicit wildcard requirements are valid; missing requirements are not wildcards.
pub(crate) fn from_manifest(manifest: &Manifest) -> Result<Map<Text, DependencySpec>> {
    let mut dependencies = Map::new();
    for (name, dependency) in &manifest.dependencies {
        let published = match dependency {
            Dependency::Simple(version) => {
                validate_version(name, version)?;
                DependencySpec::Simple(version.clone())
            }
            Dependency::Detailed {
                version,
                path,
                git,
                branch,
                tag,
                rev,
                features,
                optional,
                default_features,
            } => {
                for (field, present) in [
                    ("path", path.is_some()),
                    ("git", git.is_some()),
                    ("branch", branch.is_some()),
                    ("tag", tag.is_some()),
                    ("rev", rev.is_some()),
                ] {
                    if present {
                        return Err(CliError::Custom(format!(
                            "Cannot publish dependency '{name}': source field '{field}' cannot be represented by registry publication metadata"
                        )));
                    }
                }
                let version = version.as_ref().ok_or_else(|| {
                    CliError::Custom(format!(
                        "Cannot publish dependency '{name}': an explicit registry version requirement is required"
                    ))
                })?;
                validate_version(name, version)?;
                DependencySpec::Detailed {
                    version: Some(version.clone()),
                    features: features.clone(),
                    optional: *optional,
                    default_features: *default_features,
                }
            }
        };
        dependencies.insert(name.clone(), published);
    }
    Ok(dependencies)
}

pub(super) fn validate_version(name: &Text, version: &Text) -> Result<()> {
    VersionReq::parse(version.as_str()).map_err(|error| {
        CliError::Custom(format!(
            "Cannot publish dependency '{name}': invalid version requirement '{version}': {error}"
        ))
    })?;
    Ok(())
}
