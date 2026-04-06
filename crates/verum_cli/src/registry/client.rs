// Cog registry HTTP client: package fetching, publishing, authentication

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
}

impl RegistryClient {
    /// Create new registry client
    pub fn new(base_url: impl Into<Text>) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("verum-cli/1.0.0")
            .build()
            .map_err(|e| CliError::Network(e.to_string()))?;

        Ok(Self {
            base_url: base_url.into(),
            client,
        })
    }

    /// Create default registry client
    pub fn default() -> Result<Self> {
        Self::new(super::DEFAULT_REGISTRY)
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
        let url = format!(
            "{}/cogs/{}/{}/download",
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

    /// Publish package
    pub fn publish(
        &self,
        manifest: &CogMetadata,
        cog_file: &Path,
        token: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/cogs/publish",
            super::registry_api_url(self.base_url.as_str())
        );

        let package_bytes = std::fs::read(cog_file)?;

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(manifest)
            .body(package_bytes)
            .send()
            .map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Registry(format!(
                "Publish failed: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// Check for vulnerabilities
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
            // No vulnerabilities found
            return Ok(VulnerabilityReport {
                package: name.into(),
                version: version.into(),
                vulnerabilities: List::new(),
            });
        }

        response
            .json()
            .map_err(|e| CliError::Registry(format!("Failed to parse vulnerability report: {}", e)))
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
