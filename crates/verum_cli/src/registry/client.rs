// Cog registry HTTP client: package fetching, publishing, authentication

use super::publication::{PUBLICATION_CONTENT_TYPE, PublicationLimits, encode_publication};
use super::publication_receipt::verify_publication_receipt;
use super::types::*;
use crate::error::{CliError, Result};
use reqwest::blocking::Client;
use std::path::Path;
use std::time::Duration;
use verum_common::{List, Text};

/// Registry client for interacting with package repository
pub struct RegistryClient {
    base_url: Text,
    client: Client,
    publication_client: Option<Client>,
    publication_limits: PublicationLimits,
}

impl RegistryClient {
    /// Create new registry client
    pub fn new(base_url: impl Into<Text>) -> Result<Self> {
        Ok(Self {
            base_url: base_url.into(),
            client: http_client_builder()
                .build()
                .map_err(|error| CliError::Network(error.to_string()))?,
            publication_client: None,
            publication_limits: PublicationLimits::default(),
        })
    }

    /// Create default registry client.
    ///
    /// Uses the hardcoded `DEFAULT_REGISTRY` URL. Surface a
    /// `tracing::debug!` when an active manifest carries
    /// `[registry].index` set to a non-default value, so
    /// embedders see when their override isn't being honored.
    /// The proper fix is to migrate callers to `from_manifest`
    /// which consults the manifest layer; this surfaces the
    /// discrepancy until that migration completes.
    pub fn default() -> Result<Self> {
        if let Ok(dir) = crate::config::Manifest::find_manifest_dir() {
            let path = crate::config::Manifest::manifest_path(&dir);
            if let Ok(manifest) = crate::config::Manifest::from_file(&path) {
                if manifest.registry.index.as_str() != super::DEFAULT_REGISTRY {
                    tracing::debug!(
                        "RegistryClient::default(): manifest [registry].index = {:?} \
                         differs from hardcoded DEFAULT_REGISTRY {:?} — manifest override \
                         is not yet honored at this construction site; use \
                         `RegistryClient::from_manifest` instead",
                        manifest.registry.index.as_str(),
                        super::DEFAULT_REGISTRY,
                    );
                }
            }
        }
        Self::new(super::DEFAULT_REGISTRY)
    }

    /// Create registry client from manifest, honouring the
    /// `[registry].index` field. Falls back to `DEFAULT_REGISTRY`
    /// when no manifest is present. Closes the inert-defense
    /// pattern around `RegistryConfig.index`: pre-fix the field
    /// was TOML-parseable but every call site went through
    /// `default()` which hardcodes the URL.
    pub fn from_manifest() -> Result<Self> {
        let dir = match crate::config::Manifest::find_manifest_dir() {
            Ok(dir) => dir,
            Err(CliError::ProjectNotFound(_)) => return Self::new(super::DEFAULT_REGISTRY),
            Err(error) => return Err(error),
        };
        let path = crate::config::Manifest::manifest_path(&dir);
        // An unreadable or malformed project must not silently select a public
        // registry instead of the source its owner intended.
        let manifest = crate::config::Manifest::from_file(&path)?;
        Self::new(manifest.registry.index)
    }

    /// Apply validated local source-publication limits.
    pub fn with_publication_limits(mut self, limits: PublicationLimits) -> Self {
        self.publication_limits = limits;
        self
    }

    /// Registry selected for this operation, also recorded in its lockfile.
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Source archive endpoint on the same API that supplied its metadata.
    pub fn download_url(&self, name: &str, version: &str) -> Text {
        format!(
            "{}/cogs/{}/{}/download",
            super::registry_api_url(self.base_url()),
            name,
            version
        )
        .into()
    }

