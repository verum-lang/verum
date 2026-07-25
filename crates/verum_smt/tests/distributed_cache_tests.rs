#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Comprehensive tests for distributed verification cache
//!
//! Tests cover:
//! - Cache entry creation and expiration
//! - Cache key generation
//! - Cache statistics tracking
//! - Trust level verification
//! - Local cache operations
//! - Ed25519 signature verification
//! - S3 URL parsing (when distributed-cache feature enabled)

use std::time::Duration;
use verum_common::Maybe;
use verum_smt::distributed_cache::{
    CacheCredentials, CacheEntry, CacheStats, CachedResult, DistributedCache,
    DistributedCacheConfig, EntryMetadata, TrustLevel, generate_cache_key,
};

// ==================== Cache Entry Tests ====================

#[test]
fn test_cache_entry_creation() {
    let metadata = EntryMetadata::new(1500);
    let entry = CacheEntry::new("test-key", CachedResult::Proved, metadata);

    assert_eq!(entry.key.as_str(), "test-key");
    assert_eq!(entry.result, CachedResult::Proved);
    assert_eq!(entry.metadata.original_time_ms, 1500);
}

#[test]
fn test_cache_entry_not_expired() {
    let metadata = EntryMetadata::new(100);
    let entry = CacheEntry::new("fresh-key", CachedResult::Proved, metadata);

    // 30 days max age - entry should not be expired
    let max_age = Duration::from_secs(30 * 24 * 60 * 60);
    assert!(!entry.is_expired(max_age));
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
fn test_cached_result_counterexample() {
    let ce = CachedResult::counterexample("y = 42, z = -1");
    if let CachedResult::Counterexample { value } = ce {
        assert!(value.to_string().contains("42"));
        assert!(value.to_string().contains("-1"));
    } else {
        panic!("Expected Counterexample variant");
    }
}

// ==================== Cache Key Generation Tests ====================

#[test]
fn test_generate_cache_key_deterministic() {
    let key1 = generate_cache_key("file1", "fn foo() -> Int", "proof");
    let key2 = generate_cache_key("file1", "fn foo() -> Int", "proof");

    assert_eq!(key1, key2, "Same inputs should produce same key");
}

#[test]
fn test_generate_cache_key_different_files() {
    let key1 = generate_cache_key("file1", "fn foo() -> Int", "proof");
    let key2 = generate_cache_key("file2", "fn foo() -> Int", "proof");

    assert_ne!(key1, key2, "Different files should produce different keys");
}

#[test]
fn test_generate_cache_key_different_signatures() {
    let key1 = generate_cache_key("file1", "fn foo() -> Int", "proof");
    let key2 = generate_cache_key("file1", "fn bar() -> Int", "proof");

    assert_ne!(
        key1, key2,
        "Different signatures should produce different keys"
    );
}

#[test]
fn test_generate_cache_key_different_modes() {
    let key1 = generate_cache_key("file1", "fn foo() -> Int", "proof");
    let key2 = generate_cache_key("file1", "fn foo() -> Int", "runtime");

    assert_ne!(key1, key2, "Different modes should produce different keys");
}

#[test]
fn test_generate_cache_key_is_hex() {
    let key = generate_cache_key("test_file", "fn test() -> Bool", "static");

    // SHA-256 produces 64 hex characters
    assert_eq!(key.to_string().len(), 64);
    assert!(key.to_string().chars().all(|c| c.is_ascii_hexdigit()));
}

// ==================== Cache Statistics Tests ====================

#[test]
fn test_cache_stats_default() {
    let stats = CacheStats::default();

    assert_eq!(stats.remote_hits, 0);
    assert_eq!(stats.remote_misses, 0);
    assert_eq!(stats.local_hits, 0);
    assert_eq!(stats.uploads, 0);
    assert_eq!(stats.download_time_ms, 0);
    assert_eq!(stats.upload_time_ms, 0);
}

#[test]
fn test_cache_stats_total_hits() {
    let mut stats = CacheStats::default();
    stats.local_hits = 10;
    stats.remote_hits = 5;

    assert_eq!(stats.total_hits(), 15);
}

#[test]
fn test_cache_stats_hit_rate() {
    let mut stats = CacheStats::default();
    stats.local_hits = 10;
    stats.remote_hits = 5;
    stats.remote_misses = 2;

    let expected = 15.0 / 17.0;
    assert!((stats.hit_rate() - expected).abs() < 0.001);
}

#[test]
fn test_cache_stats_hit_rate_zero_total() {
    let stats = CacheStats::default();
    assert_eq!(stats.hit_rate(), 0.0);
}

#[test]
fn test_cache_stats_avg_download_time() {
    let mut stats = CacheStats::default();
    stats.remote_hits = 5;
    stats.download_time_ms = 500;

    assert_eq!(stats.avg_download_time_ms(), 100.0);
}

#[test]
fn test_cache_stats_avg_download_time_zero_hits() {
    let stats = CacheStats::default();
    assert_eq!(stats.avg_download_time_ms(), 0.0);
}

#[test]
fn test_cache_stats_avg_upload_time() {
    let mut stats = CacheStats::default();
    stats.uploads = 4;
    stats.upload_time_ms = 400;

    assert_eq!(stats.avg_upload_time_ms(), 100.0);
}

#[test]
fn test_cache_stats_avg_upload_time_zero_uploads() {
    let stats = CacheStats::default();
    assert_eq!(stats.avg_upload_time_ms(), 0.0);
}

#[test]
fn test_cache_stats_format_report() {
    let mut stats = CacheStats::default();
    stats.remote_hits = 10;
    stats.remote_misses = 2;
    stats.local_hits = 5;
    stats.uploads = 3;

    let report = stats.format_report();
    assert!(report.to_string().contains("Remote hits:"));
    assert!(report.to_string().contains("Remote misses:"));
    assert!(report.to_string().contains("Local hits:"));
    assert!(report.to_string().contains("Uploads:"));
}

// ==================== Trust Level Tests ====================

#[test]
fn test_trust_level_none() {
    let config =
        DistributedCacheConfig::new("s3://test-bucket/cache").with_trust_level(TrustLevel::None);

    assert_eq!(config.trust_level, TrustLevel::None);
}

#[test]
fn test_trust_level_signatures() {
    let config = DistributedCacheConfig::new("s3://test-bucket/cache")
        .with_trust_level(TrustLevel::Signatures);

    assert_eq!(config.trust_level, TrustLevel::Signatures);
}

#[test]
fn test_trust_level_sampling() {
    let config = DistributedCacheConfig::new("s3://test-bucket/cache")
        .with_trust_level(TrustLevel::Sampling { sample_rate: 0.1 });

    if let TrustLevel::Sampling { sample_rate } = config.trust_level {
        assert!((sample_rate - 0.1).abs() < 0.001);
    } else {
        panic!("Expected Sampling variant");
    }
}

// ==================== Config Tests ====================

#[test]
fn test_config_new() {
    let config = DistributedCacheConfig::new("s3://my-bucket/prefix");

    assert_eq!(config.storage_url.to_string(), "s3://my-bucket/prefix");
    assert_eq!(config.trust_level, TrustLevel::Signatures);
    assert_eq!(config.max_age.as_secs(), 30 * 24 * 60 * 60);
    assert!(matches!(config.credentials, Maybe::None));
}

#[test]
fn test_config_with_max_age() {
    let config = DistributedCacheConfig::new("s3://bucket/cache")
        .with_max_age(Duration::from_secs(7 * 24 * 60 * 60));

    assert_eq!(config.max_age.as_secs(), 7 * 24 * 60 * 60);
}

#[test]
fn test_config_with_credentials() {
    let creds = CacheCredentials::new("access_key", "secret_key").with_region("us-west-2");

    let config = DistributedCacheConfig::new("s3://bucket/cache").with_credentials(creds);

    if let Maybe::Some(creds) = config.credentials {
        assert_eq!(creds.access_key.to_string(), "access_key");
        assert_eq!(creds.secret_key.to_string(), "secret_key");
        if let Maybe::Some(region) = creds.region {
            assert_eq!(region.to_string(), "us-west-2");
        } else {
            panic!("Expected region to be set");
        }
    } else {
        panic!("Expected credentials to be set");
    }
}

// ==================== Credentials Tests ====================

#[test]
fn test_credentials_new() {
    let creds = CacheCredentials::new("access", "secret");

    assert_eq!(creds.access_key.to_string(), "access");
    assert_eq!(creds.secret_key.to_string(), "secret");
    assert!(matches!(creds.region, Maybe::None));
}

#[test]
fn test_credentials_with_region() {
    let creds = CacheCredentials::new("access", "secret").with_region("eu-central-1");

    if let Maybe::Some(region) = creds.region {
        assert_eq!(region.to_string(), "eu-central-1");
    } else {
        panic!("Expected region to be set");
    }
}

// ==================== Distributed Cache Tests ====================

#[test]
fn test_cache_creation_no_signatures() {
    let config = DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

    let cache = DistributedCache::new(config);
    assert_eq!(cache.local_size(), 0);
}

#[test]
fn test_cache_creation_with_signatures() {
    let config =
        DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::Signatures);

    let cache = DistributedCache::new(config);
    assert_eq!(cache.local_size(), 0);
}

