# Tier-Coherence Pillars — replacing heuristic families with contracts

Status: DESIGN (evidence-complete). Owner: meta-conformance line.
Origin: 2026-07-06..08 conformance campaign — every pillar below is
backed by shipped fixes whose SHAPE revealed the same disease:
runtime/lowering HEURISTICS approximating facts the compiler already
knew earlier in the pipeline. The cure in every case is to CARRY the
fact, not re-guess it.

Ledger discipline for this document: each pillar lists (Disease →
Evidence → Contract → Kills → Migration). "Kills" enumerates the
crutches the contract retires.

---

## Pillar 1 — Typed references in VBC (RefLocal / RefObj split)

**Disease.** `RefMut(reg)`/`Ref(reg)` are UNTYPED. The interpreter
gives them CBGR register-ref semantics; Tier-1 lowering must GUESS
between alloca-address and object-pointer passthrough from ~24
per-register heuristic mark sets (text/list/map/set/deque/btree/
chan/struct/inline/variant/maybe/obj-type/sticky/ref-param/...),
each populated by its own ad-hoc site and cleared in set_register.

**Evidence.** The Display-formatter chain (write_str received a
STACK address as receiver — fmt re-reffed its `&mut Formatter`
param); the iterderef slot-vs-object runtime probe; #41 scalar
`&T` un-strip + CmpI deref; value==ref comparisons; closure
size-0 views (#13); the blanket-mark experiment corrupting the
bake (falsified 2026-07-08); sticky_type_hints as channel-of-
last-resort.

**Contract.** The type checker KNOWS, at every ref-creation site,
whether the referent is (a) a stack-local scalar slot or (b) a
heap object. Carry it: split the instruction —
  * `RefLocal dst, src` — alloca semantics; writeback conventions;
    only ever emitted for scalar locals.
  * `RefObj dst, src` — object identity; Tier-1 lowering is a MOV
    of the pointer; Tier-0 keeps CBGR ref semantics unchanged.
Re-refs of reference-typed params are ALWAYS RefObj by typing.
Lowering consults NOTHING but the opcode.

**Kills.** The heap-type mark family in lower_ref_mut; sticky-hint
consultation for refs; the per-fn CallM-receiver scan (candidate-a);
the slot-vs-object runtime probe in Deref (already landed, becomes
an assert); the #41 ref-param scalar set (subsumed by RefLocal).

**Migration.** (1) checker: annotate Ref/RefMut emission with the
referent class (it already resolves the type); (2) VBC: add opcodes
(format minor bump), emit split; interp maps BOTH to today's
behavior (zero interp risk); (3) Tier-1: trivial lowerings; (4)
delete heuristics behind a kill-switch env for one release; (5)
regression: the whole 2026-07 canary battery (loopdiff/canary/
pushlen/ctx_disp/fmtchain/psA-C/m_ti-m_tr).

---

## Pillar 2 — Content-addressed global identity (types, functions, metadata)

**Disease.** Identity is SIMPLE-NAME + module-local ids + positional
slots, patched by: first-wins policies, alias-preference collision
rules, suffix scans, name#arity mirrors, id re-homing on collision,
health-check warnings, and per-wave determinism sorts.

**Evidence.** §46 type-name collisions (Group OOB); §40 positional
field-intern latents (`.lock` on SpanFlags canary); metadata
simple-name first-wins (Span alias record lost per bake dice);
#17 bare-name registry (THREE bare `range`s; Text.new misbind);
TYPE-ID-COLLISION-1/2/3 (drop glue roulette); #27 serialization
nondeterminism (module order, stub order, #arity slot flips).

**Contract.** Every decl's canonical identity = fully-qualified
path + blake3(content). ALL tables (registry, metadata, archive
indices, method tables) key by canonical identity; bare-name access
is a QUERY LAYER (resolve → canonical) never a storage key.
Serialization is CANONICAL FORM: emit tables sorted by canonical
identity — determinism BY CONSTRUCTION, not by chasing HashMap walks.

**Kills.** Every first-wins/alias-preference/suffix-scan/arity-
mirror policy; the §40 latent class entirely; #27 as a hunt (the
canonical writer replaces per-site sorts); metadata collision
policy (54d0ae1d4) becomes a compatibility shim then retires.

**Sequencing constraint discovered 2026-07-09 (nine determinism
waves + two falsified canonicalization attempts):** runtime
bare-suffix dispatch tie-breaks by LOWEST FUNCTION ID, and stub
spelling (`name#arity`) is load-bearing for arity disambiguation —
so id/name canonicalization FLIPS dispatch winners (39-test
signature) until dispatch resolution is made name-deterministic.
Revised order: (0) name-deterministic dispatch tie-breaks (qualified
resolution everywhere the bare scan wins today), THEN id
canonicalization. Waves 1-8 (landed) killed the string-table and
semantic (TypeParamId) dice; the function-section dice remain gated
on step (0).

**Migration.** Staged: (0) dispatch tie-break determinism; (1) canonical-id computation + DUAL keying
(new key added alongside old, warn on divergence); (2) flip readers
to canonical; (3) canonical serializer (archive major bump);
(4) retire old keys. Acceptance at each stage: byte-identical
double-bakes + the full canary battery.

---

## Pillar 3 — One generic-signature truth (end the split brain)

**Disease.** A method's generic structure lives in FIVE mutually-
lossy representations: checker TypeScheme (impl_var_count set at 6+
sites, default 0), AST decls, VBC descriptors (param-type names
lost; is_generic was a dead flag — memory 2026-07-03), core-
metadata (PARAM-STRIPPED signatures — impl-vs-method split
unrecoverable), and monomorph requests (CallG type_args).

**Evidence.** #26 seventh-layer hunt: scheme birth in the metadata
loader with impl_var_count=0; receiver-position derivation impossible
(no params); type-generics-count derivation regressed 102 tests
(ordering unrecoverable). The CallG encode/decode mismatch and
dead is_generic (memory). `dyn:` tokens at AOT (mono gaps).

**Contract.** ONE serialized form for a callable's signature:
ordered generic params each tagged {impl-level | method-level} +
full param/return types. Checker, VBC descriptor, and metadata all
carry or derive views of THE SAME record. bind_limit reads the tag,
never counts.

**Kills.** impl_var_count set-site whack-a-mole; bind-limit
heuristics; param-stripped metadata (the #26 carry is the first
increment of this pillar); FunctionInfo.return_type_name stringly
typing (long-term).

**Migration.** Increment 1 = the #26 carry (impl-generic tags on
metadata method descriptors, minor bump). Increment 2 = VBC
descriptor unification (subsume register_type_hints' fn-shape
uses). Increment 3 = checker consumes the carried tags, deletes
derivations.

