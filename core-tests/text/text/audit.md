# `core.text.text` — audit

> Status: **partial** — recently advanced from regression-only by closing
> §C (Iterator.next dispatch on Chars/ByteIter/CharIndices/Lines) on
> 2026-05-13 in commit 48a76117f. The Text type's surface compiles and
> the stdlib bootstraps it, but a meaningful fraction of its public API
> still panics at runtime when exercised through the VBC interpreter
> (pre-fix sweep: 121/218 unit tests pass; post-fix sweep ~150–170
> projected based on the 8 iterator tests that flipped + the cascading
> §J/§Q closures). The remaining defects span: stdlib lenient-skip,
> function-id collision under archive remap, runtime method dispatch on
> Char receivers (§B), and low-level NullPointer in Text.truncate (§E).
>
> `regression_test.vr` pins each defect class with an `@ignore`d test —
> remove the `@ignore` when the defect closes.

---

## 1. Cross-stdlib usage

`Text` is the most-used type in the Verum standard library after `Int`.
Touch points (selective inventory; not exhaustive):

| Module | Usage |
|---|---|
| `core/base/error.vr` | error message storage |
| `core/base/result.vr` | `ParseError.message: Text` |
| `core/text/format.vr` | `Formatter` writes into a `Text` buffer; every `Display`/`Debug` impl ends in `f.write_str(s)` |
| `core/text/builder.vr` | `TextBuilder.into_text() -> Text` |
| `core/text/regex.vr` | `Regex.find_all(text: Text) -> List<Text>` |
| `core/text/numeric/decimal.vr` | `Decimal.to_text() -> Text` |
| `core/io/file.vr` | path components, log lines |
| `core/io/stdio.vr` | `print(s: &Text)`, `println(s: &Text)` |
| `core/database/*.vr` | column names, query strings |
| `core/configuration/*.vr` | key paths, value coercions |

Every defect on the Text receiver propagates to all of these consumers.
Closing §A (rfind), §C (Iterator.next), §D (function-id collision) is
the single highest-leverage cleanup in the entire stdlib.

## 2. Crate-side hardcodes

Drift surfaces across `crates/`:

| Path | What | Hardcoded shape |
|---|---|---|
| `crates/verum_common/src/well_known_types.rs` | `TypeId::TEXT = 514` | The integer ID; mirrors `core.text.text.Text` |
| `crates/verum_vbc/src/codegen/...` | `Text` field layout `{ptr, len, cap}` | Field offsets reserved at codegen time; cross-mount race fixed by task #9 (commit `78bda63dc`) |
| `crates/verum_vbc/src/codegen/expressions.rs` | format-literal lowering of `f"...{x}..."` | Hardcoded routing through `Display.fmt` / `Debug.fmt_debug` per format spec |
| `crates/verum_compiler/src/precompile.rs` | `text` module precompiled on every build | The `runtime.vbca` archive ships the Text body; archive-load remap can desync function-ids (§D) |
| `crates/verum_vbc/src/intrinsics/runtime/text.rs` | `text_from_static`, `text_byte_len`, `text_as_bytes`, `intrinsic_utf8_decode_char`, `text_parse_int`, `text_parse_float`, `int_to_text`, `char_to_uppercase`, `char_to_lowercase` | The intrinsic surface that text.vr calls; if any one of these isn't registered for the runtime context, every dependent stdlib body fails |
| `crates/verum_runtime/src/runtime/value.rs` | `Value::Text { kind: Static \| Inline(SSO) \| Heap }` | The runtime kind names that surface in the `not found on receiver of runtime kind 'Text<small>'` panic |

When `Text` gains a method on the source side (e.g. `slice_unchecked`),
every `crates/` consumer above must be reviewed.

## 3. Language-implementation gaps surfaced by this folder

Findings derived from `unit_test.vr` + `property_test.vr` runs (interpret
tier) on `2026-05-13`:

