//! Distributed verification cache with S3, Redis, and filesystem backends
//!
//! Enables teams to share verification results across CI/CD pipelines,
//! dramatically reducing verification time on unchanged code.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐     ┌─────────────────┐     ┌────────────────┐
//! │ Local Cache │────►│ Distributed     │────►│ Remote Storage │
//! │ (LRU)       │     │ Cache           │     │ (S3/Redis/FS)  │
//! └─────────────┘     └─────────────────┘     └────────────────┘
//!       │                     │                        │
//!       │ Fast (µs)           │ Medium (ms)            │ Slow (10-100ms)
//!       └─────────────────────┴────────────────────────┘
//! ```
//!
//! # Backend Options
//!
//! | Backend | Feature Flag | Use Case |
//! |---------|--------------|----------|
//! | S3 | `distributed-cache` | Team/CI sharing via cloud storage |
//! | Redis | `redis-cache` | Low-latency team sharing |
//! | Filesystem | (always available) | Local persistent fallback |
//!
//! # Features
//!
//! - **S3-compatible storage**: Works with AWS S3, MinIO, Cloudflare R2, etc.
//! - **Redis storage**: Low-latency distributed caching for teams
//! - **Filesystem fallback**: Persistent local cache when cloud unavailable
//! - **Trust levels**: None, Signatures, Sampling
//! - **Cryptographic signatures**: Ed25519 signing for cache entry integrity
//! - **TTL-based expiration**: Configurable max age for cache entries
//! - **Local cache layer**: Minimize network round-trips
//!
//! # Example
//!
//! ```rust,no_run
//! use verum_smt::distributed_cache::{DistributedCache, DistributedCacheConfig, TrustLevel, CachedResult};
//! use verum_common::Maybe;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DistributedCacheConfig {
//!     storage_url: "s3://my-bucket/verum-cache".into(),
//!     trust_level: TrustLevel::Signatures,
//!     max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
//!     credentials: Maybe::None,
//!     filesystem_fallback: Maybe::Some(".verum/cache".into()),
//!     redis_url: Maybe::None,
//! };
//!
//! let mut cache = DistributedCache::new(config);
//!
//! // Check if result is cached
//! if let Maybe::Some(entry) = cache.get("some-key").await {
//!     println!("Cache hit! Saved time: {}ms", entry.metadata.original_time_ms);
//! } else {
//!     // Run verification, then cache result
//!     cache.put("some-key", CachedResult::Proved, 1500).await?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Enabling Features
//!
//! To use S3 backend:
//! ```toml
//! [dependencies]
//! verum_smt = { version = "*", features = ["distributed-cache"] }
//! ```
//!
//! To use Redis backend:
//! ```toml
//! [dependencies]
//! verum_smt = { version = "*", features = ["redis-cache"] }
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use verum_common::{Map, Maybe, Text};


#[cfg(feature = "distributed-cache")]
use {
    base64::Engine,
    hmac::{Hmac, Mac},
    reqwest::Client,
};

#[cfg(feature = "redis-cache")]
use redis::{AsyncCommands, Client as RedisClient};

// ==================== Configuration ====================

/// Configuration for distributed cache
#[derive(Debug, Clone)]
pub struct DistributedCacheConfig {
    /// S3-compatible storage URL (e.g., "s3://bucket/verum-cache")
    pub storage_url: Text,
    /// Trust level for cached results
    pub trust_level: TrustLevel,
    /// Max age before re-verification
    pub max_age: Duration,
    /// Optional access credentials
    pub credentials: Maybe<CacheCredentials>,
    /// Filesystem fallback path for persistent local cache
    /// When S3/Redis are unavailable, cache entries are stored here
    pub filesystem_fallback: Maybe<Text>,
    /// Redis URL for low-latency distributed caching
    /// Example: "redis://localhost:6379" or "redis://user:pass@host:6379/0"
    pub redis_url: Maybe<Text>,
}

impl DistributedCacheConfig {
    /// Create new configuration with default settings
    pub fn new(storage_url: impl Into<Text>) -> Self {
        Self {
            storage_url: storage_url.into(),
            trust_level: TrustLevel::Signatures,
            max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
            credentials: Maybe::None,
            filesystem_fallback: Maybe::Some(".verum/cache".into()),
            redis_url: Maybe::None,
        }
    }

    /// Create configuration for filesystem-only caching (no cloud storage)
    pub fn filesystem_only(cache_dir: impl Into<Text>) -> Self {
        Self {
            storage_url: "".into(),
            trust_level: TrustLevel::None,
            max_age: Duration::from_secs(30 * 24 * 60 * 60),
            credentials: Maybe::None,
            filesystem_fallback: Maybe::Some(cache_dir.into()),
            redis_url: Maybe::None,
        }
    }

    /// Create configuration for Redis-backed caching
    #[cfg(feature = "redis-cache")]
    pub fn redis(redis_url: impl Into<Text>) -> Self {
        Self {
            storage_url: "".into(),
            trust_level: TrustLevel::Signatures,
            max_age: Duration::from_secs(30 * 24 * 60 * 60),
            credentials: Maybe::None,
            filesystem_fallback: Maybe::Some(".verum/cache".into()),
            redis_url: Maybe::Some(redis_url.into()),
        }
    }

    /// Set trust level
    pub fn with_trust_level(mut self, level: TrustLevel) -> Self {
        self.trust_level = level;
        self
    }

    /// Set maximum age
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Set credentials
    pub fn with_credentials(mut self, creds: CacheCredentials) -> Self {
        self.credentials = Maybe::Some(creds);
        self
    }

    /// Set filesystem fallback path
    pub fn with_filesystem_fallback(mut self, path: impl Into<Text>) -> Self {
        self.filesystem_fallback = Maybe::Some(path.into());
        self
    }

    /// Set Redis URL
    pub fn with_redis_url(mut self, url: impl Into<Text>) -> Self {
        self.redis_url = Maybe::Some(url.into());
        self
    }

    /// Disable filesystem fallback
    pub fn without_filesystem_fallback(mut self) -> Self {
        self.filesystem_fallback = Maybe::None;
        self
    }
}

/// Trust level for cached results
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrustLevel {
    /// No verification of cached results
    None,
    /// Verify cryptographic signature
    Signatures,
    /// Re-verify a sample of cached results
    Sampling { sample_rate: f64 },
}

/// Cache credentials for S3-compatible storage
#[derive(Debug, Clone)]
pub struct CacheCredentials {
    pub access_key: Text,
    pub secret_key: Text,
    pub region: Maybe<Text>,
}

impl CacheCredentials {
    /// Create new credentials
    pub fn new(access_key: impl Into<Text>, secret_key: impl Into<Text>) -> Self {
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            region: Maybe::None,
        }
    }

    /// Set region
    pub fn with_region(mut self, region: impl Into<Text>) -> Self {
        self.region = Maybe::Some(region.into());
        self
    }
}

// ==================== Cache Entry ====================

/// Cache entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Unique key (hash of: file content + function signature + verification mode)
    pub key: Text,
    /// Verification result
    pub result: CachedResult,
    /// Entry metadata
    pub metadata: EntryMetadata,
}

