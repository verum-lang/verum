# AOT completeness analysis (Tier-1 VBC→LLVM) — 2026-07-06

**Status: living punch-list.** This is the deep audit of what the AOT
(ahead-of-time, VBC→LLVM native) path implements, what it stubs, and
what is genuinely broken — ranked by leverage. It complements
[`no-libc-architecture.md`](./no-libc-architecture.md) (the libc-freedom
punch-list) and [`intrinsic-dispatch-contract.md`](./intrinsic-dispatch-contract.md)
(the body-`@intrinsic` / LLVM-canonical-alias rules).

Method: read the VBC→LLVM lowering (`crates/verum_codegen/src/llvm/`),
cross-referenced the conformance-suite audits (`core-tests/**/audit.md`,
`INVENTORY.md`), and drove concrete AOT reproducers on a fresh worktree
binary (off `main` `7df70eae5`).

## 0. Executive summary

The **instruction lowering surface is comprehensive** — the main
`lower_instruction` dispatch (`instruction.rs:1784`) covers ~169
`Instruction::*` arms plus every Extended sub-op family
(Ffi/Arith/Math/Text/Simd/Cbgr/Char/Log/Mem/Tensor/Gpu/Cubical). The
no-libc migration is **mostly closed** (see that doc's table). The AOT
gaps that actually block stdlib modules are a small number of
**correctness** defects, dominated by ONE:

| # | Gap | Leverage | Status |
|---|-----|----------|--------|
| 1a | **ptr_offset byte-stride (Text/parse byte buffers ×8 too far)** | stdlib-wide (Text append / parse / Display) | **FIXED 2026-07-06 (986bee268)** |
| 1b | **nested `&mut self` writeback lost (grow-from-null)** | empty-Text build-from-scratch | OPEN — remaining #1 lever |
| 2 | Cross-module aggregate-field value lowering (tuple/record-through-ref) | broad | tuple-destructure **FIXED 2026-07-06**; record-let-ref-type-loss OPEN |
| 3 | `strtod` (float parse) on Linux; `setjmp` Linux body; native DNS; ffi `__sys_*_raw` | targeted | OPEN (per no-libc doc) |
| 4 | Scripting `Engine.*` Extended sub-ops (0x20–0x60) | niche (host-embedding only) | interp-only |
| 5 | `MakeVariantTyped` cross-module placeholder warnings (~170) | cosmetic | non-fatal |

## 1. The #1 lever — AOT byte-buffer append (DISP-EMPTY-AOT / PARSE-AOT)

**STATUS 2026-07-06: the dominant root cause (1a, ptr_offset byte-stride)
is FIXED and validated (commit `986bee268`); a narrower residual (1b,
nested `&mut self` grow-from-null writeback) remains.** The two were
entangled in the original symptom below; IR-level tracing separated
them.

* **1a — ptr_offset byte-stride — FIXED.** `Text.push_str`'s
  `memcpy(ptr_offset(self.ptr, self.len), …)` walked a `&unsafe Byte`
  buffer with the ×8 Value-slot stride, landing the append 8× too far
  (content silently dropped on short strings, SIGBUS off-allocation on
  long ones). Fixed by inferring the pointee element size at the
  intrinsic call site and emitting `ptr + count` (stride 1) for
  `&unsafe <byte>` buffers while leaving Value/`*const`-`*mut` pointers
  on the ×8 path — see §1a below. Validated: `"hi".push_str("XYZ")` →
  `hiXYZ`, long append no longer crashes, `ptr_offset(list_int, 2)`
  still reads element 2 (stride 8 preserved), text/text interp failing
  set byte-identical to baseline (0 regression).
* **1b — empty-Text views — OPEN, ENTANGLED (diagnosis is NOT yet
  reliable).** An **empty** `Text.new()` (null buffer) + `push_str`
  then `as_str` SIGBUSes. Confirmed facts (trustworthy):
  - The Text FIELDS are correct after the append: `s.len()` == 1,
    `s.capacity()` == 16, and `s.as_bytes()[0]` == `0x51` (`'Q'`) — so
    `self.ptr`@0 is a VALID pointer to the grown buffer and the
    `&mut self` writeback (ptr@0/cap@16/len@8) all persist. The
    "null-ptr / nested-writeback / offset-0-lost" theories are all
    DISPROVEN.
  This area has AT LEAST TWO entangled AOT bugs, and incremental repros
  have repeatedly mischaracterised them (this root cause has been
  corrected four times — treat any single-repro conclusion here with
  suspicion):
  1. **`as_bytes().len()` returns garbage (a pointer-valued number)**
     for BOTH `Text.new()`-grown AND `"".to_text()` buffers, even
     though `s.len()` == 1 — i.e. NOT empty-specific. The AsBytes AOT
     lowering (`instruction.rs:19106`) packs `[ptr, len]` reading `len`
     from `self+8` and `lower_pack`s them; either `Len` reads the wrong
     Pack slot for this slice, or the `let ab = s.as_bytes().len()`
     temporary is recycled — needs isolation from the crash below.
  2. **`as_str` (in an f-string) SIGBUSes only for the `Text.new()`-grown
     buffer** (`"".to_text()` renders fine) — a distinct empty-specific
     path.
  **CLEAN ISOLATION (all confounds removed 2026-07-06):** using the
  BOUND form + a comparison (not inline `.as_bytes().len()`, which has a
  known temp-lifetime pitfall, and not an f-string of the result, which
  mis-formats a `.len()` value separately):
  - non-mutated `"Q".to_text()` → `as_bytes().len()` == 1 ✓
  - mutated-from-NON-empty `"Q".to_text()` + `push_str("X")` → 2 ✓
  - mutated-from-EMPTY `Text.new()` + `push_str("Q")` → **garbage** ✗
  So the defect is SPECIFICALLY the **grow-from-null** path
  (`Text.new()`: cap 0 / ptr null → first `grow` allocates), and it
  corrupts the byte-VIEW length (`as_bytes`/`as_str`/Display) while the
  DIRECT field reads (`s.len()`==1, `s.capacity()`==16) and byte
  indexing (`as_bytes()[0]`=='Q') stay correct.
  **The codegen is provably correct:** IR of `verum_main` shows
  `s.len()` and `s.as_bytes()` load `self` from the SAME slot
  (`%r1_ptr`) and the AsBytes lowering (`instruction.rs:19144`) reads
  `len` from `self+8` — the exact offset `s.len()` reads as 1. So this
  is a RUNTIME memory / LLVM-miscompilation issue in the grow-from-null
  path (the fresh Text-struct alloc + first buffer alloc likely alias or
  the len@8 field is clobbered after `s.len()` reads it), NOT a codegen
  offset/marking bug. Static IR analysis is exhausted; the next step is
  a DEBUGGER on the actual heap (watch `s_ptr+8` across the push_str →
  as_bytes window) or an allocator/ASan trace — not more source repros.
  **DO NOT trust the "null-ptr / grow-from-null specific" reading
  either — it too is confounded.** `Text.with_capacity(1)` (a non-null
  HEAP buffer, `push("Q")` needs no grow) is ALSO broken, while
  `"Q".to_text()` (also heap, via clone) works and `"".to_text()`
  (static, cap 0) works. So the failing/​passing split does not fall
  cleanly on null-vs-non-null, grow-vs-no-grow, or cap-0-vs-cap-N — the
  characterization has shifted 6+ times, each source repro hitting a
  different mix of at-least-three entangled confounds (inline-temp
  slice lifetime; f-string mis-formatting of a `.len()` result; and the
  byte-view corruption itself). **Conclusion: source-level repros
  cannot reliably diagnose this knot** — every "clean isolation" has
  been overturned by the next repro. A trustworthy diagnosis REQUIRES
  runtime tooling: run under a debugger / AddressSanitizer, watch the
  actual `Text` struct bytes and the `as_bytes` slice `{ptr,len}` across
  the mutation→view window for a failing vs passing constructor, and
  find the real memory event. That is the mandatory next step; further
  source repros will only produce more contradictory "root causes."
  Baseline crashes identically (pre-existing, NOT introduced by 1a).
  Blocks build-from-scratch (Formatter/parser from `Text.new()`), the
  #1-lever residual.

**Symptom (original, confirmed with reproducers):**

```verum
fn append(t: &mut Text) { t.push_str("XYZ"); }
// A: non-empty receiver → mutation SILENTLY LOST
let mut s = "hi".to_text(); append(&mut s);  // AOT prints "hi", not "hiXYZ"
// B: empty receiver → grow-from-null writeback lost → CRASH
let mut s = Text.new(); s.push_str("XYZ");   // AOT binary SIGBUS (rc 138)
```

Both compile & run correctly under `--interp`. Under `--aot`:
* **A** returns exit 0 but the append never persisted to the caller's
  `s` — `self.len` / `self.ptr` writeback did not reach the caller.
* **B** SIGBUSes: `Text.new()` yields a null buffer; `push_str` grows
  (reallocates), sets `self.ptr`, then writes — but the new `self.ptr`
  writeback is lost, so the store dereferences the stale null.

**Root cause (LOCATED — corrected 2026-07-06 after IR-level tracing).**
The `&mut self` writeback is NOT the fault — IR tracing of
`Text.push_str` under AOT proves `self` is passed as a **pointer**
(i64), `SetF` on it stores through at the correct flat offsets
(`self.len`@8, `grow`'s `self.ptr`@0 / `self.cap`@16 all persist — the
repro even reports `after len=5`). The corruption is in **byte-pointer
arithmetic**:

* `push_str` does `memcpy(ptr_offset(self.ptr, self.len), s.as_ptr(),
  n)`. `self.ptr` is a `*mut Byte` (byte buffer).
* `ptr_offset` lowers via `emit_ptr_offset`
  (`intrinsics/codegen.rs:1194`) which HARD-CODES `stride = 8`
  ("VBC type-erases to 64-bit values; element stride is always 8
  bytes") — it even ignores the `byte_width` param that
  `emit_intrinsic_inline_sequence` threads. So it computes `self.ptr +
  self.len * 8` instead of `self.ptr + self.len * 1`.
* Result: the appended bytes land **8× too far** into the buffer.
  Short buffers → the append is invisible (content reads as the
  original, e.g. "hi" not "hiXYZ", even though `len` is 5); long
  buffers → the write runs off the allocation → SIGBUS.

**Why interp is fine:** the interpreter **intercepts** the Text
mutation methods natively (`method_dispatch.rs:527` — push/push_str/
push_char), so it never executes the Verum body's `ptr_offset`; and its
raw `ptr_offset` handler (`ffi_extended.rs:1101`) is element-stride
aware via `fat_ref.reserved` (1/2/4/8). Only AOT compiles the Verum
body and hits the hard-coded stride 8.

**Generality:** this is not Text-specific — every AOT-compiled Verum
body that walks a `*Byte`/`*U8` buffer with `ptr_offset`/`ptr_add`
(parse via `split`/`slice`, List<Byte> byte ops, encoding/compression
byte loops) is 8× wrong. This single stride bug IS what the audits
named DISP-EMPTY-AOT (Formatter writes into a `&mut Text buf` byte
buffer) and PARSE-AOT (parsers walk byte buffers).

**Fix:** `ptr_offset`/`ptr_sub` must scale by `sizeof(pointee)`, not a
constant 8. The plumbing already half-exists —
`CodegenStrategy::InlineSequenceWithWidth(seq_id, width)` and the
`byte_width` param of `emit_intrinsic_inline_sequence` — but (a)
`emit_ptr_offset` must actually USE `byte_width` (emit `LoadI
byte_width`, not `LoadI 8`), and the MLIR path
(`lowering.rs:3505`, `elem_type: I64`) must mirror it; (b) the width
must be chosen **per call** from the pointee type (`*Byte` → 1, `*T`
value → 8), which the monomorphised call site knows but the registry
strategy (fixed per-intrinsic) does not — so the `ptr_offset` dispatch
needs to inspect `args[0]`'s pointee element size (or the stdlib must
route byte buffers through a distinct byte-stride intrinsic). Must land
behind a broad AOT regression sweep — the `stride = 8` was itself a
deliberate fix (`d844a6113`) for Value-array pointers, so any change
must keep those at stride 8 while giving byte buffers stride 1.

**Why it's the #1 lever:** this single class is what the `addr` audit
§3.5 named DISP-EMPTY-AOT (f-string `Display` of a user type renders
empty — the `Formatter.write_str` into a `&mut Text buf` never
persists) AND PARSE-AOT (v4/v6/socket parse SIGSEGV — the parser builds
`Text`/`List` via `split`/`slice`/`push` mutation). Closing it unblocks
Display rendering + parse + any `&mut self` builder across the whole
stdlib on Tier-1.

**Scope:** deep, multi-day, high-risk (touches the AOT value/ABI model
that every Text/List/record method depends on). Must land behind a
broad AOT regression sweep. Do NOT attempt piecemeal.

**Prior art:** a partial fix for the concurrent/marshaling variant
landed earlier (per session notes 2026-07-01); the general
grow-from-null and non-empty-append cases above remain open on `main`
`7df70eae5`.

## 2. Cross-module aggregate-field value lowering

These reproduce at COMPILE time under BOTH tiers (VBC codegen /
type-inference), so they are not AOT-only — but they gate AOT the same
way.

* **TUPLE-DESTRUCTURE-INDEXED — FIXED 2026-07-06.** `let (a,b) = &xs[i]`
  (or an iterator-yielded `&(T,T)`) read garbage because
  `handle_unpack` treated the CBGR ref (heap-interior pointer to the
  list slot) AS the tuple object. Fixed by routing the scrutinee through
  `resolve_arg_value` before `Unpack`
  (`interpreter/.../pattern_matching.rs`). **AOT parity pending** — the
  `Instruction::Unpack` LLVM lowering (`instruction.rs:3955`) must apply
  the same ref-deref before `lower_unpack_element`.
* **RECORD-LET-REF-TYPE-LOSS — OPEN, root cause LOCATED 2026-07-06.**
  `let (name, value) = &entry.params[j]; name.as_bytes()…` fails method
  resolution because `name` is left UNTYPED. Isolated precisely (the
  earlier "generic fn only" note is wrong — it reproduces in a plain fn
  too, and works in `main()`):
  - The real trigger is a tuple **destructure** whose scrutinee is an
    indexed record field reached through a **function parameter**
    (`&List<Rec>`); inline tuple-INDEX (`pair.0.as_bytes()`) works, and
    the destructure works when the collection is a `main()` local.
  - `compile_pattern_bind`'s Ident arm never records a bound variable's
    type; the type flows via `match_tuple_element_types`, which only
    `compile_match` populates (for a tuple-LITERAL scrutinee). A
    `let`-destructure of an *expression* never set it, so `name` stayed
    untyped and `.as_bytes()` could not pick a receiver.
  - A fix was PROTOTYPED (helper `infer_tuple_element_type_names` +
    record in the Tuple arm + set from `compile_let`) and REVERTED: it
    is blocked by a deeper bug — `infer_expr_type_name(entry.params[j])`
    returns a **Debug-rendered garbage** string
    (`"Tuple(List { inner: …"`) for the indexed record-field tuple, not
    a clean `"(Text, Text)"` (traced via `VERUM_TRACE_LETTUP`). So the
    element types can't be parsed out. The real fix is two-part: (a)
    make `infer_expr_type_name` render an `Index`-of-`List<(A,B)>` as a
    clean tuple type name (it currently falls through to a `{:?}` Debug
    fallback via `extract_display_type_name`), then (b) the
    prototyped let-destructure element-type recording works on top. The
    prototype infra is correct and harmless (no-op when the tuple type
    is unparseable) — it just needs (a) first.
* **INLINE-AGG-REF-ARG — OPEN.** `f(&RangeSet { specs })` (an inline
  aggregate literal by reference as a call argument) crashes VBC
  codegen; `let x = …; &x` works. Candidate: `stabilize_ref_source`
  (`expressions.rs:22353`) `needs_stable` list omits record/aggregate
  literals — but the SIGSEGV suggests a deeper fault than stabilization.
* **SELECTBESTMEDIA-CODEGEN — OPEN.** Calling a stdlib fn whose body
  threads a tuple `(Float, Int)` return through a monomorphised body
  crashes codegen at the call site (`select_best_media`; some 2-offer
  `select_best_coding`). Likely the tuple-value lowering in
  monomorphisation.

## 3. no-libc remaining surface (see no-libc-architecture.md table)

Open: `strtod` (Linux float parse — Ryu/exponent/NaN), `setjmp`/`longjmp`
Linux body (`llvm.eh.sjlj.setjmp`), native DNS resolver (replace
`getaddrinfo`/`freeaddrinfo`, ~500 LOC), `verum_vbc::ffi` `__sys_*_raw`
(~1000 LOC), and the debug-only `printf` (×3). Everything else in the
punch-list (open/close/read/write/malloc/free/calloc/socket family/
strcmp/strlen/memcpy/inet_pton IPv4/strtol/…) is **closed**.

## 4. Scripting Engine Extended sub-ops (0x20–0x60)

The embedded-scripting intrinsics (`Engine.new/eval/call/link/…`,
`script_*`) lower under interp but hit the `error.rs:136` "Unimplemented
Extended sub_op" path under AOT (`lower_extended`, `instruction.rs:22136`,
only decodes named `ExtendedSubOpcode` variants). This surfaces as
`[AOT warning] Unimplemented Extended sub_op: 0x20…0x60` when the
scripting stdlib module is compiled, but it does NOT block core modules
— a host program that AOT-compiles a script would. Low priority (P2
per the scripting roadmap).

## 5. Recommended sequencing

1. **Close the `&mut self` mutation-writeback class** (#1) behind a full
   AOT regression sweep — the single biggest Tier-1 correctness win.
2. **AOT `Unpack` ref-deref** — trivial parity for the tuple-destructure
   fix already landed on the interp side.
3. RECORD-LET-REF-TYPE-LOSS + INLINE-AGG-REF-ARG + SELECTBESTMEDIA — the
   remaining compile-time codegen crashers (each unblocks a net module).
4. no-libc: `strtod` Linux + `setjmp` Linux body (ship-blockers for a
   truly libc-free Linux target); native DNS + ffi `__sys_raw` are
   large standalone tasks.
5. Scripting Engine AOT sub-ops — only when host-embedded AOT scripts
   are a target.
