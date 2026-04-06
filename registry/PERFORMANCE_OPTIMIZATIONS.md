# Verum Registry - CBGR Performance Optimizations

## Overview

This document describes the `&checked T` reference optimizations applied to hot paths in the verum-registry codebase. These optimizations eliminate the ~15ns CBGR (Capability-Based Generational References) overhead per reference access while maintaining full memory safety through compiler-proven escape analysis.

## CBGR Three-Tier Reference Model

| Tier | Syntax | Overhead | Use Case |
|------|--------|----------|----------|
| 0 | `&T` | ~15ns | Default, full CBGR protection |
| 1 | `&checked T` | 0ns | Compiler-proven safe (escape analysis) |
| 2 | `&unsafe T` | 0ns | Manual safety proof required |

## Performance Impact Summary

### Expected Savings Per Operation Type

| Operation | Frequency | Savings | Total Impact |
|-----------|-----------|---------|--------------|
| **Package Publish** | 100-1000/day | ~75ns | High |
| **Package Search** | 10000+/day | ~30ns | Very High |
| **User Registration** | 10-100/day | ~45ns | Medium |
| **User Login** | 1000+/day | ~30ns | High |
| **Checksum Calculation** | Per publish | ~15ns + iteration | Critical |
| **Pagination** | Every list request | ~30ns | Very High |

### Breakdown by Component

#### 1. Checksum Service (Critical Hot Path)
- **Files:** `src/services/checksum_service.vr`
- **Functions Optimized:**
  - `compute_checksum()` - 15ns savings
  - `compute_checksum_with_algorithm()` - 15ns savings
  - `verify_checksum()` - 15ns savings
  - `compute_all_checksums()` - 45ns savings (3 algorithms)
  - `verify_any_checksum()` - 15ns * iterations
  - `compute_sha256()` - 15ns + iteration benefits
  - `compute_sha512()` - 15ns + iteration benefits
  - `compute_blake3()` - 15ns + iteration benefits

**Impact:** For large tarballs (multi-megabyte), iteration over millions of bytes with &checked eliminates significant overhead.

#### 2. Validation Service (Critical Hot Path)
- **Files:** `src/services/validation_service_checked.vr`
- **Functions Optimized:**
  - `validate_package_name_checked()` - 15ns per publish/search
  - `validate_version_checked()` - 15ns per publish
  - `validate_email_checked()` - 15ns per registration
  - `validate_username_checked()` - 15ns per user operation
  - `validate_password_checked()` - 15ns per auth
  - `validate_not_empty_checked()` - 15ns per field
  - `validate_max_length_checked()` - 15ns per field
  - `validate_length_range_checked()` - 15ns per field

**Impact:** Called on EVERY request validation, accumulates significantly across API endpoints.

#### 3. Package Converter (Hot Path)
- **Files:** `src/converters/package_converter.vr`
- **Functions Optimized:**
  - `parse_semver()` - 15ns per publish
  - `split_by_char()` - 15ns per parse
  - `index_of()` - 15ns per search
  - `parse_int()` - 45ns total (called 3x per version)
  - `char_to_digit()` - benefits from tight loop

**Impact:** Version parsing happens on every publish, optimization stacks with helper functions.

#### 4. User Converter (Hot Path)
- **Files:** `src/converters/user_converter.vr`
- **Functions Optimized:**
  - `user_to_dto()` - 15ns per user in list
  - `user_to_profile_dto()` - 15ns per profile view
  - `token_to_dto()` - 15ns per token
  - `tokens_to_dtos()` - 15ns * list size
  - `register_request_to_user()` - 15ns per registration

**Impact:** User and token conversions happen frequently in authenticated operations.

#### 5. Search Converter (Critical Hot Path)
- **Files:** `src/converters/search_converter.vr`
- **Functions Optimized:**
  - `search_request_to_query()` - 15ns per search
  - `filters_to_map()` - 15ns per search with filters

**Impact:** Search is one of the highest-frequency operations.

#### 6. Pagination Utilities (Critical Hot Path)
- **Files:** `src/domain/refinements.vr`
- **Functions Optimized:**
  - `calculate_offset()` - **30ns savings** (15ns * 2 params)

**Special Case:** Combines refinement types with &checked:
1. Refinement types eliminate runtime bounds checking
2. &checked eliminates CBGR overhead
3. Result: **Zero-cost abstraction** for pagination

**Impact:** Called on EVERY paginated endpoint (search, lists, etc.)

#### 7. Package Service
- **Files:** `src/services/package_service.vr`
- **Functions Optimized:**
  - `compute_checksum()` - 15ns per tarball upload

## Double Optimization: Refinement Types + &checked

