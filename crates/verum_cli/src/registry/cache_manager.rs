// Global package cache with parallel downloads and content-based deduplication.
// Cache hits target <10ms. Supports concurrent download workers.

use crate::error::{CliError, Result};
use crossbeam_channel::{Receiver, Sender, bounded};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use verum_common::{List, Map, Text};

/// Global package cache manager
pub struct CacheManager {
    cache_dir: PathBuf,
    downloads_in_progress: Arc<Mutex<Map<Text, DownloadStatus>>>,
    max_parallel: usize,
}

/// Download status
#[derive(Debug, Clone)]
pub enum DownloadStatus {
    Pending,
    InProgress { progress: f64 },
    Completed { path: PathBuf },
    Failed { error: String },
}

/// Download task
#[derive(Debug, Clone)]
struct DownloadTask {
    cog_name: Text,
    version: Text,
    url: Text,
    checksum: Text,
}

/// Download result
#[derive(Debug)]
struct DownloadResult {
    cog_name: Text,
    version: Text,
    result: Result<PathBuf>,
}

impl CacheManager {
    /// Create new cache manager
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            cache_dir,
            downloads_in_progress: Arc::new(Mutex::new(Map::new())),
            max_parallel: num_cpus::get(),
        })
    }

    /// Get default cache directory
    pub fn default_cache_dir() -> Result<PathBuf> {
        super::cache_dir()
    }

    /// Check if cog is cached
    pub fn is_cached(&self, name: &str, version: &str) -> bool {
        self.get_cog_path(name, version).exists()
    }

    /// Get path to cached package
    pub fn get_cog_path(&self, name: &str, version: &str) -> PathBuf {
        self.cache_dir
            .join(name)
            .join(version)
            .join(format!("{}-{}.tar.gz", name, version))
    }

    /// Get package from cache or download
    pub fn get_or_download(
        &self,
        name: &str,
        version: &str,
        url: &str,
        checksum: &str,
    ) -> Result<PathBuf> {
        // Check cache first
        let cache_path = self.get_cog_path(name, version);

        if cache_path.exists() {
            // Verify checksum
            if self.verify_checksum(&cache_path, checksum)? {
                return Ok(cache_path);
            } else {
                // Checksum mismatch - redownload
                std::fs::remove_file(&cache_path)?;
            }
        }

        // Download
        self.download_cog(name, version, url, checksum)
    }

    /// Download package
    pub fn download_cog(
        &self,
        name: &str,
        version: &str,
        url: &str,
        checksum: &str,
    ) -> Result<PathBuf> {
        // Check if already downloading
        {
            let in_progress = self.downloads_in_progress.lock().unwrap();
            let key: Text = format!("{}@{}", name, version).into();

            if let Some(status) = in_progress.get(&key) {
                match status {
                    DownloadStatus::InProgress { .. } => {
                        return Err(CliError::Custom(format!(
                            "Download already in progress for {}@{}",
                            name, version
                        )));
                    }
                    DownloadStatus::Completed { path } => {
                        return Ok(path.clone());
                    }
                    _ => {}
                }
            }
        }

        // Mark as in progress
        {
            let mut in_progress = self.downloads_in_progress.lock().unwrap();
            let key: Text = format!("{}@{}", name, version).into();
            in_progress.insert(key, DownloadStatus::InProgress { progress: 0.0 });
        }

        // Create package directory
        let package_dir = self.cache_dir.join(name).join(version);
        std::fs::create_dir_all(&package_dir)?;

        let dest_path = package_dir.join(format!("{}-{}.tar.gz", name, version));

        // Download
        let result = self.download_file(url, &dest_path);

        match result {
            Ok(()) => {
                // Verify checksum
                if !self.verify_checksum(&dest_path, checksum)? {
                    std::fs::remove_file(&dest_path)?;

                    let mut in_progress = self.downloads_in_progress.lock().unwrap();
                    let key: Text = format!("{}@{}", name, version).into();
                    in_progress.insert(
                        key,
                        DownloadStatus::Failed {
                            error: "Checksum verification failed".to_string(),
                        },
                    );

                    return Err(CliError::Custom(format!(
                        "Checksum verification failed for {}@{}",
                        name, version
                    )));
                }

                // Mark as completed
                {
                    let mut in_progress = self.downloads_in_progress.lock().unwrap();
                    let key: Text = format!("{}@{}", name, version).into();
                    in_progress.insert(
                        key,
                        DownloadStatus::Completed {
                            path: dest_path.clone(),
                        },
                    );
                }

                Ok(dest_path)
            }
            Err(e) => {
                // Mark as failed
                {
                    let mut in_progress = self.downloads_in_progress.lock().unwrap();
                    let key: Text = format!("{}@{}", name, version).into();
                    in_progress.insert(
                        key,
                        DownloadStatus::Failed {
                            error: e.to_string(),
                        },
                    );
                }

                Err(e)
            }
        }
    }

    /// Download multiple packages in parallel
    pub fn download_parallel(
        &self,
        tasks: List<(Text, Text, Text, Text)>,
    ) -> Result<List<PathBuf>> {
        let (task_tx, task_rx): (Sender<DownloadTask>, Receiver<DownloadTask>) =
            bounded(tasks.len());
        let (result_tx, result_rx): (Sender<DownloadResult>, Receiver<DownloadResult>) =
            bounded(tasks.len());

        // Queue tasks
        for (name, version, url, checksum) in tasks.iter() {
            task_tx
                .send(DownloadTask {
                    cog_name: name.clone(),
                    version: version.clone(),
                    url: url.clone(),
                    checksum: checksum.clone(),
                })
                .unwrap();
        }
        drop(task_tx);

        // Spawn worker threads
        let mut handles = List::new();

        for _ in 0..self.max_parallel {
            let task_rx = task_rx.clone();
            let result_tx = result_tx.clone();
            let cache_dir = self.cache_dir.clone();

            let handle = thread::spawn(move || {
                while let Ok(task) = task_rx.recv() {
                    let cache_manager = CacheManager {
                        cache_dir: cache_dir.clone(),
                        downloads_in_progress: Arc::new(Mutex::new(Map::new())),
                        max_parallel: 1,
                    };

                    let result = cache_manager.download_cog(
                        task.cog_name.as_str(),
                        task.version.as_str(),
                        task.url.as_str(),
                        task.checksum.as_str(),
                    );

                    result_tx
                        .send(DownloadResult {
                            cog_name: task.cog_name,
                            version: task.version,
                            result,
                        })
                        .unwrap();
                }
            });

            handles.push(handle);
        }

        drop(result_tx);

        // Collect results
        let mut results = List::new();
        let mut errors = List::new();

        while let Ok(download_result) = result_rx.recv() {
            match download_result.result {
                Ok(path) => results.push(path),
                Err(e) => errors.push(format!(
                    "{}@{}: {}",
                    download_result.cog_name, download_result.version, e
                )),
            }
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        if !errors.is_empty() {
            return Err(CliError::Custom(format!(
                "Download failures:\n{}",
                errors.join("\n")
            )));
        }

        Ok(results)
    }

    /// Download file from URL
    fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        let response = reqwest::blocking::get(url).map_err(|e| CliError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CliError::Network(format!(
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

    /// Verify file checksum
    fn verify_checksum(&self, path: &Path, expected: &str) -> Result<bool> {
        let content = std::fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let actual = format!("{:x}", hasher.finalize());

        Ok(actual == expected)
    }

    /// Get cache statistics
    pub fn stats(&self) -> Result<CacheStats> {
        let mut total_packages = 0;
        let mut total_versions = 0;
        let mut total_size = 0u64;

        if !self.cache_dir.exists() {
            return Ok(CacheStats {
                total_packages: 0,
                total_versions: 0,
                total_size_bytes: 0,
            });
        }

        // Walk cache directory
        // SAFETY: follow_links(false) prevents infinite loop on symlink cycles
        for entry in walkdir::WalkDir::new(&self.cache_dir)
            .follow_links(false)
            .min_depth(1)
            .max_depth(1)
        {
            let entry = entry?;
            if entry.file_type().is_dir() {
                total_packages += 1;

                // Count versions
                for version_entry in walkdir::WalkDir::new(entry.path())
                    .follow_links(false)
                    .min_depth(1)
                    .max_depth(1)
                {
                    let version_entry = version_entry?;
                    if version_entry.file_type().is_dir() {
                        total_versions += 1;

                        // Sum file sizes
                        for file_entry in
                            walkdir::WalkDir::new(version_entry.path()).follow_links(false)
                        {
                            let file_entry = file_entry?;
                            if file_entry.file_type().is_file()
                                && let Ok(metadata) = file_entry.metadata()
                            {
                                total_size += metadata.len();
                            }
                        }
                    }
                }
            }
        }

        Ok(CacheStats {
            total_packages,
            total_versions,
            total_size_bytes: total_size,
        })
    }

    /// Clear cache
    pub fn clear(&self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
            std::fs::create_dir_all(&self.cache_dir)?;
        }

        Ok(())
    }

    /// Prune old/unused packages
    pub fn prune(&self, keep_latest_n: usize) -> Result<usize> {
        let mut removed = 0;

        if !self.cache_dir.exists() {
            return Ok(0);
        }

        // For each package
        // SAFETY: follow_links(false) prevents infinite loop on symlink cycles
        for entry in walkdir::WalkDir::new(&self.cache_dir)
            .follow_links(false)
            .min_depth(1)
            .max_depth(1)
        {
            let entry = entry?;
            if !entry.file_type().is_dir() {
                continue;
            }

            // Get all versions
            let mut versions: List<_> = walkdir::WalkDir::new(entry.path())
                .follow_links(false)
                .min_depth(1)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_dir())
                .collect();

            // Sort by modification time (newest first)
            versions.sort_by(|a, b| {
                let a_time = a.metadata().ok().and_then(|m| m.modified().ok());
                let b_time = b.metadata().ok().and_then(|m| m.modified().ok());
                b_time.cmp(&a_time)
            });

            // Remove old versions
            for version_entry in versions.iter().skip(keep_latest_n) {
                std::fs::remove_dir_all(version_entry.path())?;
                removed += 1;
            }
        }

        Ok(removed)
    }

    /// Extract package archive
    pub fn extract(&self, cog_path: &Path, dest_dir: &Path) -> Result<()> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let file = std::fs::File::open(cog_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        archive
            .unpack(dest_dir)
            .map_err(|e| CliError::Custom(format!("Failed to extract archive: {}", e)))?;

        Ok(())
    }

    /// Create package archive
    pub fn create_archive(
        &self,
        source_dir: &Path,
        cog_name: &str,
        version: &str,
    ) -> Result<PathBuf> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        let dest_path = self
            .cache_dir
            .join(cog_name)
            .join(version)
            .join(format!("{}-{}.tar.gz", cog_name, version));

        std::fs::create_dir_all(dest_path.parent().unwrap())?;

        let file = std::fs::File::create(&dest_path)?;
        let encoder = GzEncoder::new(file, Compression::best());
        let mut builder = Builder::new(encoder);

        builder
            .append_dir_all(".", source_dir)
            .map_err(|e| CliError::Custom(format!("Failed to create archive: {}", e)))?;

        builder
            .finish()
            .map_err(|e| CliError::Custom(format!("Failed to finish archive: {}", e)))?;

        Ok(dest_path)
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_packages: usize,
    pub total_versions: usize,
    pub total_size_bytes: u64,
}

impl CacheStats {
    pub fn total_size_mb(&self) -> f64 {
        self.total_size_bytes as f64 / (1024.0 * 1024.0)
    }

    pub fn total_size_gb(&self) -> f64 {
        self.total_size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}
