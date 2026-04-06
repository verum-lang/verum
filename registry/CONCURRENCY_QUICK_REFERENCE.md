# Concurrency Quick Reference

## Import

```verum
import super.super.services.concurrency_service.{
    join2, join3, join4, join5,
    parallel_map, parallel_map_limited,
    with_timeout, with_timeout_optional,
    parallel_filter, collect_ok, retry
};
```

## Common Patterns

### Run 2-5 Operations Concurrently

```verum
// 2 operations
let (a, b) = join2(op_a(), op_b()).await?;

// 3 operations
let (a, b, c) = join3(op_a(), op_b(), op_c()).await?;

// 4 operations
let (a, b, c, d) = join4(op_a(), op_b(), op_c(), op_d()).await?;

// 5 operations
let (a, b, c, d, e) = join5(op_a(), op_b(), op_c(), op_d(), op_e()).await?;
```

### Run N Operations Concurrently

```verum
// Map function over all items in parallel
let results = parallel_map(items, |item| async {
    process(item).await
}).await?;

// With concurrency limit (bulkhead pattern)
let results = parallel_map_limited(items, mapper, 10).await?;
```

### Add Timeout

```verum
// Returns error on timeout
let result = with_timeout(slow_operation(), 5000).await?;

// Returns Maybe.None on timeout
let maybe_result = with_timeout_optional(operation(), 5000).await;
```

### Filter Errors (Graceful Degradation)

```verum
// Spawn tasks
let mut tasks = List.new();
for item in items {
    tasks.push(spawn async { process(item).await });
}

// Collect successful results only
let mut results = List.new();
for task in tasks {
    match task.await {
        Result.Ok(value) => results.push(value),
        Result.Err(_) => {} // Skip errors
    }
}
```

### Retry with Backoff

```verum
let result = retry(|| fetch_data(), 3).await?;
```

## Real-World Examples

### Health Check

```verum
pub async fn health_check() -> Result<HealthResponseDto, RegistryError>
    using [Database, Storage, Search]
{
    let (db_health, storage_health, search_health) = join3(
        check_database_health(),
        check_storage_health(),
        check_search_health()
    ).await?;

    // Aggregate and return
}
```

### Load User Dashboard

```verum
pub async fn get_user_dashboard(token: Text) -> Result<UserDashboardDto, RegistryError>
    using [Database, Auth]
{
    let user = Auth.verify_token(token).await?;

    let (profile, packages, tokens, stats) = join4(
        Database.fetch_one_with("SELECT * FROM users WHERE id = $1", params),
        Database.query_with("SELECT * FROM packages WHERE owner = $1", params),
        Database.query_with("SELECT * FROM tokens WHERE user_id = $1", params),
        Database.fetch_one_with("SELECT SUM(downloads) FROM packages WHERE owner = $1", params)
    ).await?;

    // Build and return dashboard
}
```

### Batch Load Packages

```verum
pub async fn load_packages_batch(names: List<Text>) -> Result<List<PackageDto>, RegistryError>
    using [Database]
{
    let mut tasks = List.new();

    for name in names {
        tasks.push(spawn async {
            Database.fetch_optional_with(
                "SELECT * FROM packages WHERE name = $1",
                sql_params_1(name)
            ).await
        });
    }

    let mut results = List.new();
    for task in tasks {
        match task.await {
            Result.Ok(Maybe.Some(pkg)) => results.push(pkg),
            _ => {} // Skip not found or errors
        }
    }

    Result.Ok(results)
}
```

### Search with Suggestions

```verum
pub async fn search_with_suggestions(query: Text) -> Result<SearchResponseDto, RegistryError>
    using [Search, Database]
{
    let (results, autocomplete, trending) = join3(
        Search.search(query.clone()),
        Search.autocomplete(query, 10),
        Database.get_trending(7, 5)
    ).await?;

    // Merge and return
}
```

## Performance Guidelines

| Pattern | Use When | Speedup |
|---------|----------|---------|
| `join2-5` | 2-5 independent operations | 2-5x |
| `parallel_map` | Process list concurrently | Nx |
| `with_timeout` | External/slow operations | Prevents hangs |
| `parallel_map_limited` | Large batches | Nx (controlled) |
| Error filtering | Batch operations | Better UX |

## Best Practices

### ✅ DO

```verum
// Use join for independent operations
let (db, cache) = join2(db_check(), cache_check()).await?;

// Add timeouts to external calls
let data = with_timeout(api_call(), 5000).await?;

// Limit concurrency for large batches
let results = parallel_map_limited(items, process, 10).await?;

// Filter errors for batch operations
for task in tasks {
    match task.await {
        Result.Ok(v) => results.push(v),
        Result.Err(_) => {} // Continue processing
    }
}
```

### ❌ DON'T

```verum
// Don't execute sequentially when you can use join
let db = db_check().await?;
let cache = cache_check().await?;

// Don't forget timeouts on external calls
let data = slow_external_api().await?; // Could hang forever

// Don't spawn unlimited concurrent tasks
for item in millions_of_items {
    spawn async { process(item) }; // Resource exhaustion
}

// Don't fail entire batch on first error
let results = parallel_map(items, process).await?; // First error stops all
```

## Cheat Sheet

```verum
// 2 concurrent ops
join2(a, b).await?

// 3 concurrent ops
join3(a, b, c).await?

// 4 concurrent ops
join4(a, b, c, d).await?

// 5 concurrent ops
join5(a, b, c, d, e).await?

// Map over list (unlimited concurrency)
parallel_map(items, mapper).await?

// Map over list (limited concurrency)
parallel_map_limited(items, mapper, 10).await?

// With 5s timeout
with_timeout(op, 5000).await?

// With timeout, return Maybe
with_timeout_optional(op, 5000).await

// Filter concurrently
parallel_filter(items, predicate).await?

// Retry 3 times
retry(|| op(), 3).await?
```

## When to Use Concurrency

### ✅ Use Concurrency When

- Operations are **independent** (don't depend on each other)
- Operations are **I/O-bound** (database, network, storage)
- Operations can run **in parallel** without conflicts
- You need to **reduce latency** for users

### ❌ Avoid Concurrency When

- Operations **depend on each other** (sequential by nature)
- Operations are **CPU-bound** (better to use parallelism)
- Operations **share mutable state** (race conditions)
- Order of execution **matters** for correctness

## Monitoring

```verum
// Log concurrent operation
Logger.info(f"Starting concurrent load of {items.len()} items");

let start = unix_timestamp();
let results = parallel_map(items, mapper).await?;
let elapsed = unix_timestamp() - start;

Logger.info(f"Completed in {elapsed}ms: {results.len()} success");
```

## Testing

```verum
#[test]
async fn test_concurrency() {
    let start = unix_timestamp();

    // Should complete in ~50ms (not 150ms)
    let result = concurrent_operation().await.unwrap();

    let elapsed = unix_timestamp() - start;
    assert!(elapsed < 100);
}
```
