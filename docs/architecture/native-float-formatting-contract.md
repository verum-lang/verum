# Native Float formatting contract

Status: implementation prerequisite, measured 2026-10-04. T1582 remains open;
this document does not claim a corrected native formatter or completed no-libc
migration. The [no-libc invariant](no-libc-architecture.md) applies to ordinary
numeric output. The remaining `printf` calls are defects, not a permitted runtime
requirement.

## Current boundary

The grammar defines formatting syntax, not a six-significant-digit default.
Existing execution paths disagree:

| Path | Current implementation | Measured behavior |
|---|---|---|
| Interpreter FloatToText / interpolation | Rust Float Display in `text_extended.rs` | Shortest round-trip digits, fixed notation; `-0`, `inf`, `-inf`, `NaN` |
| Interpreter Text.from_float | `text_static_runtime.rs` | Same default Float Display policy |
| Interpreter print(Float) | `debug.rs` | Display spelling, with `.0` on finite integral values, including `-0.0` |
| Native FloatToText / interpolation / lower_to_string | `verum_float_to_text` → `verum_internal_f64_to_decimal` | Six fractional digits; tiny values lose magnitude, large finite values reach out-of-range `fptosi`, signed zero is lost |
| Native print(Float) | Two Float arms of `lower_debug_print` | Buffered libc `printf("%g")`, six significant digits; mixed writer ordering differs |
| Pure source Text.from_float | `core/text/text.vr` | Independent six-digit/scientific implementation; treats finite magnitudes above 1.7e308 as infinity |

The first two rows establish the compatibility target for default conversion.
The `%g` spelling is not elevated into a language guarantee. Explicit precision
and exponential format specifications remain separate policies. A shortest
representation must not be rounded a second time to implement a precision
specifier: that can introduce double rounding.

The existing native integer formatter and target-aware writer are independent
of this correction. They must remain unchanged.

## Default f64 contract

1. Decode the sign, exponent and mantissa from IEEE-754 bits. Classify zero,
   infinity and NaN before the finite conversion kernel. Do not convert an
   arbitrary Float to an integer with `fptosi`/`fptoui`.
2. For a finite, nonzero f64, compute a decimal coefficient of at most 17 digits
   and a decimal exponent whose value rounds back to the original f64 under
   IEEE round-to-nearest, ties-to-even parsing. Choose the shortest coefficient
   within the valid interval and the nearest candidate at that length.
3. When two shortest decimal candidates are equally near, choose greater
   magnitude, matching the currently measured interpreter Display behavior.
   This **decimal output tie choice** is distinct from the ties-to-even binary
   parser rule. For example, exact 562949953421312.25 is displayed as
   `562949953421312.3`; both `.2` and `.3` round back to the same f64.
4. Default Display renders that coefficient and exponent in fixed notation,
   without exponent syntax, redundant trailing fractional zeros or a trailing
   decimal point. Preserve the sign of zero as `-0`. Examples: `1.25`,
   `0.0000001`, `100000000000000000000`.
5. Debug printing shares the same numeric kernel and spelling policy, adding
   `.0` only when a finite result otherwise has no decimal point. This yields
   `3.0` and `-0.0`. It must not rescale, recalculate or re-round the value.
6. NaNs render `NaN`; NaN sign/payload are not encoded in text. Infinities render
   `inf` and `-inf`. These classes are not finite round-trip-bit assertions.
7. Formatting is independent of locale, process rounding-mode changes, libc
   buffering and host target. The numeric kernel performs integer arithmetic;
   the final output uses the existing target-aware writer.

Float32 requires its own declared-precision policy. Widening an f32 to f64 and
then finding the shortest f64 representation does not in general produce the
shortest f32 representation. This f64 contract must not silently establish an
f32 contract from a widened carrier. Explicit precision, padding and alternate
exponent formats also require their own tests.

## Kernel and buffer design