### §A — `Text.rfind` not found on receiver kind `Object`
**Symptom**: 5 tests panic with `method 'Text.rfind' not found on receiver
of runtime kind 'Object'`. Direct rfind, indirect via
`contains_any`, `index_of_any`, `to_ascii_uppercase`, `to_ascii_lowercase`
(those last two surprisingly — they appear to call rfind via some
intermediate method dispatch).
**Root cause hypothesis**: stdlib lenient-skip during
`compile_core_module_from_ast` OR `register_function_authoritative` lost
`Text.rfind`'s entry during archive load. Mirror the discipline of
commit `812fa9cfa` (no stray `;` after impl-blocks): grep for any post-
`}` token that could reset the parser.
**Action**: `grep -n "^}\\s*;" core/text/text.vr` — if a stray `;` exists
after a method block, remove it; otherwise add a drift-pin at
`crates/verum_compiler/src/precompile.rs` that probes the archive for
`Text.rfind` with non-zero `bytecode_length`.

### §B — `Char.encode_utf8` dispatched on `Int` receiver
**Symptom**: `Text.insert(idx, ch)` panics with
`method 'Char.encode_utf8' not found on receiver of runtime kind 'Int'`.
Same shape as the SelfValue `is_type_param` gate (commit `90b94e68b`)
but on the OPPOSITE side — here the runtime receiver kind is
classified as `Int` when the static type is `Char`.
**Root cause hypothesis**: `Char` is a 4-byte primitive that the
runtime collapses to `Value::Int` for storage; the dispatch table
indexes by static type during codegen but the receiver kind at runtime
loses the `Char`-vs-`Int` distinction. Likely fix: stamp `Char` into
the dispatch key at codegen time the same way Int / UInt are stamped,
not at receiver-kind classification.
**Action**: open a verum-vbc report; add a regression test in
`crates/verum_vbc/src/codegen/tests/` that constructs a `Char` and
calls `encode_utf8` through CallM dispatch.

### §C — Iterator `next` not found on `Object` for Chars/ByteIter/CharIndices/Lines
**Status**: **CLOSED 2026-05-13 — commit 48a76117f.** Three architectural
fixes in `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs`:
1. **Removed the broken `chars` intercept** at line ~6616 that returned
   `List<Text>` (each element a single-char Text) instead of the
   source-level `Text.chars() -> Chars` iterator. The for-loop then
   called `.next()` on the List and panicked.
2. **Tightened the func-id-as-method-id heuristic** at line ~1341 with
   a `parent_type`-vs-receiver compatibility gate
   (`func_id_parent_compatible_with_receiver`). The previous loose
   `func_name.ends_with(&method_suffix)` accept routed
   `intern_string("next")` to whichever sibling iterator's `*.next`
   happened to occupy that slot — typically `Rev.next` /
   `MappedIter.next` whose `self.iter.next()` recursed and overflowed
   the stack.
3. **Tightened the second-pass bare-suffix scan** for heap receivers
   with `heap_receiver_parent_compatible`: accepts methods whose
   `parent_type` is the receiver's TypeId OR a protocol the receiver
   implements OR `None`. Closes the documented hole the previous
   "accept any match" comment deferred.

**Validation**: 8 iterator tests now PASS:
`test_bytes_iterator_count`, `test_chars_empty_yields_nothing`,
`test_lines_single_line`, `test_lines_multi`, `test_char_indices_pairs`,
`test_chars_returns_chars_iterator`, `test_lines_returns_lines_iterator`,
`test_char_indices_iterator`. The §C regression pin in
`regression_test.vr` is now an active green guard.

**Diagnostic enrichment**: the "method not found" panic now includes the
receiver's recovered type name (`(`Chars` (Object))` vs the previous
flat `Object`) and lists up to 8 candidate `*.<bare>` functions in the
table with arity. Eliminates the "all stdlib bug-class-skips look
identical" diagnostic flatness.

