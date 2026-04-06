# Repository Pattern Implementation

## Overview

This document describes the Repository pattern implementation in the Verum Registry, following the **Dependency Inversion Principle (DIP)** to decouple handlers from direct database access.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Handlers Layer                            │
│  (HTTP request handlers - high-level business logic)        │
│                                                               │
│  Using: [PackageRepository, UserRepository, ...]            │
└───────────────────────┬─────────────────────────────────────┘
                        │ depends on (abstractions)
                        ▼
┌─────────────────────────────────────────────────────────────┐
│                  Domain Layer                                │
│  - Repository Protocols (interfaces)                         │
│  - Domain Entities (Package, User, ApiToken)                │
│  - Error Types (RegistryError)                              │
│  - Refinement Types (ValidatedPackageName, Email, ...)      │
└───────────────────────▲─────────────────────────────────────┘
                        │ implements
                        │
┌───────────────────────┴─────────────────────────────────────┐
│               Infrastructure Layer                           │
│  - PostgresPackageRepository                                 │
│  - PostgresUserRepository                                    │
│  - PostgresTokenRepository                                   │
│  - PostgresPackageVersionRepository                          │
│                                                               │
│  Using: [Database] internally                               │
└─────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
src/
├── domain/
│   ├── repositories/
│   │   ├── mod.vr                          # Repository module exports
│   │   ├── package_repository.vr           # PackageRepository protocol
│   │   ├── user_repository.vr              # UserRepository protocol
│   │   ├── token_repository.vr             # TokenRepository protocol
│   │   └── version_repository.vr           # PackageVersionRepository protocol
│   ├── package.vr                          # Package domain entities
│   ├── user.vr                             # User domain entities
│   ├── errors.vr                           # RegistryError types
│   └── refinements.vr                      # Validated types
│
├── infrastructure/
│   ├── mod.vr                              # Infrastructure module exports
│   └── postgres/
│       ├── mod.vr                          # Postgres implementations exports
│       ├── package_repository.vr           # PostgresPackageRepository
│       ├── user_repository.vr              # PostgresUserRepository
│       ├── token_repository.vr             # PostgresTokenRepository
│       └── version_repository.vr           # PostgresPackageVersionRepository
│
└── handlers/
    ├── packages.vr                         # Package handlers (original)
    └── packages_with_repositories.vr       # Package handlers (updated with repos)
```

## Repository Protocols

### 1. PackageRepository

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/repositories/package_repository.vr`

**Purpose:** Abstracts package persistence operations.

**Key Methods:**
```verum
pub protocol PackageRepository {
    async fn get_by_id(id: EntityId) -> Result<Maybe<Package>, RegistryError>;
    async fn get_by_name(name: PackageName) -> Result<Maybe<Package>, RegistryError>;
    async fn save(package: Package) -> Result<(), RegistryError>;
    async fn delete(name: PackageName) -> Result<(), RegistryError>;
    async fn exists(name: PackageName) -> Result<Bool, RegistryError>;
    async fn find_by_owner(owner: Username) -> Result<List<Package>, RegistryError>;
    async fn list_all(offset: QueryOffset, limit: QueryLimit) -> Result<List<Package>, RegistryError>;
    async fn count() -> Result<Int, RegistryError>;
    async fn search(query: Text, offset: QueryOffset, limit: QueryLimit) -> Result<List<Package>, RegistryError>;
    async fn increment_downloads(name: PackageName) -> Result<(), RegistryError>;
    async fn update_metadata(package: Package) -> Result<(), RegistryError>;
}
```

### 2. UserRepository

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/repositories/user_repository.vr`

**Purpose:** Abstracts user account persistence.

**Key Methods:**
```verum
pub protocol UserRepository {
    async fn get_by_id(id: EntityId) -> Result<Maybe<User>, RegistryError>;
    async fn get_by_username(username: Username) -> Result<Maybe<User>, RegistryError>;
    async fn get_by_email(email: Email) -> Result<Maybe<User>, RegistryError>;
    async fn save(user: User) -> Result<(), RegistryError>;
    async fn delete(id: EntityId) -> Result<(), RegistryError>;
    async fn exists_by_username(username: Username) -> Result<Bool, RegistryError>;
    async fn exists_by_email(email: Email) -> Result<Bool, RegistryError>;
    async fn update_role(user_id: EntityId, new_role: UserRole) -> Result<(), RegistryError>;
    async fn deactivate(user_id: EntityId) -> Result<(), RegistryError>;
    async fn activate(user_id: EntityId) -> Result<(), RegistryError>;
}
```

### 3. TokenRepository

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/repositories/token_repository.vr`

