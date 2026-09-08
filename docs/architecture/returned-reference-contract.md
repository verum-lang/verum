# Returned-Reference Contract

Pinned architectural rules for the question a `-> &T` accessor forces and
the type system does not answer: **when a function hands back a reference
into storage it does not own, what does the caller's register hold?**

    fn as_str(&self) -> &Text { &self.inner }

Three lines like that exist all over `core/`, and until T1188 the two
tiers disagreed about every one of them. The interpreter was right each
time; AOT produced an empty string, a length of zero, an address printed
where a name belonged, and a NULL dereference in a path shim — four
symptoms, one missing rule, chosen by whatever the neighbouring bytes
happened to be.

---

## 0. TL;DR — the five rules

1. **A reference is produced by one of three opcodes, and they do not
   agree about what the register holds.** Two yield an ADDRESS; one
   yields the already-loaded VALUE.
2. **The descriptor cannot tell you which.** An archive descriptor erases
   `&` from the return type exactly as it erases it from parameters. The
   callee's BODY is the only honest source.
3. **The caller owes the difference.** A returned address needs one load
   before any consumer applies a layout to it; a returned pre-loaded
   element needs its `interior_list_ref` mark restored so the consumer
   does *not* load.
4. **Every uncertainty resolves to "do nothing".** Both walks that decide
   this are allowed to be incomplete and are not allowed to be wrong; an
   unreadable instruction ends a walk with the answer that reproduces the
   behaviour the compiler had before the rule existed.
5. **A reference can also arrive WRAPPED.** `Maybe<&T>` crosses the
   boundary as a variant payload: no producer opcode runs in the caller,
   and the extraction marks the register "already a value" — so the
   caller owes a load *and* has been told it owes nothing. This one is
   decidable at the call site, because here the descriptor does keep the
   `&`.

---

## 1. The three producers

| Source spelling | Opcode | Register holds |
|-----------------|--------|----------------|
| `&self.field` | `CbgrExtended{RefField}` `0x0C` | the slot **ADDRESS** |
| `unsafe { &(*ptr).field }` | `FfiExtended{StructFieldAddr}` `0x4F` | the slot **ADDRESS** |
| `&self.items[i]` | `CbgrExtended{RefListElement}` `0x0B` | the element **VALUE** |

Dumped, because the source spelling does not predict the opcode and
reading the source is how this was got wrong once already:

```text
fn 'Path.as_str' (3 instrs):
  0: CbgrExtended { sub_op: 12, operands: [1, 0, 0] }
  1: Mov { dst: Reg(2), src: Reg(1) }
  2: Ret { value: Reg(2) }

fn 'Shared.deref' (5 instrs):
  0: GetF { dst: Reg(1), obj: Reg(0), field_idx: 0 }
  1: FfiExtended { sub_op: 79, operands: [2, 1, 16, 0, 1] }
  2: Mov { dst: Reg(1), src: Reg(2) }
  3: Mov { dst: Reg(3), src: Reg(1) }
  4: Ret { value: Reg(3) }

fn 'Bag.nth_ref' (4 instrs):
  0: GetF { dst: Reg(2), obj: Reg(0), field_idx: 0 }
  1: CbgrExtended { sub_op: 11, operands: [3, 2, 1] }
  2: Mov { dst: Reg(2), src: Reg(3) }
  3: Ret { value: Reg(2) }
```

`Shared.deref` is the reason rule 1 names opcodes rather than syntax: it
is spelled like a field reference, it behaves like one, and it is emitted
by a different instruction because the field lives behind a raw pointer.
A predicate that knew only `RefField` was blind to every `unsafe`
accessor in `core/base/memory.vr`.

`RefListElement` is the odd one, and deliberately so. Its lowering arm
performs the load itself and marks its register `interior_list_ref`, so
the `Deref` arm passes the value through instead of loading a second
time (`DEREF-INTERIOR-1`). That is correct *within* a function and it is
what makes the boundary dangerous.

### A fourth path: the reference that arrives WRAPPED

The three producers above answer "what did this opcode put in my
register". None of them covers a reference that crosses the boundary
*inside* a value:

    fn next(&mut self) -> Maybe<&T> { … Maybe.Some(item) }

No producer opcode runs in the caller at all. `CallM` returns a `Maybe`,
`GetVariantData` extracts field 0, and the register holds the payload
word — which is the slot ADDRESS, because that is what the callee
wrapped. Measured (T1260) on `[1, 2, 3]`:

```text
match xs.iter().next() {
    Maybe.Some(v) => { print(f"raw={v}"); let d = *v; print(f"deref={d}"); },
    Maybe.None => print("raw=none"),
}

           raw            deref
tier 0     1              1
tier 1     4421320704     4421320704
```

Read the second column. The explicit `*` was INERT, not merely absent:
`GetVariantData` marks every extraction `pass_through_ref` on the stated
ground that "extracted variant fields ARE the values themselves" — true
of `Maybe<Int>`, false of `Maybe<&Int>` — and that mark makes the `Deref`
arm identity. So the wrapped case breaks rule 3 twice over: the caller
owes a load, and has been told it owes nothing.

