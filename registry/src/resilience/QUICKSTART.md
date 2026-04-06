# Resilience Patterns - Quick Start Guide

Get started with fault tolerance patterns in 5 minutes.

## Installation

The resilience module is already integrated into the verum-registry project:

```
registry/verum-registry/src/resilience/
```

## Basic Usage

### 1. Circuit Breaker (Prevent Cascading Failures)

```verum
import super.resilience.{CircuitBreaker, execute_with_circuit_breaker};

// Create circuit breaker
let db_circuit = CircuitBreaker.new(5, 3, 30000);

// Use it
let (result, updated_circuit) = execute_with_circuit_breaker(
    db_circuit,
    || Database.query("SELECT * FROM users"),
    || Result.Err(RegistryError.service_unavailable("DB circuit open"))
);

db_circuit = updated_circuit;
```

### 2. Retry (Handle Transient Failures)

```verum
import super.resilience.{with_retry, RetryConfig};

let result = with_retry(
    || ExternalApi.call(),
    RetryConfig.default(),  // 3 attempts, exponential backoff
    |err| err.is_recoverable()
);
```

### 3. Timeout (Prevent Blocking)

```verum
import super.resilience.{with_timeout};

let result = with_timeout(
    || Database.long_query(),
    5000  // 5 second timeout
).await;
```

### 4. Bulkhead (Limit Concurrency)

```verum
import super.resilience.{database_bulkhead, with_bulkhead};

let bulkhead = database_bulkhead();  // 10 concurrent, 50 queue

let (result, updated_bulkhead) = with_bulkhead(
    bulkhead,
    || Database.query("SELECT * FROM packages")
).await;
```

### 5. Fallback (Graceful Degradation)

```verum
import super.resilience.{with_fallback};

let result = with_fallback(
    || Database.get_package(name),
    |err| Cache.get_package(name)  // Fallback to cache
);
```

## Complete Example

Combine all patterns for comprehensive protection:

```verum
import super.resilience.{
    ResilienceConfig,
    execute_resilient,
};

pub async fn get_package(name: Text) -> Result<PackageDto, RegistryError>
    using [Database, Cache]
{
    // Configure all patterns at once
    let config = ResilienceConfig.database_default()
        .with_fallback(|_| Cache.get(name));

    // Execute with full protection
    execute_resilient(
        config,
        || Database.get_package(name)
    ).await
}
```

## Pre-configured Settings

Use pre-configured settings for common scenarios:

### Database Operations
```verum
let config = ResilienceConfig.database_default();
// - Circuit: Open after 5 failures, 30s timeout
// - Retry: 3 attempts with exponential backoff
// - Timeout: 5 seconds
// - Bulkhead: 10 concurrent connections
```

### External API Calls
```verum
let config = ResilienceConfig.api_default();
// - Circuit: Open after 3 failures, 60s timeout
// - Retry: 5 attempts (conservative)
// - Timeout: 10 seconds
// - Bulkhead: 5 concurrent calls
```

### Cache Operations
```verum
let config = ResilienceConfig.cache_default();
// - Timeout: 1 second (fast)
// - Retry: 3 attempts (aggressive)
```

## Common Patterns

### Pattern 1: Database with Cache Fallback

```verum
pub async fn get_user(id: Int) -> Result<User, RegistryError>
    using [Database, Cache]
{
    let config = ResilienceConfig.database_default()
        .with_fallback(|_| Cache.get(f"user:{id}"));

    let result = execute_resilient(
        config,
        || Database.get_user(id)
    ).await;

    // Cache on success
    match result {
        Result.Ok(user) => {
            Cache.set(f"user:{id}", user.clone()).await;
            Result.Ok(user)
        },
        err => err
    }
}
```

### Pattern 2: Multi-level Fallback Chain

```verum
import super.resilience.{with_fallback_chain};

pub async fn get_config(key: Text) -> Result<Text, RegistryError>
    using [ConfigService, Database, Cache]
{
    with_fallback_chain([
        || ConfigService.get(key),    // Try remote config
        || Database.get_config(key),  // Try database
        || Cache.get_config(key),     // Try cache
        || Result.Ok("default_value") // Return default
    ]).await
}
```

### Pattern 3: Retry with Custom Logic

```verum
import super.resilience.{with_custom_retry, RetryConfig};

pub async fn publish_package(pkg: Package) -> Result<(), RegistryError>
    using [Storage]
{
    with_custom_retry(
        || Storage.upload(pkg),
        |err| match err {
            // Network errors: aggressive retry
            RegistryError.NetworkError { _ } =>
                Maybe.Some(RetryConfig.aggressive()),

            // Storage errors: conservative retry
            RegistryError.StorageError { _ } =>
                Maybe.Some(RetryConfig.conservative()),

            // Other errors: don't retry
            _ => Maybe.None
        }
    ).await
}
```