**Purpose:** Abstracts API token persistence and lifecycle management.

**Key Methods:**
```verum
pub protocol TokenRepository {
    async fn get_by_id(id: EntityId) -> Result<Maybe<ApiToken>, RegistryError>;
    async fn get_by_value(token_value: TokenValue) -> Result<Maybe<ApiToken>, RegistryError>;
    async fn save(token: ApiToken) -> Result<(), RegistryError>;
    async fn delete(id: EntityId) -> Result<(), RegistryError>;
    async fn get_by_user(user_id: EntityId) -> Result<List<ApiToken>, RegistryError>;
    async fn update_last_used(token_id: EntityId, timestamp: Int) -> Result<(), RegistryError>;
    async fn deactivate(token_id: EntityId) -> Result<(), RegistryError>;
    async fn delete_expired(current_time: Int) -> Result<Int, RegistryError>;
}
```

### 4. PackageVersionRepository

**Location:** `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/domain/repositories/version_repository.vr`

**Purpose:** Abstracts package version persistence.

**Key Methods:**
```verum
pub protocol PackageVersionRepository {
    async fn get_version(package_name: PackageName, version: SemVer) -> Result<Maybe<PackageVersion>, RegistryError>;
    async fn get_all_versions(package_name: PackageName) -> Result<List<PackageVersion>, RegistryError>;
    async fn save(version: PackageVersion) -> Result<(), RegistryError>;
    async fn delete(package_name: PackageName, version: SemVer) -> Result<(), RegistryError>;
    async fn get_latest_version(package_name: PackageName) -> Result<Maybe<PackageVersion>, RegistryError>;
    async fn yank_version(package_name: PackageName, version: SemVer) -> Result<(), RegistryError>;
    async fn unyank_version(package_name: PackageName, version: SemVer) -> Result<(), RegistryError>;
    async fn is_yanked(package_name: PackageName, version: SemVer) -> Result<Bool, RegistryError>;
}
```

## PostgreSQL Implementations

### Implementation Pattern

All PostgreSQL implementations follow this pattern:

1. **Zero-state struct** - No fields needed, uses Database context
2. **Database context usage** - All operations delegate to Database context
3. **Parameterized queries** - Use `$1, $2, ...` to prevent SQL injection
4. **Row-to-domain mapping** - Convert database rows to domain entities
5. **Error conversion** - DatabaseError → RegistryError

**Example:**

```verum
pub type PostgresPackageRepository is {
    // No fields needed - uses Database context
};

implement PostgresPackageRepository {
    pub fn new() -> PostgresPackageRepository {
        PostgresPackageRepository {}
    }
}

implement PackageRepository for PostgresPackageRepository {
    async fn get_by_name(name: PackageName) -> Result<Maybe<Package>, RegistryError>
        using [Database]  // Uses Database context internally
    {
        let params = sql_params_1(name.clone());
        let row = Database.fetch_optional_with(
            "SELECT * FROM packages WHERE name = $1",  // Parameterized query
            params
        ).await;

        match row {
            Result.Ok(Maybe.Some(r)) => {
                match row_to_package(r) {  // Row-to-domain mapping
                    Result.Ok(pkg) => Result.Ok(Maybe.Some(pkg)),
                    Result.Err(e) => Result.Err(e),
                }
            },
            Result.Ok(Maybe.None) => Result.Ok(Maybe.None),
            Result.Err(db_err) => Result.Err(database_error_to_registry_error(db_err)),  // Error conversion
        }
    }
}
```

### File Locations

1. **PostgresPackageRepository**
   - Path: `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/package_repository.vr`
   - Implements: PackageRepository protocol
   - Uses: Database context

2. **PostgresUserRepository**
   - Path: `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/user_repository.vr`
   - Implements: UserRepository protocol
   - Uses: Database context