This is rule 2 read from the other side. The descriptor DOES carry the
`&` here — `ListIter.next`'s return-type name dumps as `Maybe<&T>` — so
the call site can decide without the callee's body. What the name cannot
give is the associated-type projection: `Maybe<Self.Item>` renders as
`Maybe<T0>` and the `&` is gone, so the fact is taken from the receiver
type's `Item` binding instead. Both sources are consulted, and either
suffices.

Scope, stated because guessing here is expensive:

* **`Maybe<&…>` only.** `Result<&T, E>` carries a payload at field 0 in
  BOTH variants, so a mark on the whole result would peel an `Err`
  payload that is not a reference. That needs the resolved tag, not the
  type name.
* **Immutable only.** `Maybe<&mut T>` must keep its address, or the
  consumer's write-through lands somewhere arbitrary.

Regression guard:
`vcs/specs/L0-critical/vbc/ref-payload-from-variant-loads-its-element.vr`.
Its six cells separate the load from the mark on purpose, because the
two fail differently and needed different fixes: a `List<Point>` element
reaches `p.x` (which used to ABORT the binary — the field was being
looked for on a slot address), while a `List<Text>` element needed the
element type carried onto the `Maybe` as well (T1261). With the slot
peeled and no mark, the register held a correct Text handle and the
f-string still printed `4301226128`: formatting resolves statically from
a register mark, field access resolves at runtime from the object
header.

---

## 2. Why the address, and not the value, leaves the callee

The `RefField` arm yields the slot address on purpose, and the reason is
recorded in the arm because it was learned by reverting the alternative:

* `BODY-FACT-RET-ADDR-1` (T1056) — returning the loaded value made
  `fn get(&self) -> &Int { &self.v }` hand back the number, and the
  caller's `*r` faulted at exactly the stored value (`0x2a` for 42).
  Every `*guard` in the language goes through this shape.
* `REFFIELD-SCALAR-ADDR-1` — yielding the address *unconditionally* was
  also tried, and broke every value consumer.

So the arm decides per site, by asking whether the address escapes: a
`Deref` of it here, or a `Ret` of it, means the address is the answer.
That decision is right and this contract does not change it. What it adds
is the other half — the caller was never told.

---

## 3. What the caller owes

| Callee returns | Caller does |
|----------------|-------------|
| `SlotAddress` and this function wants the CONTENTS | one `load` before the value is stored to the destination register |
| `SlotAddress` and this function wants the ADDRESS | nothing — it returns it onward, derefs it, or writes through it |
| `InteriorElementValue` | restore `mark_interior_list_ref` on the destination register |
| anything else | nothing |

"Wants the contents" is the *only* case that loads, and it is decided by
this function's own body: a result that is never `Deref`ed, never
`DerefMut`ed and never `Ret`urned is a result somebody is about to read
through a layout.

This is done at the CALL, not at each consumer, for a reason worth
keeping: a consumer-side rule needs a per-register mark, and T1167 and
T1194 are both the bill for per-register marks that outlived their value.
A load has no afterlife. The one mark this contract *does* set (rule 3,
third row) restores a classification the callee already made, and it
lives in `reg_types`, which `set_register` already clears.

### The call site is not the opcode you expect

A user method resolved statically is emitted as `Call { func_id }`, not
`CallM`:

```text
13: Call { dst: Reg(6), func_id: 39013, args: ... }   b.lend_items()
14: Len  { dst: Reg(8), arr: Reg(6), type_hint: 1 }   .len()
```

Both stores need the rule. A version that placed it only on the
method-call store passed its own `cargo check`, emitted seven loads
elsewhere in the stdlib, and moved not one of its own acceptance poles.

---

## 4. How the two questions are decided, and why they are asked
differently

Both halves read VBC bodies. They are deliberately unequal in rigour,
because their errors are unequal in cost.

**Callee side — must be sound.** A false "this returns an address" emits
a load through a value, which is a *new* wrong answer. It walks BACKWARD
from `Ret`: "what last defined the returned register" has one answer and
needs no alias set. Any instruction it cannot read as a definition is a
barrier that ends the walk with `None`.

The forward version of this walk was wrong, and the way it was wrong is
the standing lesson:

```text
fn 'Duration.cmp' (6 instrs):
  1: CbgrExtended{RefField, dst=3}     &other.secs
  2: Mov  { dst: 4, src: 3 }           alias {3,4}
  3: CallM{ dst: 3, ... }              r3 is an Ordering now — but a
                                       CallM is not a Mov, so the alias
                                       SURVIVED
  5: Ret  { value: 2 }                 -> "returns an address", false
```

Seven such loads were emitted, every one through a value that was not an
address. A per-register fact outliving its value — the T1167 class,
reproduced inside the walk chosen to avoid per-register facts.

**Caller side — may be imprecise.** Every way it can be wrong ends in "do
not load", which is what the compiler did before this rule existed. It
walks forward and its alias set never kills, because a linear walk over
an instruction list with branches in it sees two writes to one register
on mutually exclusive arms as consecutive:

