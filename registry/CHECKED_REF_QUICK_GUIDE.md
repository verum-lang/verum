# &checked T Quick Reference Guide

## TL;DR

Use `&checked T` instead of `&T` in hot paths to eliminate ~15ns CBGR overhead per reference access. The compiler proves safety via escape analysis.

## Quick Decision Tree

```
Is this a hot path function (called frequently)?
├─ NO → Use regular &T
└─ YES → Continue ↓

Does the reference escape the function?
├─ YES → Use regular &T
└─ NO → Continue ↓

Is the reference read-only during the call?
├─ NO → Use regular &T
└─ YES → Use &checked T ✓
```

## Examples

### ✅ Good: Validation (Hot Path)

```verum
/// SAFETY: &checked safe - name immutable during validation, doesn't escape
pub fn validate_package_name(name: &checked Text) -> Result<ValidatedPackageName, Error> {
    if name.len() < 2 { return Result.Err(...) }
    if name.starts_with("-") { return Result.Err(...) }
    Result.Ok(name.clone())
}
```

**Why:** Called on every publish/search, reference is read-only, doesn't escape.

### ✅ Good: Conversion (Hot Path)

```verum
/// SAFETY: &checked safe - user immutable, doesn't escape
pub fn user_to_dto(user: &checked User) -> UserDto {
    UserDto {
        id: user.id,
        username: user.username.clone()
    }
}
```

**Why:** Called for every user in listings, reference is read-only.

### ✅ Good: Pagination (Critical Hot Path)

```verum
/// SAFETY: Refinement types + &checked = zero-cost abstraction
pub fn calculate_offset(page: &checked PageNumber, size: &checked PageSize) -> Int {
    (page - 1) * size
}
```

**Why:** Called on EVERY paginated request, refinements prove bounds, &checked eliminates overhead.

### ✅ Good: Iteration (Large Data)

```verum
/// SAFETY: &checked safe - data immutable during iteration
fn compute_sha256(data: &checked List<Int>) -> Text {
    let mut hash = 0;
    for byte in data {  // Millions of iterations with 0ns overhead
        hash = (hash * 31 + byte) % 1000000007;
    }
    f"{hash}"
}
```

**Why:** Large data iteration benefits most from &checked.

### ❌ Bad: Reference Escapes

```verum
// DON'T DO THIS
pub fn store_user(user: &checked User) -> StoredUser {
    StoredUser {
        user_ref: user  // ❌ Reference escapes!
    }
}
```

**Why:** Reference stored in struct, escapes function scope. Use regular `&T`.

### ❌ Bad: Mutable Access

```verum
// DON'T DO THIS
pub fn update_user(user: &checked mut User) {
    user.username = "new";  // ❌ Mutation during &checked
}
```

**Why:** &checked requires immutability. Use regular `&mut T`.

### ❌ Bad: Not a Hot Path

```verum
// DON'T DO THIS
pub fn rarely_called(data: &checked Text) {
    // Called once per day
}
```

**Why:** Not worth the cognitive overhead for rare operations.

## SAFETY Comment Template

```verum
/// SAFETY: Uses &checked reference - [FREQUENCY] hot path.
/// - [param] is immutable during [operation]
/// - compiler proves no escape or concurrent modification
/// Performance: Saves ~[N]ns per [operation]
pub fn function_name(param: &checked Type) -> Result {
    ...
}
```

Fill in:
- `[FREQUENCY]`: "CRITICAL", "high-frequency", etc.
- `[param]`: Parameter name
- `[operation]`: What the function does
- `[N]`: Expected savings (usually 15ns, or 15*params for multiple)

## Common Hot Paths in Registry

| Pattern | Example | Use &checked? |
|---------|---------|---------------|
| **Validation** | `validate_email(email)` | ✅ Yes |
| **Conversion** | `user_to_dto(user)` | ✅ Yes |
| **Parsing** | `parse_semver(version)` | ✅ Yes |
| **Iteration** | `for byte in data` | ✅ Yes |
| **Pagination** | `calculate_offset(page, size)` | ✅ Yes |
| **String ops** | `split()`, `contains()` | ✅ Yes |
| **Storage** | `store.save(entity)` | ❌ No (escapes) |
| **Mutation** | `entity.update(...)` | ❌ No (mutable) |
| **Rare calls** | Setup/teardown | ❌ No (not hot) |

## Performance Impact

| Optimization | Savings | Typical Frequency | Total Impact |
|--------------|---------|-------------------|--------------|
| Single &checked | ~15ns | Per call | Low |
| Validation chain | ~45-75ns | Per request | Medium |
| Large iteration | ~15ns + cache | Per tarball | High |
| Pagination | ~30ns | Per list request | Very High |
| Refinements + &checked | ~30ns | Per operation | Optimal |

## Checklist Before Using &checked

- [ ] Function is hot path (called frequently)
- [ ] Reference is read-only during call
- [ ] Reference doesn't escape function
- [ ] No concurrent modification possible
- [ ] Added SAFETY comment explaining why
- [ ] Documented performance impact

## When in Doubt

**Use regular `&T`.**

The compiler will accept it, and you can always optimize later with profiling data. Premature optimization with &checked adds cognitive overhead.

## Testing

After adding &checked optimizations:

```bash
# 1. Correctness
cargo test

# 2. Benchmark
cargo bench --bench your_bench

# 3. Load test
wrk -t4 -c100 -d30s http://localhost:8080/api/endpoint
```

## Common Patterns

### Pattern 1: Validation Function

```verum
pub fn validate_field(value: &checked Text, field: &checked Text) -> Result<(), Error> {
    if value.is_empty() {
        return Result.Err(error_empty(field));
    }
    Result.Ok(())
}
```

### Pattern 2: DTO Conversion

```verum
pub fn domain_to_dto(entity: &checked DomainType) -> DtoType {
    DtoType {
        field1: entity.field1.clone(),
        field2: entity.field2.clone(),
    }
}
```

### Pattern 3: Helper Function

```verum
fn parse_helper(input: &checked Text) -> ParsedType {
    let parts = input.split(".");
    // ... parsing logic
    ParsedType::new(parts)
}
```

### Pattern 4: Refinement + &checked

```verum
pub fn calculate(validated: &checked ValidatedType) -> Result {
    // Refinement proves bounds, &checked eliminates overhead
    validated.value() * 2
}
```

## FAQ

**Q: Does &checked make code unsafe?**
A: No. The compiler proves safety via escape analysis. If it can't prove safety, it rejects the code.

**Q: Can I use &checked everywhere?**
A: You can, but it adds cognitive overhead. Use only in hot paths where performance matters.

**Q: What if the compiler rejects my &checked?**
A: Use regular `&T`. The compiler is telling you it can't prove safety.

**Q: How much faster is &checked?**
A: ~15ns per reference access. Accumulates in loops and validation chains.

**Q: Should I rewrite all my code with &checked?**
A: No. Profile first, optimize hot paths, leave cold paths alone.

## Resources

- **CLAUDE.md**: Full CBGR documentation
- **PERFORMANCE_OPTIMIZATIONS.md**: Detailed optimization guide
- **CBGR_OPTIMIZATION_SUMMARY.md**: Summary of all changes

## Remember

**Safety First, Performance Second**

The compiler won't let you write unsafe code with &checked. If you're unsure, use regular `&T` and optimize later with profiling data.