The `calculate_offset()` function demonstrates the power of combining Verum features:

```verum
pub fn calculate_offset(page: &checked PageNumber, size: &checked PageSize) -> QueryOffset {
    (page - 1) * size
}
```

**Optimizations Applied:**
1. **Refinement Types** (`PageNumber`, `PageSize`) prove at compile-time:
   - `page >= 1` (no negative/zero pages)
   - `1 <= size <= 100` (sensible pagination bounds)
   - Eliminates all runtime bounds checking

2. **&checked References** prove at compile-time:
   - No escape beyond function scope
   - No concurrent modification
   - Eliminates CBGR generation/epoch checks

**Result:** The most performance-critical pagination operation becomes a zero-cost abstraction.

## Safety Guarantees

All `&checked T` optimizations maintain full memory safety through:

1. **Escape Analysis:** Compiler proves reference doesn't outlive its referent
2. **Immutability:** References are read-only during checked scope
3. **No Aliasing:** Compiler proves no concurrent mutations
4. **Local Scope:** References used within single function context

## SAFETY Comment Pattern

All optimized functions include a SAFETY comment documenting:
- Why &checked is safe (escape analysis, immutability, etc.)
- Performance impact (ns savings)
- Frequency of calls (hot path justification)
- Special considerations (e.g., large data iteration)

Example:
```verum
/// SAFETY: Uses &checked reference - CRITICAL hot path.
/// - called on every package publish and search request
/// - name is immutable during validation
/// - multiple string operations (starts_with, ends_with, contains) benefit from checked ref
/// - compiler proves no escape or concurrent modification
/// Performance: Saves ~15ns per validation * high request frequency = major gain
pub fn validate_package_name_checked(name: &checked Text) -> Result<ValidatedPackageName, RegistryError>
```

## Usage Guidelines

### When to Use &checked

✅ **Use &checked when:**
- Function is called frequently (hot path)
- Reference is read-only during function execution
- Reference doesn't escape function scope
- No concurrent modification possible
- Compiler can prove safety via escape analysis

✅ **Especially beneficial for:**
- Iteration over large collections
- Multiple string operations on same reference
- Validation functions called per request
- Conversion functions in request/response pipeline
- Pagination and filtering operations

❌ **Don't use &checked when:**
- Reference might escape (stored in struct, returned, etc.)
- Concurrent modification is possible
- Safety isn't obvious to compiler
- Function isn't performance-critical

### Prefer &checked over &unsafe

Unless absolutely necessary, prefer `&checked T` over `&unsafe T`:
- &checked: Compiler-proven safety (zero trust needed)
- &unsafe: Manual proof required (high trust burden)

## Validation Service: Standard vs. Checked

The codebase provides two versions of validation functions:

1. **Standard (`validation_service.vr`):**
   - Uses regular `&T` references
   - Full CBGR protection (~15ns overhead)
   - Use for non-critical paths

2. **Checked (`validation_service_checked.vr`):**
   - Uses `&checked T` references
   - Zero CBGR overhead
   - Use for performance-critical paths (publish, search, auth)

Import the appropriate version based on performance requirements.

## Performance Testing

To verify performance improvements:

1. **Benchmark before/after:**
   ```bash
   cargo bench --bench validation_bench
   cargo bench --bench checksum_bench
   cargo bench --bench pagination_bench
   ```

2. **Expected results:**
   - Checksum calculation: 15-30ns improvement per call
   - Validation: 15ns improvement per validation
   - Pagination: 30ns improvement per calculation
   - Large data iteration: Significant improvement (cache locality + no checks)

3. **Load testing:**
   ```bash
   # Test search endpoint (high pagination frequency)
   wrk -t4 -c100 -d30s http://localhost:8080/api/packages/search?q=test

   # Test publish endpoint (checksum + validation)
   # (requires authenticated requests)
   ```

## Future Optimizations

Potential areas for further `&checked` optimization:

1. **Handler functions:** Request/response processing
2. **Middleware:** Logging, auth, rate limiting
3. **Database mappers:** Result row to domain conversion
4. **Cache operations:** Serialization/deserialization helpers

## Conclusion

The `&checked T` optimizations applied to verum-registry hot paths demonstrate:

1. **Measurable Impact:** 15-30ns savings per operation × high frequency = significant gains
2. **Maintained Safety:** Compiler-proven memory safety via escape analysis
3. **Zero-Cost Abstractions:** Combining refinements + &checked achieves optimal performance
4. **Industrial Quality:** Proper documentation, safety comments, clear upgrade path

These optimizations align with Verum's philosophy: **semantic honesty** (types describe meaning), **gradual safety** (choose your performance/safety tradeoff), and **zero-cost abstractions** (pay only for what you use).
