# VBC Defect-Class Catalogue

> Moved from the public website into the repo (T0135, 2026-07-16):
> an internal engineering changelog (defect classes, commit hashes,
> fix disciplines) belongs next to the code it describes, not on the
> user-facing site. Live counterpart: docs/architecture/tech-debt-register.md.


# VBC Defect-Class Catalogue

A living catalogue of recurring VBC codegen defect patterns that
surface as the stdlib conformance suite (`core-tests/`) is exercised
end-to-end. Each entry documents the **stable trigger** (the source-
side pattern that causes the defect), the **manifestation** (the
runtime symptom), and the **established fix discipline** (a source-
side workaround that closes the surface without waiting on the
underlying VBC codegen fix).

The catalogue is the single source of truth for "if you see this
symptom, look for this pattern, and apply this discipline" — both
for contributors writing new stdlib code and for the conformance
suite itself when it identifies new failure modes.

## 1. `extend_from_slice` intrinsic-chain SIGSEGV

| Field | Value |
|---|---|
| Defect class id | **EXTSLICE-1** |
| Stable trigger | `out.extend_from_slice(&src[start..end])` inside a stdlib helper function reachable from a user test module. |
| Manifestation | `verum: internal compiler error — fatal signal SIGSEGV (11)` inside `llvm::SmallVectorBase<unsigned long long>::grow_pod` during the precompile cascade for the stdlib module. |
| Probe | Call the helper from a user `@test`. SIGSEGV at compile time. |
| Fix discipline | Replace with byte-by-byte `out.push(src[i])` walk: `let mut __i = start; while __i < end { out.push(src[__i]); __i = __i + 1; }`. |
| Examples | net/cidr.slice_text (commit `be64f4e1e`), net/http_range.slice_range, net/http_cache.slice_vec, net/content_negotiation.trim_ws, net/link_header.slice_text, net/uri_template literal collection, encoding/cbor encode + decode, encoding/jcs push_bytes, encoding/msgpack push_all. |

## 2. `b"literal"` byte-string-literal SIGSEGV

| Field | Value |
|---|---|
| Defect class id | **BSTRLIT-1** |
| Stable trigger | `push_bytes(&mut out, b"literal")` or `out.extend_from_slice(b"literal")` to emit a fixed ASCII prefix. |
| Manifestation | Same LLVM SmallVector SIGSEGV as EXTSLICE-1. |
| Probe | Call the function that emits the prefix from a user test. |
| Fix discipline | Inline the literal byte-by-byte: `out.push(':' as Byte); out.push(':' as Byte); ...`. For 4-8 byte prefixes the cost is negligible vs the dispatch overhead. |
| Examples | ipv6_canonical.format_v4_mapped (`b"::ffff:"` → 7 individual pushes; commit `8233fad28`), http_range.encode_unsatisfiable (`b"bytes */"`), http_range.encode_content_range (`b"bytes "`), link_header.format_link_header (`b", "` + `b"; "`). |

## 3. Closure-via-`?`-chain on `Result` / `Maybe`

| Field | Value |
|---|---|
| Defect class id | **CLOSURE-RESULT-1** |
| Stable trigger | `parse_int(bytes).ok_or_else(|| StdlibError.Malformed(...))?` or `Other.parse(text).map_err(|e| MyError.Wrap(e))?` — closure inside a `?`-chained `Result`/`Maybe` conversion. |
| Manifestation | LLVM SmallVector SIGSEGV at the closure-desugaring + `?`-operator surface. |
| Probe | Same — call the wrapping function from a user test. |
| Fix discipline | Replace closure with explicit `match`: `match parse_int(bytes) { Maybe.Some(v) => v, Maybe.None => return Err(StdlibError.Malformed(...)) }`. |
| Examples | cidr.parse prefix_len lookup (commit `f649312c6`), ipv6_canonical.parse Ipv6Addr.parse wrap (same commit). |

## 4. Qualified `Result.Ok(`/`Result.Err(` match arms

| Field | Value |
|---|---|
| Defect class id | **QUALRESULT-1** |
| Stable trigger | `match r { Result.Ok(v) => ..., Result.Err(e) => assert(e.kind == X) }` — qualified arm form. The destructured binding goes through a wrong type path, corrupting subsequent field reads on the matched value. |
| Manifestation | Field-access on the bound variable returns corrupted data (e.g. `e.kind == X` returns `false` even when `e.kind is X` returns `true`). Originally surfaced as URL-8 (test_parse_empty_rejects). |
| Probe | Write the same test with bare `Ok(...)/Err(...)` arms — passes. With qualified `Result.Ok(...)/Result.Err(...)` — fails. |
| Fix discipline | Use bare `Ok(...)` / `Err(...)` match arms. `Result` is special-cased by the type checker; it isn't at risk of bare-variant first-wins collision. Bulk sweep landed across 82 stdlib test files in commit `74c074176` (718 sites). |
| Examples | URL-8 close (commit `8cf21a8be`); stdlib-wide sweep (commit `74c074176`). |

## 5. Transitive `&mut self` propagation through nested method calls

| Field | Value |
|---|---|
| Defect class id | **TRANSIENTMUT-1** |
| Stable trigger | `fn outer(&mut self, ...) { ... self.inner_method(...); }` where `inner_method` mutates `self.field`. The mutation is performed but doesn't persist after `outer` returns — VBC codegen loses the propagation. |
| Manifestation | `add_text` returns `Ok(())` but `set.len()` remains 0; subsequent reads on the supposedly-mutated field see the pre-call state. |
| Probe | Call `outer` then check the field; compare with the in-place equivalent that doesn't nest the call. |
| Fix discipline | Inline the inner mutation directly in `outer`: `self.field.push(c)` instead of `self.inner_method(c)`. |
| Examples | CidrSet.add_text (commit `92480c76b`) — replaced `self.add(c)` with `self.blocks.push(c)`. |

## 6. Chained `.wrapping_add(...)` on primitive types

| Field | Value |
|---|---|
| Defect class id | **CHAINMETHOD-1** |
| Stable trigger | `a.wrapping_add(b).wrapping_add(c).wrapping_add(d)` on `UInt32` (or analogous primitive). The chained dispatch misidentifies the receiver type as a generic iterator type (`SkipWhileIter`), producing "method not found" panic + dispatch retry loop. |
| Manifestation | `runtime: Panic { message: "method 'UInt32.wrapping_add' not found on receiver of runtime kind SkipWhileIter" }` OR `runtime: StackOverflow { depth: 16384, max_depth: 16384 }` (when the retry loop fires). |
| Probe | Call any function containing the chain (e.g. SHA-1/SHA-256/SHA-512 compress block from `Sha1.finalize()`). |
| Fix discipline | Break the chain into sequential `let`-bindings, one per `.wrapping_add` call: `let t1 = rotl32(a, 5); let t2 = t1.wrapping_add(f); let t3 = t2.wrapping_add(e); ...`. Each call now has a single-type receiver context. |
| Examples | Sha1.compress_block (commit `92a85244b`), Sha256.compress_block (commit `400dccb78`), Sha512.compress_block (commit `400dccb78`). |
| Residual | Workaround is partial — applied broadly but WS-6 (websocket accept_key) still stack-overflows post-rebuild; the actual dispatch defect is in a different code path within Sha1 (candidate: `[0; 64]` array initialization producing SkipWhileIter, or hot-loop array indexing). Multi-day VBC codegen investigation required for the underlying defect. |

## 7. `for x in slice` lowering SIGSEGV (CLOSED 2026-05-14)

| Field | Value |
|---|---|
| Defect class id | **SLICEITER-1** |
| Status | **CLOSED** at compiler layer 2026-05-14 (commit `7cbd0585d`). Discipline pinned for stdlib contributors. |
| Stable trigger | `for x in slice` where `slice` is a bare `&[T]` slice value (NOT `slice.iter()`). The `for x in &[T]` lowering tripped an LLVM `SmallVectorBase::grow_pod` SIGSEGV at codegen time. |
| Manifestation | LLVM SmallVector SIGSEGV during precompile cascade — same surface as EXTSLICE-1 but rooted in IR-emission for slice-iter lowering rather than List.extend_from_slice intrinsic dispatch. |
| Probe | `grep -rn "for [a-z_]+ in [a-z_]+\.as_bytes()\|for [a-z_]+ in bytes\b" core/` MUST return zero broken patterns at every commit. |
| Fix discipline | Use indexed-while: `let n = slice.len(); let mut i: Int = 0; while i < n { let x = slice[i]; ... ; i = i + 1; }`. Routing through `for x in slice.iter()` is also safe because it goes through the custom-iterator path (has_next/next CallM). |
| Examples | `Hasher.write` + `Formatter.write_bytes` migrated to indexed-while pattern (commit `7cbd0585d`) — closed the `Text.rfind` family transitively. |

## 8. Deferred-init `let x: T;` assigned in branch arms (CLOSED 2026-05-29)

| Field | Value |
|---|---|
| Defect class id | **DEFERRED-INIT-1** |
| Status | **CLOSED** at compiler layer 2026-05-29. |
| Stable trigger | A binding declared without an initializer (`let x: T;`) and then assigned in **more than one** mutually-exclusive control-flow arm (both legs of an `if`/`else`, or several `match` arms). |
| Manifestation | The function is lenient-compiled to a panic-stub; at runtime: `runtime: Panic { message: "[lenient] <fn> compiled to panic-stub: cannot assign to immutable variable: <name>" }`. |
| Root cause | VBC codegen tracked definite-assignment with a single flat `is_initialized` flag that was not branch-scoped. The first arm's assignment flipped it `true`; the sibling arm's assignment then tripped the immutable-reassignment guard. |
| Fix | Added a `declared_uninit` flag to `RegisterInfo` (`crates/verum_vbc/src/codegen/registers.rs`), set at the deferred-init declaration in `compile_let` (`statements.rs`). The guard in `expressions.rs` now reads `!is_mutable && is_initialized && !declared_uninit`, exempting deferred-init bindings while still rejecting reassignment of a normal `let x = v;` immutable. |
| Examples | `core/net/http2/hpack.vr::HpackDecoder.decode_literal` (`let name: Text;` / `let start: Int;`); also unblocks `core/database/mysql/wkb_decoder.vr` (3 sites). Pinned by `core-tests/net/http2/hpack/{unit_test,regression_test}.vr`. |

## 9. `Text.as_bytes()` slice passed across a call boundary SIGSEGV (CLOSED 2026-05-30)

| Field | Value |
|---|---|
| Defect class id | **TEXT-SMALLSTR-ASBYTES-1** (ENCODE-1) |
| Status | **CLOSED** at the stdlib layer 2026-05-30. The original "small-string slice dangles" framing was a **misdiagnosis** — corrected below. |
| Stable trigger | A `&[Byte]` produced by `Text.as_bytes()` passed as a function argument and then **iterated** by the callee via `slice.iter()` — canonically `encode_string(field.name.as_bytes(), false, &mut out)` → `out.extend_from_slice(input)`. |
| Manifestation | `verum: internal compiler error — fatal signal SIGSEGV (11)` at runtime under `--interp`. |
| Root cause (corrected) | NOT a dangling small-string slice. `Text.as_bytes()`'s FatRef is valid — `.len()` and index `[i]` read correct bytes for **both** small (NaN-box-inline) and heap Text (the `AsBytes` handler already heap-copies small strings). The crash is `List.extend_from_slice`'s `for item in slice.iter()`: a `SliceIter` stores the parameter slice in a struct field and yields elements via `&self.slice[front]`, and that stored-slice element-**reference** derivation is wrong for a FatRef whose provenance is not a List backing array. `slice.get(i)` (also `&self[idx]`) shares the flaw; only **index-by-value** (`slice[i]`) is provenance-safe. |
| Fix | Rewrote `core/collections/list.vr::extend_from_slice` from `for item in slice.iter() { push(item.clone()) }` to an index loop `while i < n { push(slice[i].clone()) }` — provenance-agnostic, clone-preserving, same defensive rationale as the `.get(i)` routing in `slice.vr` §B.2. |
| Validation | `--interp`: `encode_string(field.name.as_bytes(), false, …)` round-trips with correct content (small + heap); full `HpackEncoder.encode`→`decode` **raw** round-trip (single + multi) green; hpack suite 51/51. Un-ignores `core-tests/net/http2/hpack/unit_test.vr §7`. |
| Residual | (1) The deeper `SliceIter` element-reference derivation for non-List FatRef provenances is unfixed — any `for x in <as_bytes-slice>.iter()` still trips it. (2) The `HpackEncoder` Huffman default (`huffman_enabled: true`) round-trip mismatches — separate defect **HPACK-HUFFMAN-1**. |
| Examples | `core/collections/list.vr::extend_from_slice` (fix site), `core/net/http2/hpack.vr::HpackEncoder.encode_one` (trigger); characterised in `core-tests/net/http2/hpack/audit.md §3.3`. |

## 10. Bare-variant first-wins collision for archive-loaded payload ADTs (OPEN)

