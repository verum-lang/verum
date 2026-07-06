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
* **1b — grow-from-empty derived-view length garbage — OPEN.** An
  **empty** `Text.new()` (null buffer) + `push_str` then `as_str`
  SIGBUSes. Deep diagnosis (do NOT trust the first-order "null ptr"
  reading — it is wrong):
  - The Text FIELDS are all correct after the append: `s.len()` == 1,
    `s.capacity()` == 16, and the buffer content is right — reading
    `s.as_bytes()[0]` yields 81 (`'Q'`), which proves `self.ptr`@0 is a
    valid pointer to the grown buffer. So grow's `&mut self` writeback
    (ptr@0 / cap@16) and push_str's len@8 all persist correctly. The
    earlier "nested-writeback / offset-0-lost" theories are DISPROVEN.
  - The fault is in the DERIVED views: `s.as_bytes().len()` returns
    garbage (e.g. 4385112096) even though `s.len()` == 1, and `as_str`
    (which builds the same view) then reads that bogus length → OOB →
    SIGBUS. `s.as_bytes()[0]` still returns the right byte because
    indexing uses the (valid) base pointer, not the (garbage) length.
  So `as_str`/`as_bytes` compute a byte-view length that is correct for
  a heap-backed Text but garbage specifically for a Text whose buffer
  was grown from the `Text.new()` null-buffer state — a length-source
  (byte_len helper / view struct) that reads the wrong slot for that
  post-grow layout, NOT a pointer-writeback bug. Next step: dump the
  `as_bytes`/`verum_text_get_ptr` view construction and compare the
  length source for a `Text.new()`-grown buffer vs a `"".to_text()`
  buffer (the latter works). Baseline crashes identically (pre-existing,
  NOT introduced by 1a). Blocks build-from-scratch (Formatter/parser
  from `Text.new()`), so it is the remaining #1-lever work.

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
* **RECORD-LET-REF-TYPE-LOSS — OPEN.** In a GENERIC fn
  (`fn f<'a>(list: &'a List<LinkEntry>)`), `list[i].params[j].0.as_bytes()`
  fails method resolution (the tuple-field type is lost); the
  non-generic `format_link_header` with the identical chain works. The
  fix surface is `infer_expr_type_name` / `extract_expr_type_name` for
  field/tuple-index chains inside a lifetime-generic function body.
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
