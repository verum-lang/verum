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

## 0. TL;DR — the four rules

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
* **Transitive forwarding.** `fn hop(&self) -> &Int { self.inner.get() }`
  has no producer opcode of its own, so it answers `None` and its callers
  get nothing. It is invisible in practice because the natural way to
  consume such a value is `*h.hop()`, which the `Deref` arm already
  handles. Making it transitive needs the method table to resolve
  `CallM.method_id` inside a body, which a call site gets for free and a
  body walk does not.

Both are omissions of coverage, not of correctness: in each case the
compiler does what it did before this contract existed.