impl CacheEntry {
    /// Create new cache entry
    pub fn new(key: impl Into<Text>, result: CachedResult, metadata: EntryMetadata) -> Self {
        Self {
            key: key.into(),
            result,
            metadata,
        }
    }

    /// Check if entry is expired
    pub fn is_expired(&self, max_age: Duration) -> bool {
        let now = current_timestamp();
        let age = now.saturating_sub(self.metadata.cached_at);
        age > max_age.as_secs()
    }
}

/// Entry metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMetadata {
    /// When this was cached (Unix timestamp)
    pub cached_at: u64,
    /// Verum version used
    pub verum_version: Text,
    /// SMT solver version
    pub solver_version: Text,
    /// Cryptographic signature (if trust_level = Signatures)
    pub signature: Maybe<Text>,
    /// Time saved by cache hit (original verification time in ms)
    pub original_time_ms: u64,
}

impl EntryMetadata {
    /// Create new metadata
    pub fn new(original_time_ms: u64) -> Self {
        Self {
            cached_at: current_timestamp(),
            verum_version: env!("CARGO_PKG_VERSION").into(),
            solver_version: get_z3_version(),
            signature: Maybe::None,
            original_time_ms,
        }
    }

    /// With signature
    pub fn with_signature(mut self, signature: impl Into<Text>) -> Self {
        self.signature = Maybe::Some(signature.into());
        self
    }
}

/// Cached verification result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CachedResult {
    /// SMT proof succeeded
    Proved,
    /// SMT solver found counterexample
    Counterexample { value: Text },
    /// Solver timeout
    Timeout,
    /// Solver returned "unknown"
    Unknown,
}

impl CachedResult {
    /// Create counterexample result
    pub fn counterexample(value: impl Into<Text>) -> Self {
        Self::Counterexample {
            value: value.into(),
        }
    }
}

// ==================== Cache Statistics ====================

/// Cache performance statistics
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Remote cache hits
    pub remote_hits: usize,
    /// Remote cache misses
    pub remote_misses: usize,
    /// Local cache hits
    pub local_hits: usize,
    /// Number of uploads
    pub uploads: usize,
    /// Total download time (milliseconds)
    pub download_time_ms: u64,
    /// Total upload time (milliseconds)
    pub upload_time_ms: u64,
}

impl CacheStats {
    /// Get total hits (local + remote)
    pub fn total_hits(&self) -> usize {
        self.local_hits + self.remote_hits
    }

    /// Get cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.total_hits() + self.remote_misses;
        if total == 0 {
            0.0
        } else {
            self.total_hits() as f64 / total as f64
        }
    }

    /// Get average download time per hit (milliseconds)
    pub fn avg_download_time_ms(&self) -> f64 {
        if self.remote_hits == 0 {
            0.0
        } else {
            self.download_time_ms as f64 / self.remote_hits as f64
        }
    }

    /// Get average upload time (milliseconds)
    pub fn avg_upload_time_ms(&self) -> f64 {
        if self.uploads == 0 {
            0.0
        } else {
            self.upload_time_ms as f64 / self.uploads as f64
        }
    }

    /// Format statistics for display
    pub fn format_report(&self) -> Text {
        format!(
            "Distributed Cache Statistics:\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             Remote hits:      {} ({:.1}% hit rate)\n\
             Remote misses:    {}\n\
             Local hits:       {}\n\
             Uploads:          {}\n\
             Avg download:     {:.1}ms\n\
             Avg upload:       {:.1}ms\n",
            self.remote_hits,
            self.hit_rate() * 100.0,
            self.remote_misses,
            self.local_hits,
            self.uploads,
            self.avg_download_time_ms(),
            self.avg_upload_time_ms()
        ).into()
    }
}

// ==================== Distributed Cache ====================

/// Distributed cache backend
pub struct DistributedCache {
    config: DistributedCacheConfig,
    local_cache: Arc<RwLock<Map<Text, CacheEntry>>>,
    stats: Arc<RwLock<CacheStats>>,
    signing_key: Maybe<SigningKey>,
    verifying_key: Maybe<VerifyingKey>,
    #[cfg(feature = "distributed-cache")]
    http_client: Client,
    #[cfg(feature = "redis-cache")]
    redis_client: Maybe<RedisClient>,
}

impl DistributedCache {
    /// Create new distributed cache
    pub fn new(config: DistributedCacheConfig) -> Self {
        let (signing_key, verifying_key) = if config.trust_level == TrustLevel::Signatures {
            let sk = generate_signing_key();
            let vk = sk.verifying_key();
            (Maybe::Some(sk), Maybe::Some(vk))
        } else {
            (Maybe::None, Maybe::None)
        };

        // Initialize Redis client if configured
        #[cfg(feature = "redis-cache")]
        let redis_client = if let Maybe::Some(ref url) = config.redis_url {
            match RedisClient::open(url.as_str()) {
                Ok(client) => {
                    tracing::info!("Redis cache client initialized for: {}", url);
                    Maybe::Some(client)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize Redis client: {}. Falling back to filesystem cache.",
                        e
                    );
                    Maybe::None
                }
            }
        } else {
            Maybe::None
        };

        // Ensure filesystem cache directory exists if configured
        if let Maybe::Some(ref cache_dir) = config.filesystem_fallback {
            if let Err(e) = std::fs::create_dir_all(cache_dir.as_str()) {
                tracing::warn!(
                    "Failed to create filesystem cache directory '{}': {}",
                    cache_dir,
                    e
                );
            }
        }

