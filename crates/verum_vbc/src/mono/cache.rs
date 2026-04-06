//! Persistent monomorphization cache.

use std::collections::HashMap;
use std::path::PathBuf;

/// Persistent cache for monomorphized functions.
///
/// Stores specialized bytecode on disk for reuse across compilations.
/// Cache directory structure: `~/.verum/cache/mono/{hash}.vbc`
#[derive(Clone)]
pub struct MonomorphizationCache {
    /// Cache directory.
    cache_dir: PathBuf,
    /// In-memory cache of loaded specializations.
    loaded: HashMap<u64, Vec<u8>>,
}

impl MonomorphizationCache {
    /// Creates a new cache with the given directory.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            loaded: HashMap::new(),
        }
    }

    /// Creates a default cache in ~/.verum/cache/mono.
    pub fn default_cache() -> Option<Self> {
        let home = Self::get_home_dir()?;
        let cache_dir = home.join(".verum").join("cache").join("mono");
        Some(Self::new(cache_dir))
    }

    /// Gets the user's home directory in a cross-platform way.
    fn get_home_dir() -> Option<PathBuf> {
        #[cfg(unix)]
        {
            std::env::var("HOME").ok().map(PathBuf::from)
        }
        #[cfg(windows)]
        {
            std::env::var("USERPROFILE")
                .ok()
                .or_else(|| {
                    std::env::var("HOMEDRIVE").ok().and_then(|d| {
                        std::env::var("HOMEPATH").ok().map(|p| format!("{}{}", d, p))
                    })
                })
                .map(PathBuf::from)
        }
        #[cfg(not(any(unix, windows)))]
        {
            None
        }
    }

    /// Returns the cache directory path.
    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }

    /// Looks up a cached specialization.
    pub fn get(&mut self, hash: u64) -> Option<&Vec<u8>> {
        // Check in-memory cache first
        if self.loaded.contains_key(&hash) {
            return self.loaded.get(&hash);
        }

        // Try to load from disk
        let path = self.cache_dir.join(format!("{:016x}.vbc", hash));
        if path.exists()
            && let Ok(data) = std::fs::read(&path) {
                self.loaded.insert(hash, data);
                return self.loaded.get(&hash);
            }

        None
    }

    /// Stores a specialization in the cache.
    pub fn put(&mut self, hash: u64, bytecode: Vec<u8>) -> std::io::Result<()> {
        // Ensure cache directory exists
        std::fs::create_dir_all(&self.cache_dir)?;

        // Write to disk
        let path = self.cache_dir.join(format!("{:016x}.vbc", hash));
        std::fs::write(&path, &bytecode)?;

        // Store in memory
        self.loaded.insert(hash, bytecode);

        Ok(())
    }

    /// Checks if a specialization is cached (without loading it).
    pub fn contains(&self, hash: u64) -> bool {
        if self.loaded.contains_key(&hash) {
            return true;
        }
        let path = self.cache_dir.join(format!("{:016x}.vbc", hash));
        path.exists()
    }

    /// Removes a specialization from the cache.
    pub fn remove(&mut self, hash: u64) -> std::io::Result<bool> {
        self.loaded.remove(&hash);
        let path = self.cache_dir.join(format!("{:016x}.vbc", hash));
        if path.exists() {
            std::fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clears all cached specializations.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.loaded.clear();
        if self.cache_dir.exists() {
            for entry in std::fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|ext| ext == "vbc") {
                    std::fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    /// Returns the number of cached specializations (in memory).
    pub fn len(&self) -> usize {
        self.loaded.len()
    }

    /// Returns true if there are no cached specializations (in memory).
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// Garbage collects old cache entries.
    ///
    /// Removes entries older than `max_age`.
    pub fn gc(&mut self, max_age: std::time::Duration) -> std::io::Result<usize> {
        let now = std::time::SystemTime::now();
        let mut removed = 0;

        if self.cache_dir.exists() {
            for entry in std::fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let metadata = entry.metadata()?;

                if let Ok(modified) = metadata.modified()
                    && now.duration_since(modified).unwrap_or(std::time::Duration::ZERO) > max_age {
                        std::fs::remove_file(entry.path())?;
                        removed += 1;
                    }
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_new() {
        let cache = MonomorphizationCache::new(PathBuf::from("/tmp/test-cache"));
        assert_eq!(cache.cache_dir(), &PathBuf::from("/tmp/test-cache"));
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_put_get() {
        let dir = std::env::temp_dir().join("verum-test-mono-cache");
        let _ = std::fs::create_dir_all(&dir);
        let mut cache = MonomorphizationCache::new(dir.clone());

        let bytecode = vec![1, 2, 3, 4, 5];
        let hash = 0x123456789ABCDEF0;

        cache.put(hash, bytecode.clone()).unwrap();
        assert_eq!(cache.get(hash), Some(&bytecode));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cache_contains() {
        let dir = std::env::temp_dir().join("verum-test-mono-cache-2");
        // Clean up any leftover files from previous runs
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let mut cache = MonomorphizationCache::new(dir.clone());

        let hash = 0xFEDCBA9876543210;
        assert!(!cache.contains(hash));

        cache.put(hash, vec![1, 2, 3]).unwrap();
        assert!(cache.contains(hash));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
