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
| 1 | **`&mut self` mutation-writeback lost under AOT** | stdlib-wide (Text/List/parse/Display) | **OPEN — #1 lever** |
| 2 | Cross-module aggregate-field value lowering (tuple/record-through-ref) | broad | tuple-destructure **FIXED 2026-07-06**; record-let-ref-type-loss OPEN |
| 3 | `strtod` (float parse) on Linux; `setjmp` Linux body; native DNS; ffi `__sys_*_raw` | targeted | OPEN (per no-libc doc) |
| 4 | Scripting `Engine.*` Extended sub-ops (0x20–0x60) | niche (host-embedding only) | interp-only |
| 5 | `MakeVariantTyped` cross-module placeholder warnings (~170) | cosmetic | non-fatal |

## 1. The #1 lever — AOT `&mut self` mutation-writeback (DISP-EMPTY-AOT / PARSE-AOT)

**Symptom (confirmed with reproducers):**

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

**Root cause (LOCATED):** `lower_set_field` (`instruction.rs:31640`)
branches on the receiver's LLVM value shape:

* If `obj_val` is a 3-field CBGR-ref struct `{ptr, i32, i32}` (line
  31665), it extracts the pointer, GEPs past the object header to the
  field, and `build_store`s — a genuine **write-through** that persists
  to the caller. ✓
* If `obj_val` is any OTHER `StructValue` (line 31691), it does
  `build_insert_value` + `set_register(obj, new_struct)` — a
  **functional update** that rebuilds the struct value in the callee's
  register and never touches the caller's storage. ✗

Text is a flat 24-byte value (`{ptr@0, len@8, cap@16}`), so a `&mut
self` Text receiver arrives as a plain `StructValue`, NOT a CBGR ref —
it takes the `insert_value` branch, so `self.len`/`self.ptr` writes are
lost at the call boundary. `insert_value` is CORRECT for a local
struct mutation (`let mut p = Point{..}; p.x = 5`) but WRONG for a `&mut
self` receiver, and `lower_set_field` cannot tell the two apart from the
value shape alone.

**Fix (ABI change):** `&mut self` methods (and `&mut Text`/`&mut List`/
`&mut <record>` params) must receive the receiver as a **pointer** to
the caller's alloca — not the by-value struct — so every `SetF` takes
the store-through path. This is a caller-side + prologue change
(`lower_call_method` receiver marshalling `instruction.rs:11815` + the
VBC codegen `takes_self_mut_ref` receiver lowering, which currently
yields a by-value struct for flat aggregates rather than a pointer);
no callee-side patch to `lower_set_field` alone can fix it, because the
callee's `insert_value` result has nowhere to propagate. Requires a
broad AOT regression sweep (every Text/List/record method depends on
this path).

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