        Self {
            config,
            local_cache: Arc::new(RwLock::new(Map::new())),
            stats: Arc::new(RwLock::new(CacheStats::default())),
            signing_key,
            verifying_key,
            #[cfg(feature = "distributed-cache")]
            http_client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            #[cfg(feature = "redis-cache")]
            redis_client,
        }
    }

    /// Look up verification result
    ///
    /// First checks local cache, then falls back to remote storage.
    pub async fn get(&self, key: &str) -> Maybe<CacheEntry> {
        // 1. Check local cache first
        {
            let cache = self.local_cache.read().unwrap();
            let key_text: Text = key.to_string().into();
            if let Maybe::Some(entry) = cache.get(&key_text) {
                // Check if expired
                if !entry.is_expired(self.config.max_age) {
                    self.stats.write().unwrap().local_hits += 1;
                    return Maybe::Some(entry.clone());
                }
            }
        }

        // 2. Fetch from remote
        let start = std::time::Instant::now();
        match self.fetch_remote(key).await {
            Ok(entry) => {
                let elapsed = start.elapsed().as_millis() as u64;

                // Verify entry integrity
                if self.verify_entry(&entry) {
                    // Store in local cache
                    self.local_cache
                        .write()
                        .unwrap()
                        .insert(key.to_string().into(), entry.clone());

                    // Update stats
                    let mut stats = self.stats.write().unwrap();
                    stats.remote_hits += 1;
                    stats.download_time_ms += elapsed;

                    Maybe::Some(entry)
                } else {
                    tracing::warn!("Cache entry verification failed for key: {}", key);
                    self.stats.write().unwrap().remote_misses += 1;
                    Maybe::None
                }
            }
            Err(e) => {
                tracing::debug!("Cache miss for key {}: {}", key, e);
                self.stats.write().unwrap().remote_misses += 1;
                Maybe::None
            }
        }
    }

    /// Store verification result
    pub async fn put(
        &self,
        key: &str,
        result: CachedResult,
        time_ms: u64,
    ) -> Result<(), DistributedCacheError> {
        let mut metadata = EntryMetadata::new(time_ms);

        // Sign entry if required
        if let Maybe::Some(ref signing_key) = self.signing_key {
            let signature = self.sign_entry(key, &result, signing_key);
            metadata = metadata.with_signature(signature);
        }

        let entry = CacheEntry::new(key, result, metadata);

        // Store locally
        self.local_cache
            .write()
            .unwrap()
            .insert(key.to_string().into(), entry.clone());

        // Upload to remote
        let start = std::time::Instant::now();
        self.upload_remote(&entry).await?;
        let elapsed = start.elapsed().as_millis() as u64;

        // Update stats
        let mut stats = self.stats.write().unwrap();
        stats.uploads += 1;
        stats.upload_time_ms += elapsed;

        Ok(())
    }

    /// Verify entry integrity based on trust level
    fn verify_entry(&self, entry: &CacheEntry) -> bool {
        match self.config.trust_level {
            TrustLevel::None => true,
            TrustLevel::Signatures => {
                if let Maybe::Some(ref sig) = entry.metadata.signature {
                    // Register our verifying key for this verification
                    if let Maybe::Some(ref vk) = self.verifying_key {
                        set_session_verifying_key(*vk);
                    }
                    verify_signature(&entry.key, &entry.result, sig)
                } else {
                    tracing::warn!(
                        "Cache entry {} has no signature but trust level requires signatures",
                        entry.key
                    );
                    false
                }
            }
            TrustLevel::Sampling { sample_rate } => {
                // Probabilistically verify - sample_rate% of entries are verified
                use rand::RngExt;
                let mut rng = rand::rng();
                if rng.random_bool(sample_rate) {
                    // Verify this entry
                    if let Maybe::Some(ref sig) = entry.metadata.signature {
                        if let Maybe::Some(ref vk) = self.verifying_key {
                            set_session_verifying_key(*vk);
                        }
                        verify_signature(&entry.key, &entry.result, sig)
                    } else {
                        true // No signature to verify in sampling mode is ok
                    }
                } else {
                    true // Skip verification for this sample
                }
            }
        }
    }

    /// Sign entry for integrity verification
    fn sign_entry(&self, key: &str, result: &CachedResult, signing_key: &SigningKey) -> Text {
        use ed25519_dalek::Signer;

        // Create canonical representation for signing
        let canonical = format!(
            "{}:{}:{}",
            key,
            serde_json::to_string(result).unwrap(),
            env!("CARGO_PKG_VERSION")
        );

        let signature = signing_key.sign(canonical.as_bytes());
        hex::encode(signature.to_bytes()).into()
    }

    /// Fetch from S3-compatible storage
    ///
    /// Falls back to filesystem cache if S3 credentials are not available.
    #[cfg(feature = "distributed-cache")]
    async fn fetch_remote(&self, key: &str) -> Result<CacheEntry, DistributedCacheError> {
        tracing::debug!(
            "Fetching cache entry from {}/{}",
            self.config.storage_url,
            key
        );

        // Check if credentials are available; if not, fall back to filesystem
        let credentials = match self.get_credentials() {
            Ok(creds) => creds,
            Err(_) => {
                tracing::debug!(
                    "No S3 credentials available, trying filesystem cache for key: {}",
                    key
                );
                return self.fetch_from_filesystem(key);
            }
        };
        let (access_key, secret_key, region) = credentials;

        let (bucket, prefix, endpoint) = parse_s3_url(&self.config.storage_url)?;
        let object_key = format!("{}/{}.json", prefix, key);

        // Build signed request
        let url = format!("{}/{}/{}", endpoint, bucket, object_key);
        let now = chrono::Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Create canonical request for AWS Signature V4
        let host = extract_host(&endpoint);
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let payload_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"; // Empty payload hash

        let canonical_request = format!(
            "GET\n/{}/{}\n\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n\n{}\n{}",
            bucket, object_key, host, payload_hash, amz_date, signed_headers, payload_hash
        );

        let scope = format!("{}/{}/s3/aws4_request", date_stamp, region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            scope,
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        let signature = calculate_signature(&secret_key, &date_stamp, &region, &string_to_sign);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            access_key, scope, signed_headers, signature
        );

        let response = self
            .http_client
            .get(&url)
            .header("Host", host.as_str())
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", &authorization)
            .send()
            .await
            .map_err(|e| {
                DistributedCacheError::S3Error(format!("HTTP request failed: {}", e).into())
            })?;

        if !response.status().is_success() {
            return Err(DistributedCacheError::S3Error(
                format!("S3 GET failed with status {}: {}", response.status(), key).into(),
            ));
        }

        let data = response.bytes().await.map_err(|e| {
            DistributedCacheError::S3Error(format!("Failed to read response: {}", e).into())
        })?;

        let entry: CacheEntry = serde_json::from_slice(&data)?;
        Ok(entry)
    }

    /// Fetch from remote storage (filesystem fallback when S3 feature disabled)
    ///
    /// When the `distributed-cache` feature is disabled, this falls back to:
    /// 1. Redis (if `redis-cache` feature enabled and configured)
    /// 2. Filesystem cache (if `filesystem_fallback` configured)
    ///
    /// # Enabling S3 Support
    /// ```toml
    /// verum_smt = { version = "*", features = ["distributed-cache"] }
    /// ```
    #[cfg(not(feature = "distributed-cache"))]
    async fn fetch_remote(&self, key: &str) -> Result<CacheEntry, DistributedCacheError> {
        // Try Redis first if configured
        #[cfg(feature = "redis-cache")]
        if let Maybe::Some(ref client) = self.redis_client {
            match self.fetch_from_redis(client, key).await {
                Ok(entry) => {
                    tracing::debug!("Redis cache hit for key: {}", key);
                    return Ok(entry);
                }
                Err(e) => {
                    tracing::debug!("Redis cache miss for key {}: {}", key, e);
                }
            }
        }

        // Fall back to filesystem cache
        self.fetch_from_filesystem(key)
    }

    /// Upload to S3-compatible storage
    ///
    /// If credentials are not available (neither in config nor environment),
    /// this method falls back to filesystem cache. This is useful for development
    /// and testing without S3 access.
    #[cfg(feature = "distributed-cache")]
    async fn upload_remote(&self, entry: &CacheEntry) -> Result<(), DistributedCacheError> {
        tracing::debug!(
            "Uploading cache entry to {}/{}",
            self.config.storage_url,
            entry.key
        );

        // Always persist to filesystem as backup
        if let Err(e) = self.upload_to_filesystem(entry) {
            tracing::warn!("Filesystem backup failed for key {}: {}", entry.key, e);
        }

        // Check if credentials are available; if not, we're done (filesystem-only mode)
        let credentials = match self.get_credentials() {
            Ok(creds) => creds,
            Err(_) => {
                tracing::debug!(
                    "No S3 credentials available, operating in filesystem-only mode for key: {}",
                    entry.key
                );
                return Ok(());
            }
        };
        let (access_key, secret_key, region) = credentials;

        let (bucket, prefix, endpoint) = parse_s3_url(&self.config.storage_url)?;
        let object_key = format!("{}/{}.json", prefix, entry.key);
        let data = serde_json::to_vec(entry)?;

        // Build signed request
        let url = format!("{}/{}/{}", endpoint, bucket, object_key);
        let now = chrono::Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Create canonical request for AWS Signature V4
        let host = extract_host(&endpoint);
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let payload_hash = hex::encode(Sha256::digest(&data));

        let canonical_request = format!(
            "PUT\n/{}/{}\n\ncontent-type:application/json\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n\n{}\n{}",
            bucket, object_key, host, payload_hash, amz_date, signed_headers, payload_hash
        );

        let scope = format!("{}/{}/s3/aws4_request", date_stamp, region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            scope,
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        let signature = calculate_signature(&secret_key, &date_stamp, &region, &string_to_sign);
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            access_key, scope, signed_headers, signature
        );

        let response = self
            .http_client
            .put(&url)
            .header("Host", host.as_str())
            .header("Content-Type", "application/json")
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", &payload_hash)
            .header("Authorization", &authorization)
            .body(data)
            .send()
            .await
            .map_err(|e| {
                DistributedCacheError::S3Error(format!("HTTP request failed: {}", e).into())
            })?;

        if !response.status().is_success() {
            return Err(DistributedCacheError::S3Error(
                format!(
                    "S3 PUT failed with status {}: {}",
                    response.status(),
                    entry.key
                )
                .into(),
            ));
        }

        Ok(())
    }

    /// Upload to remote storage (filesystem fallback when S3 feature disabled)
    ///
    /// When the `distributed-cache` feature is disabled, this stores to:
    /// 1. Redis (if `redis-cache` feature enabled and configured)
    /// 2. Filesystem cache (if `filesystem_fallback` configured)
    ///
    /// # Enabling S3 Support
    /// ```toml
    /// verum_smt = { version = "*", features = ["distributed-cache"] }
    /// ```
    #[cfg(not(feature = "distributed-cache"))]
    async fn upload_remote(&self, entry: &CacheEntry) -> Result<(), DistributedCacheError> {
        // Try Redis first if configured
        #[cfg(feature = "redis-cache")]
        if let Maybe::Some(ref client) = self.redis_client {
            match self.upload_to_redis(client, entry).await {
                Ok(()) => {
                    tracing::debug!("Uploaded to Redis cache: {}", entry.key);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to upload to Redis cache: {}. Falling back to filesystem.",
                        e
                    );
                }
            }
        }

        // Always persist to filesystem as backup
        self.upload_to_filesystem(entry)
    }

    /// Get AWS credentials from config or environment
    #[cfg(feature = "distributed-cache")]
    fn get_credentials(&self) -> Result<(String, String, String), DistributedCacheError> {
        if let Maybe::Some(ref creds) = self.config.credentials {
            let region = match &creds.region {
                Maybe::Some(r) => r.to_string(),
                Maybe::None => {
                    std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string())
                }
            };
            Ok((
                creds.access_key.to_string(),
                creds.secret_key.to_string(),
                region,
            ))
        } else {
            // Try environment variables
            let access_key = std::env::var("AWS_ACCESS_KEY_ID")
                .map_err(|_| DistributedCacheError::S3Error("AWS_ACCESS_KEY_ID not set".into()))?;
            let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| {
                DistributedCacheError::S3Error("AWS_SECRET_ACCESS_KEY not set".into())
            })?;
            let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
            Ok((access_key, secret_key, region))
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        self.stats.read().unwrap().clone()
    }

    /// Calculate time saved by cache hits
    pub fn time_saved(&self) -> Duration {
        let cache = self.local_cache.read().unwrap();
        let total_ms: u64 = cache
            .values()
            .map(|entry| entry.metadata.original_time_ms)
            .sum();
        Duration::from_millis(total_ms)
    }

    /// Clear all cached entries (local only)
    pub fn clear_local(&self) {
        self.local_cache.write().unwrap().clear();
    }

    /// Get number of entries in local cache
    pub fn local_size(&self) -> usize {
        self.local_cache.read().unwrap().len()
    }

    // ==================== Filesystem Backend ====================

    /// Fetch entry from filesystem cache
    fn fetch_from_filesystem(&self, key: &str) -> Result<CacheEntry, DistributedCacheError> {
        let cache_dir = match &self.config.filesystem_fallback {
            Maybe::Some(dir) => dir,
            Maybe::None => {
                return Err(DistributedCacheError::Other(
                    "No filesystem cache configured. Set filesystem_fallback in config.".into(),
                ));
            }
        };

        let cache_path = self.get_cache_file_path(cache_dir, key);

        // Check if file exists
        if !cache_path.exists() {
            return Err(DistributedCacheError::Other(
                format!("Cache entry not found: {}", key).into(),
            ));
        }

        // Check file age (expiration)
        if let Ok(metadata) = std::fs::metadata(&cache_path) {
            if let Ok(modified) = metadata.modified() {
                let age = SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or(Duration::ZERO);
                if age > self.config.max_age {
                    // Entry expired - remove and return miss
                    let _ = std::fs::remove_file(&cache_path);
                    return Err(DistributedCacheError::Expired);
                }
            }
        }

        // Read and deserialize
        let data = std::fs::read(&cache_path).map_err(|e| {
            DistributedCacheError::Other(format!("Failed to read cache file: {}", e).into())
        })?;

        let entry: CacheEntry = serde_json::from_slice(&data)?;

        tracing::debug!("Filesystem cache hit for key: {}", key);
        Ok(entry)
    }

    /// Upload entry to filesystem cache
    fn upload_to_filesystem(&self, entry: &CacheEntry) -> Result<(), DistributedCacheError> {
        let cache_dir = match &self.config.filesystem_fallback {
            Maybe::Some(dir) => dir,
            Maybe::None => {
                tracing::debug!("No filesystem cache configured, skipping persistence");
                return Ok(());
            }
        };

        let cache_path = self.get_cache_file_path(cache_dir, &entry.key);

        // Ensure the full directory path exists (including subdirectory)
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DistributedCacheError::Other(
                    format!(
                        "Failed to create cache directory '{}': {}",
                        parent.display(),
                        e
                    ).into(),
                )
            })?;
        }

        // Serialize entry
        let data = serde_json::to_vec_pretty(entry)?;

        // Write atomically using temp file
        let temp_path = cache_path.with_extension("tmp");
        std::fs::write(&temp_path, &data).map_err(|e| {
            DistributedCacheError::Other(format!("Failed to write cache file: {}", e).into())
        })?;

        std::fs::rename(&temp_path, &cache_path).map_err(|e| {
            DistributedCacheError::Other(format!("Failed to rename cache file: {}", e).into())
        })?;

        tracing::debug!("Persisted to filesystem cache: {}", entry.key);
        Ok(())
    }

    /// Get the filesystem path for a cache key
    fn get_cache_file_path(&self, cache_dir: &str, key: &str) -> PathBuf {
        // Use first 2 characters of key as subdirectory to avoid too many files in one dir
        let subdir = if key.len() >= 2 { &key[..2] } else { "00" };
        PathBuf::from(cache_dir)
            .join(subdir)
            .join(format!("{}.json", key))
    }

    /// Clear filesystem cache entries
    pub fn clear_filesystem_cache(&self) -> Result<usize, DistributedCacheError> {
        let cache_dir = match &self.config.filesystem_fallback {
            Maybe::Some(dir) => dir,
            Maybe::None => return Ok(0),
        };

        let cache_dir_path = PathBuf::from(cache_dir.as_str());
        if !cache_dir_path.exists() {
            return Ok(0);
        }

        let mut removed = 0;
        for entry in std::fs::read_dir(&cache_dir_path).map_err(|e| {
            DistributedCacheError::Other(format!("Failed to read cache directory: {}", e).into())
        })? {
            if let Ok(entry) = entry {
                if entry.path().is_dir() {
                    // Subdirectory - iterate and remove files
                    if let Ok(subdir) = std::fs::read_dir(entry.path()) {
                        for subentry in subdir.flatten() {
                            if subentry.path().extension().is_some_and(|e| e == "json") {
                                if std::fs::remove_file(subentry.path()).is_ok() {
                                    removed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("Cleared {} entries from filesystem cache", removed);
        Ok(removed)
    }

    /// Get filesystem cache size (number of entries)
    pub fn filesystem_cache_size(&self) -> usize {
        let cache_dir = match &self.config.filesystem_fallback {
            Maybe::Some(dir) => dir,
            Maybe::None => return 0,
        };

        let cache_dir_path = PathBuf::from(cache_dir.as_str());
        if !cache_dir_path.exists() {
            return 0;
        }

        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(&cache_dir_path) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Ok(subdir) = std::fs::read_dir(entry.path()) {
                        count += subdir
                            .flatten()
                            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                            .count();
                    }
                }
            }
        }
        count
    }

    // ==================== Redis Backend ====================

    /// Fetch entry from Redis cache
    #[cfg(feature = "redis-cache")]
    async fn fetch_from_redis(
        &self,
        client: &RedisClient,
        key: &str,
    ) -> Result<CacheEntry, DistributedCacheError> {
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(
                    format!("Failed to connect to Redis: {}", e).into(),
                )
            })?;

        let redis_key = self.get_redis_key(key);
        let data: Option<Vec<u8>> = conn.get(&redis_key).await.map_err(|e| {
            DistributedCacheError::RedisError(format!("Redis GET failed: {}", e).into())
        })?;

        match data {
            Some(bytes) => {
                let entry: CacheEntry = serde_json::from_slice(&bytes)?;

                // Check if expired
                if entry.is_expired(self.config.max_age) {
                    // Delete expired entry
                    let _: () = conn.del(&redis_key).await.unwrap_or(());
                    return Err(DistributedCacheError::Expired);
                }

                Ok(entry)
            }
            None => Err(DistributedCacheError::Other(
                format!("Key not found in Redis: {}", key).into(),
            )),
        }
    }

    /// Upload entry to Redis cache
    #[cfg(feature = "redis-cache")]
    async fn upload_to_redis(
        &self,
        client: &RedisClient,
        entry: &CacheEntry,
    ) -> Result<(), DistributedCacheError> {
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(
                    format!("Failed to connect to Redis: {}", e).into(),
                )
            })?;

        let redis_key = self.get_redis_key(&entry.key);
        let data = serde_json::to_vec(entry)?;
        let ttl_secs = self.config.max_age.as_secs() as i64;

        // Set with expiration
        let _: () = conn
            .set_ex(&redis_key, data, ttl_secs as u64)
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(format!("Redis SETEX failed: {}", e).into())
            })?;

        Ok(())
    }

    /// Get Redis key with namespace prefix
    #[cfg(feature = "redis-cache")]
    fn get_redis_key(&self, key: &str) -> String {
        format!("verum:smt:cache:{}", key)
    }

    /// Clear all entries from Redis cache
    #[cfg(feature = "redis-cache")]
    pub async fn clear_redis_cache(&self) -> Result<usize, DistributedCacheError> {
        let client = match &self.redis_client {
            Maybe::Some(c) => c,
            Maybe::None => return Ok(0),
        };

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(
                    format!("Failed to connect to Redis: {}", e).into(),
                )
            })?;

        // Find all keys with our prefix
        let pattern = "verum:smt:cache:*";
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(format!("Redis KEYS failed: {}", e).into())
            })?;

        let count = keys.len();
        if !keys.is_empty() {
            let _: () = conn.del(keys).await.map_err(|e| {
                DistributedCacheError::RedisError(format!("Redis DEL failed: {}", e).into())
            })?;
        }

        tracing::info!("Cleared {} entries from Redis cache", count);
        Ok(count)
    }

    /// Test Redis connection
    #[cfg(feature = "redis-cache")]
    pub async fn test_redis_connection(&self) -> Result<bool, DistributedCacheError> {
        let client = match &self.redis_client {
            Maybe::Some(c) => c,
            Maybe::None => return Ok(false),
        };

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(
                    format!("Failed to connect to Redis: {}", e).into(),
                )
            })?;

        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                DistributedCacheError::RedisError(format!("Redis PING failed: {}", e).into())
            })?;

        Ok(pong == "PONG")
    }
}