---

## Pillar 4 — One meta engine (finish #18 convergence)

**Disease.** Two compile-time meta executors (tree-walk evaluator
238KB + VBC executor) with 17+ pinned semantic divergences
(i128-panic vs wrap; Int+Float PANIC vs error; Paren/f-string
quote-swallowing; Text subset; short-circuit; char-as-i64...).

**Evidence.** The differential harness (landed): 14 agree / 8
pinned / 0 new + wrapper TokenStream hardcode + FunctionId(0)-by-
convention running a stdlib fn instead of the meta fn.

**Contract.** The VBC executor is THE meta engine. The tree-walk
becomes (step ii) a thin adapter: @const/tagged-literal/interpolation
route through execute_raw + the structural extractor (donor code
landed in the harness crate); (step iii) the evaluator is deleted;
the harness corpus becomes the regression gate (agree-set must be
100% of retained semantics; pins become executable spec).

**Convergence map (2026-07-10).** Landed under the
`VERUM_META_ENGINE=vbc` fail-closed gate (any engine error falls
back to the tree-walk; divergences belong to the harness):
  * @meta zero-arg bodies (pipeline.rs phase_meta_evaluation) ✓
  * @const blocks (collect_const_blocks_from_expr) ✓
  * async task expressions (meta/async_executor execute_task_expr) ✓
  * value domain: scalars + Text exactly; lists/maps structurally;
    records as declared-order field tuples via
    `InterpreterState::record_named_fields` (matches the tree-walk's
    record→tuple reduction; the SOURCE-order divergence is pinned by
    `probe_record_field_order`) ✓
  * enclosing-module TYPE declarations ride into the synthetic
    module (`execute_raw_with_items`) so record-typed meta code
    compiles ✓

