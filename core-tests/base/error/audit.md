# Audit — `core/base/error.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/error.vr` (349 lines) |
| Tests | NEW from-scratch — `unit_test.vr` (~250 LOC), `property_test.vr` (~280 LOC, laws + @property + @test_case), `integration_test.vr` (~200 LOC, ?-chains, error chain, backtrace round-trip) |
| Hardcodes in `crates/` | minimal — the file is mostly metadata structures + protocol definitions |

## §1  Cross-stdlib usage

`ErrorProtocol` extends `ErrorSource` (in `protocols.vr`) which extends
`Describable`. The chain is well-formed — `protocols.vr` is the protocol
*definition* layer, `error.vr` adds backtrace support on top.

Consumers across `core/` are sparse — most stdlib errors today implement
only `Describable` + `ErrorSource` (the simpler protocol pair) and skip
the full `ErrorProtocol` (which adds `message()` / `backtrace()`).
Quick survey:

- `core/base/result.vr` — defines a simple `Error` newtype (not the same
  as `ErrorProtocol`); used as the boxed-error variant for `StdResult<T>`.
- `core/io/*` — most define their own error types implementing
  `Describable + ErrorSource` directly, skipping `ErrorProtocol`.
- `core/net/*` — same pattern.

**Observation:** `ErrorProtocol`'s value-add is the `backtrace()` slot,
but the runtime intrinsic that powers `Backtrace.capture()` is currently
a stub returning `Backtrace.empty()` (line 163-167 of error.vr). Until
that lands, `ErrorProtocol` is functionally indistinguishable from the
simpler `ErrorSource` protocol — explaining the low adoption.

**Action item (deferred):** wire `Backtrace.capture()` to the
DWARF/symbolize path in `verum_codegen` (POSIX) and the equivalent on
Windows. This unblocks meaningful adoption of `ErrorProtocol`.
*Scope:* sizeable — DWARF parsing or libunwind integration; ~600 LOC.

## §2  `format_error_chain` is single-level

`error.vr:340-349` documents the limitation explicitly:

> Note: Only one level of source chaining is supported because
> `ErrorSource.source()` returns `&dyn Describable` (which lacks a
> `source()` method). For deeper chains, use `ErrorProtocol` and
> traverse the chain manually.

The signature mismatch is the root cause: returning
`&dyn Describable` instead of `&dyn ErrorSource` collapses the chain
walker after one step. Multi-level chains would require either:

- Changing `ErrorSource.source()` to return `Maybe<&dyn ErrorSource>`
  (breaking change for current implementors).
- Adding a `source_protocol()` variant that returns the richer type
  alongside the existing one.

**Action item (deferred):** evaluate the breaking change. The current
implementor count is small (audit §1 inventory) so the impact is
contained, but it's still a stdlib API change that should land
deliberately, with a migration note.

## §3  `ErrorChain` iterator is incomplete

`error.vr:309-320` defines `ErrorChain` with a `current` field and a
`new()` constructor — but **no `Iterator` impl**. The doc-comment
example shows `for desc in chain { ... }` but the for-loop won't
typecheck against an iterator-less type.

This is a stdlib defect: either implement `Iterator` for `ErrorChain`,
or remove the type and the example. Same root cause as §2 — chain
traversal hits the `&dyn Describable` ceiling.

**Action item:** decide between completing `ErrorChain` (requires §2's
fix) or removing it. Logged as deferred — the right call depends on §2.

## §4  Drift surfaces

The `error.vr` file imports protocols from `protocols.vr`:
```text
mount core.base.protocols.{
    Describable, ErrorSource, Debug, Display, Clone, Eq, Hash, Hasher,
    Formatter, FormatError, Default, PartialEq,
};
```

If any of these are renamed or removed in `protocols.vr`, this file
breaks at the import site (compile-time error — no silent drift). The
mount-list is the contract.

No Rust-side hardcoding of error.vr-specific names found in `crates/`.
The file is pure stdlib.

## §5  Action items landed in this branch

- [x]  Scaffold `core-tests/base/error/` (no prior tests)
- [x]  Write `unit_test.vr` covering StackFrame construction +
       Eq/Clone/Display/Debug, Backtrace empty/from_frames/capture/Default/Clone +
       Display/Debug, ErrorProtocol custom-type impl,
       format_error_chain single-level
- [x]  Write `property_test.vr` covering StackFrame round-trip + Eq laws,
       Backtrace count laws + clone preservation, format_error_chain
       contains description, plus @property samples and @test_case truth
       table
- [x]  Write `integration_test.vr` covering custom error in Result,
       ?-operator chains with custom errors, format_error_chain on real
       two-level chain, backtrace wrapped in error
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **Backtrace.capture() — wire DWARF/libunwind.** Highest-leverage:
   makes ErrorProtocol meaningful. Multi-platform; ~600 LOC.
2. **format_error_chain — multi-level traversal.** Requires changing
   `ErrorSource.source()` return type or adding a parallel API. Breaking
   change; needs migration plan.
3. **ErrorChain iterator implementation** — currently incomplete; either
   complete (after §2) or remove with deprecation notice.