// ==================== Helper Functions ====================

/// Generate cache key from components
pub fn generate_cache_key(file_hash: &str, func_sig: &str, mode: &str) -> Text {
    let mut hasher = Sha256::new();
    hasher.update(file_hash.as_bytes());
    hasher.update(b":");
    hasher.update(func_sig.as_bytes());
    hasher.update(b":");
    hasher.update(mode.as_bytes());
    hex::encode(hasher.finalize()).into()
}

/// Verify cryptographic signature using Ed25519
///
/// SECURITY: This performs actual Ed25519 signature verification.
/// Signatures are created using the signing key and verified using the
/// corresponding verifying key. Invalid or tampered signatures will fail.
fn verify_signature(key: &str, result: &CachedResult, signature: &str) -> bool {
    use ed25519_dalek::{Signature, Verifier};

    tracing::debug!("Verifying Ed25519 signature for key: {}", key);

    // Decode hex signature
    let sig_bytes = match hex::decode(signature) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!("Failed to decode signature hex: {}", e);
            return false;
        }
    };

    // Signature must be exactly 64 bytes
    if sig_bytes.len() != 64 {
        tracing::warn!(
            "Invalid signature length: {} (expected 64)",
            sig_bytes.len()
        );
        return false;
    }

    // Create signature from bytes
    let sig_array: [u8; 64] = match sig_bytes.try_into() {
        Ok(arr) => arr,
        Err(_) => return false,
    };
    let signature_obj = Signature::from_bytes(&sig_array);

    // Create canonical representation (must match sign_entry)
    let canonical = format!(
        "{}:{}:{}",
        key,
        serde_json::to_string(result).unwrap_or_default(),
        env!("CARGO_PKG_VERSION")
    );

    // Get the global verifying key for this session
    // In production, this would be loaded from a trusted source or the cache entry metadata
    // For now, we use a thread-local key registry
    if let Some(vk) = get_session_verifying_key() {
        match vk.verify(canonical.as_bytes(), &signature_obj) {
            Ok(()) => {
                tracing::debug!("Signature verification succeeded for key: {}", key);
                true
            }
            Err(e) => {
                tracing::warn!("Signature verification failed for key {}: {}", key, e);
                false
            }
        }
    } else {
        // No verifying key available - this is a security issue in production
        tracing::warn!("No verifying key available for signature verification");
        false
    }
}