## Monitoring

### Health Check with Circuit Breaker Status

```verum
pub fn health_check(circuits: ServiceCircuitBreakers) -> HealthStatus {
    let db_state = circuits.database.get_state();

    HealthStatus {
        healthy: match db_state {
            CircuitState.Closed => true,
            _ => false
        },
        details: circuits.database.metrics().to_string()
    }
}
```

### Collect Metrics

```verum
pub fn get_metrics(
    circuits: ServiceCircuitBreakers,
    bulkheads: ServiceBulkheads
) -> ResilienceMetrics {
    ResilienceMetrics {
        circuit_breaker: Maybe.Some(circuits.database.metrics()),
        bulkhead: Maybe.Some(bulkheads.database.stats()),
        retry: Maybe.Some(retry_stats),
        timeout: Maybe.Some(timeout_stats),
        fallback: Maybe.Some(fallback_stats),
    }
}
```

## Configuration Examples

### Conservative (Critical Operations)

```verum
let config = ResilienceConfig.new()
    .with_circuit_breaker(CircuitBreaker.new(10, 5, 60000))  // Very tolerant
    .with_retry(RetryConfig.conservative())  // 5 attempts, long delays
    .with_timeout(30000)  // 30 second timeout
    .with_bulkhead(Bulkhead.new(5, 20));  // Limited concurrency
```

### Aggressive (Fast Operations)

```verum
let config = ResilienceConfig.new()
    .with_circuit_breaker(CircuitBreaker.new(3, 2, 10000))  // Quick to open
    .with_retry(RetryConfig.aggressive())  // 3 attempts, short delays
    .with_timeout(1000)  // 1 second timeout
    .with_bulkhead(Bulkhead.new(20, 50));  // High concurrency
```

### Balanced (Default)

```verum
let config = ResilienceConfig.database_default();
// Good for most use cases
```

## Testing

Test resilience patterns with controlled failures:

```verum
// Test circuit breaker
let mut cb = CircuitBreaker.new(3, 2, 1000);

// Simulate failures
cb = cb.record_failure();
cb = cb.record_failure();
cb = cb.record_failure();

// Verify circuit opened
assert_eq!(cb.get_state(), CircuitState.Open);
assert_eq!(cb.should_allow(), false);

// Wait for timeout
sleep(1000);
cb = cb.try_reset();

// Verify half-open
assert_eq!(cb.get_state(), CircuitState.HalfOpen);
```

## Best Practices

### ✅ Do

- Use pre-configured settings as a starting point
- Combine patterns for comprehensive protection
- Monitor metrics and adjust thresholds
- Test failure scenarios
- Document your configuration choices

### ❌ Don't

- Don't retry non-idempotent operations without safeguards
- Don't set timeouts longer than your SLA
- Don't ignore circuit breaker state in health checks
- Don't use the same bulkhead for different resource types
- Don't fallback to slower operations

## Performance Tips

1. **Circuit Breaker**: Nearly zero overhead (~10ns)
2. **Timeout**: Minimal overhead (~100ns)
3. **Bulkhead**: Small overhead (~50ns)
4. **Retry**: Cost = delay × attempts (only on failure)
5. **Fallback**: Cost = fallback operation (only on failure)

## Common Issues

### Issue: Circuit breaker opens too frequently
**Solution:** Increase `failure_threshold` or decrease timeout

### Issue: Retries causing too much load
**Solution:** Reduce `max_attempts` or increase delays

### Issue: Timeouts too aggressive
**Solution:** Use `AdaptiveTimeout` or increase timeout value

### Issue: Bulkhead rejecting too many requests
**Solution:** Increase `max_concurrent` or `queue_size`

### Issue: Fallback always returning stale data
**Solution:** Check primary service health, adjust circuit breaker

## Next Steps

1. Read the [full documentation](README.md)
2. Review [usage examples](usage_example.vr)
3. Study the [architecture](ARCHITECTURE.md)
4. Explore each pattern's module:
   - [circuit_breaker.vr](circuit_breaker.vr)
   - [retry.vr](retry.vr)
   - [timeout.vr](timeout.vr)
   - [bulkhead.vr](bulkhead.vr)
   - [fallback.vr](fallback.vr)

## Support

For questions or issues, refer to:
- [README.md](README.md) - Comprehensive documentation
- [IMPLEMENTATION_SUMMARY.md](IMPLEMENTATION_SUMMARY.md) - Implementation details
- [ARCHITECTURE.md](ARCHITECTURE.md) - System architecture
