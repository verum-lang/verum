# `core.text.format` — audit

> Status: **complete**.  Sweep on 2026-05-27: 108 / 108 unit tests pass
> (+0 @ignore in unit_test.vr; +1 @ignore in regression_test.vr pinning
> the underlying §H auto-deref defect class).  41 pre-existing unit
> tests preserved; +67 new in commit `aebc0cd8f` across FormatSpec
> advanced builders, `int_to_text` + `int_to_text_radix` canonical
> entry points (with `Int.MIN`-safety pins), `format_hex` /
> `format_binary` / `format_octal` / `format_lower_exp` /
> `format_upper_exp` module-level wrappers, TextFormatter direct API
> (`write_str` / `write_char` / `write_int` / `write_int_base`
> accessors / padding-under-Left|Right|Center / precision-truncation /
> custom fill / type-spec dispatch / `alternate_form` prefix emission),
> and TextFormatter sign handling.
>
> **Closed since 2026-05-13 sweep**:
>   * §A — `format_display<T: Display>` / `format_debug<T: Debug>` —
>     transitively closed via the cumulative primitive method dispatch
>     fixes (regression pins flipped active 2026-05-15).
>   * §H — `format_hex` / `format_binary` / `format_octal` returned
>     hex-of-pointer-bits instead of hex-of-value (e.g. `format_hex(&255)`
>     returned `"-6c00000002"` instead of `"ff"`).  Surfaced 2026-05-27
>     by the new Section 12 unit tests.  Fix in `core/text/format.vr`
>     (commit `460508d8f` Maybe.Some/None sweep batch): explicit
>     `let v: Int = *value; v.to_hex()` materialises the deref into
>     a fresh `Int` register so the dispatch lands at the pointee
>     value, not the CBGR pointer bits.  See §H below for the
>     underlying defect class pin.

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
**Status**: **CLOSED 2026-05-15** — transitively via the cumulative
primitive method dispatch fixes earlier in the session.  Regression
pins `regression_a_format_display_int_pinned` and
`regression_a_format_debug_int_pinned` are now active green guards.

### §H — `(&Primitive).value_self_method()` auto-deref defect class
**Symptom**: every stdlib free function shaped
`public fn f(value: &Int) -> T { value.value_self_method() }` —
e.g. the pre-fix `format_hex(value: &Int) -> Text { value.to_hex() }` —
silently passed the raw CBGR pointer bits as the `self: Int` arg
instead of dereferencing the reference.  Pre-fix
`format_hex(&255)` returned `"-6c00000002"` (the hex of the pointer
to the receiver) instead of `"ff"` (the hex of the pointee value).
Same shape for `format_binary` / `format_octal`.

**Root cause**: Tier-0 codegen's `compile_method_call` for primitive
value-self methods (`Int.to_hex(self)`, `Int.to_binary(self)`,
`Int.to_octal(self)`) dispatched against a `&Int` receiver passes
the reference register as the `self` operand instead of materialising
the dereferenced value.  Equivalent to the codegen treating
`value.method()` as `(*value).method()` only when the method itself
takes `&self`, not when it takes `self`.

**Workaround landed** (commit `460508d8f` Maybe.Some/None sweep batch):
explicit local-binding deref in the three module-level wrappers:

```verum
public fn format_hex(value: &Int) -> Text {
    let v: Int = *value;   // materialises the deref into a fresh register
    v.to_hex()
}
```

The `(*value).to_hex()` shorthand is folded back into the bare-receiver
dispatch by current codegen — only the let-binding shape survives.

**Validation**: 3/3 `test_format_hex_*` + 3/3 `test_format_binary_*`
+ 3/3 `test_format_octal_*` + 4/4 `guard_format_*` GREEN.

**Underlying defect pinned**: `regression_h_ref_int_to_hex_auto_deref_pinned`
in `regression_test.vr` is `@ignore`d — it verifies that
`(&Int).to_hex()` returns `"ff"` for `v = 255`.  Removing `@ignore`
when the codegen fix lands flips it green.

**Fundamental fix path**:
`crates/verum_vbc/src/codegen/expressions.rs::compile_method_call`
— when the receiver register holds a `&Primitive` and the method
dispatch table entry's `self_kind` is `Value` (not `Ref` / `RefMut`),
emit a `Deref { src, dst }` before the `Call`/`CallM`.  Currently
this path falls through to the bare-receiver dispatch.

**Architectural rule** (pinned in `format.vr` source + this audit):
every stdlib free function of shape `public fn f(value: &Primitive)`
that calls a value-self method on `value` MUST materialise the deref
via local binding: `let v: Primitive = *value; v.method()`.  The
bare `value.method()` form passes pointer bits as `self` and produces
garbage.  Sibling pattern already established by `format_lower_exp` /
`format_upper_exp` (which use `float_to_exp(*value, …)`).

---

## 4. Action items

### Landed in this branch
- **108 unit tests** (41 pre-existing + 67 new in commit
  `aebc0cd8f`), 17 property tests + 7 integration tests + 2 regression
  pins + 12 PASS-GUARDs (8 pre-existing + 4 new §H guards).
- Validates: TextAlignment / Sign / WriteErrorKind / FormatSpec
  entirely (default + builder methods including `alternate_form` /
  `zero_padded` / `with_type`); WriteError factory functions;
  `int_to_text` + `int_to_text_radix` canonical entry points
  (Int.MIN-safety in both decimal and hex); `format_hex` /
  `format_binary` / `format_octal` / `format_lower_exp` /
  `format_upper_exp` module-level wrappers; TextFormatter direct API
  (`write_str` / `write_char` / `write_int` / `write_int_base` /
  accessors / padding-under-all-alignments / precision-truncation /
  custom fill / type-spec dispatch / `alternate_form` prefix
  emission); TextFormatter sign handling for `Plus|Space|Minus`
  modes; f-string interpolation for Int / Text / Bool / inline
  arithmetic; Debug specifier for primitives.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §H — fundamental `(&Primitive).value_self_method()` auto-deref fix in `compile_method_call` | multi-session VBC codegen | 1 (`regression_h_ref_int_to_hex_auto_deref_pinned`) + unblocks every stdlib free fn that could naturally take `&Primitive` and call a value-self method |

### Drift-pin recommendations
1. Pin the four-variant alignment / three-variant sign / three-variant
   write-error-kind layouts in
   `crates/verum_common/src/well_known_types.rs::FORMAT_VARIANT_PIN`.
2. Pin the `FormatSpec` field set + default values so a future stdlib
   refactor that renames fields surfaces immediately.
3. Add a stdlib-source-lint that rejects `value.method()` where
   `value: &Primitive` and `method` takes `self` (value form) — the
   `*value` discipline is currently enforced only by reviewer
   discipline (see commit `460508d8f` for the established pattern).