    /// Search for packages
    pub fn search(&self, query: &str, limit: usize) -> Result<List<SearchResult>> {
        let url = format!("{}/search", super::registry_api_url(self.base_url.as_str()));

        let response = self
            .client
            .get(&url)
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Search failed: {}",
                response.status()
            )));
        }

        response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse search results: {}", e)))
    }

    /// Get package metadata
    pub fn get_metadata(&self, name: &str, version: &str) -> Result<CogMetadata> {
        let url = format!(
            "{}/cogs/{}/{}",
            super::registry_api_url(self.base_url.as_str()),
            name,
            version
        );

        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Cog not found: {} {}",
                name, version
            )));
        }

        response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse metadata: {}", e)))
    }

    /// Get latest version of package
    pub fn get_latest_version(&self, name: &str) -> Result<Text> {
        let url = format!(
            "{}/cogs/{}/latest",
            super::registry_api_url(self.base_url.as_str()),
            name
        );

        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!("Cog not found: {}", name)));
        }

        #[derive(serde::Deserialize)]
        struct LatestVersion {
            version: Text,
        }

        let latest: LatestVersion = response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse version: {}", e)))?;

        Ok(latest.version)
    }

    /// Download package
    pub fn download(&self, name: &str, version: &str, dest: &Path) -> Result<()> {
        let url = self.download_url(name, version);

        let response = self
            .client
            .get(url.as_str())
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Download failed: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .map_err(|e| CliError::Network(e.to_string()))?;

        std::fs::write(dest, &bytes)?;

        Ok(())
    }

    /// Validate the same request used by publication, without HTTP or credentials.
    pub fn validate_publication(&self, manifest: &CogMetadata, cog_file: &Path) -> Result<()> {
        encode_publication(manifest, cog_file, self.publication_limits).map(|_| ())
    }

    /// Publish package
    pub fn publish(&self, manifest: &CogMetadata, cog_file: &Path, token: &str) -> Result<()> {
        let url = format!(
            "{}/cogs/publish",
            super::registry_api_url(self.base_url.as_str())
        );

        let envelope = encode_publication(manifest, cog_file, self.publication_limits)?;

        // Construct the no-redirect transport only for publication. Metadata
        // and download clients retain their existing initialization behavior.
        let publication_client = match &self.publication_client {
            Some(client) => client.clone(),
            None => publication_client_builder()
                .build()
                .map_err(|error| CliError::Network(error.to_string()))?,
        };
        let response = publication_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", PUBLICATION_CONTENT_TYPE)
            // reqwest's owned body uses Vec at the external API boundary.
            .body(Vec::from(envelope))
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        verify_publication_receipt(response, manifest)
    }

    /// Obtain an advisory report for exactly the requested release.
    ///
    /// Unavailable or unrelated advisory data is an error, never evidence of
    /// an empty report. Callers must decide how to handle that failure before
    /// downloading artifacts or mutating the project.
    pub fn check_vulnerabilities(&self, name: &str, version: &str) -> Result<VulnerabilityReport> {
        let url = format!(
            "{}/security/vulnerabilities/{}/{}",
            super::registry_api_url(self.base_url.as_str()),
            name,
            version
        );

        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Vulnerability check failed for {} {}: {}",
                name,
                version,
                response.status()
            )));
        }

        let report: VulnerabilityReport = response.json().map_err(|e| {
            CliError::Registry(format!("Failed to parse vulnerability report: {}", e))
        })?;
        if report.package.as_str() != name || report.version.as_str() != version {
            return Err(CliError::Registry(format!(
                "Vulnerability report does not match requested cog {} {}",
                name, version
            )));
        }
        Ok(report)
    }

    /// Get package index
    pub fn get_index(&self, name: &str) -> Result<IndexEntry> {
        let url = format!(
            "{}/index/{}",
            super::registry_index_url(self.base_url.as_str()),
            name
        );

        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Index not found for package: {}",
                name
            )));
        }

        response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse index: {}", e)))
    }

    /// Login to registry
    pub fn login(&self, username: &str, password: &str) -> Result<Text> {
        let url = format!(
            "{}/auth/login",
            super::registry_api_url(self.base_url.as_str())
        );

        #[derive(serde::Serialize)]
        struct LoginRequest {
            username: Text,
            password: Text,
        }

        #[derive(serde::Deserialize)]
        struct LoginResponse {
            token: Text,
        }

        let response = self
            .client
            .post(&url)
            .json(&LoginRequest {
                username: username.into(),
                password: password.into(),
            })
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Login failed: {}",
                response.status()
            )));
        }

        let login_response: LoginResponse = response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse login response: {}", e)))?;

        Ok(login_response.token)
    }

    /// Yank a published version
    pub fn yank(&self, name: &str, version: &str, token: &str) -> Result<()> {
        let url = format!(
            "{}/cogs/{}/{}/yank",
            super::registry_api_url(self.base_url.as_str()),
            name,
            version
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Yank failed: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// Unyank a yanked version
    pub fn unyank(&self, name: &str, version: &str, token: &str) -> Result<()> {
        let url = format!(
            "{}/cogs/{}/{}/unyank",
            super::registry_api_url(self.base_url.as_str()),
            name,
            version
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Unyank failed: {}",
                response.status()
            )));
        }

        Ok(())
    }
}

fn http_client_builder() -> reqwest::blocking::ClientBuilder {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("verum-cli/1.0.0")
}

fn publication_client_builder() -> reqwest::blocking::ClientBuilder {
    // A redirect must not send unpublished package bytes to another origin.
    http_client_builder().redirect(reqwest::redirect::Policy::none())
}

#[cfg(test)]
#[path = "../../tests/registry/publication_transport.rs"]
mod publication_transport;

#[cfg(test)]
#[path = "../../tests/registry/publication_receipts.rs"]
mod publication_receipts;