```text
fn 'MapEntry.or_insert_with'
   6: Call { dst: Reg(5), ... }     entry.get_mut()   seed {5}
   7: Mov  { dst: 2, src: 5 }       Occupied arm      alias {2,5}
  17: Mov  { dst: 2, src: 6 }       Vacant arm — a KILLING walk drops 2
  20: Ret  { value: 8 }             8 came from 2, unseen
```

A killing walk answers "dropped here" about a value the function
RETURNS, and `*map.entry(k).or_insert_with(f) += 1` would write to a
stack temp instead of the map — no crash, no diagnostic, a map that
silently stops counting.

The `RefField` arm keeps the killing walk bit-for-bit, because changing
that arm's verdict is a separate question with T1056 attached to it.

---

## 5. Verifying

`VERUM_TRACE_RETSLOT=1` prints one line per call site considered, not
only per site changed — the distinction matters, because "how many did I
change" cannot tell you the rule is looking at the right opcode and "how
many did I consider" can:

```text
sites considered: 7584
loads emitted:       4
[retslot] main -> Bag.lend_items dst=6 kind=SlotAddress
                                 fate_here=Dropped load=true
```

Read the emitted loads BY NAME, never only their count. Counting says the
rule fires; reading says it is right. The `or_insert_with` false positive
above was found that way and nowhere else.

Regression guard:
`vcs/specs/L0-critical/vbc/returned-reference-is-a-slot-address.vr`
(differential, tiers 0 and 1).

---

## 6. What this contract does NOT cover

* **Slices crossing into a syscall wrapper.** `path.as_str().as_bytes()`
  feeding `copy_path_nul` is the FFI byte-buffer contract, not this one —
  a slice is not a slot address and giving it a load would be a third
  wrong answer. See `ffi-byte-buffer-contract.md`.
* **The same offsets do NOT imply the same readers.** `Shared<T>` is two
  hops — `Shared.ptr` then `SharedInner.value` at +16 — and both tiers
  agree on those offsets, measured independently: AOT from the IR that
  `Shared.deref` emits (`StructFieldAddr` operands `[…, 16, 0, 1]`), the
  interpreter from `bridge_extent_room` returning 24 for a three-field
  packed block. But the REPRESENTATIONS differ per hop and per tier:

  | | interpreter | AOT |
  |---|---|---|
  | `Shared.ptr` slot | NaN-boxed Int carrying the address | raw address |
  | `SharedInner` block | packed, 24 bytes, no tags | packed, no tags |

  So a peel ported between tiers by copying its offsets inherits the
  wrong reader. That is close to how the interpreter's two peels drifted:
  `.add(1)` was frozen against a shape that moved, and reading the inner
  block through `Value::tag()` cannot work at any index because the block
  carries no tags at all. AOT's peel is correct partly by accident — it
  reads raw at both levels because AOT stores raw at both levels — and it
  bound-checks nothing, where the interpreter's `bridge_scalar_slot`
  validates that the eight bytes lie inside a live extent.

* **A `dyn` receiver is a different mechanism, not a fourth face.**
  T1186 recorded one defect wearing four faces — plain, `Shared<T>`,
  `Shared<dyn P>`, `Heap<T>`. Three were one defect and are now closed.
  The fourth is not: `VERUM_AOT_TRACE_CALLM` prints NOTHING for
  `describe` on a `Shared<dyn Source>`, so the call never reaches the
  `CallM` path and neither the deref-hop nor the type-id switch is
  involved — a `dyn` receiver dispatches through a vtable. Worth stating
  because "same symptom, same cause" is the inference this whole
  document exists to discourage, and it was made here by the row's own
  author.

* **Wrapper transparency on the FIELD path.** `s.method()` on a
  `Shared<T>` reaches through the wrapper — `handle_call_method` has a
  documented Deref last resort, and T1183 / T1186 fixed and extended it.
  `s.field` has no counterpart, so the two syntaxes disagree about what
  a `Shared<T>` is: field 0 answers a slot of the WRAPPER itself (a
  silent wrong number — the declared `Shared { ptr, generation, epoch }`,
  NOT a substituted carrier; that reading was mine and was retracted
  after the raw dump showed slot 0 holds a NaN-boxed `ptr`) and field 1
  dereferences null. Tracked as T1202, and it
  is the same shape as this contract seen from the other side — this
  document is about a boundary erasing WHICH KIND of reference you
  hold, T1202 about a wrapper being transparent to one syntax and
  opaque to another. Both are two paths to one value, built at
  different times and never reconciled.

* **Transitive forwarding.** `fn hop(&self) -> &Int { self.inner.get() }`
  has no producer opcode of its own, so it answers `None` and its callers
  get nothing. It is invisible in practice because the natural way to
  consume such a value is `*h.hop()`, which the `Deref` arm already
  handles. Making it transitive needs the method table to resolve
  `CallM.method_id` inside a body, which a call site gets for free and a
  body walk does not.

These are omissions of coverage, not of correctness: in each case the
compiler does what it did before this contract existed.