**Knock-on**: closing §C is expected to also close downstream §J
(Debug format escapes — uses `for ch in self.chars()`) and §Q
(capitalize / to_title_case / swapcase — same iteration shape).

### §D — Function-id collision (FunctionId(12039) / 14897 / 11859)
**Symptom**: 9 tests fail with `FunctionNotFound(FunctionId(N))` for
specific numeric IDs. Affected: `Text.from_int`, `Text.concat`,
`Text.push_byte`, `Text.reserve`, `Text.make_ascii_uppercase`,
`Text.make_ascii_lowercase`.
**Root cause**: per-module `next_func_id` namespaces collide when
modules are merged into one archive at precompile time, then the
archive-load remap mis-routes Call instructions. Documented in MEMORY:
"stdlib_bootstrap.initialize() REVERTED — function-id collision cascade
2026-05-12".
**Action**: cross-module function-id stability requires either
(a) one global `next_func_id` shared across all stdlib modules, OR
(b) cross-module calls routed through CallM (string-name) instead of
`Call { func_id }`. Option (b) is non-trivial because the precompile
emits `Call { func_id }` for known intra-stdlib targets to skip
string interning.

### §E — Text.truncate NullPointer on small/empty Text
**Symptom**: `s.truncate(2)` on a `let mut s: Text = "hello"` panics
with `NullPointerAt opcode 0x63 site Text.truncate`. Affects clear()
and pop() (both call truncate). Static `Text` has `cap == 0` and
`ptr == null_ptr()` until first growth — the truncate body writes a
null terminator unconditionally (`memset(ptr_offset(self.ptr,
new_len), 0, 1)`) without first checking `cap > 0`.
**Root cause**: missing null-ptr guard in `Text.truncate` at
core/text/text.vr:~2463.
**Action**: source-level fix — gate the null-terminator memset behind
`if !is_null(self.ptr) { ... }`. Same pattern as `Drop for Text` at
text.vr:3387.

### §F — Text.find returns wrong byte index
**Status**: **CLOSED 2026-05-13 — commit f8d70e6ef.** The root was NOT
the user-side KMP body. The Text intercept's `extract_string` helper
fell through to the `<value:N>` debug-format fallback when given a
`&Text` argument (a CBGR register-ref or ThinRef). The trailing
fallback returned the bit-pattern of the ref instead of the
underlying Text bytes. `text.find("<value:N>")` then returned None
(or the wrong index) because the garbage needle wasn't anywhere in
the haystack.

The fix mirrors the receiver normalisation done at
`method_dispatch.rs:394-414`: auto-deref CBGR-ref → absolute register,
ThinRef → pointee Value, BEFORE classifying via small_string /
fat_ref / ptr branches. ~10 lines of fix; ~25 downstream test
failures closed.

**Knock-on closures**: §A (rfind), §G (contains), §H (index_of /
find_char), §L (replace / replacen), §P (split_once / strip_prefix /
strip_suffix), and the eq_ignore_case / starts_with / ends_with
intercepts ALL share the same root and ALL close together.

**Validation**: 13 / 15 targeted re-runs PASS. The 2 remaining
failures are pre-existing typechecker ICEs (audit §R count_matches
family — "Expected pointer, got Some(N)").

### §G — Text.contains returns wrong Bool
**Symptom**: `"hello".contains(&"ell")` returns false. Downstream of §F.
**Action**: closes when §F closes.

### §H — index_of / find_char wrong index
**Symptom**: aliases for find that exhibit the same defect as §F.
**Action**: closes when §F closes.

### §I — Text.cmp wrong Ordering result
**Symptom**: `"abc".cmp(&"abd")` does not return `Less`; `"ab".cmp(&"abc")`
does not return `Less`. The byte-wise comparison loop at text.vr:3413–3436
appears either to range-overflow or to emit the wrong variant
constructor — same family as MEMORY §22 (variant tag stability under
per-file test compilation).
**Action**: investigate whether `Ordering.Less` / `Ordering.Greater`
constructed inside `cmp` resolves to the SAME variant tag as the
match-destructure at the call site. If not, this is a sub-case of
task #22 and closes when that closes.