// Thread-local storage for session verifying key
std::thread_local! {
    static SESSION_VERIFYING_KEY: std::cell::RefCell<Option<VerifyingKey>> = const { std::cell::RefCell::new(None) };
}

/// Set the session verifying key for signature verification
pub fn set_session_verifying_key(vk: VerifyingKey) {
    SESSION_VERIFYING_KEY.with(|cell| {
        *cell.borrow_mut() = Some(vk);
    });
}

/// Get the session verifying key
fn get_session_verifying_key() -> Option<VerifyingKey> {
    SESSION_VERIFYING_KEY.with(|cell| *cell.borrow())
}

/// Parse S3 URL into (bucket, prefix, endpoint)
///
/// Supports:
/// - `s3://bucket/prefix` → (bucket, prefix, https://s3.amazonaws.com)
/// - `s3://bucket/prefix?endpoint=https://custom.endpoint` → with custom endpoint
/// - `https://s3.region.amazonaws.com/bucket/prefix` → regional endpoint
#[cfg(feature = "distributed-cache")]
fn parse_s3_url(url: &str) -> Result<(String, String, String), DistributedCacheError> {
    if url.starts_with("s3://") {
        // s3://bucket/prefix format
        let path = &url[5..];
        let parts: Vec<&str> = path.splitn(2, '/').collect();

        if parts.is_empty() {
            return Err(DistributedCacheError::S3Error(
                "Invalid S3 URL: no bucket".into(),
            ));
        }

        let bucket = parts[0].to_string();
        let prefix = if parts.len() > 1 {
            parts[1].split('?').next().unwrap_or("").to_string()
        } else {
            String::new()
        };

        // Check for custom endpoint in query string
        let endpoint = if url.contains("endpoint=") {
            url.split("endpoint=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .unwrap_or("https://s3.amazonaws.com")
                .to_string()
        } else {
            "https://s3.amazonaws.com".to_string()
        };

        Ok((bucket, prefix, endpoint))
    } else if url.starts_with("https://") {
        // https://s3.region.amazonaws.com/bucket/prefix format
        let without_scheme = &url[8..];
        let slash_pos = without_scheme.find('/').unwrap_or(without_scheme.len());
        let host = &without_scheme[..slash_pos];
        let path = &without_scheme[slash_pos..];

        let path_parts: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();

        if path_parts.is_empty() {
            return Err(DistributedCacheError::S3Error(
                "Invalid S3 URL: no bucket in path".into(),
            ));
        }

        let bucket = path_parts[0].to_string();
        let prefix = if path_parts.len() > 1 {
            path_parts[1].to_string()
        } else {
            String::new()
        };
        let endpoint = format!("https://{}", host);

        Ok((bucket, prefix, endpoint))
    } else {
        Err(DistributedCacheError::S3Error(
            format!("Unsupported URL scheme: {}", url).into(),
        ))
    }
}

/// Extract host from URL
#[cfg(feature = "distributed-cache")]
fn extract_host(url: &str) -> Text {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .into()
}

/// Calculate AWS Signature V4
#[cfg(feature = "distributed-cache")]
fn calculate_signature(
    secret_key: &str,
    date_stamp: &str,
    region: &str,
    string_to_sign: &str,
) -> Text {
    type HmacSha256 = Hmac<Sha256>;

    // Step 1: kDate = HMAC("AWS4" + kSecret, Date)
    let k_secret = format!("AWS4{}", secret_key);
    let mut mac =
        HmacSha256::new_from_slice(k_secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(date_stamp.as_bytes());
    let k_date = mac.finalize().into_bytes();

    // Step 2: kRegion = HMAC(kDate, Region)
    let mut mac = HmacSha256::new_from_slice(&k_date).expect("HMAC can take key of any size");
    mac.update(region.as_bytes());
    let k_region = mac.finalize().into_bytes();

    // Step 3: kService = HMAC(kRegion, "s3")
    let mut mac = HmacSha256::new_from_slice(&k_region).expect("HMAC can take key of any size");
    mac.update(b"s3");
    let k_service = mac.finalize().into_bytes();

    // Step 4: kSigning = HMAC(kService, "aws4_request")
    let mut mac = HmacSha256::new_from_slice(&k_service).expect("HMAC can take key of any size");
    mac.update(b"aws4_request");
    let k_signing = mac.finalize().into_bytes();

    // Step 5: Signature = HMAC(kSigning, StringToSign)
    let mut mac = HmacSha256::new_from_slice(&k_signing).expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    let signature = mac.finalize().into_bytes();

    hex::encode(signature).into()
}

/// Get current Unix timestamp
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Get Z3 version string
fn get_z3_version() -> Text {
    // Use z3 crate's full_version API to get actual version
    z3::full_version().to_string().into()
}

// ==================== Signing Key Management ====================

type SigningKey = ed25519_dalek::SigningKey;
type VerifyingKey = ed25519_dalek::VerifyingKey;

/// Generate new signing key
fn generate_signing_key() -> SigningKey {
    use ed25519_dalek::SigningKey;
    use rand::RngExt;

    // Generate 32 random bytes for the signing key
    let mut seed = [0u8; 32];
    let mut rng = rand::rng();
    rng.fill(&mut seed);

    SigningKey::from_bytes(&seed)
}

// ==================== Error Handling ====================

/// Distributed cache errors
#[derive(Debug, thiserror::Error)]
pub enum DistributedCacheError {
    /// S3 operation failed
    #[error("S3 operation failed: {0}")]
    S3Error(Text),

    /// Redis operation failed
    #[error("Redis operation failed: {0}")]
    RedisError(Text),

    /// Filesystem operation failed
    #[error("Filesystem operation failed: {0}")]
    FilesystemError(Text),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Signature verification failed
    #[error("Signature verification failed")]
    InvalidSignature,

    /// Entry expired
    #[error("Cache entry expired")]
    Expired,

    /// Not yet implemented
    ///
    /// This error is returned when a feature is not available.
    /// Enable the corresponding feature flag to use this functionality:
    /// - `distributed-cache`: S3-compatible storage backend
    /// - `redis-cache`: Redis caching backend
    #[error("Not implemented: {0}")]
    NotImplemented(Text),

    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionError(Text),

    /// Other error
    #[error("Cache error: {0}")]
    Other(Text),
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::ToText;

    #[test]
    fn test_cache_entry_creation() {
        let metadata = EntryMetadata::new(1500);
        let entry = CacheEntry::new("test-key", CachedResult::Proved, metadata);

        assert_eq!(entry.key, "test-key".to_text());
        assert_eq!(entry.result, CachedResult::Proved);
        assert_eq!(entry.metadata.original_time_ms, 1500);
    }

    #[test]
    fn test_cache_entry_expiration() {
        let metadata = EntryMetadata {
            cached_at: current_timestamp() - 31 * 24 * 60 * 60, // 31 days ago
            verum_version: "1.0.0".into(),
            solver_version: "4.8.10".into(),
            signature: Maybe::None,
            original_time_ms: 1000,
        };

        let entry = CacheEntry::new("old-key", CachedResult::Proved, metadata);
        let max_age = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

        assert!(entry.is_expired(max_age));
    }

    #[test]
    fn test_generate_cache_key() {
        let key1 = generate_cache_key("file1", "fn foo() -> Int", "proof");
        let key2 = generate_cache_key("file1", "fn foo() -> Int", "proof");
        let key3 = generate_cache_key("file2", "fn foo() -> Int", "proof");

        assert_eq!(key1, key2); // Same inputs produce same key
        assert_ne!(key1, key3); // Different inputs produce different keys
    }

    #[test]
    fn test_cache_stats() {
        let mut stats = CacheStats::default();
        stats.local_hits = 10;
        stats.remote_hits = 5;
        stats.remote_misses = 2;

        assert_eq!(stats.total_hits(), 15);
        assert_eq!(stats.hit_rate(), 15.0 / 17.0);
    }

    #[tokio::test]
    async fn test_distributed_cache_local_get() {
        // Use a temp directory for filesystem fallback to avoid permission issues
        let temp_dir = std::env::temp_dir().join("verum_smt_test_cache");
        let config = DistributedCacheConfig::new("s3://test-bucket/cache")
            .with_filesystem_fallback(temp_dir.to_string_lossy().to_string());
        let cache = DistributedCache::new(config);

        // Store entry locally
        cache
            .put("test-key", CachedResult::Proved, 100)
            .await
            .unwrap();

        // Retrieve from local cache
        let entry = cache.get("test-key").await;
        assert!(matches!(entry, Maybe::Some(_)));

        let stats = cache.stats();
        assert_eq!(stats.local_hits, 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_trust_level_none() {
        let config =
            DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

        let cache = DistributedCache::new(config);

        let entry = CacheEntry::new("test", CachedResult::Proved, EntryMetadata::new(100));

        assert!(cache.verify_entry(&entry));
    }

    #[test]
    fn test_cached_result_variants() {
        let proved = CachedResult::Proved;
        let counterex = CachedResult::counterexample("x = -5");
        let timeout = CachedResult::Timeout;
        let unknown = CachedResult::Unknown;

        assert!(matches!(proved, CachedResult::Proved));
        assert!(matches!(counterex, CachedResult::Counterexample { .. }));
        assert!(matches!(timeout, CachedResult::Timeout));
        assert!(matches!(unknown, CachedResult::Unknown));
    }

    #[test]
    fn test_ed25519_signature_verification() {
        use ed25519_dalek::Signer;

        // Generate a key pair
        let signing_key = generate_signing_key();
        let verifying_key = signing_key.verifying_key();

        // Create test data
        let key = "test-function-hash";
        let result = CachedResult::Proved;

        // Create canonical representation
        let canonical = format!(
            "{}:{}:{}",
            key,
            serde_json::to_string(&result).unwrap(),
            env!("CARGO_PKG_VERSION")
        );

        // Sign the data
        let signature = signing_key.sign(canonical.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        // Set the verifying key for this test
        set_session_verifying_key(verifying_key);

        // Verify the signature
        assert!(verify_signature(key, &result, &sig_hex));
    }

    #[test]
    fn test_ed25519_signature_tampered_fails() {
        use ed25519_dalek::Signer;

        // Generate a key pair
        let signing_key = generate_signing_key();
        let verifying_key = signing_key.verifying_key();

        // Create test data
        let key = "test-function-hash";
        let result = CachedResult::Proved;

        // Create canonical representation
        let canonical = format!(
            "{}:{}:{}",
            key,
            serde_json::to_string(&result).unwrap(),
            env!("CARGO_PKG_VERSION")
        );

        // Sign the data
        let signature = signing_key.sign(canonical.as_bytes());
        let mut sig_bytes = signature.to_bytes();

        // Tamper with the signature
        sig_bytes[0] ^= 0xFF;
        let tampered_sig_hex = hex::encode(sig_bytes);

        // Set the verifying key for this test
        set_session_verifying_key(verifying_key);

        // Verify the tampered signature should fail
        assert!(!verify_signature(key, &result, &tampered_sig_hex));
    }

    #[test]
    fn test_ed25519_wrong_data_fails() {
        use ed25519_dalek::Signer;

        // Generate a key pair
        let signing_key = generate_signing_key();
        let verifying_key = signing_key.verifying_key();

        // Create test data
        let key = "test-function-hash";
        let result = CachedResult::Proved;

        // Create canonical representation
        let canonical = format!(
            "{}:{}:{}",
            key,
            serde_json::to_string(&result).unwrap(),
            env!("CARGO_PKG_VERSION")
        );

        // Sign the data
        let signature = signing_key.sign(canonical.as_bytes());
        let sig_hex = hex::encode(signature.to_bytes());

        // Set the verifying key for this test
        set_session_verifying_key(verifying_key);

        // Verify with different data should fail
        let wrong_result = CachedResult::Timeout;
        assert!(!verify_signature(key, &wrong_result, &sig_hex));
    }

    #[test]
    fn test_signature_without_verifying_key_fails() {
        // Clear any existing verifying key
        SESSION_VERIFYING_KEY.with(|cell| {
            *cell.borrow_mut() = None;
        });

        // Attempt to verify without a key
        let result = verify_signature("key", &CachedResult::Proved, "00".repeat(64).as_str());
        assert!(!result);
    }

    #[test]
    fn test_invalid_signature_hex_fails() {
        let signing_key = generate_signing_key();
        set_session_verifying_key(signing_key.verifying_key());

        // Invalid hex
        assert!(!verify_signature("key", &CachedResult::Proved, "not-hex"));

        // Wrong length
        assert!(!verify_signature(
            "key",
            &CachedResult::Proved,
            "0011223344"
        ));
    }

    #[tokio::test]
    async fn test_cache_with_signature_verification() {
        // Use a temp directory for filesystem fallback to avoid permission issues
        let temp_dir = std::env::temp_dir().join("verum_smt_test_cache_sig");

        // Create cache with signature verification
        let config = DistributedCacheConfig::new("s3://test-bucket/cache")
            .with_trust_level(TrustLevel::Signatures)
            .with_filesystem_fallback(temp_dir.to_string_lossy().to_string());

        let cache = DistributedCache::new(config);

        // Store an entry (will be signed)
        cache
            .put("signed-key", CachedResult::Proved, 500)
            .await
            .unwrap();

        // Retrieve and verify
        let entry = cache.get("signed-key").await;
        assert!(matches!(entry, Maybe::Some(_)));

        if let Maybe::Some(e) = entry {
            // Entry should have a signature
            assert!(matches!(e.metadata.signature, Maybe::Some(_)));
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "distributed-cache")]
    #[test]
    fn test_parse_s3_url() {
        // Standard S3 URL
        let (bucket, prefix, endpoint) = parse_s3_url("s3://my-bucket/cache/prefix").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "cache/prefix");
        assert_eq!(endpoint, "https://s3.amazonaws.com");

        // S3 URL with custom endpoint
        let (bucket, prefix, endpoint) =
            parse_s3_url("s3://my-bucket/cache?endpoint=https://minio.local:9000").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "cache");
        assert_eq!(endpoint, "https://minio.local:9000");

        // HTTPS URL
        let (bucket, prefix, endpoint) =
            parse_s3_url("https://s3.us-west-2.amazonaws.com/my-bucket/prefix").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "prefix");
        assert_eq!(endpoint, "https://s3.us-west-2.amazonaws.com");
    }

    #[cfg(feature = "distributed-cache")]
    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://s3.amazonaws.com/bucket"),
            "s3.amazonaws.com"
        );
        assert_eq!(
            extract_host("http://localhost:9000/bucket"),
            "localhost:9000"
        );
    }

    #[cfg(feature = "distributed-cache")]
    #[test]
    fn test_aws_signature_v4() {
        // Test vector from AWS documentation
        let secret_key = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
        let date_stamp = "20130524";
        let region = "us-east-1";
        let string_to_sign = "AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972";

        let signature = calculate_signature(secret_key, date_stamp, region, string_to_sign);

        // The signature should be a 64-character hex string
        assert_eq!(signature.len(), 64);
        // Should only contain hex characters
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_filesystem_only_config() {
        let temp_dir = std::env::temp_dir().join("verum_test_fs_only");
        let config =
            DistributedCacheConfig::filesystem_only(temp_dir.to_string_lossy().to_string());

        assert_eq!(config.storage_url.as_str(), "");
        assert_eq!(config.trust_level, TrustLevel::None);
        assert!(matches!(config.filesystem_fallback, Maybe::Some(_)));
        assert!(matches!(config.redis_url, Maybe::None));
    }

    #[tokio::test]
    async fn test_filesystem_cache_persistence() {
        let temp_dir = std::env::temp_dir().join("verum_test_fs_persist");
        let _ = std::fs::remove_dir_all(&temp_dir); // Clean up any previous test

        let config =
            DistributedCacheConfig::filesystem_only(temp_dir.to_string_lossy().to_string());
        let cache = DistributedCache::new(config.clone());

        // Store an entry
        cache
            .put("persist-key", CachedResult::Proved, 250)
            .await
            .unwrap();

        // Verify the file was created
        let cache_path = temp_dir.join("pe").join("persist-key.json");
        assert!(
            cache_path.exists(),
            "Cache file should exist at {:?}",
            cache_path
        );

        // Read the file directly and verify contents
        let data = std::fs::read_to_string(&cache_path).unwrap();
        let entry: CacheEntry = serde_json::from_str(&data).unwrap();
        assert_eq!(entry.key.as_str(), "persist-key");
        assert_eq!(entry.result, CachedResult::Proved);

        // Create a new cache instance and verify we can read the persisted entry
        let cache2 = DistributedCache::new(config);
        cache2.clear_local(); // Clear in-memory cache to force filesystem read

        let retrieved = cache2.get("persist-key").await;
        assert!(matches!(retrieved, Maybe::Some(_)));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_cache_file_path_generation() {
        let config = DistributedCacheConfig::filesystem_only("/tmp/cache");
        let cache = DistributedCache::new(config);

        // Test path generation with subdirectory
        let path = cache.get_cache_file_path("/tmp/cache", "abcdef123456");
        assert_eq!(path.to_string_lossy(), "/tmp/cache/ab/abcdef123456.json");

        // Test path generation for short key
        let path2 = cache.get_cache_file_path("/tmp/cache", "x");
        assert_eq!(path2.to_string_lossy(), "/tmp/cache/00/x.json");
    }

    #[tokio::test]
    async fn test_filesystem_cache_size() {
        let temp_dir = std::env::temp_dir().join("verum_test_fs_size");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config =
            DistributedCacheConfig::filesystem_only(temp_dir.to_string_lossy().to_string());
        let cache = DistributedCache::new(config);

        // Initially empty
        assert_eq!(cache.filesystem_cache_size(), 0);

        // Add some entries
        cache.put("key1", CachedResult::Proved, 100).await.unwrap();
        cache.put("key2", CachedResult::Timeout, 200).await.unwrap();
        cache.put("key3", CachedResult::Unknown, 300).await.unwrap();

        // Should have 3 entries
        assert_eq!(cache.filesystem_cache_size(), 3);

        // Clear and verify
        let cleared = cache.clear_filesystem_cache().unwrap();
        assert_eq!(cleared, 3);
        assert_eq!(cache.filesystem_cache_size(), 0);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_builder_methods() {
        let config = DistributedCacheConfig::new("s3://bucket/path")
            .with_trust_level(TrustLevel::Sampling { sample_rate: 0.1 })
            .with_max_age(Duration::from_secs(60))
            .with_filesystem_fallback("/custom/path")
            .with_redis_url("redis://localhost:6379");

        assert_eq!(config.storage_url.as_str(), "s3://bucket/path");
        assert!(
            matches!(config.trust_level, TrustLevel::Sampling { sample_rate } if (sample_rate - 0.1).abs() < 0.001)
        );
        assert_eq!(config.max_age, Duration::from_secs(60));
        assert!(
            matches!(&config.filesystem_fallback, Maybe::Some(p) if p.as_str() == "/custom/path")
        );
        assert!(
            matches!(&config.redis_url, Maybe::Some(u) if u.as_str() == "redis://localhost:6379")
        );
    }

    #[test]
    fn test_disable_filesystem_fallback() {
        let config = DistributedCacheConfig::new("s3://bucket/path").without_filesystem_fallback();

        assert!(matches!(config.filesystem_fallback, Maybe::None));
    }

    #[test]
    fn test_error_variants() {
        // Test that all error variants can be constructed
        let s3_err = DistributedCacheError::S3Error("test".into());
        let redis_err = DistributedCacheError::RedisError("test".into());
        let fs_err = DistributedCacheError::FilesystemError("test".into());
        let sig_err = DistributedCacheError::InvalidSignature;
        let exp_err = DistributedCacheError::Expired;
        let not_impl = DistributedCacheError::NotImplemented("test".into());
        let conn_err = DistributedCacheError::ConnectionError("test".into());
        let other = DistributedCacheError::Other("test".into());

        // Verify Display impl works
        assert!(format!("{}", s3_err).contains("S3"));
        assert!(format!("{}", redis_err).contains("Redis"));
        assert!(format!("{}", fs_err).contains("Filesystem"));
        assert!(format!("{}", sig_err).contains("Signature"));
        assert!(format!("{}", exp_err).contains("expired"));
        assert!(format!("{}", not_impl).contains("Not implemented"));
        assert!(format!("{}", conn_err).contains("Connection"));
        assert!(format!("{}", other).contains("error"));
    }
}
