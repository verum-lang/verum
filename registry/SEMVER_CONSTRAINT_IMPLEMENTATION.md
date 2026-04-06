# Semantic Version Constraint Implementation

## Overview

This document describes the implementation of semantic version constraint matching for the Verum Registry, replacing the TODO at line 130-131 in `version_repository.vr`.

## Implementation Summary

### 1. New Module: `semver_constraint.vr`

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/semver_constraint.vr`

**Key Functions:**

#### `parse_constraint(constraint_str: Text) -> Result<VersionConstraint, RegistryError>`

Parses version constraint strings into `VersionConstraint` enum values. Supports:

- **Exact**: `"1.0.0"` or `"=1.0.0"` - matches exactly this version
- **Caret**: `"^1.0.0"` - compatible changes (npm/cargo style)
  - `^1.2.3` → `>=1.2.3 <2.0.0`
  - `^0.2.3` → `>=0.2.3 <0.3.0` (breaking in 0.x)
  - `^0.0.3` → `>=0.0.3 <0.0.4` (breaking in 0.0.x)
- **Tilde**: `"~1.0.0"` - approximately equivalent
  - `~1.2.3` → `>=1.2.3 <1.3.0`
- **Greater than**: `">1.0.0"`
- **Greater or equal**: `">=1.0.0"`
- **Less than**: `"<2.0.0"`
- **Less or equal**: `"<=2.0.0"`
- **Wildcard**: `"*"`, `"1.*"`, `"1.2.*"`
- **Any**: `"*"` - matches all versions

#### `satisfies(version: SemVer, constraint: VersionConstraint) -> Bool`

Checks if a version satisfies a constraint. Delegates to `VersionConstraint.matches()`.

#### `sort_versions_descending(versions: &mut List<PackageVersion>)`

Sorts package versions in descending order (highest version first) using bubble sort.

**Helper Functions:**

The module includes string manipulation helpers since they may not be in stdlib yet:
- `trim()` - remove whitespace
- `starts_with()` - check prefix
- `contains()` - check substring
- `substring()` - extract substring
- `split()` - split by delimiter
- `parse_int()` - parse integer from string

### 2. Enhanced `version.vr`

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/version.vr`

**Additions:**

#### `SemVer.parse(version_str: Text) -> Result<SemVer, Text>`

Parses version strings like `"1.2.3"` or `"1.2.3-alpha.1+build.123"` into `SemVer` structs.

**Helper Functions:**
- `parse_int_component()` - parse numeric version component
- `parse_patch_component()` - parse patch with optional pre-release/build metadata

**Updated `VersionConstraint.matches()`:**

Enhanced wildcard matching to support:
- `minor == -1` → matches any minor version (e.g., `"1.*"`)
- `minor >= 0` → matches specific major.minor (e.g., `"1.2.*"`)

**Updated `VersionConstraint.to_string()`:**

Properly formats wildcard constraints with `-1` minor version.

### 3. Updated `version_repository.vr`

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/version_repository.vr`

**Changes:**

Replaced the TODO (lines 130-131) with full implementation:

```verum
async fn find_matching_versions(package_name: PackageName, constraint: Text) -> Result<List<PackageVersion>, RegistryError>
    using [Database]
{
    // Get all versions for the package
    let all_versions = get_all_versions(package_name).await?;

    // Parse the constraint string
    let parsed_constraint = match parse_constraint(constraint) {
        Result.Ok(c) => c,
        Result.Err(e) => return Result.Err(e),
    };

    // Filter versions that satisfy the constraint
    let mut matching = List.new();
    for version in all_versions {
        if satisfies(version.version, parsed_constraint) {
            matching.push(version);
        }
    }

    // Sort by version (highest first)
    sort_versions_descending(&mut matching);

    Result.Ok(matching)
}
```

### 4. Updated `domain/mod.vr`

Added module declaration and exports:

```verum
module semver_constraint;

pub import .semver_constraint.{
    parse_constraint,
    satisfies,
    sort_versions_descending
};
```

## Test Coverage

### Unit Tests: `semver_constraint_tests.vr`

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/tests/semver_constraint_tests.vr`

**Test Cases:**

1. **Exact Constraints:**
   - `test_exact_constraint()` - basic exact matching
   - `test_exact_constraint_with_equals()` - with `=` prefix

2. **Caret Constraints:**
   - `test_caret_constraint_stable()` - stable versions (1.x)
   - `test_caret_constraint_zero_major()` - pre-1.0 (0.x)
   - `test_caret_constraint_zero_minor()` - very early (0.0.x)

3. **Tilde Constraints:**
   - `test_tilde_constraint()` - basic tilde matching
   - `test_tilde_constraint_zero_minor()` - edge cases

4. **Comparison Constraints:**
   - `test_greater_than_constraint()`
   - `test_greater_than_or_equal_constraint()`
   - `test_less_than_constraint()`
   - `test_less_than_or_equal_constraint()`