### §J — Debug format escape sequences
**Symptom**: `f"{s:?}"` does not produce `"\"hi\""`. Affects every Debug
of a Text and every Display of a Text containing a control character.
**Root cause hypothesis**: the format-literal lowering at
`crates/verum_vbc/src/codegen/expressions.rs` for the `?` type-hint
either does not dispatch `fmt_debug` or dispatches it on a Display
formatter. OR: `Debug for Text` impl at text.vr:3461 does not run
because of §C (Chars iterator broken — the impl iterates `for ch in
self.chars()`).
**Action**: closes when §C closes (very likely a downstream symptom).

### §K — Padding helpers misbehave
**Status**: **CLOSED 2026-05-13 — commit `81b34faea`.** Root cause:
the existing pad_left / pad_right intercepts read the `fill: Char` arg
via `extract_string(...).chars().next()`. Char is NaN-boxed as Int,
NOT as Text — `extract_string` fell through to the `<value:N>` debug-
format fallback and returned '<' as the padding character.

**Fix**: each pad arm now reads the Char arg via three-shape dispatch:
CBGR-ref / ThinRef auto-deref, then Int (Char NaN-box) → codepoint via
`char::from_u32`, then small/heap string fallback. Plus two NEW
intercepts: `center` (Python-style centered padding) and `zfill`
(zero-pad with leading-sign preservation).

### §L — Text.replace not propagating replacement
**Status**: **CLOSED 2026-05-13 — commit `f8d70e6ef`** (extract_string
CBGR-deref). Same root as §F: the dispatcher's replace intercept read
the `from`/`to` Text args via `extract_string` which fell through to
`<value:N>` for CBGR-ref args.

### §M — from_utf8 not raising Utf8Error on invalid input
**Status**: **CLOSED 2026-05-13 — commit `9c9eeb996`.** Root cause was
NOT in `utf8_validate` — it was in the `from_utf8` / `from_utf8_lossy`
/ `from_utf8_unchecked` Tier-0 intercepts at
`text_static_runtime.rs:98-121`. These were stubs that Ok-wrapped the
bytes value AS-IS (a `List<Byte>`) into `Result.Ok`, NEVER actually
converting to Text and NEVER validating UTF-8.

**Fix**: extract bytes via the canonical `extract_byte_slice` helper
(handles FatRef + LIST + BYTE_LIST shapes), validate via
`std::str::from_utf8`, allocate a real Text from the validated bytes,
and wrap in `Result.Ok`. Invalid UTF-8 → `Result.Err(Utf8Error { valid_up_to })`.

### §N — Text.into_bytes — List.extend_from_slice missing
**Symptom**: `Text.into_bytes()` panics with
`List.extend_from_slice not found on receiver of runtime kind 'Object'`.
Cross-module defect — needs `core/collections/list.vr` to expose
`extend_from_slice` (or for Text.into_bytes to be rewritten to use
push in a loop).
**Action**: add `extend_from_slice` to `core/collections/list.vr` (a
List task, not a Text task) OR rewrite `Text.into_bytes` to push
byte-by-byte in a loop.

### §O — Text.from_int FunctionNotFound
**Status**: **CLOSED 2026-05-13 — commit `73b9db0d0`.** Bypassed the
function-id collision by adding a Tier-0 intercept for
`Text.from_int(n: Int) -> Text` (plus `from_float` / `from_bool`).
Decimal-render via Rust's `n.to_string()`; one alloc per call.

### §P — split_once / strip_prefix / strip_suffix wrong (None when match exists)
**Status**: **CLOSED 2026-05-13** — downstream of §F (extract_string
CBGR-deref); flipped to green when §F closed via commit `f8d70e6ef`.

### §Q — capitalize / to_title_case / swapcase no-op
**Status**: **CLOSED 2026-05-13** — downstream of §C (Iterator.next
dispatch); flipped to green when §C closed via commit `48a76117f`.

