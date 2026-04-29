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
// Tests for verification_cache module
// Migrated from src/verification_cache.rs per CLAUDE.md standards

use verum_smt::verification_cache::*;

#[test]
fn test_cache_basic() {
    let cache = VerificationCache::new();

    // Initially empty
    assert_eq!(cache.stats().current_size, 0);
    assert_eq!(cache.hit_rate(), 0.0);
}

#[test]
fn test_cache_stats() {
    let cache = VerificationCache::new();
    let stats = cache.stats();

    assert_eq!(stats.cache_hits, 0);
    assert_eq!(stats.cache_misses, 0);
    assert_eq!(stats.hit_rate(), 0.0);
}

#[test]
fn test_cache_config() {
    let config = CacheConfig::default();
    assert_eq!(config.max_size, 2_000);

    let dev_config = CacheConfig::development();
    assert_eq!(dev_config.max_size, 1_000);

    let prod_config = CacheConfig::production();
    assert_eq!(prod_config.max_size, 50_000);
}

// =============================================================================
// CacheConfig stats-driven gating wiring tests
// =============================================================================
//
// Pin: `CacheConfig.{statistics_driven, min_decisions_to_cache,
// min_conflicts_to_cache, min_solve_time_ms}` reach the public
// `get_or_verify_with_stats` call-site. Pre-wire the four fields
// routed through `should_cache_with_stats` only via
// `insert_with_stats`, but the only production caller of the
// cache used `get_or_verify` (unconditional caching). The new
// stats-aware sibling makes the gate reachable without a verify.rs
// migration.

mod stats_driven_gating {
    use super::*;

    #[test]
    fn should_cache_with_stats_default_caches_expensive_only() {
        // Default config: statistics_driven = true, thresholds at
        // 1000 decisions / 100 conflicts / 100ms. A cheap query
        // (zero everywhere) should NOT be cached.
        let cfg = CacheConfig::default();
        assert!(!cfg.should_cache_with_stats(0, 0, 0));
    }

    #[test]
    fn should_cache_with_stats_caches_when_decisions_threshold_exceeded() {
        let cfg = CacheConfig::default();
        // 1000 decisions matches the default threshold (≥ caches)
        assert!(cfg.should_cache_with_stats(1000, 0, 0));
    }

    #[test]
    fn should_cache_with_stats_caches_when_conflicts_threshold_exceeded() {
        let cfg = CacheConfig::default();
        assert!(cfg.should_cache_with_stats(0, 100, 0));
    }

    #[test]
    fn should_cache_with_stats_caches_when_time_threshold_exceeded() {
        let cfg = CacheConfig::default();
        assert!(cfg.should_cache_with_stats(0, 0, 100));
    }

    #[test]
    fn should_cache_with_stats_disabled_caches_everything() {
        // Pin: when `statistics_driven = false`, the gate
        // returns true unconditionally — every query gets cached
        // regardless of its expense profile.
        let mut cfg = CacheConfig::default();
        cfg.statistics_driven = false;
        cfg.min_decisions_to_cache = u64::MAX;
        cfg.min_conflicts_to_cache = u64::MAX;
        cfg.min_solve_time_ms = u64::MAX;
        assert!(cfg.should_cache_with_stats(0, 0, 0));
    }

    #[test]
    fn dev_preset_lowers_thresholds() {
        // Pin: the development preset lowers all three
        // thresholds proportionally so smaller test queries
        // exercise the cache during local iteration.
        let cfg = CacheConfig::development();
        assert!(cfg.statistics_driven);
        assert!(cfg.min_decisions_to_cache < CacheConfig::default().min_decisions_to_cache);
        assert!(cfg.min_conflicts_to_cache < CacheConfig::default().min_conflicts_to_cache);
        assert!(cfg.min_solve_time_ms < CacheConfig::default().min_solve_time_ms);
    }

    #[test]
    fn prod_preset_raises_thresholds() {
        // Pin: the production preset raises all three
        // thresholds proportionally so the cache only retains
        // truly expensive queries.
        let cfg = CacheConfig::production();
        assert!(cfg.statistics_driven);
        assert!(cfg.min_decisions_to_cache > CacheConfig::default().min_decisions_to_cache);
        assert!(cfg.min_conflicts_to_cache > CacheConfig::default().min_conflicts_to_cache);
        assert!(cfg.min_solve_time_ms > CacheConfig::default().min_solve_time_ms);
    }
}
