# `archive/tar` audit

Module: `core/archive/tar.vr` (721 LOC) — POSIX/USTAR/PAX tar
archive encoder + decoder. Defines EntryType 9-variant + TarEntry
record + TarError 9-variant + entry_type_byte mapping +
entry_for_file/directory/symlink convenience ctors +
read_archive/write_archive top-level functions.

Tests: `unit_test.vr` (~36 unit tests covering 9-variant EntryType
+ USTAR typeflag byte mapping + 9-variant TarError + 3 convenience
ctors). Full read_archive/write_archive round-trip tests deferred.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.storage.s3` | tar+gzip is common multipart upload payload. |
| `core.cli` | `verum publish` packages cog source as tar+zstd. |
| Application backup/snapshot code | tar streams over network/disk. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/...` archive emission (if implemented)
must produce bit-identical USTAR header bytes verified by the
canonical-byte mapping in entry_type_byte tests above.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified entry_type_byte match arms

Pre-fix used bare `RegularFile` / `Directory` etc. Task #17/#39
collision risk. Qualified to `EntryType.<Variant>`.

### §3.2 Display + Eq + Debug for EntryType + TarEntry + TarError

EntryType + TarEntry + TarError don't have explicit Display impls
(TarError has Display per the source — but EntryType + TarEntry
lack them). Add for diagnostic dumps.

**Effort:** medium (~1h) — 9 variants × 2 types.

### §3.3 No `entry_for_block_device` / `_char_device` / `_fifo` ctors

Convenience ctors exist for file/directory/symlink. Add for the
remaining 4 EntryType variants (BlockDevice/CharDevice/Fifo +
PaxExtendedHeader/PaxGlobalHeader if applicable).

**Effort:** small (~30 min).

### §3.4 `TarEntry.uid` / `gid` is `Maybe<Int>` — semantics

Setting uid/gid to None on write — does the encoder emit zero or
omit? Document the contract. (Likely emits zero per USTAR spec —
no field can be truly omitted in 512-byte header.)

## Action items landed in this branch

* `core/archive/tar.vr` — qualified entry_type_byte match arms.
* `core-tests/archive/tar/unit_test.vr` — 36 unit tests over
  EntryType 9-variant + entry_type_byte typeflag canonical bytes
  (one test per variant verifying the USTAR character) +
  TarError 9-variant + 3 convenience ctors.
* `core-tests/archive/tar/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Display/Debug/Eq for EntryType + TarEntry | `core/archive/tar.vr` + tests | 1h |
| Add entry_for_block_device / char_device / fifo ctors | `core/archive/tar.vr` + tests | 30 min |
| Document Maybe<Int> uid/gid encode semantics | `core/archive/tar.vr` doc | 10 min |
| Add property_test.vr (write→read round-trip, USTAR byte invariants) | this folder | 2-3h |
| Add integration_test.vr against canonical USTAR/PAX fixtures | this folder | 1 day |
| Sister tests for `core.archive.mod` umbrella + compress integration | sister folders | 2h |
EOF