3. **PostgresTokenRepository**
   - Path: `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/token_repository.vr`
   - Implements: TokenRepository protocol
   - Uses: Database context

4. **PostgresPackageVersionRepository**
   - Path: `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/infrastructure/postgres/version_repository.vr`
   - Implements: PackageVersionRepository protocol
   - Uses: Database context

## Handler Updates

### Before (Direct Database Access)

```verum
pub async fn get_package(name: Text, version: Text) -> Result<PackageVersionDto, RegistryError>
    using [Database, Storage]
{
    let package_id = f"{name}-{version}";

    // Direct SQL query embedded in handler
    Database.fetch_one(f"SELECT * FROM packages WHERE id = '{package_id}'").await?;

    // ...
}
```

**Problems:**
- ❌ Handler depends on concrete Database implementation
- ❌ SQL queries embedded in business logic
- ❌ Hard to test (requires real database)
- ❌ Tight coupling to PostgreSQL
- ❌ SQL injection risk if not careful

### After (Repository Pattern)

```verum
pub async fn get_package(name: Text, version: Text) -> Result<PackageVersionDto, RegistryError>
    using [PackageVersionRepository, Storage]  // Depends on abstraction
{
    // Validate and parse inputs
    let pkg_name = ValidatedPackageName.try_from(name.clone())?;
    let semver = SemVer.parse(version.clone())?;

    // Use repository (no SQL in handler)
    let maybe_version = PackageVersionRepository.get_version(pkg_name, semver).await?;

    match maybe_version {
        Maybe.Some(pkg_version) => {
            // Convert domain entity to DTO
            // ...
        },
        Maybe.None => Result.Err(RegistryError.version_not_found(name, version))
    }
}
```

**Benefits:**
- ✅ Handler depends on abstraction (protocol), not concrete implementation
- ✅ No SQL in business logic
- ✅ Easy to test with mock repositories
- ✅ Can swap storage implementations
- ✅ Type-safe domain operations
- ✅ SQL injection impossible (parameterized queries in repository)

## Testing Strategy

### Unit Testing Handlers (with Repository Pattern)

```verum
// Create mock repository for testing
pub type MockPackageRepository is {
    packages: List<Package>
};

implement PackageRepository for MockPackageRepository {
    async fn get_by_name(name: PackageName) -> Result<Maybe<Package>, RegistryError> {
        // Return test data from in-memory list
        for pkg in self.packages.clone() {
            if pkg.name == name {
                return Result.Ok(Maybe.Some(pkg));
            }
        }
        Result.Ok(Maybe.None)
    }

    // ... implement other methods
}

// Test handler with mock repository
fn test_get_package() {
    let test_packages = List.new();
    test_packages.push(Package.new("test-pkg", "testuser"));

    let mock_repo = MockPackageRepository { packages: test_packages };

    provide [PackageRepository = mock_repo] {
        let result = get_package("test-pkg", "1.0.0").await;
        assert!(result.is_ok());
    }
}
```

### Integration Testing Repositories

```verum
// Test repository with real database
fn test_postgres_package_repository() {
    provide [Database = test_database] {
        let repo = PostgresPackageRepository.new();

        // Test save
        let pkg = Package.new("test-pkg", "testuser");
        repo.save(pkg).await.unwrap();

        // Test get_by_name
        let result = repo.get_by_name("test-pkg").await.unwrap();
        assert!(result.is_some());
    }
}
```

## Benefits Summary

### Dependency Inversion Principle (DIP)

**Before:**
```
Handlers → Database (concrete implementation)
```

**After:**
```
Handlers → Repository Protocol (abstraction) ← PostgresRepository
                                            ← MockRepository (for tests)
                                            ← RedisRepository (future)
```

### Key Benefits

1. **Testability**
   - Handlers can be tested with mock repositories
   - No need for real database in unit tests
   - Fast test execution

2. **Flexibility**
   - Can swap PostgreSQL for Redis, SQLite, etc.
   - Easy to implement caching layer
   - Can add read replicas

3. **Separation of Concerns**
   - Domain logic in handlers
   - SQL queries in repositories
   - Clear boundaries