#[test]
fn test_cache_clear_local() {
    let config = DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

    let cache = DistributedCache::new(config);
    cache.clear_local();
    assert_eq!(cache.local_size(), 0);
}

#[tokio::test]
async fn test_cache_put_and_get() {
    let config = DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

    let cache = DistributedCache::new(config);

    // Put an entry
    cache
        .put("test-key", CachedResult::Proved, 100)
        .await
        .unwrap();
    assert_eq!(cache.local_size(), 1);

    // Get the entry back
    let entry = cache.get("test-key").await;
    assert!(matches!(entry, Maybe::Some(_)));

    if let Maybe::Some(e) = entry {
        assert_eq!(e.result, CachedResult::Proved);
        assert_eq!(e.metadata.original_time_ms, 100);
    }
}

#[tokio::test]
async fn test_cache_local_hit_tracking() {
    let config = DistributedCacheConfig::new("s3://test/cache")
        .with_trust_level(TrustLevel::None)
        .without_filesystem_fallback();

    let cache = DistributedCache::new(config);

    // Put and get twice - second get should be a local hit
    cache.put("key1", CachedResult::Proved, 50).await.unwrap();

    let _ = cache.get("key1").await;
    let _ = cache.get("key1").await;

    let stats = cache.stats();
    assert_eq!(stats.local_hits, 2);
}