5. **Wildcard Constraints:**
   - `test_wildcard_any()` - `*` matches all
   - `test_wildcard_major_minor()` - `1.2.*` patterns

6. **Error Cases:**
   - `test_invalid_version_string()`
   - `test_empty_constraint()`
   - `test_incomplete_version()`

7. **Real-World Scenarios:**
   - `test_npm_style_caret_ranges()`
   - `test_pre_1_0_compatibility()`
   - `test_parsing_with_whitespace()`

### Integration Tests: `version_repository_constraint_tests.vr`

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/tests/version_repository_constraint_tests.vr`

**Test Cases:**

1. `test_filter_versions_with_caret_constraint()` - filter with `^1.0.0`
2. `test_filter_versions_with_greater_than_constraint()` - filter with `>=1.0.0`
3. `test_sort_versions_descending()` - verify sorting
4. `test_wildcard_matching()` - filter with `1.2.*`
5. `test_any_wildcard_matching()` - filter with `*`
6. `test_tilde_constraint_filtering()` - filter with `~1.0.0`

## Constraint Semantics

### Caret (^) - Compatible Changes

Following SemVer 2.0 compatibility rules:

| Constraint | Min Version | Max Version (exclusive) | Use Case |
|------------|-------------|------------------------|----------|
| `^1.2.3`   | 1.2.3       | 2.0.0                  | Stable API, allow minor/patch updates |
| `^0.2.3`   | 0.2.3       | 0.3.0                  | Pre-1.0, minor is breaking |
| `^0.0.3`   | 0.0.3       | 0.0.4                  | Experimental, patch is breaking |

### Tilde (~) - Approximately Equivalent

Allows patch-level changes only:

| Constraint | Min Version | Max Version (exclusive) |
|------------|-------------|------------------------|
| `~1.2.3`   | 1.2.3       | 1.3.0                  |
| `~1.0.0`   | 1.0.0       | 1.1.0                  |

### Wildcards

| Constraint | Matches |
|------------|---------|
| `*`        | Any version |
| `1.*`      | Any 1.x.y version |
| `1.2.*`    | Any 1.2.x version |

## Implementation Notes

### Design Decisions

1. **In-Memory Filtering**: The implementation fetches all versions from the database and filters in-memory. For production with many versions, consider SQL-level filtering.

2. **String Helpers**: Implemented custom string manipulation functions as they may not be in stdlib. These should be replaced with stdlib equivalents when available.

3. **Sorting Algorithm**: Uses bubble sort (O(n²)) which is adequate for typical package version counts (< 100). For packages with hundreds of versions, consider a more efficient algorithm.

4. **Error Handling**: All parsing errors are converted to `RegistryError.InvalidVersion` with descriptive messages.

5. **Delegation**: Version matching logic delegates to the existing `VersionConstraint.matches()` method to avoid duplication.

### Future Optimizations

1. **SQL-Level Filtering**: For large version counts, convert constraints to SQL WHERE clauses:
   ```sql
   WHERE (major = $1 AND minor >= $2) OR (major > $1 AND major < $3)
   ```

2. **Constraint Caching**: Cache parsed constraints to avoid re-parsing frequently used constraints.

3. **Index Optimization**: Add database indices on (package_name, major, minor, patch) for faster filtering.

4. **Pre-release Handling**: Currently ignores pre-release versions in comparisons. Future work could support pre-release constraints.

## Files Modified/Created

### Created:
1. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/semver_constraint.vr` (344 lines)
2. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/tests/semver_constraint_tests.vr` (265 lines)
3. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/tests/version_repository_constraint_tests.vr` (147 lines)
4. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/SEMVER_CONSTRAINT_IMPLEMENTATION.md` (this file)

### Modified:
1. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/mod.vr`
   - Added `module semver_constraint;`
   - Added public exports

2. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/version.vr`
   - Added `SemVer.parse()` method
   - Enhanced `VersionConstraint.matches()` for wildcard support
   - Updated `VersionConstraint.to_string()` for wildcards
   - Added helper functions: `parse_int_component()`, `parse_patch_component()`

3. `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/version_repository.vr`
   - Replaced TODO (lines 130-131) with full constraint matching implementation
   - Added import for `semver_constraint` functions

## Usage Example

```verum
// In a handler or service
let package_name = ValidatedPackageName.try_from("my-package")?;

// Find all versions compatible with 1.x
let versions = version_repo
    .find_matching_versions(package_name, "^1.0.0")
    .await?;

// Returns sorted list (highest first): [1.9.5, 1.5.2, 1.2.0, 1.0.0]
```

## Compliance

This implementation follows:
- **SemVer 2.0.0**: Semantic versioning specification
- **npm/Cargo conventions**: Caret (^) and tilde (~) behavior matches popular package managers
- **Verum principles**: No magic, explicit types, gradual verification