4. **Type Safety**
   - Domain entities (Package, User) instead of raw rows
   - Refinement types enforce validation
   - Compile-time guarantees

5. **Maintainability**
   - SQL queries centralized in repositories
   - Easy to optimize queries
   - Single place to update schema changes

## Migration Guide

### Step 1: Update Imports

**Before:**
```verum
import super.super.contexts.{Database, Storage, Auth};
```

**After:**
```verum
import super.super.domain.repositories.{PackageRepository, UserRepository};
import super.super.contexts.{Storage, Auth};  // Keep non-data contexts
```

### Step 2: Update Function Signatures

**Before:**
```verum
pub async fn get_package(name: Text, version: Text) -> Result<PackageVersionDto, RegistryError>
    using [Database, Storage]
```

**After:**
```verum
pub async fn get_package(name: Text, version: Text) -> Result<PackageVersionDto, RegistryError>
    using [PackageVersionRepository, Storage]
```

### Step 3: Replace Database Calls with Repository Calls

**Before:**
```verum
let row = Database.fetch_one_with(
    "SELECT * FROM packages WHERE name = $1",
    params
).await?;
```

**After:**
```verum
let maybe_package = PackageRepository.get_by_name(pkg_name).await?;
```

### Step 4: Convert Between Domain and DTOs

**Before:** Worked with raw database rows
**After:** Work with domain entities, convert to DTOs

```verum
match maybe_package {
    Maybe.Some(package) => {
        // Convert Package (domain) to PackageDto (DTO)
        let dto = package_to_dto(package);
        Result.Ok(dto)
    },
    Maybe.None => Result.Err(RegistryError.package_not_found(name))
}
```

## Example: Complete Handler Migration

See `/Users/taaliman/projects/luxquant/axiom/registry/verum-registry/src/handlers/packages_with_repositories.vr` for complete examples showing:

- ✅ get_package
- ✅ publish_package
- ✅ yank_package
- ✅ list_packages
- ✅ list_package_versions
- ✅ update_package_metadata
- ✅ download_package_artifact

Each example shows the before/after comparison and demonstrates proper use of the Repository pattern.

## Future Enhancements

1. **Caching Layer**
   ```verum
   pub type CachedPackageRepository is {
       cache: Cache,
       backend: PostgresPackageRepository
   };

   implement PackageRepository for CachedPackageRepository {
       async fn get_by_name(name: PackageName) -> Result<Maybe<Package>, RegistryError> {
           // Check cache first
           if let Some(cached) = self.cache.get(name) {
               return Result.Ok(Maybe.Some(cached));
           }

           // Fallback to database
           let result = self.backend.get_by_name(name).await?;

           // Update cache
           if let Maybe.Some(pkg) = &result {
               self.cache.set(name, pkg.clone());
           }

           Result.Ok(result)
       }
   }
   ```

2. **Read Replicas**
   ```verum
   pub type ReplicatedPackageRepository is {
       write_db: Database,
       read_db: Database
   };

   implement PackageRepository for ReplicatedPackageRepository {
       async fn get_by_name(name: PackageName) -> Result<Maybe<Package>, RegistryError>
           using [read_db as Database]  // Use read replica
       {
           // ...
       }

       async fn save(package: Package) -> Result<(), RegistryError>
           using [write_db as Database]  // Use primary
       {
           // ...
       }
   }
   ```

3. **Event Sourcing**
   ```verum
   pub type EventSourcedPackageRepository is {
       event_store: EventStore,
       snapshot_store: SnapshotStore
   };

   implement PackageRepository for EventSourcedPackageRepository {
       async fn save(package: Package) -> Result<(), RegistryError> {
           // Save event instead of state
           let event = PackageCreated { package: package };
           self.event_store.append(event).await
       }
   }
   ```

## Conclusion

The Repository pattern implementation successfully decouples handlers from direct database access, following the Dependency Inversion Principle. This provides:

- **Better testability** through mock repositories
- **More flexibility** to swap storage implementations
- **Cleaner separation** between domain and infrastructure
- **Type-safe operations** with domain entities and refinement types
- **Production-quality code** with proper error handling and documentation

All repository protocols and PostgreSQL implementations are complete and ready for use.