| Field | Value |
|---|---|
| Defect class id | **BAREVAR-ADT-1** |
| Status | **OPEN** — tracked (compiler task #17/#39). Source-side qualified-form discipline keeps the suite green. |
| Stable trigger | A payload-carrying user ADT defined in `core/` writes its **constructors or match-arm patterns in bare form** (`NotFound { .. }`, `Other { .. }`) inside its own `impl` / `Eq` / `Display` / `Debug`, AND the variant name is **not globally unique** across the stdlib. |
| Manifestation | When the ADT is loaded from the precompiled core archive, the bare variant name resolves **first-wins** to the *first* sibling-module variant registered under that name. The match arm then never fires on a real value of the intended type → `Eq.eq` falls to `_ => false`, `match` falls to `_`, `from_*` constructors return a mis-tagged value. Payload-free (unit) variants are unaffected. |
| Probe | `let a = T.Pay { x: 1 }; let b = T.Pay { x: 1 }; assert(a == b);` for an archive-resident `T` with a colliding variant name → fails. Same construction+match **in the same fresh-compiled file** passes (proves it is archive-load-specific). |
| Fix discipline | Qualify **every** constructor and match-arm pattern to `T.<Variant>` form in the ADT's own impls. (`Result`/`Maybe`/`Ordering` are type-checker-special-cased and exempt — see QUALRESULT-1.) Because the stdlib is embedded in the `verum` binary at `cargo build` time (`crates/verum_compiler/build.rs`), a `cargo build --release --bin verum` rebuild is required for the source edit to take effect. |
| Deep fix | Type-directed resolution of bare variant names when the enclosing expression's type is known, so the bare form binds to the correct ADT. Multi-day VBC codegen work. |
| Examples | `core/context/error.vr::ContextError` Eq (prior), `core/sys/windows/io.vr::WindowsIoDriverError`, `core/sys/windows/tls.vr::WindowsTlsError`, `core/sys/windows/thread.vr::WindowsThreadError` (this branch, 2026-05-29). |

## 11. Silent over-wide integer-literal truncation (OPEN)

| Field | Value |
|---|---|
| Defect class id | **INTLIT-OVERFLOW-1** |
| Status | **OPEN** — tracked. No silent-corruption guard yet; surfaces as wrong test values rather than a diagnostic. |
| Stable trigger | An integer literal whose magnitude exceeds its declared/suffixed type — e.g. an 18-hex-digit `0x5645525545_4D5F_5443_u64` (72 bits) assigned to a `UInt64`. |
| Manifestation | **No diagnostic.** The literal is parsed to `i128` (`verum_fast_parser/src/expr.rs:3304`, via `i128::from_str_radix(..).unwrap_or(0)`) and then **silently narrowed mod 2⁶⁴** to the runtime value. The 72-bit literal above becomes `4995148692846498883` (its low 64 bits) with no error. Literals exceeding `i128` silently become `0` (the `unwrap_or(0)`). |
| Probe | `let b: UInt64 = 0x5645525545_4D5F_5443_u64; print(f"{b}")` → prints the wrapped value, not an error. |
| Root cause | The lexer correctly stores the raw digits (`IntegerLiteral::as_u64` returns `None` on overflow), but lowering neither (a) range-checks the literal against its suffix/inferred type nor (b) rejects values exceeding `i128`. |
| Fix surface | Range-validate suffixed integer literals at parse/type-check time and emit a diagnostic (`E`-class) on overflow, mirroring Rust/Swift. Contained but touches the literal→type-check path; needs a stdlib-wide validation pass (some constants rely on two's-complement forms like `0xFFFFFFF6_u32` which *do* fit their suffix and must keep compiling). |
| Examples | `core-tests/sys/windows/tls/unit_test.vr:47` + `core-tests/sys/windows/mod/unit_test.vr:130` asserted `TCB_MAGIC` against an 18-hex-digit typo'd literal; both tests were silently *failing* (the wrong literal wrapped to a value ≠ the real 16-digit `TCB_MAGIC`). Fixed the test typo; the language defect remains. |

## 12. Single-field-newtype method-return / arithmetic-return unboxing (OPEN)

| Field | Value |
|---|---|
| Defect class id | **NEWTYPE-UNBOX-1** |
| Status | **OPEN** — tracked. Source-side working idiom keeps the suite green. |
| Stable trigger | A value of a single-field newtype (`type NtStatus is (Int32)`, `type WindowsDuration is (UInt64)`) produced by a **method/operator body** (e.g. `IoStatusBlock.status()` returning `NtStatus(...)`, or `d1.add(d2)` returning a `WindowsDuration`), then immediately consumed by a chained method (`.is_success()`, `.as_millis()`). |
| Manifestation | The newtype value (unboxed to its inner `Int`) reaches a `CallM` that dispatches by `(method_id, receiver runtime-kind)` with no type hint. When the method name is **ambiguous across the loaded import closure** (e.g. 8 distinct `.is_success` impls — `OSError`/`NtStatus`/`StatusCode`/…), codegen cannot statically resolve `<NewType>.<method>`, falls to `CallM`, and the runtime fails on the `Int` receiver: `NtStatus.is_success not found on receiver of runtime kind Int`. A Map/match-bound newtype value also corrupts control flow (the test "exits" with the raw inner value). Direct newtype values (e.g. `STATUS_*` consts, single-candidate closures) dispatch fine. |
| **Closure-dependence (important)** | The defect is **import-closure-sensitive**: in a *small* test closure with a single `.method` candidate, codegen resolves the call directly and it **passes**; in the *full* stdlib closure (many same-named methods) it fails. An isolated probe therefore UNDER-counts candidates and can give a misleading PASS — always confirm under the full conformance closure (`verum test --filter wx_`), not a 1–3-test isolation. |
| Probe | Under the full closure: `IoStatusBlock.new().status().is_success()` panics; a Map-retrieved `WindowsDuration` used via `.as_nanos()`/`.add()` corrupts. (`d1.add(d2).as_millis()` and `STATUS_SUCCESS.is_success()` pass — single-candidate / intercepted paths.) |
| Fix discipline | Read the inner field directly (`.0` / `as_nanos`) and avoid loop-accumulating Map-retrieved newtypes via the operator method. Where unavoidable, the call site loses nothing by being `@ignore`-pinned until the codegen fix lands. |
| Deep fix | Type-directed direct-call emission in `compile_method_call` (consult `infer_expr_type_name(receiver)` when `static_receiver_type` is None and the inferred type's method exists) + single-field-newtype boxing parity. See task #3 plan. |
| Examples | `core-tests/sys/windows/ntdll` (IoStatusBlock.status().is_success, full closure), `core-tests/sys/windows/time` (Map-retrieved WindowsDuration sum); same class as the core `Duration` unboxing defect (2026-05-27). |

## 13. Inline `&`-payload-variant arg clobbers `&mut self` field writeback (OPEN)

| Field | Value |
|---|---|
| Defect class id | **MUTSELF-MATCH-1** |
| Status | **OPEN** at the codegen layer — tracked. **Source-side fix discipline keeps the suite green:** bind payload-bearing variant args to a local before passing to a `&mut self` method (`let e = E.X { … }; fsm.step(&e)`). With this discipline the full multi-step FSM suite is GREEN; only two explicit inline-event probes stay `@ignore`'d to pin the defect. |
| Fix discipline | `let e = StreamEvent.X { … }; fsm.step(&e)` — bind first, never pass an inline `&E.X { … }` payload variant to a `&mut self` method. Inline **unit** variants (`&E.SendRstStream`) are safe; only **payload-bearing** inline variants trip it. |
| Stable trigger | A `&mut self` method (`fn step(&mut self, event: &E)`) called with an **inline-constructed payload-bearing variant** argument (`fsm.step(&E.SendHeaders { end_stream: false })`), where the method later writes a field (`self.state = next`). |
| Manifestation | The method computes and **returns** the correct value, but the trailing `self.<field> = …` writeback **does not persist** to the caller — the object never advances across calls. Tests that check the *return value* pass; tests that read the object's field after the call fail. |
| Probe | `let mut fsm = StreamFsm.new(1); let _ = fsm.step(&StreamEvent.SendHeaders { end_stream: false }); assert_eq(fsm.state(), StreamState.Open);` — fails (state still `Idle`). The **parameter-passed** event variant (`fn helper(setup: &E) { fsm.step(setup); … }`) persists correctly, isolating the trigger to the inline `&`-constructed payload-variant argument aliasing the `&mut self` receiver storage. |
| Not the cause | A stdlib reformulation binding `let cur = self.state;` before `match (cur, event)` did **not** fix it — the defect is at the call site (inline `&`-arg vs `&mut self`), not the match shape. Unit (payload-free) variant args and direct field-write methods (`Http2DynamicTable.insert`) persist fine. |
| Fix surface | VBC codegen: an inline `&`-constructed argument must be materialised in storage that does not alias / clobber the `&mut self` receiver, so the post-call field writeback survives. |
| Examples | `core/net/http2/stream.vr::StreamFsm.step`; characterised in `core-tests/net/http2/stream/audit.md §3.1`. |

## 14. `<` between two no-suffix-declared `Float` consts mis-compares (CLOSED 2026-05-30)

| Field | Value |
|---|---|
| Defect class id | **FLOATCONST-CMP-1** |
| Status | **CLOSED** at the codegen layer 2026-05-30 (commit `e0ac5220a`). `infer_expr_type_kind` now resolves a cross-module Float const's type via its const-FunctionInfo `return_type_name`, so ordered compares select `CmpF`. |
| Stable trigger | An **ordered** comparison (`<` / `>`) between two `public const`s of type `Float` declared **without** a `_f64` literal suffix (`public const X: Float = 0.7;`). |
| Manifestation | The ordered compare returns the wrong result (`0.7 < 0.85` → `false`), even though `assert_eq(X, 0.7_f64)` and `assert_eq(Y, 0.85_f64)` both pass and `Y < 1.0_f64` (const-vs-literal) passes. The value compares **equal** via `EqF` but **mis-orders** via `LtF`/`CmpF`. |
| Probe | `assert(CUBIC_BETA < CUBIC_FAST_CONV_FACTOR)` (0.7 < 0.85) fails; `assert(CUBIC_FAST_CONV_FACTOR < 1.0_f64)` passes; `_f64`-suffixed consts (`core/net/quic/recovery/cc/bbr.vr`) compare fine const-vs-const. |
| Fix surface | VBC codegen for a no-suffix `Float`-typed `public const` must store the canonical f64 NaN-box so `LtF`/`CmpF` reads it identically to `EqF`. |
| Fix discipline | Declare `Float` consts with the `_f64` suffix: `public const X: Float = 0.7_f64;`. |
| Examples | `core/net/quic/recovery/cc/cubic.vr` (CUBIC_C/BETA/FAST_CONV); characterised in `core-tests/net/quic/recovery/cc/cubic/audit.md §3.1`. |

## 15. Cross-module record field-index shift (CLASS-9 / D2b, CLOSED 2026-05-30)

| Field | Value |
|---|---|
| Defect class id | **CROSS-MODULE-FIELDSHIFT-1** (CLASS-9 / D2b) |
| Status | **CLOSED** at the compiler layer 2026-05-30 (commit `64607bb8e`). |
| Stable trigger | A record type declared in one stdlib module is constructed or read (`self.<field>` in an instance-method body, or a field write in a `.new()` / `.null_pointer()` static constructor) from a body compiled in a **different** module — i.e. the accessing module did not itself declare the type, so it has no local `TypeDescriptor` for it. |
| Manifestation | `self.<field>` reads and constructor field writes land on **wrong slot offsets** (e.g. reading `self.type_name` returns slot 2 = `expected_epoch`). Downstream this surfaces as wrong field values, `Eq` mismatches, and — when a mis-read slot is interpreted as a pointer and dereferenced — SIGSEGV. The long-standing CLASS-9 / field-shift family. |
| Root cause | `resolve_field_index` resolves a locally-declared type via its string-authoritative `TypeDescriptor`, then falls back to the positional `type_field_layouts` map, then to a non-positional **global-intern** fallback. For a record declared in another (earlier-compiled) module the accessing codegen had neither a local descriptor nor a `type_field_layouts` entry, so it hit the global-intern fallback and wrote non-positional indices. |
| Fix | A **TypeId-free** type-layout registry threaded through the stdlib bootstrap (`verum_compiler/src/pipeline/stdlib_bootstrap.rs` + `pipeline.rs`): each module exports its record field layouts (type-name → declared-field-names, declaration order) into a global registry; every subsequently-compiled module imports them (additive, first-wins) via `import_type_layouts` (`verum_vbc/src/codegen/mod.rs`). `resolve_field_index`'s positional `type_field_layouts` path now resolves cross-module record fields instead of the global-intern fallback. |
| Why it is safe | **Descriptor-first** — a module's own `TypeDescriptor` still resolves locally-declared types first, so an imported layout can never shadow a local declaration. **TypeId-free** — the registry carries pure name→field-name lists and cannot perturb TypeId allocation or descriptor tables; this is precisely why it succeeds where the reverted `type_name_to_id` backfill (`ab8e707f4` → `585728904`) regressed `UseAfterFreeError` record-literal field writes. Payload-less sum types never enter the registry, so variant dispatch is structurally unaffected. |
| Validation | `--interp`, 83 GREEN, 0 regressions: `UseAfterFreeError` 13/13 (`.message()` body field reads + `.new()` / `.null_pointer()` cross-module constructors + record-literal canary), epoch D1 2/2, `RevocationError` 19/19, `thin_ref` 10/10, `epoch_cache` 5/5, `http2/error` 29/29, hpack `HeaderField` 5/5. Full `core/*.vr` re-precompile clean. Un-ignores the D2/CLASS-9 pins in `core-tests/mem/{mod,thin_ref,epoch}`. |
| Examples | `core/mem/mod.vr::UseAfterFreeError.message`, `core/mem/thin_ref.vr::UseAfterFreeError.new` / `.null_pointer`; pinned by `core-tests/mem/{mod,thin_ref,epoch}/unit_test.vr`. |

## 16. `List<T>` equality compares ObjectHeader bytes (CLOSED 2026-05-30)

| Field | Value |
|---|---|
| Defect class id | **LISTEQ-1** |
| Status | **CLOSED** at the stdlib layer 2026-05-30. |
| Stable trigger | `==` / `!=` / `assert_eq` between two **non-empty** `List<T>` values. |
| Manifestation | Returns *not-equal* for two **equal** non-empty lists — `assert_eq([1, 2, 3], [1, 2, 3])` fails, `[5] == [5]` is `false`. **Empty** lists compare equal (the comparison loop is skipped), which is what made the bug latent. |
| Root cause | `implement<T: Eq> Eq for List<T>` read elements via an inline `unsafe { &*self.ptr.offset(i) }`. In a method body codegen resolves `self.ptr` to the heap-object **START** (not past `OBJECT_HEADER_SIZE`), so the comparison read `ObjectHeader` bytes instead of elements and mismatched for every non-empty list. `List.get(i)` avoids this because it is **runtime-intercepted** to the canonical `[len@0, cap@1, backing_ptr@2]` layout; the inline `ptr.offset` is not. Same defect family as the documented `List.append` raw-pointer note. |
| Why it stayed latent | The 145-test `core-tests/collections/list` suite asserts element-wise / by index / by length — it never used `assert_eq(list, list)`. `core-tests/base/ordering` `in_map` / `in_filter` were among the few call sites that compared whole lists. |
| Fix | Route `eq` / `ne` through `self.get(i)` / `other.get(i)` (the §B.2 defensive accessor) instead of inline `self.ptr.offset(i)`. |
| Validation | `--interp`: `assert_eq([1,2,3],[1,2,3])`, negatives, `[5]==[5]`, `!=` all green; `base/ordering` 9/9 (`in_map`/`in_filter` fixed); list suite regression green. |
| Examples | `core/collections/list.vr` `Eq for List<T>` (eq/ne); surfaced by `core-tests/base/ordering/unit_test.vr` (`test_ordering_in_map` / `_in_filter`). |

## 17. AOT `verum_internal_memcpy` no-op stub — int/float f-string renders empty (CLOSED 2026-05-31)

| Field | Value |
|---|---|
| Defect class id | **AOT-MEMCPY-1** |
| Status | **CLOSED** at the codegen layer 2026-05-31 (commit `89604fe94`). |
| Stable trigger | Any AOT (Tier-1 native) program that interpolates an **Int or Float** into an f-string: `f"x={5}"`, `f"{2+3}"`, `f"{3.5}"`. |
| Manifestation | The interpolated number renders **EMPTY** under AOT (`f"x={5}"` → `x=`) while `--interp` is correct — a silent cross-tier soundness divergence. `Text`-only interpolation (`f"{some_text}"`) was unaffected, masking it. |
| Probe | `print(f"x={5}")` → AOT prints `x=`, interp prints `x=5`. Confirmed by `otool -tV` disassembly: `_verum_internal_memcpy` was `mov x0,#0; ret`. |
| Root cause | `verum_int_to_text` / `verum_float_to_text` copy their decimal digits via `verum_internal_memcpy`. `define_text_ir_helpers` pre-declares that symbol **bodyless**; `get_or_declare_memcpy` early-returned on *any* existing function and never emitted the body, so the linker lowered the bodyless internal function to a no-op stub → 0 bytes copied → fresh Text buffer stayed zeroed → `strlen()==0` → empty. (`Text.Concat` uses `llvm.memcpy` directly, so literal concatenation worked.) |
| Fix | `get_or_declare_memcpy` (`verum_codegen/llvm/runtime.rs`) only early-returns when the function already has a body (`count_basic_blocks() > 0`); otherwise it fills the existing bodyless forward declaration with the real `llvm.memcpy`-wrapping body. |
| Validation | `--interp` ≡ AOT for `f"{int}"` (incl. negatives / large), multi-placeholder, and `f"{float}"`; exit 0. |

## 18. AOT `for x in list.iter()` SIGSEGV (OPEN)

| Field | Value |
|---|---|
| Defect class id | **AOT-ITER-1** |
| Status | **OPEN** (one of two sub-bugs CLOSED 2026-05-31, commit `5bb3b83f8`). Found while validating AOT-MEMCPY-1. |
| Stable trigger | AOT-compiled `for v in xs.iter()` (or `it.next()`) over a `List<T>`. List construction, `.len()`, and indexing (`xs[i]`) all work; the iterator deref crashes. `--interp` is correct. |
| Manifestation | Native binary **SIGSEGVs** (exit 139) on the first `next()`. |
| Root-cause (via `otool -tV` disassembly) | Two distinct sub-bugs. **(1) generic_eq deref of the sentinel — FIXED:** `next()`/`next_back()` guarded with `self.ptr == self.end` on raw `&unsafe T` pointers; AOT lowered `==` to `verum_generic_eq`, which **dereferences** operands to compare contents, and `self.end` is the one-past-the-end sentinel → deref past array → segv. **(2) raw backing-pointer deref — OPEN:** after fix (1), `&*self.ptr` still faults even though `xs[i]` works. `OBJECT_HEADER_SIZE=24`, so `iter()`'s offsets (`len@0x18`=slot0, `ptr@0x28`=slot2) are *correct*; indexing reads the same slot2 fine via the runtime intercept. So the raw `&unsafe T` backing pointer is **not directly dereferenceable** the way the intercepted `xs[i]` accessor reads it — a pointer-representation difference (likely a CBGR-managed/tagged backing ptr the intercept normalises and raw `&*ptr` does not). Same family as **LISTEQ-1** ("raw `self.ptr` in method bodies"). |
| Fix (1), landed | Compare bounds by address: `(self.ptr as Int) == (self.end as Int)` (the `as Int` form `size_hint`/`len` already use). `next()` now emits `subs` instead of `bl verum_generic_eq`; interp unaffected. |
| Fix (2), pending | Either (A) redesign `ListIter` index-based over an **AOT-safe `&T`-yielding** accessor (note `List.get` returns `Maybe<T>` *by value*, but the iterator's `Item=&T` and for-loop bodies use `*v`, so a `get_ref`-style intercepted accessor is required), or (B) make raw `&unsafe T` field deref normalise the backing pointer like the intercept. Both deep; part of the "AOT largely unvalidated" gap. |
| Root cause CONFIRMED 2026-06-03 (lldb) | The `&unsafe T` struct field is laid at the WRONG offset. `verum_codegen/src/llvm/register_types.rs:451` handles `TypeRef::Reference { inner, .. } => Self::from_type_ref(inner)` ("references are transparent in AOT — use inner type"), so `ListIter.ptr`/`ListIter.end` (`&unsafe T`) collapse to their generic inner `T` (variable-size GenericParam) instead of a pointer (8B). The generic struct then lays `end` at **offset 0x20** (FatRef size) while the ctor writes a thin ptr lower; `next()`'s `ldr x8,[x8]` reads the wrong offset → garbage ptr → SIGSEGV on `&*self.ptr`. lldb confirmed `x8 = 0x20` at the faulting load. Two earlier leads were **superseded**: (a) the mono specializer (`specializer.rs:960-961`) does clone fields without substituting type params but its sole caller is `specialize_new_g`, the generic-construction path, not iterator field resolution; (b) the `is_value_type_ptr` deref-marking at `instruction.rs:4010` is downstream of the bad offset. **Fix = a ref-typed struct FIELD must be pointer-sized (8B) regardless of inner**, scoped to struct-field-layout context (the transparent-ref model is used everywhere else — method receivers, `&self` — so a blanket change is high-regression). Rebuild + full AOT re-run; gated on a focused low-contention window so the ~528 passing AOT tests aren't put at risk. |
| Mitigation (in progress 2026-06-03) | `core-tests/mem/*` for-iter tests are being converted to the **verified AOT-safe index form** — `let mut i=0; while i<xs.len() { let v = xs[i]; …; i=i+1; }` with `*v`→`v` (indexing yields the value through the runtime intercept). Range-iters (`for i in 0..n`) are already AOT-safe and left unchanged. This keeps the mem suite green on **both** tiers while Fix (2) is gated. 53 sites across 24 files; `cap_audit` done first as the worked reference. |

## 19. Archive-loaded record ref-returning method SIGSEGV + bare-name field-shift (CLASS-9 residual — BOTH FACETS CLOSED)

| Field | Value |
|---|---|
| Defect class id | **ARCHIVE-METHOD-MAYBEREF-1** (CLASS-9 residual) |
| Status | **BOTH FACETS CLOSED** (found 2026-06-01 building `core-tests/context/`). |
| Facet #1 — ref-returning method | Calling an **archive-loaded** record method returning `Maybe<&T>` borrowed from `self.<List-field>[i].as_ref()` (e.g. `Row.get_index(0)`) used to SIGSEGV at execution-compile. **CLOSED** by codegen between 2026-06-01 and `a1293ff52` (record-alloc / `Self {}`-literal / transparent-ref family — §15 lineage). Validated: the `get_index` regression pin + 3 unit tests (`match` + `*s` deref) GREEN; un-`@ignore`'d. |
| Facet #2 — bare-name record-vs-variant collision (NEW CLOSE 2026-06-04) | Reading the record's OWN fields from user code (`r.columns`/`r.values`) panicked `field index 4 ... type='Row'/'List'`. ROOT CAUSE (`[RECTYPE]` instrument): `Row` is a 3-way name collision (a PROTOCOL `Row`, the `context.standard.Row` RECORD, the `StepResult.Row(reg_start,n_cols)` VARIANT). At `compile_let`'s `extract_expr_type_name`, the record is **not yet loaded** (lazy, via `compile_record`) so `type_name_to_id["Row"]`=Protocol (first-wins), `type_field_layouts["Row"]`=absent, no Record in `self.types` — leaving only `find_variant_parent_type_by_args("Row", 2)`, which matched `StepResult.Row` by ARG COUNT → `variable_type_names["r"]="StepResult"` → every `r.<field>` read shifted to a global field-intern index → OOB. **FIX:** `extract_expr_type_name` now FIELD-NAME-verifies the variant match (`find_variant_in_type_descriptors(parent, name)` — the variant's parent IS loaded): accept the variant parent only if the literal's field names match the variant's declared fields; else the literal is its own record. `columns,values ≠ reg_start,n_cols` → resolves to `Row`. |
| General lesson | A bare record literal whose head collides with a same-named variant can be mis-typed to the variant's parent. Resolvers that match variants by ARG COUNT (`find_variant_parent_type_by_args`) must also verify FIELD NAMES — count alone is ambiguous across a record + a same-arity variant. The record half may be unfindable at `compile_let` time (lazy load + protocol shadowing `type_name_to_id` first-wins), so verify on the VARIANT side (which is loaded), not the record side. |
| Pins | `core-tests/context/standard/regression_test.vr` (`regression_row_get_index_bounds_guarded` + `regression_row_direct_field_read_after_collision`); `core-tests/context/standard/unit_test.vr` (3 get_index); `core-tests/context/mod/unit_test.vr` (umbrella reads fields directly). |

## 20. `f"{Type.Variant}"` direct-ctor interpolation skips Display (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **FSTRING-CTOR-DISPLAY-1** |
| Status | **FULLY CLOSED.** Direct-ctor case `19bb51b3a` (2026-06-01); record-variant bound-var case 2026-06-03 (see Residual). Found building `core-tests/context/`; refined the general "enum-Display under --interp" symptom. |
| Stable trigger | A **direct variant constructor** in an interpolation placeholder: `f"{ContextLogLevel.Info}"`. |
| Manifestation (pre-fix) | Rendered the variant name (`"Info"`) instead of dispatching the user `Display` impl (`"INFO"`). Binding first — `let l = ContextLogLevel.Info; f"{l}"` — dispatched correctly. `Debug` (`:?`) worked in both forms. |
| Root-cause | Codegen `try_emit_display_dispatch` for interpolations (cf. §9-family) detected `<TypeName>.fmt` at compile time, but for a placeholder whose expr is a bare variant constructor the receiver type wasn't threaded through → emitted the generic ToString fast path (variant name). |
| Fix (landed) | `infer_expr_type_name` now recognises `Field{Path(Type), Variant}` and returns `<Type>` when it declares that variant, so the interpolation routes through `<Type>.fmt`. Verified on a clean worktree build of the fix commit: `f"{ContextLogLevel.Trace}"` → `"TRACE"`; standard module 69/0/4 GREEN on the post-fix shared binary. The pin `regression_display_direct_ctor_renders_uppercase_name` was un-`@ignore`'d. |
| Residual (was OPEN — **CLOSED 2026-06-03**) | The **record-variant** case — `f"{err}"` for `ContextError` via a **bound var** — rendered the default `NotFound(...)` instead of `Display`→`message()`. ROOT CAUSE: it was NOT a gate-detection path at all — the *variable's recorded type* was wrong. `let e = ContextError.NotFound { ... }` was typed via `extract_expr_type_name` (which `compile_let` consults BEFORE `infer_expr_type_name`); its `ExprKind::Record` arm keyed on the bare last path segment (`lookup_function_in_scope` / `find_variant_parent_type_by_args`), both of which MISS for record-variant ADTs → recorded `e: NotFound`, so the Display gate looked up the nonexistent `NotFound.fmt`. The "per-variant flaky" appearance was a first-wins bare-name collision artifact. FIX: both `extract_expr_type_name` and `infer_expr_type_name` `ExprKind::Record` arms now resolve the enclosing type from the *qualified* 2nd-to-last segment via `find_variant_in_type_descriptors(type, variant)` BEFORE the bare-name fallbacks → `"ContextError"`. The 5 error Display pins were **un-`@ignore`'d**; 187/0/4 across error+standard+scope, zero regressions. See `core-tests/context/error/audit.md §3.4`. |

## 21. AOT umbrella re-export of free functions unresolved (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **AOT-UMBRELLA-REEXPORT-1** (a.k.a. D1) |
| Status | **CLOSED** at the compiler layer 2026-06-01 (`crates/verum_types/src/infer/modules.rs`). Found while bringing `core-tests/sys/linux/bpf` + `bitfield` to the AOT tier. |
| Stable trigger | `mount <umbrella>.{free_fn}` where `<umbrella>/mod.vr` re-exports the fn from a submodule via `public mount .sub.{free_fn}` — e.g. `mount core.sys.{extract_bits}` (declared in `core.sys.bitfield`). The DIRECT mount (`mount core.sys.bitfield.{extract_bits}`) always worked. |
| Manifestation | Strict compile (`verum check` / `verum build` / `verum run --interp` / `verum test --aot`) failed `E100 unbound variable: extract_bits` at the use site. The Tier-0 **test harness** masked it via a lenient global function table, so `verum test --interp` passed — a cross-tier divergence that hid the bug across many sessions ("AOT pending"). |
| Root-cause (traced via `VERUM_TRACE_TASK20`) | Embedded-stdlib import resolution (`import_item_from_module_body`) resolves a re-exported fn through three fallbacks: AST-direct, AST-reexport-walk, then `resolve_function_via_metadata_reexports`. For the umbrella case the first two miss (the registry's `mod.vr` AST is a synthetic stub) and the metadata fallback found the correct source via `metadata.module_reexports["core.sys"]` (311 leaves, `extract_bits → core.sys.bitfield` — `scan_module_reexports` is correct) BUT then `metadata.functions["core.sys.bitfield.extract_bits"]` was **absent** — the precompiled function table is keyed by declaring module and omits some `pure` leaf functions. With no 4th fallback the name was left unbound. |
| Fix (landed) | Added `reexport_source_module_for(module, item)` (reads `metadata.module_reexports`) and a 4th fallback in the `ExportKind::Function` arm: when the metadata function-descriptor is absent but the re-export source module IS known, recurse `import_item_from_module_inner(source_module, item, …)` against the **live `ModuleRegistry`**, whose AST resolves the fn via the same "AST direct" path a direct submodule mount uses. Cycle-guarded by `imports_in_progress` on the distinct `(source_module, item)` key. Fully general — no hardcoded stdlib knowledge. Collapsed the sys AOT failure histogram's single largest class (~2000 `unbound variable` instances across the bitfield/errno-helper re-exports). |
| Probe | `mount core.sys.{extract_bits}; extract_bits(0xAB0,4,8) == 0xAB` — `verum run --interp` prints `r=171`; `verum check` clean. Trace: `[task20] registry recurse: mod='core.sys' item='extract_bits' -> source='core.sys.bitfield'`. |
| Examples | `core-tests/sys/bitfield`, `core-tests/sys/linux/bpf/mod` (umbrella re-export regression pins). |

## 22. AOT bitwise-NOT on `USize`/`ISize` rejected (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **AOT-NOT-USIZE-1** (a.k.a. D7) |
| Status | **CLOSED** at the compiler layer 2026-06-01 (`crates/verum_types/src/infer/expr.rs`). |
| Stable trigger | Bitwise complement `!x` where `x: USize` (or `ISize` / lowercase `usize`/`u32`/… spellings). The stdlib `core.sys.bitfield` uses `value & !field_mask(...)` to clear bit ranges, with `mask: USize`. |
| Manifestation | Strict (AOT) typecheck: `Cannot apply NOT operator to type: USize. Expected Bool or integer type` (147 instances across the bitfield suite). `--interp` accepted it — another cross-tier divergence. |
| Root-cause | The unary-NOT type-inference arm (`infer/expr.rs`, `Type::Named`) enumerated integer type names but listed only `UIntSize`/`IntSize` — the idiomatic canonical names `USize`/`ISize` (and the lowercase `usize`/`u8`…`u128`/`i8`…`i128` aliases) were absent, so `!` on a `USize` fell through to the error arm. |
| Fix (landed) | Added the canonical pointer-width names + lowercase aliases to the accepted-type match, mirroring the alias matrix used by `dispatch_primitive_method`. |
| Examples | `core-tests/sys/bitfield` (the full module's `clear_bit`/`clear_bits`/`set_bits` paths). |

## 23. Parallel `verum test --aot` aborts the whole run with a compiler SIGSEGV (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **AOT-PARALLEL-1** (two independent root causes) |
| Status | **CLOSED 2026-06-01**. Bug A (artifact-path collision) commit `f1c0510e3`; Bug B (LLVM backend race) follow-up commit. |
| Stable trigger | `verum test --aot` with `[test].parallel = true` (the DEFAULT) over more than a handful of test files. Crashes ~2–7 min into a large run. `--interp` and `--test-threads 1` are unaffected. |
| Manifestation | The ENTIRE run aborts with an in-process compiler `SIGSEGV` during `compiler.phase.generate_native` — 0 test results out of N. Backtrace deep in LLVM (`run_passes` / `OuterAnalysisManagerProxy::invalidate` / `CallBase::getParamAttrOnCalledFunction` / `IntervalMap::deleteNode`). Non-deterministic timing. This is why every `core-tests/*` module sat at `--interp`-only (no module had ever reached `complete`, which requires both tiers green). |
| Root cause A — colliding artifact paths | The runner keyed every per-test build artifact on the test file's `file_stem`, which REPEATS across modules (every `unit_test.vr`/`property_test.vr`/… → stem `"unit_test"`/…): merged source `target/test/test_<stem>.merged.vr` (and its content differs per `@test`), output binary `test_<stem>`, the derived `<stem>.o`/`.ll`, and the fixed `verum_runtime_stubs.c`/`.o`. `par_iter` workers wrote / compiled / `remove`d the SAME files at once → corrupt merged source → malformed VBC/LLVM IR → SIGSEGV in the backend. (The LLVM backtrace was a SYMPTOM of bad input.) `--interp` is immune: it compiles in-process via `compiled_test_module` memoised by `test.file` (unique), with no disk artifacts. |
| Root cause B — LLVM backend not thread-safe | Even with unique artifacts (valid IR), the run still SIGSEGVs in `Module::run_passes`. LLVM's per-process state — the global pass registry, target subtarget caches, and `cl::opt` command-line globals — is not safe to drive from multiple threads concurrently, even when each owns its `LLVMContext`/`Module`/`TargetMachine`. Two `par_iter` workers in the optimisation/codegen pipeline at once race that global state. |
| Fix A | `unique_merged_stem(test_file, test_fn, stem)` (fixed-key `DefaultHasher` over the source path + test fn) feeds the merged-file stem (`crates/verum_cli/src/commands/test.rs`), the output binary name (`run_test_aot`), and the runtime-stub `tag` (`native_codegen.rs::generate_runtime_stubs`); `.o`/`.ll` inherit it. Every concurrent compile targets its own files. |
| Fix B — what it actually is | **Per-test subprocess isolation** in the test runner (`run_aot_subprocess`, `crates/verum_cli/src/commands/test.rs`): a parallel `--aot` run compiles + runs each test in its OWN `verum` process (`verum test --aot --test-threads 1 --exact --filter <name>`), so each compilation owns its LLVM state and the parent fans them out in parallel with **zero shared-state races AND full throughput**. The single-test child runs in-process (recursion-terminated by `active.len() > 1`); interp stays in-process (no LLVM). Three in-process attempts were rejected first and are recorded so they aren't re-tried: (1) a lock scoped to opt+emit — still raced `lower_module` vs `run_passes`; (2) a lock over the whole LLVM window — still crashed (the racing workers aren't *in* codegen, they're rayon park/wake threads hitting the `__cxa_guard` semaphore); (3) a serial first-test warm-up of the pass guards — still crashed (too much other shared LLVM global state). Process isolation is the only robust fix; it also preserves parallelism, unlike a global codegen lock. |
| Why it matters | Unblocks the `--aot` channel for the WHOLE conformance suite — the prerequisite for promoting any stdlib module to `complete`. |
| Companion (harness) | `module_qualified_prefix()` (commit `993679a01`) — test names are now `mem/capability/unit_test::fn` (were colliding `unit_test::fn` across directories), so `--filter mem/` scopes a subtree and the README-documented `--filter module::` works. |

## 24. AOT newtype tuple-constructor unregistered for cfg-gated modules (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **AOT-NEWTYPE-CTOR-1** (a.k.a. D3) |
| Status | **CLOSED 2026-06-01** (commit `d329cbf62`, `verum_types/infer/expr.rs`). Validated: `core-tests/sys/darwin/mach` unit AOT 20/20 (was 0/20), interp 20/20 (no regression). |
| Fix (landed) | A Call-inference `func_ty` match arm: when the callee resolves to a `Type::Named` that is a transparent-wrapper newtype (has a `__newtype_inner_<X>` binding — always registered alongside the type, also powering `<X>.0` access), treat `X(v)` as construction — check the single arg against the inner type and yield the newtype. Recovers the ctor UNIFORMLY at the use site, independent of which stdlib-load path bound `env[X]` (lazy / eager / import). Non-newtype Named types lack the key and correctly fall through to `NotAFunction`. Two earlier `verum_types` *loader* fix attempts (register the ctor in the eager + lazy metadata loaders) were reverted — the ctor binding was overwritten by a later type-binding from the import path (the binding-site whack-a-mole is why the call-site fix is the robust one). |
| Stable trigger | A single-field tuple-newtype constructor in expression position — `KernReturn(7 as Int32)` / `VmProt(...)` (`type X is (Int32)` in the `@cfg(target_os="macos")`-gated `core.sys.darwin.mach`) — under `verum test --aot`. |
| Manifestation | **`--interp` PASSES** (mach unit test 20/20 GREEN) but **`--aot` FAILS** every test: `not a function: KernReturn` / `not a function: VmProt` at the construction site. Const-value field access (`KERN_SUCCESS.0`) works on both tiers — so the inner-type binding IS registered; only the CONSTRUCTOR is missing under AOT. |
| Triangulation | An identically-declared NON-gated newtype — `core.sys.cabi.CInt` (`type CInt is (Int32)`) — works on both tiers (cabi is "complete"). So it is not the declaration form nor the `is_transparent_wrapper` mechanism (CInt relies on it). The interpreter registers the ctor via the full-stdlib bootstrap's in-source `resolve_type_definition` (Tuple arm, `infer/decls.rs`); the AOT per-test compile loads the stdlib from precompiled `core_metadata`, where the ctor is never bound. A minimal repro (`mount core.sys.darwin.mach.KernReturn` alone) fails on BOTH tiers — so full mount context matters; the real divergence is full-context-interp vs per-test-AOT. |
| Attempts (reverted) | Registering the ctor in the eager (`load_stdlib_from_metadata`) + lazy (`ensure_stdlib_type_loaded`) `verum_types` metadata loaders did NOT fix it: a `VERUM_TRACE_D3` build showed the lazy ctor-bind DID fire for `KernReturn`, but the binding was then overwritten by a type-binding from the import path — hence the use-site fix above instead of chasing every binding site. |

## 25. AOT integer ↔ raw-pointer cast rejected (CLOSED 2026-06-01)

| Field | Value |
|---|---|
| Defect class id | **AOT-INT-PTR-CAST-1** (a.k.a. D5-cast) |
| Status | **CLOSED 2026-06-01** (commit `719ecc42a`, `verum_types/infer/types.rs`). |
| Stable trigger | `as` between an integer (`Int` / `USize` / `ISize` / sized aliases) and a RAW pointer (`*const T` / `*mut T` / volatile) — e.g. `core.sys.mmio` `MemoryRegion.start: *const ()` constructed as `0x1000 as *const ()`, and `self.start as USize` in `.contains`. |
| Manifestation | `--interp` passed (lenient VBC cast) but `--aot` failed `E401: types are not compatible for casting` (40 instances in the mmio suite). |
| Root cause | The cast-validity checker had an `(Int, Reference/CheckedReference/UnsafeReference)` arm (int→managed reference) but NO arm for int↔raw `Pointer`/`VolatilePointer`, so `usize as *const ()` fell to the reject fallback. |
| Fix (landed) | Add `(Int | Named, Pointer | VolatilePointer)` and the reverse to the allowed casts, with the same address-safety warning as the int→reference arm when outside `@unsafe`. The `as` keyword is the explicit opt-in; valid in Verum's no-libc systems layer. |

## 26. AOT raw-pointer record-field type collapses to Unit (OPEN)

| Field | Value |
|---|---|
| Defect class id | **AOT-PTRFIELD-UNIT-1** (a.k.a. D5-ptrfield) |
| Status | **OPEN** (surfaced 2026-06-01 after AOT-INT-PTR-CAST-1 closed the E401). Affects `core.sys.mmio` (`MemoryRegion.start: *const ()`) AOT only. |
| Manifestation | Constructing `MemoryRegion { start: 0x1000 as *const () }` under `--aot` fails `E400: Type mismatch: expected 'Unit', found '*const Unit'` — the RHS is correctly `*const Unit` (post-cast-fix) but the FIELD type resolves to `Unit`: the `*const ()` raw-pointer field type collapsed to its inner `()` somewhere in the metadata round-trip. |
| Root cause (confirmed) | The VBC `TypeRef` enum (`verum_vbc/src/types.rs:335`) has NO raw-pointer variant — only Concrete / Generic / Instantiated / Function / Rank2Function / Reference / Tuple / Array / Slice. So a `*const ()` record field is stored as `Concrete(TypeId::UNIT)` (the default), DROPPING the pointer — the info is lost at the VBC level, before any metadata serialization. `parse_descriptor_type_string` also lacks `*const`/`*mut` parsing, but fixing that alone cannot help while the VBC layer can't carry the type. A full fix requires extending VBC `TypeRef` with a `Pointer { inner, mutable }` variant and threading it through codegen + serde (deep, in `verum_vbc` core infra). |
| Pragmatic alternative | `MemoryRegion.start` is used exclusively as an address (`self.start as USize` in `.contains`/`.end`); declaring it `USize` instead of `*const ()` would sidestep the raw-pointer-field round-trip entirely (a small stdlib API change + precompile rebuild). |

## 27. `cases` reserved keyword rejected in for-range upper bound (OPEN)

| Field | Value |
|---|---|
| Defect class id | **CASES-KEYWORD-1** |
| Status | **OPEN** (surfaced 2026-06-03 during the mem AOT-iter conversion). Parser-level, affects both tiers. |
| Stable trigger | `for i in 0..cases.len() { … }` where `cases` is a local variable. |
| Manifestation | `parse: unexpected keyword 'cases', expected '{'` — the for-range-bound parser, after `0..`, treats `cases` as the keyword `TokenKind::Cases` (lexer `verum_lexer/src/token.rs:894` `#[token("cases")]`, the proof-by-case-analysis construct) instead of an identifier. |
| Scope | NARROW. `cases` works fine as an identifier everywhere else tested: `let cases = …`, `cases[i]`, `cases.len()`, and `while i < cases.len()`. ONLY the for-range upper-bound expression position rejects it. |
| Workaround | Rename the local (`cases` → `case_list`). Applied in `core-tests/mem/thin_ref/property_test.vr::law_pack_injective_pinned`. |
| Fix, pending | In `verum_fast_parser`, the for-range bound-expression parser must accept contextual keywords as identifiers in primary position (the same leniency let-binding / index / while-condition parsing already applies). Parser change + rebuild + parse regression test. |

## 28. AOT types qualified variant record-construction as the variant, not the enum (OPEN)

| Field | Value |
|---|---|
| Defect class id | **QUALVAR-CONSTRUCT-1** |
| Status | **OPEN** (surfaced 2026-06-03). AOT only; `--interp` correct. Same family as the bare-vs-qualified ADT defects (BAREVAR-ADT-1). |
| Stable trigger | `let e: E = E.Variant { f1: …, f2: … };` — annotated binding from a qualified variant record-construction. |
| Manifestation | `error<E400>: Type mismatch: expected 'RevocationError', found 'RevocationError.Internal'`. The record-construction expression is typed as the VARIANT (`E.Variant`), not widened to the ENUM (`E`), so it fails to unify with the `: E` annotation. |
| Surfaced by | `core-tests/mem/thin_ref/unit_test.vr::test_revocation_error_internal_construction_via_record` (`RevocationError` def `core/mem/mod.vr:355`). Blocks the whole file's AOT compile. |
| Fix, pending | Variant record-construction must yield the enclosing enum type (or unify with the binding annotation) in the AOT type path. Deep; rebuild + full-AOT-validate gated. |

## 29. AOT lowers record-style enum variant payload as a tuple (OPEN)

| Field | Value |
|---|---|
| Defect class id | **RECVAR-TUPLE-1** |
| Status | **OPEN** (surfaced 2026-06-03). AOT only; `--interp` correct. |
| Stable trigger | A record-style enum variant — `type E is | V { a: T, b: U }` — used with a record-style pattern (`V { a, b } =>`) or alias constructor under AOT. |
| Manifestation | `error: Variant 'GenerationMismatch' has payload type (UInt32, UInt32), which is not a record. Cannot use record-style pattern`. AOT sees the record payload as a positional tuple, losing field names; record-style patterns + the `generation_mismatch(...)` alias round-trip fail. |
| Note | NOT a test bug — the stdlib's own `Display`/`Debug` impls (`core/mem/header.vr:208-219`) use the same record-style patterns. A genuine AOT representation gap for record-style variant payloads. |
| Surfaced by | `core-tests/mem/header/integration_test.vr` (lines 128, 146; `MemValidationError` def `core/mem/header.vr:187`). Blocks the whole file's AOT compile. |
| Fix, pending | Preserve record-style variant payload field metadata through AOT lowering so record patterns/construction resolve by field name (not positional tuple). Deep; rebuild + validate gated. |

## 30. Archive lazy-apply bare-leaf fanout explosion (CLOSED 2026-06-11)

| Field | Value |
|---|---|
| Defect class id | **ARCHIVE-FANOUT-1** |
| Status | **CLOSED** (`946f3d787`). |
| Stable trigger | ANY user/test code that calls a method named like a common stdlib method (`next`, `map`, `get`, `len`, …). |
| Manifestation | ~84-second COMPILE (not execution) for a trivial program; `core-tests/base/iterator/*` timed out (rc=124) under `--interp`. |
| Root cause | `ArchiveCtxCache::apply_lazy_with_types` → `SymbolGraph::reachable` seeds its transitive-closure BFS with bare method names harvested from user code. A bare leaf `next` resolves via `leaf_to_qualified` to EVERY type's same-named impl (172 distinct `*.next` in the archive); each iterator next's body calls `self.iter.next()` (another bare `next` `CallM` edge) and re-fans transitively → the closure pulled in ~most of the 585 archive modules → decoding them was the 84s. |
| Fix | Cap the per-callee bare-leaf fanout in `reachable` (`crates/verum_compiler/src/archive_ctx_loader.rs`, `MAX_BARE_LEAF_FANOUT=24`, overridable `VERUM_LEAF_FANOUT_CAP`). A high-fanout bare name is a polymorphic protocol method resolved at runtime by the receiver's concrete type — whose module loads independently — so blanket leaf-fanning is redundant for correctness and catastrophic for cost. 84730ms→3979ms (21×); regression-safe. |

## 31. Cross-module record construction bakes `NEW ()` (untyped) (CLOSED 2026-06-11)

| Field | Value |
|---|---|
| Defect class id | **XMOD-RECNEW-UNIT-1** |
| Status | **CLOSED** (`8d8214d83`). Both tiers. |
| Stable trigger | A record literal of a type defined in ANOTHER module, constructed in a body monomorphised across the module boundary — canonical: the `Iterator` protocol default combinators (`map`/`filter`/`take`/`enumerate`/…) construct `EnumerateIter<Self>` etc. monomorphised onto a concrete iterator (`TextMatches.enumerate`, `ListIter.enumerate`). |
| Manifestation | `compile_record` emitted `NEW () (fields=2)` (type_id 0, untyped). The heap object carried no concrete type → later `.next()` dispatch couldn't recover the receiver type → routed to the lowest-id same-named method (`SignalStream.next`) → infinite recursion → stack overflow / SIGSEGV. |
| Root cause | The bootstrap shares a type's FIELD LAYOUT cross-module (`import_type_layouts`, `crates/verum_vbc/src/codegen/mod.rs:3056`) but is deliberately TypeId-free, so `type_name_to_id`/`self.types` lacked the cross-module type → `type_id=0`. Confirmed via `VERUM_TRACE_RECNEW` (`in_name_to_id=false in_field_layouts=true`). 2814 sites archive-wide. |
| Fix | `compile_record`: when a plain-record literal names a type whose LAYOUT is known but whose id is not, allocate a fresh module-local `TypeId` + push a `Record` descriptor under the SAME simple name. The archive body-merge remaps type ids **BY NAME** (`merge_archive_function_bodies`, `codegen/mod.rs:16697`), so the local id resolves to the canonical descriptor at load — no `external_type_names`/linker machinery needed (that was a red herring; the by-NAME archive-load remap, not the linker's id→id map, is what matters). Validated: `xs.iter().map(...)` for-loops yield correct `2/4/6`; regression-safe. |
| Surfaced by | `core-tests/base/iterator/{unit,property,protocol_agnostic}_test.vr` (audit.md §4.2). |

## 32. `self.iter.next()?` in generic adapters yields `None` (CLOSED 2026-06-12)

| Field | Value |
|---|---|
| Defect class id | **ADAPTER-TRY-NEXT-1** |
| Status | **CLOSED** (`3858edf52`). Both tiers. |
| Stable trigger | The `?` (try) operator on a method call whose receiver is a generic-type-param field — the universal Iterator-adapter idiom `let item = self.iter.next()?;`. Reproduced in pure user code: `implement<I> Wrap<I> { fn step(&mut self) -> Maybe<Int> { let x = self.inner.next()?; Some(x) } }` returned `None` on the first call for `Counter{n:0,max:3}`; the SAME body with explicit `match` worked. |
| Manifestation | Adapter `.next()` chains yielded nothing — the entire iterator-adapter family was inert through manual `.next()`. |
| Root cause | `compile_try`'s `extract_expr_type_name` returns no Maybe-classifiable base for `self.iter.next()` (receiver is generic param `I`; `I.next` unregistered) → `success_tag` defaults to **0**, which for `Maybe` is the *None* tag → `?` tests "is None?" not "is Some?", so every `Some(x)` takes the failure path → `None`. (Confirmed via `VERUM_TRACE_MATCHTAG`: `expected_tag=0` on a value with `variant_tag=1`. The earlier "byte-identical bytecode, different result" lead was a false trail caused by the stale **script-cache** — see ARCHIVE/script-cache note.) |
| Fix | In `compile_try`, force `Maybe` classification when `?` is applied directly to a `next`/`next_back` MethodCall. Every `fn next`/`fn next_back` in `core/` that can appear under `?` returns a top-level `Maybe` (the only non-Maybe `next`s are RNG `-> UInt64`, never `?`-applied), so the override is sound and overrides a mis-resolved Result-shaped base. `AsVar` extracts the success payload positionally (field 0), correct for both Maybe-Some and Result-Ok — no payload change needed. |
| Validated | stdlib `xs.iter().enumerate()` manual `.next()` loop yields `0:10/1:20/2:30`; regression-safe (maybe/property 21/9→22/8, result 26/5, ordering 23/3). |

## 33. for-loop over non-intercepted iterator adapters → native `IterNew` SIGSEGV (CLOSED 2026-06-12)

| Field | Value |
|---|---|
| Defect class id | **ADAPTER-FORLOOP-NATIVE-1** |
| Status | **CLOSED** (`ae4b3d22a`). Both tiers. |
| Stable trigger | `for x in xs.iter().enumerate()` (and `.take`/`.skip`/`.zip`/`.chain`/…). |
| Manifestation | SIGSEGV (null-deref at offset 0x18). `map`/`filter`/`fold` for-loops worked (they're runtime-intercepted onto the native blob — eager-collect). |
| Root cause | The interpreter has two iterator systems — native `IterNew`/`IterNext` blobs (List/Map/Set/Array/Range) and stdlib `Iterator` protocol RECORDS (EnumerateIter/TakeIter/…). `is_custom_iterator_type` uses `infer_expr_type_name` (no MethodCall arm), so an adapter-chain iterator resolves to None → the loop falls to native `IterNew`, which maps every non-builtin `type_id` to `ITER_TYPE_LIST` and reads the adapter record's fields as a `List [count,cap,entries_ptr]` header → SIGSEGV. |
| Fix | `is_custom_iterator_type` recognizes the non-intercepted adapter methods (enumerate/take/skip/take_while/skip_while/zip/chain/flat_map/flatten/scan/step_by/peekable/rev/fuse/cycle/dedup/windows/chunks/intersperse/map_while/inspect/copied/cloned) in the for-loop iterator position and routes them to `compile_for_custom_iterator` (`loop { match it.next() {...} }`), which calls the record's `.next()` (correct after §32). `map`/`filter`/`fold` stay on the fast native blob path. |
| Validated | Iterator suite (`--interp`): property **SIGSEGV→13/9**, protocol_agnostic **SIGSEGV→20/2** (≈33 tests recovered from whole-file crashes). |
| Surfaced by | `core-tests/base/iterator/{unit,property,protocol_agnostic}_test.vr` (audit.md §4.4). |

## 34. Bare `UInt` registered as `Type::Generic`, not an integer (CLOSED 2026-06-13)

| Field | Value |
|---|---|
| Defect class id | **UINT-AS-GENERIC-1** |
| Status | **CLOSED** (2026-06-13). Reproduced under strict `--interp` AND `--aot`; the lenient test harness masked it. |
| Stable trigger | `let v: UInt = 100 as UInt;` — any cast to the canonical 64-bit unsigned integer `UInt`. Pervasive at the FFI boundary (`CULong is (UInt)`, `core/sys/cabi.vr`). `as UInt64` / `as USize` always worked. |
| Manifestation | `error<E401>: Cannot assign value of type 'Int' to variable of type 'UInt'` — `X as UInt` → `check_cast(Int, …)` matched no arm and fell to the final catch-all. |
| Root cause | `register_primitives()` (`crates/verum_types/src/infer/env.rs`) registered `UInt` as `Type::Generic { name: "UInt" }` inside the META-CONTEXT block (grouped with `TokenStream`/`Ident`/`TypeInfo`/`Bytes`), NOT as a real integer `Type::Named` like `UInt64`/`USize`. So `extract_integer_kind` (Named-only) and `IntegerKind::from_name` (lacked bare `"UInt"`) both missed it, and `check_cast`'s `(Int \| Named, Int \| Named)` arm never matched a `Generic`-shaped `UInt`. Codegen + `well_known_types` already mapped `"UInt"` → u64 everywhere; env.rs was the lone outlier. |
| Fix | Register `UInt` as `Type::Named{path:UInt}` (mirrors `UInt64`/`USize`) and add `IntegerKind::from_name("UInt") => UInt64`. |
| Validated | `core-tests/sys/cabi --aot` 6→32/50; `--interp` held 50/50; `base/primitives` 279→281 (no regressions). |

## 35. Qualified variant construction `T.Variant(args)` fails under generic-signature functions (CLOSED 2026-06-13)

| Field | Value |
|---|---|
| Defect class id | **QUALVAR-METHODCALL-1** |
| Status | **CLOSED** (2026-06-13). Reproduced under strict `--interp` AND `--aot`; the lenient harness masked it. Sibling of §28 (QUALVAR-CONSTRUCT-1, the annotated-record form). |
| Stable trigger | `Maybe.Some(x)` / `Result.Ok(v)` / any `T.Variant(args)` **inside a function whose signature carries ANY generic type** (param or return: `List<Int>`, `Maybe<T>`, `Result<_,_>`). Bare `Some(x)` always worked; the same `T.Variant(args)` in a non-generic-signature function worked. |
| Manifestation | `error<E400>: no method named 'Some' found for type 'None(Unit) \| Some(T)'`. The bare receiver `T` synthesised to its raw `Variant(...)` shape → routed to instance-method dispatch in `infer_method_call_inner_impl`, which has no variant-constructor recognition for `Variant`-typed receivers → `MethodNotFound`. |
| Root cause | `infer_method_call_inner_impl` (`crates/verum_types/src/infer/modules.rs`) had no recognition that `Path(TypeName).Variant(args)` is qualified variant CONSTRUCTION, not instance dispatch. It only worked incidentally when the receiver synth didn't produce a bare `Variant` — which a generic enclosing signature changes. |
| Fix | Two parts. (1) **Synth** (`infer_method_call_inner_impl`, modules.rs): before instance dispatch and gated on the first chain step (`!skip_static_lookup`), recognise `Path(TypeName).Variant(args)` via the existing arity-aware `try_resolve_variant_constructor_with_arity`, scoped to the receiver type (`parent_matches` guard) so a sibling type sharing the variant simple-name cannot capture it. (2) **Bidirectional** (`check_expr` MethodCall arm, expr.rs, 2026-06-14): the synth path returns the `Generic{Result,[T,E]}` form, which does NOT unify with the *expanded* `Variant{Ok:T,Err:E}` form a function-return/let-annotation carries — so two qualified ctors in one if/else (`Result.Err(..)` + `Result.Ok(())`) mismatched (`expected Ok(Unit)\|Err(..) found Result<_,..>`) even though each works alone. The bidirectional arm now binds the single positional argument against the expected variant's payload and returns the expected type directly (mirroring the bare-name Call arm); record/multi-arg forms fall through to synth. Stdlib-agnostic; reuses the bare-name ctor machinery. |
| Validated | `core-tests/sys/cabi --aot` 32→35/50; **`sys/common --aot` 76→82/115** (bidirectional part cleared the 14 `Result`-type-mismatch integration failures); `--interp` held; `base/primitives` 279→281; no type-error / `no method named Some/Ok/Err/None` regressions in `base/maybe`/`base/result`/`base/ordering`. |

## 36. Umbrella re-export under archive load drops a type's impl block (CLOSED 2026-06-13)

| Field | Value |
|---|---|
| Defect class id | **UMBRELLA-REEXPORT-IMPL-1** (Bug C, type facet) |
| Status | **CLOSED** (2026-06-13). `--aot` (and strict `--interp`); the lenient harness masked it. Same root as the §F `darwin/mod` Bug C and the `sys/mod` `Fd`-ctor failure. |
| Stable trigger | `mount core.sys.{MemProt}` — a type re-exported through an UMBRELLA module (`core/sys/mod.vr` does `public mount .common.{MemProt}`) — then using its impl-block items. The DIRECT `mount core.sys.common.{MemProt}` always works. |
| Manifestation | Associated const `MemProt.NONE` synthesises to `Unit` → `error<E103>: Cannot access field 'read' on non-record type: Unit`; method `to_unix_flags()` → `error<E400>: no method named 'to_unix_flags' found for type 'Unit'`. 325+ instances in `sys/common --aot`. |
| Root cause | The type's STRUCT FIELDS are carried under the umbrella (literal construction + annotated return type both type-check), but its **impl block** (associated consts + methods) is NOT. The registry's umbrella `mod.vr` AST is a synthetic stub (§21 / D1), so `import_type_export`'s AST-based impl-block import (`find_type_declaration_with_source_module` → `import_impl_blocks_for_type_in_module`) finds nothing — it never reaches the defining module's (`core.sys.common`) impl blocks. |
| Fix | `import_item_from_module_body`'s `ExportKind::Type \| Protocol` arm: after `import_type_export`, resolve the canonical source module via the precompiled `module_reexports` metadata (`reexport_source_module_for`) and, when it differs from the umbrella, recurse `import_item_from_module_inner` into it against the live registry — the same path a direct `mount <source>.{Type}` uses, which carries the full impl block. Mirrors the §21 D1 free-function fix; idempotent; cycle-guarded by `imports_in_progress`. |
| Validated | `core-tests/sys/common --aot` 6→72/115 (the 325+ `field on Unit` cluster cleared); `sys/mod --aot` 21→38/39 (the `Fd` ctor resolves; +2 test-bug fixes); `--interp` held (common 115/0, mod 39/0); `base/primitives` 281/51, `sys/cabi` 50/0 (no regressions). |
| Surfaced by | `core-tests/sys/common` (`MemProt`/`SysContextError` impl items), `core-tests/sys/mod` (`Fd` ctor, `InitError` variant). |

## 37. `&*p` on a raw pointer creates an escaping register-ref → CBGR use-after-free (CLOSED 2026-06-19)

| Field | Value |
|---|---|
| Defect class id | **REFDEREF-ESCAPE-UAF-1** |
| Status | **CLOSED** (2026-06-19, interp). `ListIter::next` and every by-reference combinator (`find`/`fold`/`position`/`min_by_key`) over a manual `while let Some(x)=it.next()` loop. |
| Stable trigger | A method returning `&*self.ptr` where `self.ptr: &unsafe T` (the canonical `ListIter::next` shape: `let item = &*self.ptr; self.ptr = self.ptr.offset(1); Maybe.Some(item)`). |
| Manifestation | `Panic: CBGR use-after-free detected: expected generation 2, found 4` on the 2nd `next()`; the gen-2 ref pointed into a recycled stack slot. |
| Root cause | `compile_unary` compiled the inner `*self.ptr` to a register holding a COPY of the loaded pointee, then wrapped that ephemeral stack temp in a CBGR register-ref encoding `next`'s frame `abs_index`. `pop_frame`/`push_frame` bump slot generations directly (`registers.rs` `try_push_frame`/`pop_frame`, NOT `bump_generation`), so on the next call the slot's generation advanced and the escaped ref dangled. Traced with `VERUM_TRACE_CBGRGEN`: the fail slot lived in `next`'s OWN frame and was bumped by frame push/pop, NOT by `DropRef` — the DropRef-over-bump hypothesis was wrong. |
| Fix | `compile_unary` folds `&*p` / `&mut *p` to `p` when `p` is a raw pointer (`&unsafe T` / `*const T` / `*mut T`), returning the heap-anchored pointer directly. Gated on side-effect-free `Path`/`Field` operands so the non-pointer fall-through (`&*heap_box`, cbgr-ref reborrow, which rely on the generic Deref emitting `cbgr_deref_source`) recompiles idempotently. |
| Validated | `base/iterator --interp`: property 13→19 pass, regression 8→9, integration 4→6; basic 13/13 + protocol_agnostic 20/22 unchanged; zero regressions. Manual `while let Some(x)=it.next()` loops now correct. |

## 38. `iter.collect()` return-type-directed `FromIterator` mis-dispatches to `FFIAbi.from_iter` (OPEN 2026-06-19)

| Field | Value |
|---|---|
| Defect class id | **COLLECT-FROMITER-RESOLVE-1** |
| Status | **OPEN** — root-caused; a call-site-rewrite fix was tried and REVERTED (introduced a worse silent-empty regression). |
| Stable trigger | `let x: List<Int> = some_iter.chain(…)/zip(…)/range.collect();` — context-dependent: simple `arr.iter().map(\|x\| *x*2).collect()` resolves correctly TODAY (`protocol_agnostic::test_collect_to_list` passes), chain/zip/range shapes do not. |
| Manifestation | `Panic: method 'FFIAbi.from_iter' not found on receiver of runtime kind Object … 8 candidate(s): Text/List/Map/Set/Deque/BinaryHeap/BTreeMap/BTreeSet.from_iter`. `FFIAbi` has NO `FromIterator` impl — it is a fabricated fallback for the unbound type-param-as-namespace `C` in `collect<C: FromIterator>(self) -> C { C.from_iter(self) }`. |
| Root cause | The return-type-directed binding of `C` (from the `let x: T =` annotation) is not threaded into the generic `collect` body's `C.from_iter` dispatch for all shapes. Works only where monomorphisation happens to bind `C` correctly. |
| Why call-site rewrite FAILS | Rewriting `iter.collect()` → `<Base>.from_iter(iter)` via `Instruction::Call{func_id}` bypasses monomorphisation — the generic `from_iter`'s inner `for item in iter` over the generic param `I` then fails to dispatch `MapIter<ListIter>::next` and silently yields a 0-length list (only `MapIter<ListIter>` collapses; `MapIter<Chain/Rev>` iterate fine). Net was property 19→21, integration 6→12, BUT protocol_agnostic 20→19 (regressed the common `list.iter().map().collect()` pattern). Silent-empty < loud-panic → reverted. |
| Real fix surface | (a) resolve `C` from the return-type-directed expected type at `collect` monomorphisation time so `C.from_iter` binds for ALL shapes; and/or (b) close the `for item in iter` over generic `I=MapIter<ListIter>` → 0 monomorphisation-keying collision (distinct from §33's adapter-for-loop routing). |
| Surfaced by | `core-tests/base/iterator/{property,integration}` (~12 tests); documented `audit.md §4.6`. |

## 39. Range arithmetic / inclusive-count `collect`+fold assertions (OPEN 2026-06-19)

| Field | Value |
|---|---|
| Defect class id | **RANGE-ARITH-ASSERT-1** |
| Status | **OPEN** — likely tied to the documented `RangeInclusive::next` field-layout intercept (`NewRange{inclusive:true}` heap layout `[current,end,inclusive]` vs declared `{current,end,done}`). |
| Stable trigger | `integration_range_sum`, `integration_range_product_for_factorial`, `law_range_inclusive_count_includes_endpoint`, `law_take_plus_skip_recovers_original`. |
| Manifestation | `AssertionFailed: left != right` (wrong arithmetic result, not a panic). |
| Surfaced by | `core-tests/base/iterator/{integration,property}`; `audit.md §4.7`. |

## 40. Type-name inference gaps mis-lower stdlib bodies at precompile (CLOSED 2026-07-05)

**Class**: TYPE-NAME-INFERENCE-1.  Four sibling gaps in
`extract_expr_type_name` / `infer_expr_type_name` each produced a
silently-wrong lowering decision downstream: (a) the **Cast arm**
resolved only single-ident `Path` targets, so `x as *mut CapAuditSlot`
reported no type and a later `(*x).field` write resolved with
`type_name=None` → global-intern slot index (wrote `CapAuditSlot.event`
at slot 4 of a 2-field record — `commit`/`recent` corruption); (b) the
**Deref arms** did not strip `*mut `/`*const `/`*volatile ` carriers,
and wrapper carriers (`Heap<T>`/`Shared<T>`) must unwrap-or-report-
UNKNOWN — a degenerate `Heap<>`/bare `Heap` name routes f-string
Display dispatch to `Heap.fmt` against the unwrapped runtime value;
(c) **SCREAMING_CASE static-mut receivers** hit the is-uppercase
TYPE-namespace heuristic whose miss-fallback returned the static's
NAME as the "type" (`let p = GLOBAL_HAZARD_DOMAIN.scan_hazards()`
typed `p: "GLOBAL_HAZARD_DOMAIN"` → sibling calls qualified as
`GLOBAL_HAZARD_DOMAIN.binary_search`); (d) **instance-method
return-type resolution** handled only Path receivers, so
`self.field.method()` chains resolved None.  Fixes: Cast → full
`extract_type_name_from_ast`; Deref strips pointer carriers +
wrapper-unwrap-or-None; static-mut receivers consult
`static_mut_type_names` and never fall through; receiver resolution is
recursive.  Fixed in `799cff9b2`; pinned by `core-tests/mem/hazard/`
+ `core-tests/mem/cap_audit_ring/` audits.

## 41. Native IterNew reads unknown records as List headers (CLOSED interp 2026-07-05; AOT twin OPEN)

**Class**: PROTOCOL-ITER-1.  The interpreter's `IterNew` type
discrimination mapped every non-builtin `type_id` to
`ITER_TYPE_LIST`, and `IterNext` then read the record's slots as a
List `[len, cap, ptr]` header — memory-unsafe, value-dependent: an
iterator record whose slot-0 value happened to decode `len <= 0`
"worked" (silent empty loop), anything else walked garbage pointers
(the hazard `reclaim` SIGSEGV that killed every parallel test run).
Fix: `ITER_TYPE_PROTOCOL` — `IterNew` resolves a 1-arg `<Type>.next`
with a real body (exact-name discipline) and `IterNext` dispatches the
Iterator protocol via `call_function_sync`, unpacking the returned
`Maybe`.  Codegen-side classification (`is_custom_iterator_type`)
remains the preferred lowering; the runtime path is the safety net
that makes the native lowering semantically identical.  The AOT
custom-iterator leg exists but depends on obj-register type marking —
the archive-body case is the open twin.  Fixed (interp) in
`799cff9b2`; pinned by `core-tests/mem/hazard/audit.md §8`.

## 42. `call_function_sync` clobbered the caller's r0 (`self`) (CLOSED 2026-07-05)

**Class**: CALLSYNC-R0-CLOBBER-1.  The synchronous nested-call helper
pushed frames with `return_reg=Reg(0)`; `do_return` unconditionally
writes the callee's result into the caller frame's `return_reg`, so
every consumer whose r0 was live — `self` in ANY method frame — had it
silently replaced by the callee's return value.  Surfaced the moment
ITER_TYPE_PROTOCOL called `Drain.next` from inside
`ThreadHazardRecord.reclaim` (`self` became the returned `Maybe`;
`self.retired = …` panicked "field write out of bounds …
type='Maybe'").  Fix: save caller r0 before the nested dispatch,
restore after (guarded on the stack returning to entry depth).  Fixed
in `799cff9b2`.

## 43. 8-byte atomic CAS vs raw-zero fresh static-mut cells (CLOSED 2026-07-05)

**Class**: ATOMIC-CAS-ZEROINIT-1.  Size-8 atomics store NaN-boxed
Value bit-patterns (task #39 contract), but `static_mut_cell_addr`
allocates cells as RAW zero — payload-equivalent to boxed 0 for LOADS,
bit-distinct for CAS.  A caller CAS-ing `expected == 0` against a
never-stored cell could never take the first transition; since the
inlined `fetch_add` lowering is a single-shot `AtomicLoad + Add +
AtomicCas` (no retry loop), `cap_audit_ring`'s `NEXT_SEQ` silently
lost every increment (`commit` returned seq=1 forever, `count()`
stayed 0).  Fix: on CAS failure with `old_raw == 0 && expected == 0`,
retry once against the raw-zero pattern.  NOTE: the missing CAS retry
loop in the inlined fetch_add lowering is itself a latent
correctness gap under real thread contention — tracked for the
threading era.  Fixed in `799cff9b2`.

## 44. Cross-module call ids overlapped module-local ids in archives (CLOSED 2026-07-05)

**Class**: XMOD-CALL-ID-BAND-1.  Archive bytecode kept ctx-GLOBAL ids
for cross-module `Call`-family references while module-local calls
were remapped to contiguous `[0, N)`.  The id-keyed
`external_function_names` map over overlapping id spaces is
structurally ambiguous in EITHER priority order: external-first
hijacked local calls (heap's local `get_heap` id 461 == the recorded
cross-module id of `atomic_fetch_add_int` → `get_heap_stats`
NullPointer whenever `core.mem.segment` was mounted); local-first had
broken cross-module calls (`Deque.reallocate`'s `realloc` at foreign
id 4 vs local `AdjacencyList.add_edge`).  Fix: cross-module ids
re-home into the reserved band `[0x2000_0000, 0x4000_0000)`
(`module::XMOD_CALL_ID_BAND_BASE`) at precompile — the id spaces are
disjoint by construction and the name-based Tier-0 resolution is
sound.  Fixed in `799cff9b2`; pinned by `core-tests/mem/heap/audit.md`.

## 45. Allocating-wrapper runtime reprs vs compiled stdlib bodies (CLOSED interp 2026-07-05)

**Class**: HEAP-INTORAW-1 / SHARED-STRONGCOUNT-1.  The Tier-0 runtime
representations of the allocating wrappers (`Heap<T>` = CBGR data
pointer past a 32-byte AllocationHeader; `Shared<T>` =
`[ObjectHeader][refcount][value]`) do not match the source-level
records in `core/base/memory.vr`, so any wrapper method whose compiled
body reaches execution misreads memory (`Heap.into_raw`'s `self.ptr`
surfaced the stored payload as a bogus type_id).  The intercept
surface must cover BOTH dispatch paths — statically-resolved `Call`
(`wrapper_runtime.rs`) and runtime `CallM` — and `clone` must bump the
refcount on ALL THREE clone dispatch arms (SHARED block,
universal-clone catch-all, cbgr-ref clone) with the binding-drop
decrement in `DropRef` and `*shared` deref-to-inner in
`handle_deref`.  Fixed in `799cff9b2`; pinned by
`core-tests/mem/allocator/integration_test.vr §4/§5`.

## Cross-reference

| Defect | Audit references | Close commits |
|---|---|---|
| REFDEREF-ESCAPE-UAF-1 (CLOSED 2026-06-19) | `core-tests/base/iterator/audit.md §4.5`; this catalogue §37 | (main HEAD — `&*p ≡ p` fold) |
| COLLECT-FROMITER-RESOLVE-1 (OPEN 2026-06-19) | `core-tests/base/iterator/audit.md §4.6`; this catalogue §38 | — (generic-body C-resolution / monomorphisation; call-site rewrite reverted) |
| RANGE-ARITH-ASSERT-1 (OPEN 2026-06-19) | `core-tests/base/iterator/audit.md §4.7`; this catalogue §39 | — |
| ARCHIVE-FANOUT-1 (CLOSED 2026-06-11) | `core-tests/base/iterator/audit.md §4.1`; this catalogue §30 | `946f3d787` |
| XMOD-RECNEW-UNIT-1 (CLOSED 2026-06-11) | `core-tests/base/iterator/audit.md §4.2`; this catalogue §31 | `8d8214d83` |
| ADAPTER-TRY-NEXT-1 (CLOSED 2026-06-12) | `core-tests/base/iterator/audit.md §4.3`; this catalogue §32 | `3858edf52` |
| ADAPTER-FORLOOP-NATIVE-1 (CLOSED 2026-06-12) | `core-tests/base/iterator/audit.md §4.4`; this catalogue §33 | `ae4b3d22a` |
| ADAPTER-TRY-NEXT-1 (OPEN 2026-06-11) | `core-tests/base/iterator/audit.md §4.3`; this catalogue §32 | — (compile_try classify + GET_VDATA + interp trace) |
| AOT-UMBRELLA-REEXPORT-1 / D1 (CLOSED 2026-06-01) | `core-tests/sys/linux/bpf/mod/audit.md`, `core-tests/sys/bitfield/audit.md` | this branch (`verum_types/infer/modules.rs` registry-recursion fallback) |
| AOT-NOT-USIZE-1 / D7 (CLOSED 2026-06-01) | `core-tests/sys/bitfield/audit.md` | this branch (`verum_types/infer/expr.rs` integer-alias list) |
| ARCHIVE-METHOD-MAYBEREF-1 / CLASS-9 residual (OPEN) | `core-tests/context/standard/audit.md §3.5/§3.7`, `core-tests/context/mod/audit.md §3.3` | — (codegen fix + rebuild pending) |
| FSTRING-CTOR-DISPLAY-1 (CLOSED 2026-06-01) | `core-tests/context/standard/audit.md §3.6` + `regression_test.vr::regression_display_direct_ctor_renders_uppercase_name` | `19bb51b3a` |
| AOT-PARALLEL-1 (CLOSED 2026-06-01) | this catalogue §23 | `f1c0510e3` (artifact paths) + LLVM-lock follow-up; harness `993679a01` |
| AOT-MEMCPY-1 (CLOSED 2026-05-31) | this catalogue §17 | `89604fe94` |
| AOT-ITER-1 (sub-bug 1 CLOSED / sub-bug 2 OPEN; mem suite mitigated 2026-06-03 via index-iter) | this catalogue §18 | `5bb3b83f8` (bounds-by-address); mem index-iter `15bfeaceb`, `c321cde54` |
| CASES-KEYWORD-1 (OPEN 2026-06-03) | this catalogue §27; `core-tests/mem/thin_ref/property_test.vr` | source workaround (`case_list` rename) |
| QUALVAR-CONSTRUCT-1 (OPEN 2026-06-03) | this catalogue §28; `core-tests/mem/thin_ref/unit_test.vr` | — (AOT type-path fix + rebuild pending) |
| RECVAR-TUPLE-1 (OPEN 2026-06-03) | this catalogue §29; `core-tests/mem/header/integration_test.vr` | — (AOT lowering fix + rebuild pending) |
| LAYOUT-FIELD-TESTBUG (test fix 2026-06-03) | `core-tests/mem/allocator/unit_test.vr::test_layout_record_construction` | `b4df86697` (size_/align_ field names; interp leniency had masked it) |
| NEWTYPE-UNBOX-1 (OPEN) | `core-tests/sys/windows/ntdll/audit.md §B`, `core-tests/sys/windows/time/audit.md` | — (working idiom in tests; codegen fix pending) |
| INTLIT-OVERFLOW-1 (OPEN) | `core-tests/sys/windows/tls/audit.md` | — (test typo fixed; language guard pending) |
| BAREVAR-ADT-1 (OPEN) | `core-tests/sys/windows/io/audit.md §A` | source qualified-form fix (this branch) |
| CROSS-MODULE-FIELDSHIFT-1 / CLASS-9 (CLOSED 2026-05-30) | `core-tests/mem/{mod,thin_ref,epoch}/unit_test.vr` | `64607bb8e` |
| LISTEQ-1 (CLOSED 2026-05-30) | `core-tests/base/ordering/unit_test.vr` (`test_ordering_in_map`/`_in_filter`) | `core/collections/list.vr` `Eq for List` via `.get(i)` |
| DEFERRED-INIT-1 (CLOSED 2026-05-29) | `core-tests/net/http2/hpack/audit.md §3.4` | (this branch) |
| TEXT-SMALLSTR-ASBYTES-1 / ENCODE-1 (CLOSED 2026-05-30) | `core-tests/net/http2/hpack/audit.md §3.3` | `core/collections/list.vr::extend_from_slice` index-loop rewrite |
| ENCSTR-LOOP-1 (CLOSED 2026-05-30) | `core-tests/net/http2/hpack/audit.md §3.5` | string-codec loop SIGABRT gone (EXTSLICE byte-copy + this branch) |
| HPACK-HUFFMAN-1 (OPEN) | `core-tests/net/http2/hpack/audit.md §3.3` | — (raw path green; Huffman codec round-trip mismatch) |
| MUTSELF-MATCH-1 (OPEN) | `core-tests/net/http2/stream/audit.md §3.1` | — |
| EXTSLICE-1 | `core-tests/net/cidr/audit.md §3.1`, `core-tests/net/http_range/audit.md §3.1`, `core-tests/encoding/base58/audit.md §A`, `core-tests/encoding/msgpack/audit.md §C`, `core-tests/encoding/value/audit.md §C` | `be64f4e1e`, `a60025262`, `b30e71f92`, `abf1033b1`, `ab9ec931b`, `41882e63b` |
| BSTRLIT-1 | `core-tests/net/ipv6_canonical/audit.md §3.1` | `8233fad28`, `abf1033b1` |
| CLOSURE-RESULT-1 | `core-tests/net/cidr/audit.md §3.1`, `core-tests/net/ipv6_canonical/audit.md §3.1` | `f649312c6` |
| QUALRESULT-1 | `core-tests/net/url/audit.md §3.4` | `8cf21a8be`, `74c074176` |
| TRANSIENTMUT-1 | `core-tests/net/cidr/audit.md §3.4` | `92480c76b` |
| CHAINMETHOD-1 | `core-tests/net/websocket/audit.md §3.6` | `92a85244b`, `400dccb78` (partial) |
| SLICEITER-1 (CLOSED 2026-05-14) | `core-tests/text/text/audit.md §A` | `8650a56ba`, `7cbd0585d` |

## See also

- [`core-tests/INVENTORY.md`](https://github.com/verum-lang/verum/tree/main/core-tests/INVENTORY.md) — full conformance suite inventory + per-round session markers.
- [`net.md` status table](/docs/stdlib/net) — current per-submodule status with CLOSED markers.
- [`status-convention.md`](/docs/stdlib/status-convention) — how to interpret the status keywords.

## 46. Cross-module simple-name type-registry collision (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **META-GROUP-XMODULE-1** |
| Stable trigger | Two stdlib modules declare types with the same SIMPLE name and both reach the lazy archive loader's walk (`meta.token.Group` record vs `math.algebra.Group` protocol vs `cli.spec.Group` record). |
| Manifestation | `field write out of bounds: field index N … type='X'` at record construction, load-order dependent (adding test files to a folder flips the failing set); AOT: `llvm::SmallVectorBase::grow_pod` SIGSEGV cascades + non-deterministic per-run failing sets. |
| Root cause | `type_name_to_id` / `type_field_layouts` / `type_field_type_names` were simple-name keyed with first-wins across every loaded archive module; the protocol-stub pass claimed the name first and `import_archive_type_with_protocol_remap` silently DROPPED later same-named types; record literals of a dropped type resolved field indices via the global-intern fallback. |
| Fix (landed) | Module-qualified registry keys (`"core.meta.Group"`) registered unconditionally alongside the first-wins simple key; mount-aware re-keying (`resolve_record_type_key` + `CodegenContext.mounted_types`); qualified-first type-id remap in `merge_archive_function_bodies`; benign-homonym downgrade in the type-table health checker; simple-name side tables ownership-gated. Commits `bfca381db` + `fe1d0b4ba`. |
| Pinned by | `core-tests/meta/token/regression_test.vr` (canary — validate across multiple suite compositions). |

## 47. Bare `type X is Y;` single-ident classification (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **META-SPAN-ALIAS-1** |
| Stable trigger | `type Span is MetaSpan;` — a bare known-type identifier after `is` (55+ alias-intent declarations across `core/`). |
| Manifestation | Alias-annotated lets of record literals compiled as VARIANT constructions (`MakeVariant` + payload-slot writes) while reads used record `GetF` — one-slot shift, `s.id` reading back heap-pointer bit patterns, `NullPointerAt` in methods. |
| Root cause | The grammar form is ambiguous (`type_expr ;` vs single-variant `variant_list ;`); the parser committed to the VARIANT reading for every bare identifier (task #13's marker-enum idiom), silently turning every alias into a bogus enum. |
| Fix (landed) | Module-level deferred classification at the parse funnel (`verum_fast_parser/src/normalize.rs`): single bare nullary variant re-classifies to `TypeDeclBody::Alias` when the name resolves to a known type (module-local decl, explicit mount, well-known primitive); marker idiom (`type E is Closed;`) and the leading-pipe form stay enums. Upstream of typecheck/VBC/AOT/metadata/LSP. |
| Pinned by | `core-tests/meta/span/unit_test.vr::test_span_alias_accepts_meta_span_value` + `tests/type_alias_test.vr`. |

## 48. Refinement-typed record fields lose runtime-representation classification (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **META-REFINED-FIELD-FLOATCMP-1** |
| Stable trigger | Record fields typed `Float{>= 0.0, <= 1.0}` / `Int{…}` (oracle.OracleConfig), compared or arithmetic'd at a call site. |
| Manifestation | `a.conf < b.conf` returns false for 0.4 < 0.7 — the compare lowered to a SIGNED-INT compare over raw IEEE-754 bits; f-strings printed values correctly (the value itself was fine). |
| Root cause | Three legs: `extract_type_name_from_ast` had no `Refined` arm (field-type names stored as a truncated debug dump), the canonical `well_known_types::type_names` classifiers matched exact names only, and `resolve_field_type_ref` had no `Refined` arm (baked descriptors carried no usable TypeRef). |
| Fix (landed) | `strip_refinement` in `verum_common::well_known_types::type_names` (all classifiers see the base); `Refined` arms in both AST-name extraction and descriptor TypeRef resolution. Refinement predicates unaffected (assert emission reads the AST). |
| Pinned by | `core-tests/meta/oracle/unit_test.vr` config-compare tests + property monotonicity laws. |

## 49. Interp test harness ran no type checker (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **META-TEST-TYPECHECK-1** |
| Stable trigger | Any type error inside a `core-tests/` file under `verum test --interp` (canonical: `candidates: List.of(a).append(List.of(b))` — `append` returns `()` into a `List<T>` field). |
| Manifestation | Test compiles, then dies at runtime far from the mistake: `NullPointerAt opcode 0x66`, `method '().append' not found on receiver of runtime kind ()`. Interp-vs-AOT "divergence" that was really the missing checker. |
| Root cause | `compile_module_with_stdlib` (the interp/property harness path) contains NO `verum_types` phase — strictly more lenient than both `verum run --interp` (validate_module) and `verum test --aot` (full pipeline). |
| Fix (landed) | `build_stdlib_test_module` preflights each standalone test file through `run_check_only` (the `verum check` entry), per-file cached. Escape hatch for triage sweeps: `VERUM_TEST_LENIENT_TYPES=1`. |
| Pinned by | The rewritten oracle/hygiene/diakrisis suites (list literals) — a reintroduced `()`-into-`List<T>` now fails compile. |

## 50. Dotted wanted keys squatted by bare-name descriptors (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **DELIM-FANOUT-SQUAT-1** |
| Stable trigger | A method call `x.m()` on a typed receiver harvests `Type.m` into the lazy loader's wanted set while ANY earlier-walked archive module exports a bare function named `m` (`close`, `open`, `read`, …). |
| Manifestation | Compile-time devirtualization binds `Type.m` to the unrelated bare function — `Delimiter.close()` dispatched to an io-driver `close(fd) -> ()`; result Unit flowed into `Maybe` matches (`method '().unwrap' not found`). Composition-dependent (which module walks first). |
| Root cause | The wanted-key suffix fanout registered a descriptor under ANY wanted key with a matching leaf; the (dotted-wanted, bare-descriptor) arm had no parent-type check. |
| Fix (landed) | That arm now requires the descriptor's own `parent_type_name` to equal the wanted key's type segment. |
| Pinned by | `core-tests/meta/token/unit_test.vr` Delimiter open/close tests + property pair-matching laws. |

## 51. `for x in &record.list_field` iterates zero times (CLOSED 2026-07-06)

| Field | Value |
|---|---|
| Defect class id | **REFFIELD-LIST-FORITER-EMPTY-1** |
| Stable trigger | `for arg in &attr.args { … }` — iterating a List FIELD through a `&record` receiver (stdlib parse_autopoietic; any user code with the same shape). |
| Manifestation | Loop body never runs (3-element list, zero iterations) — functions silently return their fall-through value; post-classifier-fix the interp SIGSEGV'd instead (field-ref address read as a List header). |
| Root cause | Two legs: the for-loop iterable classifier missed `&`-prefixed collection type names (routed to the custom `.next()` lowering whose CallM against a CBGR ref silently yields None); interp `IterNew` deref'd only the register-ref encoding, not field ThinRefs. |
| Fix (landed) | Classifier strips reference prefixes before the builtin-collection match; `IterNew` derefs via `resolve_arg_value` (all three reference shapes). |
| Pinned by | `core-tests/meta/diakrisis_attrs/integration_test.vr` parse_autopoietic both-present/order-independent. |

## 52. Generic-ctor instantiation staleness in the checker (OPEN)

| Field | Value |
|---|---|
| Defect class id | **GENERIC-CTOR-FRESHNESS-1** |
| Stable trigger | Two assignments of the same generic variant ctor with DIFFERENT payload types in one function: `epsilon = Maybe.Some(Text…); depth = Maybe.Some(3);`. |
| Manifestation | E400 `expected 'Int', found 'Text'` on the SECOND assignment (order-symmetric — swapping flips the error). `verum run`/`verum check` reject valid code; the interp test harness now surfaces it too (post §49). |
| Root cause (working) | A ctor-resolution path reachable from assignment-RHS synthesis carries a PERSISTENT registration-time TypeVar into call-site unification; the first use binds it, later uses collide. Instantiation-boundary freshening landed in `try_resolve_variant_constructor_with_arity`, but the leaking path is elsewhere (VERUM_TRACE_CTOR diagnostics landed to pin it). |
| Workaround | Bind through an annotated `let` (`let d: Maybe<Int> = Maybe.Some(3); depth = d;`) or reorder so same-payload ctors group. |
| Repro | 5-line `tup5.vr` (session scratchpad; to be promoted into `core-tests/base/maybe/regression_test.vr`). |

## §53 — PACK-HEADER-STAMP-1 (uninitialized Pack headers → runtime-indistinguishable layouts) — CLOSED 2026-07-06

| | |
|---|---|
| Class | AOT heap-object identity / nondeterminism |
| Stable trigger | A slice/tuple Pack crossing any boundary that loses compile-time register marks: stored in a record field (`Chars { bytes: s.as_bytes() }`), passed as a `&[Byte]` param, returned from a fn — then consumed by `.len()` / element reads. |
| Manifestation | `.len()` returns the Pack's PTR word (ASLR-floating garbage like 4309396287); `utf8_decode_char_len` NULL-derefs; every AOT text iterator collapses to zero iterations; run-to-run failing-set drift (headers were raw-malloc UNINITIALIZED — a genuine nondeterminism source). |
| Root cause | `emit_checked_malloc` never initialized the 24-byte ObjectHeader; AOT lists zero theirs; so no runtime consumer could distinguish Pack `{ptr@24,len@32}` from List `{len@24,cap@32,ptr@40}` once compile-time marks were lost. |
| Fix | `lower_pack` zeroes the header and stamps the canonical TUPLE TypeId (521); `lower_len` (hint-1 + list-marked arms), `lower_get_element` (runtime branch, phi merge) and `verum_generic_len` (521-check first) discriminate by the stamp. Strictly narrower: only stamped packs divert; every other tid keeps pre-fix reads. Commit 75ed5dc20. |
| Guards | `slicefield.vr`, `charsnext.vr`, `loopdiff.vr` while-let leg; meta AOT sweep. |

## §54 — FLOAT-FIELD-COMPOUND-1 (`obj.f += v` on Float fields = integer add over NaN-boxed bits) — CLOSED 2026-07-06

| | |
|---|---|
| Class | VBC codegen operator selection |
| Stable trigger | Compound assignment on a FLOAT record field: `p.a += 0.25`. Locals were fine; the Field arm never had a float branch. |
| Manifestation | Field reads back 0/garbage (int add over IEEE bit patterns). Silent data corruption — found on DAY ONE by the new refined-field runtime assert catching the garbage in an in-range test (§55). |
| Fix | Field-decl-type-driven `BinaryF` routing in `compile_compound_assignment`'s Field arm (`strip_refinement`-aware, so refined `Float{…}` fields classify too). Commit b1498acb6. Guard: `base/refinement` compound test + `fcomp.vr` both tiers. |

## §55 — REFINE-FIELD-DYNAMIC-BYPASS-1 (refined record fields unchecked for DYNAMIC values) — PHASE 1 CLOSED 2026-07-06

| | |
|---|---|
| Class | Refinement enforcement hole (red-team find) |
| Stable trigger | `Conf { level: nan_val() }` where `fn nan_val() -> Float { 0.0/0.0 }` into `level: Float{it >= 0.0, it <= 1.0}` — or any out-of-range value arriving through a call/variable rather than a literal. |
| Manifestation | Constructs successfully; field reads back NaN/out-of-range. Literals were E500-rejected by SMT; params/returns had runtime asserts (T1-F); FIELDS had neither for dynamic values. |
| Fix (phase 1) | `(type, field) → predicate` map captured at decl AST; runtime Assert emitted at all four field-write shapes (record literal, ctor-form, assignment, compound) with a conservative free-var guard, fail-open on key miss. Ordered float compares make NaN fail `>= 0.0` in BOTH tiers. Message: `refinement violation: field 'Type.field'`. Commit b1498acb6; trap specs `vcs/specs/L1-core/refinement/`. |
| Phase 2 (open) | Bake-wide predicate export/import + archive `FieldDescriptor` predicate carriage (format-versioned) so ARCHIVE-loaded types (OracleConfig) trap in user cogs; variant record-fields; documented holes: writes through `&mut` refs, `SetE`, unsafe. |

## §56 — AOT-DEVIRT-NIL-FALLBACK-1 (bare method + untyped receiver + suffix collision → silent nil) — OPEN (channel fix designed)

| | |
|---|---|
| Class | AOT devirtualization |
| Stable trigger | `for ch in s.chars() { … }` under `--aot` (any for-loop over a custom iterator OBTAINED FROM A METHOD CALL — method-call results carry no register type: the reg_types fixpoint has no CallM arm). |
| Manifestation | Loop body executes ZERO times (interp: correct). The bare `next` on the untyped `__for_iter` has 171 same-suffix candidates; every devirt strategy rejects; `lower_call_method`'s terminal fallback silently sets the dst register to nil (const_zero) — read as `Maybe.None`. |
| Failed fixes (evidence) | (1) VBC-side qualified-emit at the for-desugar — regressed interp 720→678 (qualified CallM changes interp dispatch for adapter/protocol iterables); (2) runtime type_id switch at the nil terminal — over-fired 2407×/compile (the gate matches the ENTIRE stdlib polymorphic-dispatch class: size_hint 163 candidates …) → LLVM SIGSEGV. Both reverted. |
| Designed fix | VBC→AOT register-type CHANNEL: per-function `register_type_hints` side-table in FunctionDescriptor (format minor 0→1), populated at `compile_for_custom_iterator` (the classifier KNOWS the type), consumed by the AOT reg_types pass so the EXISTING owner-equality resolves statically. Opt-in by construction — cannot over-fire. Implementation in flight. |

## §57 — COLLECT-FROMITER-2 (adapter-chain `.collect()` → bogus-owner from_iter panic) — RUNTIME LEG CLOSED 2026-07-06

| | |
|---|---|
| Class | Generic-body type-param erasure (interp dispatch) |
| Stable trigger | `.map(...).collect()` / `.chain(...).collect()` — any collect whose receiver is an adapter RECORD rather than an eagerly-intercepted List. |
| Manifestation | Panic `method 'FFIAbi.from_iter' not found …` listing the 8 real implementors. Root: generic `collect` body `C.from_iter(self)` compiles with target C ERASED into an instance-form call; runtime owner recovery reads a colliding type-id. |
| Fix (runtime leg) | `handle_call_method` intercept on the from_iter shape: locates the iterator among {receiver, arg0} (BOTH miscompile layouts observed), gated on candidate's own type having `.next`, drains via `call_function_sync(<Type>.next)` until `Maybe.None`. Erased target defaults to List (the entire pinned surface); Set/Map targets = codegen leg (open). Commit 928f6cd75; guard `collx.vr` + meta 835/0/41. |