#[tokio::test]
async fn test_cache_stats_tracking() {
    let config = DistributedCacheConfig::new("s3://test/cache")
        .with_trust_level(TrustLevel::None)
        .without_filesystem_fallback();

    let cache = DistributedCache::new(config);

    // Put entry
    cache.put("key1", CachedResult::Proved, 100).await.unwrap();

    let stats = cache.stats();
    assert_eq!(stats.uploads, 1);
}

#[tokio::test]
async fn test_cache_time_saved() {
    let config = DistributedCacheConfig::new("s3://test/cache")
        .with_trust_level(TrustLevel::None)
        .without_filesystem_fallback();

    let cache = DistributedCache::new(config);

    // Put entries with different verification times
    cache.put("key1", CachedResult::Proved, 1000).await.unwrap();
    cache.put("key2", CachedResult::Proved, 500).await.unwrap();

    let time_saved = cache.time_saved();
    assert_eq!(time_saved.as_millis(), 1500);
}

#[tokio::test]
async fn test_cache_miss() {
    let config = DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

    let cache = DistributedCache::new(config);

    // Try to get non-existent key
    let entry = cache.get("nonexistent").await;
    assert!(matches!(entry, Maybe::None));
}

#[tokio::test]
async fn test_cache_with_signature_verification() {
    let config =
        DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::Signatures);

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
}

