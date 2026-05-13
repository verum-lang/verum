# `core.text.format` — audit

> Status: **partial**. Sweep on 2026-05-13: 39 / 41 unit tests pass
> (95%). The format module is the healthiest of all text submodules
> audited so far — variants, defaults, FormatSpec builder methods, and
> the f"{x}" interpolation surface for primitives all work cleanly. The
> two failures are in the `format_display<T: Display>` / `format_debug
> <T: Debug>` generic free functions which trigger a "TypeMismatch:
> expected closure" runtime error — the generic-fn dispatch path needs
> work.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/text.vr` | `Display for Text`, `Debug for Text` impls write into `Formatter` |
| `core/text/builder.vr` | `Display for TextBuilder`, `Debug for TextBuilder` |
| Every stdlib type with `Display` / `Debug` | uses `&mut Formatter` / `&mut TextFormatter` as the writer-back-end |
| `core/io/stdio.vr` | `print` / `println` route through `Display` |

The format module is the universal ABI across the stdlib for
"convert this value to text". Closing the closure-dispatch defect (§A)
unblocks the only programmatic alternative to f-strings for type-erased
formatting.

## 2. Crate-side hardcodes

| Path | What | Pin |
|---|---|---|
| `crates/verum_vbc/src/codegen/expressions.rs` | f-string lowering | Routes `f"…{x}…"` to `Display.fmt(x, fmt)` for default specs and `Debug.fmt_debug(x, fmt)` for `:?`; hardcodes the protocol-method names |
| `crates/verum_vbc/src/intrinsics/runtime/io.rs` | `print` / `println` | OS-side write path, reads from a Text buffer |
| `crates/verum_compiler/src/precompile.rs` | format-spec parser | Parses `{…:fill align sign #0 width.precision type}` at compile time and emits a `FormatSpec` value into the call site |

## 3. Language-implementation gaps surfaced by this folder

### §A — `format_display<T: Display>(&T) -> Text` TypeMismatch closure
**Symptom**: `format_display(&5)` panics with
`TypeMismatch { expected: "closure", got: "non-pointer", operation: "call_closure" }`.
The body constructs a TextFormatter and calls `T.fmt(value, &mut fmt)`
on the generic argument. The runtime appears to receive a non-closure
value where the dispatch expects a closure.
**Root cause hypothesis**: generic-fn lowering passes the dispatch
target as a callable; for primitive `Int` (and other non-closure
types), the call_closure intrinsic mis-fires. Likely fix at
`crates/verum_vbc/src/codegen/expressions.rs` — generic fn calls
through a Display protocol method should emit `CallM { method: "fmt" }`
on the receiver, not a `call_closure` indirection.
**Action**: trace `format_display` codegen end-to-end; pin a
verum-vbc-side test that calls a Display impl through generic code
and check it emits `CallM` not `CallClosure`.

---

## 4. Action items

### Landed in this branch
- 41 unit tests + 17 property tests + 7 integration tests + 2 regression
  pins + 8 PASS-GUARDs.
- Validates: TextAlignment / Sign / WriteErrorKind / FormatSpec entirely;
  WriteError factory functions; FormatSpec builder API; f-string
  interpolation for Int / Text / Bool / inline arithmetic; Debug
  specifier for primitives.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A — generic-fn closure-dispatch path for `format_display` / `format_debug` | medium | 2 (and unblocks programmatic formatting throughout the stdlib) |

### Drift-pin recommendations
1. Pin the four-variant alignment / three-variant sign / three-variant
   write-error-kind layouts in
   `crates/verum_common/src/well_known_types.rs::FORMAT_VARIANT_PIN`.
2. Pin the `FormatSpec` field set + default values so a future stdlib
   refactor that renames fields surfaces immediately.