**Step-iv boundary (dies last, with the evaluator).** Call families
whose ARGUMENTS are AST values (`ConstValue::Expr` /
`MetaValue::Expr`) — macro expansion (`pipeline/macros.rs`
execute_user_meta_fn ×2) and the TokenStream mode — cannot route
until MetaValue's AST variants get a VBC value representation.
The hygiene lift-evaluator hook (`set_lift_evaluator`) has NO
callers — dormant API, nothing to route. Deleting the evaluator
(step iii's tail) is gated on: default engine flip after a full
release-cycle corpus soak with 0 NEW divergences, THEN the AST-value
representation lands, THEN the delete.

**Kills.** 238KB of divergent semantics; every pinned divergence
as a user-visible inconsistency; the discarded-@meta-result drops.

---

## Pillar 5 — One Text ABI (representation-tagged, growth-capable)

**Disease.** Text exists as SSO NaN-box, heap {ptr,len,cap}, rodata
const, and byte-blob builder forms; every consumer re-guesses the
form; growth on a const crashes/loses; split code points poisoned
SSO; per-byte stride confusion.

**Evidence.** SSO-UTF8-SPLIT-1 (release silently dropped Greek
prefixes!); const-seed formatter buffer ungrowable; ptr_offset
byte-vs-slot strides; builder-Text print truncation; is_text_object
heuristics inside generic_eq.

**Contract.** Tier-1 Text is ONE tagged representation: header bit
distinguishes {inline-small | heap-owned | static-const} with
growth = COW promotion (const → heap on first mutation, by
capability bit, never UB). All primitives (push/concat/len/eq/
print) read the tag, not heuristics. Tier-0's NaN-box maps 1:1.

**Kills.** is_text_object heuristics; const-grow crash class;
SSO-poisoning producers (already fixed producer-side — becomes
type-level impossible); the byte-blob special case.

**Landed (2026-07-10).**
  * COW growth: const-Text promotion on first mutation verified
    present (text.vr cap==0 static tag) and PINNED both tiers
    (core-tests/text/cow, 3/3).
  * BYTE_SLICE(528): `Text.as_bytes()` now stamps ONE cross-tier
    object `[ObjectHeader(528)][ptr:i64][len:i64]` — Tier-0 heap
    alloc mirrors Tier-1 `lower_pack_typed`; the `len <= 1_000_000`
    FatRef-as-Text heuristic is RETIRED at every Text sink
    (read_text / extract_string / eq / hash / capacity); generic
    `&[T]` slice FatRefs (cbgr re-slice ops) remain untouched.
    Cross-tier drift pinned (`byte_slice_typeid_pinned` +
    codegen stamp pin); 18 .vr probes green (interp);
    pre-existing `for b in text.as_bytes()` SIGSEGV fixed by
    construction.
  * ONE heap layout (final leg): the legacy `TypeId(0x0001)`
    `[len:u64][bytes…]` byte-blob is RETIRED. Every interpreter
    heap-Text producer emits ONE self-contained TEXT record
    `[ObjectHeader(TEXT)]{ptr,len,cap}[bytes…]` (`Heap::alloc_text`
    / `alloc_text_with_capacity`), readers dispatch through
    `heap::text_record_payload` / `value_as_text_record` — no
    dual-layout branch remains. `cap == 0` is the immutable/COW
    marker (matches AOT rodata `{ptr,len,0}`); capacity-carrying
    records reserve `cap + 1` bytes per text.vr's owned-buffer
    convention. Pinned by
    `crates/verum_vbc/tests/text_record_arch_p5_tests.rs` +
    `core-tests/text/storage/`. Folded fixes: `grow`'s cap==0
    branch now covers `len` (COW-promoting a >16-byte static
    overflowed a 17-byte buffer); builder records stored an object
    header as a bytes pointer (`reserve` garbage).

---

## Sequencing (dependency-honest)

1. Pillar 3 increment 1 (#26 carry) — unblocks array-iter NOW.
2. Pillar 1 (RefLocal/RefObj) — closes #21-Display/#13-runtime,
   deletes the mark family; medium surface, huge payoff.
3. Pillar 2 staged dual-keying — retires §40/§46/#27 as classes.
4. Pillar 4 steps ii/iii on the landed harness.
5. Pillar 5 — after 1 (ref semantics feed Text consumers).

Each pillar lands behind the day's full canary battery + the
deterministic double-bake gate.