Use the published [Ryū shortest-decimal algorithm](https://github.com/ulfjack/ryu)
with source provenance and its Apache-2.0 or Boost license retained. The examined
Rust translation is ryu 1.0.23. Separate the integer interval/coefficient kernel
from the presentation writer; do not call its default pretty printer and assume
it matches fixed Display notation.

The finite kernel uses the encoded f64 mantissa/exponent, exact multiplication
and shifts, powers-of-five tables and integer digit removal. It produces
`{ coefficient: u64, exponent: i32 }`. The decimal tie choice above is an
explicit policy difference from upstream's final decimal-even tie branch;
binary interval acceptance remains unchanged. An LLVM port must keep table
bounds, shift ranges and unsigned arithmetic exact. Avoid wide division and
inspect emitted object dependencies on each supported target rather than
assuming an integer LLVM operation cannot lower to a runtime call.

For fixed f64 notation, the largest output is 327 bytes including a minus sign:
`-0.` followed by 323 zeroes and the digit `5` for the smallest negative
subnormal. Debug's optional `.0` does not exceed this bound. A NUL-terminated
buffer therefore needs **328 bytes**. The old 32-byte buffer in
`RuntimeLowering::emit_verum_float_to_text` must change together with its
formatter; changing only the helper would create a buffer overwrite.

A caller supplies the bounded destination. The formatter returns the written
length; the Text constructor allocates exactly length + 1, copies that many
bytes and writes NUL. Ordinary print uses a fixed entry-block allocation so a
print inside a loop does not repeatedly grow the stack. No numeric path needs
`sprintf`, `snprintf`, `printf`, `fflush`, or a host formatting callback.

The LLVM emission unit should provide one shared kernel/presentation authority
for FloatToText and Float print. It must preserve idempotent helper emission,
internal linkage and target-neutral numeric IR. The writer remains the existing
target-specific syscall/system-interface authority.

## Causal evidence and its limits

The source `fn convert(value: Float) -> Text { f"{value}" }` was parsed and
compiled to VBC, then executed through both the interpreter and the emitted
LLVM/JIT path. The native function called the real `verum_float_to_text` and
actual Text allocation/access helpers; the formatter was not replaced by a
host stub. Across 12 endpoint/control inputs, the baseline had nine lexical
differences and seven finite bit-round-trip failures. Examples:

| Input | Current native conversion | Expected default Display |
|---|---|---|
| 1.25 | `1.25` | `1.25` |
| negative zero | `0` | `-0` |
| 1e-7 | `0.0` | `0.0000001` |
| -1e-7 | `-0.0` | `-0.0000001` |
| 1e20 | `9223372036854775807.775807` | `100000000000000000000` |
| f64 maximum | same invalid large-value result | fixed shortest finite decimal |
| minimum normal / minimum subnormal | `0.0` | nonzero fixed decimal |
| positive infinity | `Inf` | `inf` |

A separate reference experiment exercised the published integer kernel, then
rendered fixed notation. Its deterministic corpus contained 132,464 cases:
all encoded exponents with seven edge mantissas and both signs; three neighbors
around each representable decimal power from 1e-323 through 1e308 with both
signs; and 100,000 seeded bit patterns. The unmodified upstream decimal tie
policy produced 29 lexical differences from interpreter Display, with no
round-trip failures. Using the explicit decimal output tie policy above gave
zero lexical differences and zero round-trip failures on that corpus, with a
maximum of 327 bytes.

That reference result is **not evidence that Verum's emitted formatter is
fixed**, nor an exhaustive proof over all f64 bit patterns. The source/JIT
baseline still fails. The scratch source, logs and reference license material
are recorded in the T1582 task journal; no failing/ignored repository test is
presented as a completed implementation.

Round-trip comparisons used an independent correctly rounded Rust f64 parser.
The emitted `verum_internal_strtod` is a separate V0 parser with an i64 decimal
accumulator and repeated scaling. Its long-mantissa rounding and special-value
limitations mean it cannot be the sole formatter oracle. Native format→native
parse parity requires an independently verified parser correction; the formatter
must not truncate output to accommodate that parser.

## Required implementation gates

* Reproduce the source/JIT endpoint failures, then pass the same controls with
  the actual emitted formatter and real Text extent/length checks.
* Run the deterministic corpus against actual LLVM/JIT output, checking the
  lexical policy and finite bit round trips. Include decimal ties, neighbors of
  powers of two/ten, normal/subnormal transitions, both zero signs, both
  infinities and NaN classes.
* Check every buffer write and NUL extent with canaries, especially 327-byte
  output; keep fixed allocation outside repeated-print loops.
* Inspect the reachable numeric IR for out-of-range float-to-integer casts and
  external formatting calls. Verify object/import dependencies on supported
  targets; JIT on one host is not a cross-target no-libc proof.
* Integrate both Float print arms only after conversion passes. Redirect actual
  mixed Text/Bool/Int/Float output and assert exact source order and formatting,
  including a completion marker. Re-run the signed integer boundary controls.
* Preserve separate status for source Text.from_float, explicit precision and
  f32 if they do not yet route through the proven contract. T1581 remains open
  until its original mixed-output acceptance passes; T1582 is not closed by a
  design or reference-kernel experiment.
