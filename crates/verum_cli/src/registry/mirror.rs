// Cog registry mirroring: offline support, bandwidth optimization, local caching for airgapped environments

use super::types::*;
use crate::error::{CliError, Result};
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Text};

/// Local registry mirror
pub struct RegistryMirror {
    mirror_dir: PathBuf,
    index: Map<Text, List<VersionEntry>>,
}

impl RegistryMirror {
    /// Create new registry mirror
    pub fn new(mirror_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&mirror_dir)?;

        let index = Self::load_index(&mirror_dir)?;

        Ok(Self { mirror_dir, index })
    }

    /// Sync with upstream registry
    ///
    /// Syncs packages from upstream registry. Package specification can be:
    /// - "cog_name" - syncs latest version
    /// - "cog_name@version" - syncs specific version
    pub fn sync(&mut self, packages: List<Text>) -> Result<usize> {
        use super::client::RegistryClient;
        use tempfile::TempDir;

        let client = RegistryClient::default()?;
        let mut synced = 0;

        for package in packages {
            let package_str = package.as_str();

            // Parse package specification (name or name@version)
            let (name, version): (String, String) = if let Some(at_pos) = package_str.find('@') {
                let (n, v) = package_str.split_at(at_pos);
                (n.to_string(), v[1..].to_string())
            } else {
                // If no version specified, get latest from index
                if let Some(versions) = self.index.get(&package) {
                    if let Some(latest) = versions.last() {
                        (package_str.to_string(), latest.version.to_string())
                    } else {
                        continue;
                    }
                } else {
                    // Cog not in mirror yet, can't determine version
                    // Would need registry API to get available versions
                    continue;
                }
            };

            // Skip if already in mirror
            if self.has_cog(&name, &version) {
                continue;
            }

            // Download to temp directory
            let temp_dir = TempDir::new()?;
            let temp_file = temp_dir.path().join(format!("{}-{}.tar.gz", name, version));

            if client.download(&name, &version, &temp_file).is_ok() {
                // Get metadata
                if let Ok(metadata) = client.get_metadata(&name, &version)
                    && self.add_cog(&metadata, &temp_file).is_ok()
                {
                    synced += 1;
                }
            }
        }

        self.save_index()?;

        Ok(synced)
    }

    /// Add package to mirror
    pub fn add_cog(&mut self, metadata: &CogMetadata, cog_file: &Path) -> Result<()> {
        let package_dir = self
            .mirror_dir
            .join("packages")
            .join(&metadata.name)
            .join(&metadata.version);

        std::fs::create_dir_all(&package_dir)?;

        // Copy package file
        let dest = package_dir.join(format!("{}-{}.tar.gz", metadata.name, metadata.version));
        std::fs::copy(cog_file, &dest)?;

        // Update index
        let entry = VersionEntry {
            version: metadata.version.clone(),
            checksum: metadata.checksum.clone(),
            yanked: false,
            features: metadata.features.clone(),
        };

        self.index
            .entry(metadata.name.clone())
            .or_default()
            .push(entry);

        self.save_index()?;

        Ok(())
    }

    /// Get package from mirror
    pub fn get_cog(&self, name: &str, version: &str) -> Result<PathBuf> {
        let cog_path = self
            .mirror_dir
            .join("packages")
            .join(name)
            .join(version)
            .join(format!("{}-{}.tar.gz", name, version));

        if !cog_path.exists() {
            return Err(CliError::DependencyNotFound(format!(
                "{} {} not found in mirror",
                name, version
            )));
        }

        Ok(cog_path)
    }

    /// Check if package exists in mirror
    pub fn has_cog(&self, name: &str, version: &str) -> bool {
        self.index
            .get(&Text::from(name))
            .map(|versions| versions.iter().any(|v| v.version == version))
            .unwrap_or(false)
    }

    /// List all packages in mirror
    pub fn list_cogs(&self) -> List<(Text, List<Text>)> {
        self.index
            .iter()
            .map(|(name, versions)| {
                let version_strs = versions.iter().map(|v| v.version.clone()).collect();
                (name.clone(), version_strs)
            })
            .collect()
    }

    /// Load index from disk
    fn load_index(mirror_dir: &Path) -> Result<Map<Text, List<VersionEntry>>> {
        let index_path = mirror_dir.join("index.json");

        if !index_path.exists() {
            return Ok(Map::new());
        }

        let content = std::fs::read_to_string(&index_path)?;
        let index = serde_json::from_str(&content)?;

        Ok(index)
    }

    /// Save index to disk
    fn save_index(&self) -> Result<()> {
        let index_path = self.mirror_dir.join("index.json");
        let content = serde_json::to_string_pretty(&self.index)?;
        std::fs::write(&index_path, content)?;

        Ok(())
    }

    /// Serve mirror over HTTP
    ///
    /// Endpoints:
    /// - GET /index.json - Package index
    /// - GET /cogs/{name}/{version}/{name}-{version}.tar.gz - Package download
    pub fn serve(&self, port: u16) -> Result<()> {
        use std::net::TcpListener;

        let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
            .map_err(|e| CliError::Custom(format!("Failed to bind to port {}: {}", port, e)))?;

        println!("Mirror server running at http://localhost:{}", port);
        println!("Press Ctrl+C to stop");
        println!();
        println!("Available endpoints:");
        println!("  GET /index.json - Package index");
        println!(
            "  GET /cogs/{{name}}/{{version}}/{{name}}-{{version}}.tar.gz - Download package"
        );

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };

            if let Err(e) = self.handle_request(stream) {
                eprintln!("Error handling request: {}", e);
            }
        }

        Ok(())
    }

    /// Handle a single HTTP request
    fn handle_request(&self, mut stream: std::net::TcpStream) -> Result<()> {
        use std::io::{BufRead, BufReader};

        let buf_reader = BufReader::new(&stream);
        let request_line = buf_reader
            .lines()
            .next()
            .ok_or_else(|| CliError::Custom("Empty request".into()))??;

        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return self.send_response(&mut stream, 400, "Bad Request", &[]);
        }

        let (_method, path) = (parts[0], parts[1]);

        if path == "/index.json" {
            // Serve index
            let content = serde_json::to_string_pretty(&self.index)?;
            self.send_response(&mut stream, 200, "OK", content.as_bytes())?;
        } else if path.starts_with("/cogs/") {
            // Parse path: /cogs/{name}/{version}/{name}-{version}.tar.gz
            let path_parts: Vec<&str> = path.trim_start_matches("/cogs/").split('/').collect();
            if path_parts.len() >= 3 {
                let name = path_parts[0];
                let version = path_parts[1];
                let cog_path = self
                    .mirror_dir
                    .join("packages")
                    .join(name)
                    .join(version)
                    .join(format!("{}-{}.tar.gz", name, version));

                if cog_path.exists() {
                    let content = std::fs::read(&cog_path)?;
                    self.send_cog_response(&mut stream, 200, "OK", &content)?;
                } else {
                    self.send_response(&mut stream, 404, "Not Found", b"Cog not found")?;
                }
            } else {
                self.send_response(&mut stream, 400, "Bad Request", b"Invalid path")?;
            }
        } else if path == "/" || path == "/health" {
            let stats = self.stats();
            let body = format!(
                "Verum Mirror Server\n\nPackages: {}\nVersions: {}\nSize: {} bytes\n",
                stats.total_packages, stats.total_versions, stats.total_size_bytes
            );
            self.send_response(&mut stream, 200, "OK", body.as_bytes())?;
        } else {
            self.send_response(&mut stream, 404, "Not Found", b"Not found")?;
        }

        Ok(())
    }

    /// Send HTTP response
    fn send_response(
        &self,
        stream: &mut std::net::TcpStream,
        status: u16,
        status_text: &str,
        body: &[u8],
    ) -> Result<()> {
        use std::io::Write;

        let response = format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            status,
            status_text,
            body.len()
        );

        stream.write_all(response.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()?;

        Ok(())
    }

    /// Send package file response
    fn send_cog_response(
        &self,
        stream: &mut std::net::TcpStream,
        status: u16,
        status_text: &str,
        body: &[u8],
    ) -> Result<()> {
        use std::io::Write;

        let response = format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: application/gzip\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            status,
            status_text,
            body.len()
        );

        stream.write_all(response.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()?;

        Ok(())
    }

    /// Get statistics
    pub fn stats(&self) -> MirrorStats {
        let total_packages = self.index.len();
        let total_versions: usize = self.index.values().map(|v| v.len()).sum();

        let total_size = self.calculate_total_size();

        MirrorStats {
            total_packages,
            total_versions,
            total_size_bytes: total_size,
        }
    }

    fn calculate_total_size(&self) -> u64 {
        let packages_dir = self.mirror_dir.join("packages");

        if !packages_dir.exists() {
            return 0;
        }

        // SAFETY: follow_links(false) prevents infinite loop on symlink cycles
        walkdir::WalkDir::new(&packages_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum()
    }
}

/// Mirror statistics
#[derive(Debug, Clone)]
pub struct MirrorStats {
    pub total_packages: usize,
    pub total_versions: usize,
    pub total_size_bytes: u64,
}
