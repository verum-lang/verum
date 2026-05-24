# `storage/types` audit

Module: `core/storage/types.vr` (239 LOC) — abstract object-store
protocol + data ADTs. Defines `ObjectStore` protocol + `ObjectMetadata`,
`StorageError` (8-variant), `PutOptions`, `GetOptions`, `ListOptions`,
`ListPage`, `PresignMethod` (4-variant), `PresignOptions` records.

Tests focus on the testable data surface (`StorageError`,
`PresignMethod`). The `ObjectStore` protocol needs a backend
instance and is tested at the language level via
`vcs/specs/L2-standard/storage/` against the in-memory mock adapter
(plus `core.storage.s3` integration tests for the real-S3 path).

## 1. Cross-stdlib usage

`StorageError`:
| consumer | how |
|---|---|
| `core.storage.s3` | maps S3 error responses → StorageError. |
| Application uploads | every `put`/`get`/`head`/`delete`/`list` returns Result<_, StorageError>. |

`PresignMethod`:
| `core.storage.s3` | SigV4 signing routes through method enum. |

`ObjectMetadata`:
| every successful `put`/`get`/`head` | returns ObjectMetadata. |

## 2. Crate-side hardcodes

None today. Future Rust-side intercepts for the hot `put`/`get`
zero-copy path (when implemented in `crates/verum_runtime/`) must
preserve the StorageError variant shapes — pinned by
`test_storage_error_variants_are_disjoint`.

## 3. Language-implementation gaps

### §3.1 `PresignOptions.expires_seconds` refinement — 1..=604800

The expires_seconds field carries `Int { >= 1, <= 604800 }`
refinement constraint (the AWS S3 hard limit on presigned URL
lifetime). Test surface for refinement violations requires
`@expected-runtime-panic` fixture — deferred until that lands.

### §3.2 `PutOptions` / `GetOptions` / `ListOptions` lack `Eq` impl

These options records have @derive(Clone, Debug) but no Eq. Adding
Eq would enable round-trip tests (build PutOptions → serialise →
deserialise → compare). Deferred until @derive Eq stabilises for
records with Map<Text, Text> fields.

### §3.3 `ListPage` token-based pagination not pinned

ListPage carries a `continuation_token: Maybe<Text>` for cursor-based
pagination. The protocol contract is "if Some(token), more results
available; pass token back via ListOptions.continuation_token". The
test surface for this is end-to-end against a backend; covered at the
language level.

## Action items landed in this branch

* `core-tests/storage/types/unit_test.vr` — 23 unit tests covering
  StorageError 8-variant construction + disjointness + Display
  rendering, PresignMethod 4-variant construction + disjointness.
* `core-tests/storage/types/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| `@expected-runtime-panic` for `PresignOptions.expires_seconds` boundaries | this folder | gated on test fixture |
| Eq impl + round-trip tests for PutOptions/GetOptions/ListOptions | `core/storage/types.vr` + tests | 2h |
| ListPage + ObjectMetadata round-trip tests | this folder | 1h |
| Sister tests for `core.storage.s3` adapter | `core-tests/storage/s3/` | 1 day |
