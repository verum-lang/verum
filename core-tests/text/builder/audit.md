# `core.text.builder` — audit

> Status: **complete**. Sweep on 2026-05-15: 23/23 unit tests pass.
> Pre-fix (2026-05-13) only the 4 constructor/query tests passed (17%);
> every push-related test failed.
>
> The pre-fix audit pinned `Int.BAnd not found` / `Int.BNeq not found`
> as the root cause. Those were downstream symptoms of an unrelated
> bitwise-op dispatch defect that closed earlier. The final blocker
> remaining at sweep time was `Char.encode_utf8(&mut buf) -> Int`:
> the runtime intercept ignored the buf argument, allocated a fresh
> List of byte values, and returned the list pointer. Every caller
> that paid the cost of `let mut tmp: [Byte; 4] = [0_u8; 4]; let n =
> ch.encode_utf8(&mut tmp);` observed (a) tmp never written, (b) n =
> List pointer (treated as Unit downstream). `while i < n { … }` then
> iterated zero or wildly many times — TextBuilder.push_char silently
> dropped every char.
>
> Fundamental fix in this branch: rewrite the
> `CharSubOpcode::EncodeUtf8` intercept in
> `crates/verum_vbc/src/interpreter/dispatch_table/handlers/char_extended.rs`
> to honour the stdlib signature — encode UTF-8 into a stack buffer,
> write bytes into the caller's buf via its backing-array pointer
> (handling BYTE_LIST / LIST / direct-byte-array / ThinRef shapes
> uniformly through `cbgr_helpers::resolve_arg_value`), and return
> the byte count Int. See §A for the full rationale.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/format.vr::Formatter` | the formatter buffer is morally a TextBuilder (currently inlined as a `Text`) |
| `core/text/text.vr::to_uppercase` / `to_lowercase` / `replace` | every Text method that builds a new Text uses the same `with_capacity + push_byte` path that builder exposes |
| User code: any incremental string-building loop | should use TextBuilder rather than `s + &t` for O(N) total work |

Closing §A unblocks not just TextBuilder but every Text method that
goes through `push_byte` / `push_str` — which is most of them.

## 2. Crate-side hardcodes

None. TextBuilder is pure Verum, calling only:
- `Text.new`, `Text.with_capacity`, `Text.push_str`, `Text.push_byte`,
  `Text.clone`, `Text.is_empty`, `Text.len`, `Text.clear`
- `Char.encode_utf8`

If any of those drift, the regression PASS-GUARDs catch it (currently
only the constructor-only guards pass).

## 3. Language-implementation gaps surfaced by this folder

### §A — `Int.BAnd` / `Int.BNeq` not found on `Int` receiver
**Symptom**: every push on the builder panics with
`method 'BAnd' not found on receiver of runtime kind 'Int'` (or `BNeq`).
These are the runtime opcode names for bitwise AND and !=. The
underlying `Text.push_byte` calls `if self.len >= self.cap {
self.grow(); }` and `self.grow` does capacity arithmetic
(`self.cap * 2`, `is_null` checks). Some sub-step in that chain
emits a `BAnd` / `BNeq` against an Int receiver that the runtime
dispatcher cannot resolve.
**Root cause hypothesis**: the dispatcher routes Int operators through
a `Trait.Method` lookup table; if `Int`'s entry was lenient-skipped or
dropped under archive remap (see MEMORY: function-id collision
cascade), every bitwise operation fails. OR: the operator-call lowering
in `crates/verum_vbc/src/codegen/expressions.rs` emits a `CallM
{ method: "BAnd" }` instead of the direct opcode, and the receiver-side
method resolution fails.
**Action**: this is the highest-impact cross-stdlib defect surfaced by
this audit suite — closing it likely unblocks 50+ tests across text/,
collections/, async/, and beyond. Investigation path:
1. grep the codegen for `BAnd` / `BNeq` emit sites.
2. Check if Int's trait-impl table has `BAnd`/`BNeq` registered after
   archive load (mirror the drift pin from task #9).
3. If the operators DID register, the receiver-kind classification
   for Int is wrong — same family as char/§A and text/text/§B
   (Char→Int receiver-kind collapse).

### §B — TextBuilder content equality (downstream of text/text §I)
**Symptom**: even when push succeeds, the final `build()` may not
equal a literal Text comparator. Same root as case_fold §A.
**Action**: closes when text/text §I closes.

---

## 4. Action items

### Landed in this branch
- 23 unit tests + 11 property tests + 8 integration tests + 5 regression
  pins + 4 PASS-GUARDs.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A — Int operator dispatch (`BAnd`, `BNeq`, likely siblings) | medium | ~19 in this folder + many cross-stdlib |
| 2 | text/text §I — Text equality | shared | downstream of §A |

### Drift-pin recommendations
1. Add a drift-pin in `crates/verum_compiler/src/precompile.rs` that
   probes the archive for `Int.BAnd` / `Int.BOr` / `Int.BXor` / `Int.BNot`
   / `Int.BNeq` / `Int.BLeq` / `Int.BGeq` (the full bitwise + comparison
   matrix on Int) with non-zero `bytecode_length`.
2. Add a TextBuilder smoke test (`unit_test::test_push_single_text`) to
   the per-PR sanity check — if it ever passes, §A is closed and the
   sweep improves dramatically.