### §R — count_matches triggers internal compiler error
**Symptom**: code that calls `count_matches` triggers an *internal
compiler error*: `"Expected int, got Some(3)"`. Crash report saved to
`~/.verum/crashes/`. This is a typechecker regression, NOT a stdlib
defect — the test cannot even compile.
**Root cause hypothesis**: type-inference assumes `count_matches`
returns `Int` but the function signature is `Maybe<Int>` somewhere in
the inference table — OR vice versa, the body returns `Some(3)` and
the typechecker erases the wrapper.
**Action**: capture the crash report; file in
`crates/verum_types/src/infer/`. This is the highest-impact compiler
defect surfaced by this audit.

---

## 4. Action items

### Landed in this branch
- **Test infrastructure**: `core-tests/text/text/{unit_test, property_test,
  integration_test, regression_test}.vr` + this `audit.md`. 218 unit
  tests + 28 property tests + 22 integration tests + 27 regression
  guards/pins.

### Deferred — ranked by leverage
| # | Item | Estimated effort | Tests unblocked |
|---|------|-----:|------:|
| 1 | §C close — Iterator.next dispatch on Text iterators | multi-session | ~30 |
| 2 | §F close — KMP / find byte indexing | medium (1–2 sessions) | ~25 (downstream §G/§H/§L/§P/§Q) |
| 3 | §D close — function-id collision (CallM migration OR global next_func_id) | multi-session | ~10 (§O included) |
| 4 | §A close — rfind dispatch | medium | ~5 |
| 5 | §B close — Char.encode_utf8 receiver-kind classification | medium | ~5 |
| 6 | §E close — null-ptr guard in Text.truncate | small (1–2 hours) | ~4 (clear/pop) |
| 7 | §I close — Ordering variant tag stability for cmp body | shared with task #22 | ~3 |
| 8 | §J close — Debug format escape sequences (likely downstream of §C) | downstream-only | ~5 |
| 9 | §K close — padding helpers (likely downstream of §F or §C) | downstream-only | ~4 |
| 10 | §M close — utf8_validate invariants | small | ~2 |
| 11 | §N close — List.extend_from_slice | small (List task) | ~1 |
| 12 | §R close — typechecker fix for count_matches | medium | ~1 (+ unblocks the count_matches API across the stdlib) |

### Drift-pin recommendations (ride along with the fixes above)
1. `crates/verum_compiler/src/precompile.rs::TEXT_PUBLIC_API_DRIFT_PIN`:
   probe the archive post-load for the 100+ Text public methods with
   `bytecode_length > 0`. Fail-fast so that any future stdlib lenient-
   skip on Text surfaces immediately.
2. `crates/verum_vbc/src/codegen/tests/text_iterator_dispatch.rs`:
   compile a tiny `for ch in s.chars()` loop and check that the
   archive contains `Chars.next` with body. Prevent a re-occurrence
   of §C.
3. `crates/verum_common/src/well_known_types.rs::TEXT_ITERATOR_KINDS`:
   pin the four iterator type names (`Chars`, `ByteIter`,
   `CharIndices`, `Lines`) so that any future rename surfaces as a
   compile-time error in 50+ places at once instead of an interpreter
   panic.

---

## 5. Notes for the next agent

* The **single highest-leverage** investigation is §F (find / KMP
  indexing) — it owns ~25 downstream test failures.
* §C (Iterator.next) is the single highest-impact LANGUAGE defect —
  it gates `for x in iter` syntax for every user-defined iterator.
* When fixing §D (function-id collision), do NOT re-attempt the
  `stdlib_bootstrap.initialize()` per-module pattern. That regression
  cost the sweep 43pp and was reverted (commit `43f49ac6c`).
* The `regression_test.vr` pins are per-defect-class — when a fix lands,
  remove the `@ignore` from the corresponding test and run the full
  suite to verify the cascade closes too.