#[tokio::test]
async fn test_cache_multiple_entries() {
    let config = DistributedCacheConfig::new("s3://test/cache")
        .with_trust_level(TrustLevel::None)
        .without_filesystem_fallback();

    let cache = DistributedCache::new(config);

    // Store multiple entries
    cache.put("key1", CachedResult::Proved, 100).await.unwrap();
    cache
        .put(
            "key2",
            CachedResult::Counterexample {
                value: "x=0".into(),
            },
            200,
        )
        .await
        .unwrap();
    cache.put("key3", CachedResult::Timeout, 300).await.unwrap();

    assert_eq!(cache.local_size(), 3);

    // Verify each entry
    let e1 = cache.get("key1").await;
    let e2 = cache.get("key2").await;
    let e3 = cache.get("key3").await;

    assert!(matches!(e1, Maybe::Some(ref e) if e.result == CachedResult::Proved));
    assert!(
        matches!(e2, Maybe::Some(ref e) if matches!(e.result, CachedResult::Counterexample { .. }))
    );
    assert!(matches!(e3, Maybe::Some(ref e) if e.result == CachedResult::Timeout));
}

#[tokio::test]
async fn test_cache_clear_and_verify() {
    let config = DistributedCacheConfig::new("s3://test/cache").with_trust_level(TrustLevel::None);

    let cache = DistributedCache::new(config);

    // Store entries
    cache.put("key1", CachedResult::Proved, 100).await.unwrap();
    cache.put("key2", CachedResult::Proved, 200).await.unwrap();

    assert_eq!(cache.local_size(), 2);

    // Clear
    cache.clear_local();

    assert_eq!(cache.local_size(), 0);

    // Entries should be gone from local cache
    let e1 = cache.get("key1").await;
    // Will miss local and attempt remote (which will fail since no S3)
    // So it will be a miss
}

// ==================== Entry Metadata Tests ====================

#[test]
fn test_entry_metadata_new() {
    let metadata = EntryMetadata::new(1500);

    assert_eq!(metadata.original_time_ms, 1500);
    assert!(matches!(metadata.signature, Maybe::None));
    // cached_at should be recent (within last minute)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(metadata.cached_at <= now);
    assert!(metadata.cached_at >= now - 60);
}

#[test]
fn test_entry_metadata_with_signature() {
    let metadata = EntryMetadata::new(100).with_signature("test_signature_hex");

    if let Maybe::Some(sig) = metadata.signature {
        assert_eq!(sig.to_string(), "test_signature_hex");
    } else {
        panic!("Expected signature to be set");
    }
}

// ==================== Serialization Tests ====================

#[test]
fn test_cached_result_serialization() {
    let proved = CachedResult::Proved;
    let json = serde_json::to_string(&proved).unwrap();
    let deserialized: CachedResult = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, CachedResult::Proved);
}

#[test]
fn test_cached_result_counterexample_serialization() {
    let ce = CachedResult::counterexample("x = 42");
    let json = serde_json::to_string(&ce).unwrap();
    let deserialized: CachedResult = serde_json::from_str(&json).unwrap();

    if let CachedResult::Counterexample { value } = deserialized {
        assert!(value.to_string().contains("42"));
    } else {
        panic!("Expected Counterexample variant");
    }
}

#[test]
fn test_cache_entry_serialization() {
    let metadata = EntryMetadata::new(1000);
    let entry = CacheEntry::new("test", CachedResult::Proved, metadata);

    let json = serde_json::to_string(&entry).unwrap();
    let deserialized: CacheEntry = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.key.to_string(), "test");
    assert_eq!(deserialized.result, CachedResult::Proved);
}
