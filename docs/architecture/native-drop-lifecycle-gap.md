# Native Drop lifecycle: measured gap and implementation boundary

Status: open, 2026-10-04. T1538 tracks native owned `Drop`; T1540 records
its required ownership foundation. No lifecycle implementation or new
passing native-lifecycle claim follows from this note.

## Measured behavior

A source-driven macOS control built with the campaign's fresh pre-T1537
CLI acquired two `Mutex<Int>` values. It checked the
first inside and after a guard scope, and the second after `drop(guard)`:

```verum
mount core.sync.mutex.{Mutex};
fn main() {
    let scoped: Mutex<Int> = Mutex.new(17);
    {
        let guard = scoped.lock().unwrap();
        print(f"scope_inside={scoped.is_locked()}");
    }
    print(f"scope_after={scoped.is_locked()}");
    let explicit: Mutex<Int> = Mutex.new(29);
    let guard = explicit.lock().unwrap();
    drop(guard);
    print(f"explicit_after={explicit.is_locked()}");
}
```

The compiled binary reported `1, 1, 1`; the interpreter reported
`false, false, false`. The intended sequence is `true, false, false`.
Thus interpreter execution is not a correct ownership oracle for this
case: it releases the guard before its consumer enters the scope.
The measured fixture also included a separate nonblocking futex address
control after these observations; it crashed natively. T1537 fixes that
address-provenance defect and does not establish correct guard lifetime.
The task journal retains the exact fixture, logs and disassembly paths.

The native `Instruction::DropRef` arm in
`crates/verum_codegen/src/llvm/instruction.rs` releases CBGR reference
structs and skips plain object pointer/integer registers. It does not
invoke the declared `TypeDescriptor.drop_fn` for such objects. The
interpreter's corresponding handler does invoke user glue, which is why
premature producer cleanup can be visible there while native execution
instead keeps the lock held.

A separate source control makes the premature cleanup observable without
threads or mutexes:

```verum
type Count is { value: Int };
type Watch is { counter: &mut Count };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value + 1; }
}
fn make(counter: &mut Count) -> Watch {
    let value = Watch { counter };
    value
}
```

Before T1539, `make` emitted local `DropRef` after copying the tail into
its result register. The fresh interpreter incremented the counter before
the receiving scope began and incremented it again when that scope ended.
T1539 addresses direct block-tail handoff separately; it does not imply
that aggregate payload transfer is repaired.

## Why pointer classification cannot authorize Drop

* `Mov` aliases the same value. Independent per-register live flags would
  authorize repeated destruction of the same lifecycle.
* `Clone` does not always allocate in native lowering. The unknown-size
  arm forwards the original pointer; only a real fresh copy could create
  a new lifecycle. Shared-carrier copy policy is a separate declared rule.
* Immutable by-value parameters currently do not copy in the prologue.
  A function returning such a parameter can return a caller alias. A
  concrete return `TypeRef` therefore does not prove fresh ownership.
* `SetVariantData` stores the payload without an explicit copy/move
  event. `Mutex.lock` constructs a named guard and returns
  `Result.Ok(guard)` or a nested error wrapper; its lexical cleanup cannot
  decide ownership by checking only the final result register.
* `TypeDescriptor` carries `drop_fn` and `clone_fn`, but no affine/linear
  resource mode. `ParamDescriptor` carries the declared type and
  mutability, but no ownership-transfer contract. Merely spelling the
  current ordinary `MutexGuard` record `affine` would not supply those
  missing lowering facts.

The proposed runtime carrier is one shared ticket containing the exact
object base, declared glue identity and live state. Aliases share the
ticket; destruction consumes live state before calling user code.
Borrowed and raw views do not gain ownership. Runtime register slots must
clear on overwrite and follow control-flow joins. A stack ticket per
allocation *site* is insufficient for loops: a later iteration must not
revive a ticket still referenced by an earlier alias.

An LLVM-only instruction allowlist could support a small acyclic local
subset, declining calls, aggregate storage and escaping aliases. It was
not implemented: that would add another ownership model without fixing
its producer or archive boundaries. No failing or ignored test was added
to make that unimplemented subset appear covered.

## Reuse the CBGR pipeline, after supplying exact events

The existing pipeline is the appropriate integration point, but its
current outputs are not a destructor-safety certificate:

1. `verum_compiler::phases::cfg_constructor::CfgConstructor` builds CFG
   definitions for reference creation and uses for dereference. Its
   `DefSite`/`UseeSite` records do not encode allocation, copy, move,
   aggregate insertion, return transfer or a precise drop event.
2. `verum_cbgr::ownership_analysis::OwnershipAnalyzer` currently derives
   an unknown possible deallocation from each mutable use, without an
   allocation identity, and derives borrow events from use mutability.
   Those diagnostic approximations cannot authorize a real destructor.
3. `EnhancedEscapeAnalyzer` has SSA heap-store and call-graph parameter
   flow hooks. Without SSA or a call graph, its respective queries return
   false and rely on runtime CBGR checks. `NoEscape` obtained with missing
   facts therefore cannot authorize native owned cleanup.
4. `compile_ast_to_vbc` uses `TierAnalysisConfig::minimal()`, which disables
   ownership analysis. `TierContext::from_analysis_result` carries tier
   decisions and reasons, not ownership transfers, into VBC codegen.
   `TierAnalyzer` does not currently attach SSA or a call graph to the
   enhanced analyzer it constructs.

The next implementation should extend that shared CFG/event vocabulary
with declaration-owned value identities and explicit allocate, copy,
move, borrow, store, return and drop events. The AST producer must select
copy versus transfer according to the language's value/resource contract;
ordinary `Mov` cannot retrospectively supply that distinction. Existing
SSA, dominance, call-graph flow and ownership infrastructure can then
compute one lifecycle plan. Unknown calls, missing facts or analysis
budget exhaustion must leave ownership unproved. Carry the plan's exact
identities through VBC production, serialization, archive remapping and
monomorphization, so interpreter and native lowering consume the same
facts. Resource qualifiers and the value-copy contract need reconciliation
before declaring exclusive guards implicitly copyable or transferable.

Acceptance requires source-driven controls at both tiers for scope and
explicit drop, borrowed/raw aliases, branch joins and register reuse,
loop iterations, fresh versus aliasing copies, return transfer, and
nested variant payloads. A destructor counter must verify both timing
and exactly-once behavior. A native mutex must remain locked throughout
the guard's live scope and unlock afterward. T1538 and T1540 remain open
until the relevant guarantees are implemented and measured.
