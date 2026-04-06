// IPFS integration for decentralized package distribution.
// Content-addressed storage ensures integrity; supports pinning and gateway fallback.

use crate::error::{CliError, Result};
use reqwest::blocking::Client;
use std::fs;
use std::path::Path;
use verum_common::Text;

/// Synchronous IPFS client for decentralized package distribution
pub struct IpfsClient {
    pub api_url: Text,
    client: Client,
}

impl IpfsClient {
    /// Create new IPFS client
    pub fn new(api_url: impl Into<Text>) -> Self {
        Self {
            api_url: api_url.into(),
            client: Client::new(),
        }
    }

    /// Create default IPFS client (localhost:5001)
    pub fn default() -> Self {
        Self::new("http://127.0.0.1:5001")
    }

    /// Add file to IPFS using /api/v0/add
    pub fn add_file(&self, path: &Path) -> Result<Text> {
        let url = format!("{}/api/v0/add", self.api_url);

        // Build multipart form with the file
        let form = reqwest::blocking::multipart::Form::new()
            .file("file", path)
            .map_err(|e| CliError::Custom(format!("Failed to read file for IPFS upload: {}", e)))?;

        // Send POST request
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .map_err(|e| CliError::Network(format!("IPFS add request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(CliError::Custom(format!(
                "IPFS add failed with status {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            )));
        }

        // Parse response to extract hash
        #[derive(serde::Deserialize)]
        struct AddResponse {
            #[serde(rename = "Hash")]
            hash: String,
        }

        let add_response: AddResponse = response
            .json()
            .map_err(|e| CliError::Custom(format!("Failed to parse IPFS add response: {}", e)))?;

        Ok(add_response.hash.into())
    }

    /// Get file from IPFS using /api/v0/cat
    pub fn get_file(&self, hash: &str, dest: &Path) -> Result<()> {
        let url = format!("{}/api/v0/cat?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .map_err(|e| CliError::Network(format!("IPFS cat request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(CliError::Custom(format!(
                "IPFS cat failed with status {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            )));
        }

        // Write response bytes to destination file
        let bytes = response
            .bytes()
            .map_err(|e| CliError::Custom(format!("Failed to read IPFS response: {}", e)))?;

        fs::write(dest, bytes).map_err(CliError::Io)?;

        Ok(())
    }

    /// Pin hash to IPFS using /api/v0/pin/add
    pub fn pin(&self, hash: &str) -> Result<()> {
        let url = format!("{}/api/v0/pin/add?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .map_err(|e| CliError::Network(format!("IPFS pin request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(CliError::Custom(format!(
                "IPFS pin failed with status {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            )));
        }

        Ok(())
    }

    /// Unpin hash from IPFS using /api/v0/pin/rm
    pub fn unpin(&self, hash: &str) -> Result<()> {
        let url = format!("{}/api/v0/pin/rm?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .map_err(|e| CliError::Network(format!("IPFS unpin request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(CliError::Custom(format!(
                "IPFS unpin failed with status {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            )));
        }

        Ok(())
    }

    /// Check if IPFS daemon is running using /api/v0/version
    pub fn is_available(&self) -> bool {
        let url = format!("{}/api/v0/version", self.api_url);

        self.client
            .post(&url)
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// Asynchronous IPFS client for decentralized package distribution
pub struct AsyncIpfsClient {
    pub api_url: Text,
    client: reqwest::Client,
}

impl AsyncIpfsClient {
    /// Create new async IPFS client
    pub fn new(api_url: impl Into<Text>) -> Self {
        Self {
            api_url: api_url.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Create default async IPFS client (localhost:5001)
    pub fn default() -> Self {
        Self::new("http://127.0.0.1:5001")
    }

    /// Add file to IPFS using /api/v0/add
    pub async fn add_file(&self, path: &Path) -> Result<Text> {
        let url = format!("{}/api/v0/add", self.api_url);

        // Build multipart form with the file
        let form = reqwest::multipart::Form::new()
            .file("file", path)
            .await
            .map_err(|e| CliError::Custom(format!("Failed to read file for IPFS upload: {}", e)))?;

        // Send POST request
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| CliError::Network(format!("IPFS add request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CliError::Custom(format!(
                "IPFS add failed with status {}: {}",
                status, text
            )));
        }

        // Parse response to extract hash
        #[derive(serde::Deserialize)]
        struct AddResponse {
            #[serde(rename = "Hash")]
            hash: String,
        }

        let add_response: AddResponse = response
            .json()
            .await
            .map_err(|e| CliError::Custom(format!("Failed to parse IPFS add response: {}", e)))?;

        Ok(add_response.hash.into())
    }

    /// Get file from IPFS using /api/v0/cat
    pub async fn get_file(&self, hash: &str, dest: &Path) -> Result<()> {
        let url = format!("{}/api/v0/cat?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| CliError::Network(format!("IPFS cat request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CliError::Custom(format!(
                "IPFS cat failed with status {}: {}",
                status, text
            )));
        }

        // Write response bytes to destination file
        let bytes = response
            .bytes()
            .await
            .map_err(|e| CliError::Custom(format!("Failed to read IPFS response: {}", e)))?;

        fs::write(dest, bytes).map_err(CliError::Io)?;

        Ok(())
    }

    /// Pin hash to IPFS using /api/v0/pin/add
    pub async fn pin(&self, hash: &str) -> Result<()> {
        let url = format!("{}/api/v0/pin/add?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| CliError::Network(format!("IPFS pin request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CliError::Custom(format!(
                "IPFS pin failed with status {}: {}",
                status, text
            )));
        }

        Ok(())
    }

    /// Unpin hash from IPFS using /api/v0/pin/rm
    pub async fn unpin(&self, hash: &str) -> Result<()> {
        let url = format!("{}/api/v0/pin/rm?arg={}", self.api_url, hash);

        let response = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| CliError::Network(format!("IPFS unpin request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CliError::Custom(format!(
                "IPFS unpin failed with status {}: {}",
                status, text
            )));
        }

        Ok(())
    }

    /// Check if IPFS daemon is running using /api/v0/version
    pub async fn is_available(&self) -> bool {
        let url = format!("{}/api/v0/version", self.api_url);

        self.client
            .post(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

/// IPFS configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IpfsConfig {
    /// API URL
    pub api_url: Text,

    /// Gateway URL
    pub gateway_url: Text,

    /// Auto-pin packages
    pub auto_pin: bool,

    /// Prefer IPFS for downloads
    pub prefer_ipfs: bool,
}

impl Default for IpfsConfig {
    fn default() -> Self {
        Self {
            api_url: "http://127.0.0.1:5001".into(),
            gateway_url: "http://127.0.0.1:8080".into(),
            auto_pin: false,
            prefer_ipfs: false,
        }
    }
}
