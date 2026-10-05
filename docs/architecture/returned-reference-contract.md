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

1. **Reference conventions differ by opcode and backend.** A producer
   can yield a slot ADDRESS or the already-loaded VALUE; the historical
   producer table below includes a tier-dependent case.
2. **A declared reference type cannot tell you which.** Some archive paths
   erase `&`; even a preserved `&T` does not distinguish an address from an
   already-loaded value. The exact native producer and its argument/result
   adaptations own that fact. A VBC body is evidence only when native lowering
   actually selects that body; see the native-call prerequisite below.
3. **The caller owes the difference.** A returned address needs one load
   before any consumer applies a layout to it; a returned pre-loaded
   element needs its `interior_list_ref` mark restored so the consumer
   does *not* load.
4. **Every uncertainty resolves to "do nothing".** Both walks that decide
   this are allowed to be incomplete and are not allowed to be wrong; an
   unreadable instruction ends a walk with the answer that reproduces the
   behaviour the compiler had before the rule existed.
5. **A reference can also arrive WRAPPED.** A `Maybe<&T>` payload can
   contain either a slot address or an already-loaded value. The preserved
   `&` establishes the declared type, not the native word representation.
   Extraction and forwarding need the exact producer fact, scoped to the
   sum owner, variant tag and field. An unconditional load can double-load
   a value; an unconditional pass-through mark can conceal an address.

---

## 1. The four producers

| Source spelling | Opcode | Register holds |
|-----------------|--------|----------------|
| `&self.field` | `CbgrExtended{RefField}` `0x0C` | the slot **ADDRESS** |
| `unsafe { &(*ptr).field }` | `FfiExtended{StructFieldAddr}` `0x4F` | the slot **ADDRESS** |
| `&self.items[i]` | `CbgrExtended{RefListElement}` `0x0B` | the element **VALUE** |
| `unsafe { &*p.offset(i) }` | `MemExtended{DerefValue}` `0x1E` | the element **VALUE** (Tier 0) / the **ADDRESS** (Tier 1) |

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

### The fourth producer: `&*p` over an array of `Value`

    let item: &T = unsafe { &*self.ptr.offset(i) };

`core/collections/list.vr` is written in this idiom — `unique`, `iter`,
`ListIter.next`, `fmt_debug` — and for a long time it produced neither
convention: the register held a bare ADDRESS with nothing saying so.
`&*p` folds to `p` (the identity `&*p ≡ p`, which exists so the
reference cannot dangle when the producing frame pops), and a raw
address is INT-tagged, so `*item` reached the generic `Deref`'s integer
arm — identity — and handed the address straight back:

```text
let xs: List<Int> = [7, 7, 9];
print(f"{*a} {*b} {*c}")   ->  48624037456 48624037464 48624037472
```

Teaching `*` to load would have fixed one consumer in four. `&T` is also
`item.clone()`, `item.field`, and every method call on `item`; each of
those reads the register on its own terms, and each read the address.
So the producer takes the **pre-loaded VALUE** convention instead, the
same one `RefListElement` takes for the same storage — and for the same
reason, since `p.offset(i)` and `&self.items[i]` name the same slot.

Two conditions, both checked in codegen and neither guessable at run
time:

* **The borrow is immutable.** `&mut *p` must stay an ADDRESS or the
  write lands in a copy — that is the `RwLock`/`Mutex` guard idiom
  (`&mut *(&self.f as *const T as *mut T)`), whose whole purpose is to
  reach the storage.
* **The pointee is ONE `Value` slot.** `&unsafe T` is spelled the same
  for a run of 8-byte slots (a `List` backing) and for an INLINE record
  array (`&unsafe Slot<K, V>` — Map's entry table, 32-byte stride),
  where an address names the record and its first field is not the
  answer. The record descriptor is the discriminator, and an UNKNOWN
  pointee keeps the address it had — rule 4.

#### The one producer whose convention is TIER-DEPENDENT, and why

`DerefValue` performs the load at Tier 0 and NOTHING at Tier 1. That is
not an oversight: the opcode states "this register is a reference to a
`Value` slot", and the two tiers already disagree about where that load
belongs. Tier 0's generic `Deref` answers identity for an int-tagged
address — it cannot tell an address from a number — so the load must
happen at the producer. Tier 1's `Deref` already loads through a raw
address, measured on the pre-change binary: the six-line probe printed
`7 7` and `eq true` at Tier 1 while the interpreter printed three
addresses. Loading at the producer there too is a DOUBLE dereference —
it reads the element `7` as an address and SIGSEGVs, which is exactly
what the first landing of the AOT arm did.

So Tier 1 keeps the ADDRESS convention it already had, and the arm
propagates the marks a re-borrow must not lose (`pass_through_ref`,
list/map/set, inline-struct, element stride, obj type) — the same set
`DerefRaw`'s own pass-through arms carry.

Tier 1 has its own gap here, stated because it is easy to mistake for
this one: on the SAME pre-change binary `[1,1,2,3,3].unique()` and
`xs.tail()` both print EMPTY at Tier 1, where Tier 0 now answers
`[1, 2, 3]` and `[2, 3, 4]`. That is a separate defect in Tier-1 list
returns and nothing here moves it in either direction — the Tier-1
output is byte-identical before and after.

`DerefValue` is not `DerefRaw` at width 8. That opcode answers the FFI
question — "what integer do these bytes spell?" — and decides between a
NaN box and a C `int64_t` by inspecting the bits (A177). A `Float`
element holds a raw IEEE double with no tag, so it comes back through
that reading as an integer of the double's bit pattern. `DerefValue`
answers the language question: the address names a `Value` slot, so the
eight bytes ARE a `Value` and are taken verbatim — with no exception for
bridge memory, because every pointer-tagged arm of `handle_deref`
already reads its target that way and its bridge arm says so outright
("an address inside a live bridge block names a `Value` slot, so `*p`
READS it", T0705/T0384). Two readers of one storage that disagree is
the defect this producer exists to remove.

#### The producer had a second fault underneath it

`&*self.ptr.offset(i)` needs `self.ptr` to be the address of element 0,
which is what `list.vr` declares (`ptr: &unsafe T`). The interpreter
stored the backing ALLOCATION in that slot — an object whose first
`OBJECT_HEADER_SIZE` bytes are an `ObjectHeader` — and every intercept
that read the slot added the skip back by hand. So `offset(0)` addressed
the header's `type_id` word and `offset(3)` addressed element 0, and the
two facts composed into one wrong number.

`Text` had the convention right the whole time (`alloc_text` stores the
BYTES, not the object), which is what made the disagreement legible once
the two were put side by side. `GetF`/`SetF` now translate the slot in
both directions — the only two places compiled code sees it — so the
interpreter keeps its internal convention and the language gets the one
it declares.

The size of this was hidden by INTERCEPTION. 41 of the 96 `List` methods
that touch `self.ptr` are answered by the interpreter and never run
their own body; `List.contains` is one of them and is correct, while
`List.unique` — the same `*item == *value` spelling, three functions
away — is not intercepted and compared addresses.

### A fifth path: the reference that arrives WRAPPED

The four producers above answer "what did this opcode put in my
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

The descriptor carries the declared `&` here: `ListIter.next`'s return-type
name can render as `Maybe<&T>`. An associated-type binding can supply the same
declared-type evidence when `Maybe<Self.Item>` renders as `Maybe<T0>`. Neither
channel establishes whether the payload word is an address or a loaded value.
The legacy peel also consults the producer body; the native-call prerequisite
below explains why that body must match the implementation actually emitted.

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

### The three arms that decide what `&x` becomes

Added 2026-09-11 while chasing T1441, where every fact readable on the
CALLEE side was identical for three records that answered 1, 2 and a
stack address. The decision is on the CALLER side and two of these arms
had no diagnostic at all.

| lever | arm | what it prints |
|---|---|---|
| `VERUM_TRACE_REF` | `lower_ref` | which shape `&x` takes, and on what: `has_slot`, `has_alloca`, alloca mode, the source register's marks |
| `VERUM_TRACE_GETFIELD` | `lower_get_field` | the predicate set the exits are chosen by, plus the DESCRIPTOR the type name resolves to (kind, field count, transparent flag, size) |
| `VERUM_TRACE_DEREF` | the `Deref` arm | which of its six exits is taken — five are value-identity, so "the deref did nothing" is five symptoms, not one |

Run them with `VERUM_NO_OBJECT_CACHE=1`. A trace emitted DURING lowering
cannot appear when the object cache serves the binary without re-running
it, and the log says `replayed` when that happens — which cost one whole
measurement the night these were written.

`VERUM_TRACE_REF` is the one that ended it. `is_heap_type` carries the
term `get_obj_register_type(src).is_some()`, and

```text
[refmark] main Ref r0<-r2 heap=true  obj=None        (a Text argument)
[refmark] main Ref r7<-r5 heap=false obj=None        <- the File
[retmark] in=main fn=File.create dst=r0 SKIPPED: already obj_type
```

says in three lines what a dozen black-box controls could not: the value
moved registers and its type name did not, so a heap pointer was handled
by the recipe for `42`.

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

* **A transparent wrapper reached THROUGH a reference.** This document
  is about a boundary erasing WHICH KIND of reference you hold; this is
  the boundary erasing THAT you hold one. A transparent wrapper has no
  runtime identity, so its register holds the inner value and `x.0` is an
  identity `Mov` — correct, and wrong the moment the register holds a
  reference to the wrapper instead, because the same `Mov` hands back the
  address. The two are indistinguishable where the decision is made:
  `infer_expr_type_name` erases the `&`, so `NT` and `&NT` both answer
  `"NT"`.

  Measured 2026-09-11, five lines of Verum with no stdlib in sight:

  | | Tier 0 | Tier 1 |
  |---|---|---|
  | `NT(42).0` | 42 | 42 |
  | `fn f(p: NT) -> Int { p.0 }` | 42 | 42 |
  | `fn f(p: &NT) -> Int { p.0 }` | 42 | 6134571104 |
  | `fn f(p: &Rec) -> Int { p.a }` | 77 | 77 |

  A named-field record is unaffected because it never took the
  transparent path. The population is not small: `core/` declares 179
  transparent wrappers and spells 493 parameters `&<wrapper>` across 77
  files. Tracked as T1438; the pinning spec is
  `vcs/specs/L0-critical/reference_system/a_wrapper_reached_through_a_reference_unwraps_to_its_value.vr`,
  which runs at both tiers and keeps the VALUE forms beside the reference
  ones — a fix that stopped treating wrappers as transparent would pass
  every reference case and quietly make `NT(42).0` a heap read.

These are omissions of coverage, not of correctness: in each case the
compiler does what it did before this contract existed.

## 7. The limit: a generic adaptor erases the question itself

Sections 3 and 4 answer "what does the caller owe?" by asking the
CALLEE'S BODY — `wrapped_payload_is_slot_address` walks back from
`SetVariantData` and answers ADDRESS only for a `GetF`-produced payload.
That local test requires proof that native lowering calls the selected body.
A declaration lookup alone does not establish this; runtime replacements can
select a different implementation, as the later native-call control shows.

It stops working the moment the callee is a generic adaptor, and the
stop is not an omission that a better walker would fix.

Measured 2026-09-12, two rungs that differ in one word:

```verum
fn enum_over_list(xs: &List<Int>) -> Int {
    let mut total = 0;
    for (i, x) in xs.iter().enumerate() { total = total + i + *x; }
    total
}

fn enum_over_slice(bs: &[Byte]) -> Int {
    let mut total = 0;
    for (i, b) in bs.iter().enumerate() {
        total = total + i;
        if *b == 108 { total = total + 1000; }
    }
    total
}
```

| | Tier 0 | Tier 1 |
|---|---|---|
| `enumerate` over `List<Int>` | 63 | 63 |
| `enumerate` over `&[Byte]` | 1003 | `EXC_BAD_ACCESS` at `0x6c` |

`0x6c` is the byte `'l'` — the element VALUE used as an address.

Both iterators declare `-> Maybe<&T>` and bind `Item = &T`, and they
disagree about what the payload word holds:

| producer | body | payload word |
|---|---|---|
| `ListIter.next` | `&*self.ptr` (`RefRawAddr`, 0x0D) | ADDRESS |
| `SliceIter.next` | `&self.slice[i]` (`RefListElement`, 0x0B) | pre-loaded VALUE |

Read in `lower_cbgr_extended`, the two arms are starker than the table:
0x0B computes `elem_ptr`, LOADS through it with a width switch, stores
the VALUE and calls `mark_interior_list_ref`; 0x0D stores the ADDRESS
unchanged and calls `mark_interior_list_ref`. **Both producers set the
same mark on opposite representations** — 0x0D's own comment says it does
so "for DEREF-INTERIOR-1 parity, mirroring RefListElement", and the
parity is syntactic. The mark says "do not deref HERE"; what it implies
one frame later is exactly the question this section is about.

Between them stands `EnumerateIter.next`, which returns
`Maybe<(Int, I.Item)>`. It is ONE function — `nm` on a binary that
instantiates it over BOTH iterators shows a single `_EnumerateIter.next`
and no monomorphised copies. Inside that body `self.iter.next()` is a
dynamic dispatch, so the convention of the inner producer is not a
static fact there; and the reference then leaves inside a TUPLE, which
the caller opens with `Unpack`. At the `Deref` that follows, the
register carries no facts at all — `VERUM_TRACE_DEREF` prints
`shared=false passthru=false genptr=false inline=false struct=false
objty=None list=false text=false` — while the same loop written without
`.enumerate()` reaches the same `Deref` with `passthru=true`.

Neither default is right. `Deref` currently loads, which is correct for
the `List` rung and faults on the slice rung; inverting it to identity
would swap exactly which rung fails. **The fact has to travel, and
through a generic adaptor it cannot travel statically.**

Nor can the producers simply be made to agree by inverting 0x0B. It
loads AT THE PRODUCER because only the producer knows the element
width: `emit_container_view` classifies the receiver into {cell,
stamped pack (elem 1), unstamped list (elem 8)} at runtime, and a
reference to a packed one-byte element is not an 8-byte-loadable
address. A scratch slot holding the widened value cannot live in the
producer's frame either — it would dangle the moment `SliceIter.next`
returns, which is the same alloca-escape hazard `lower_ref` documents
in place.

And the reason neither convention simply wins is ONE missing fact on
both sides. Loading at the PRODUCER needs the pointee width: 0x0B has it
from `emit_container_view`, 0x0D does not — `&*self.ptr` on a
`&unsafe T` inside a single generic `ListIter<T>` body has no `T`.
Loading at the CONSUMER needs the same width, and `Deref` does not have
it either. So the root is not "two conventions"; it is that **the AOT
erases the pointee width at a reference**, and each convention hides that
erasure somewhere different. That also predicts which half works today:
an eight-byte element needs no width at all, which is every `List<T>`
slot — hence the `List` rung surviving `.enumerate()` while the byte rung
faults.

So making every `&T` one representation requires a stable home for the
widened copy of a narrow packed element, and the only home that outlives
the return is the receiver — i.e. a change to what an iterator IS, not
to how a call site reads one. That is the campaign; until it lands, a
reference produced by 0x0B is correct only while it stays inside the
frame that produced it or crosses a boundary whose callee body the call
site can read.

### An attempt, and what it measured

Making the producers agree was tried and reverted (2026-09-12). A
`RefListElement` whose reference LEAVES the frame handed back the address,
exactly as `RefRawAddr` does, while in-frame sites kept the pre-load only
the producer's stride can justify. It closed three cells — `.enumerate()`
over a slice, `map` over a slice, and a write through `Slice.get_mut` —
and broke two:

| | before | after |
|---|---|---|
| `skip` over a slice | 11 | 0 |
| `take` over a slice | 101 | 0 |

Silently. `SkipIter.next` FORWARDS — it lifts the inner `next`'s payload
out of its `Maybe` and re-wraps it — so the call site asks
`wrapped_payload_is_slot_address` about `SkipIter.next`, whose payload is
produced by `GetVariantData`, and that answers "already a value": true
while the producer pre-loaded, false once it hands back an address.
`enumerate` survived only because it wraps the reference in a TUPLE the
caller `Unpack`s and `Deref`s.

**The forwarding case is the same wall one level up.** `SkipIter<I>` is
ONE function over every inner iterator, so "is the forwarded payload a
reference?" has no static answer there either, and marking every forwarded
`next` payload as an address breaks `SkipIter<Range>`, whose payload is an
`Int`. Agreeing per producer moves the disagreement; it does not remove
it.

So the conclusion above is stronger than it first read: not merely that
the fact cannot travel through an adaptor, but that **no call-site rule
can recover it at any depth**. The representation has to carry it.

**And that costs more than it first looks.** `FatRef` does reserve
`metadata:8` for facts of this kind, and slices do travel as `FatRef` —
but an ordinary `&T` does not. `lower_ref` has two arms, and the one that
builds a CBGR ref struct is DEAD: `enable_alloca_mode()` is called
unconditionally ("Fix: always enable alloca mode"), and that arm's own
comment says it *skips CBGR struct creation — store the raw pointer as
i64*. So at Tier 1 a reference is a bare i64, and there is no metadata
field to fill.

Making every `&T` a `FatRef` changes the calling convention for every
reference in the language — `lower_ref`, `Deref`, `DerefMut`, `ChkRef`,
`DropRef`, every FFI boundary that hands a reference to C, and the
alloca fast path whose whole purpose is that mem2reg/SROA promote it back
to SSA at no runtime cost (dropping that once forced the LLVM pipeline
down from `default<O2>`). Tagging the pointer instead is three bits for a
1/2/4/8 stride, but an interior pointer into a packed slice is not
8-byte aligned, so those bits are not free either. Both are campaigns,
not patches.

Reproduction: the two rungs above, `verum run --tier aot`. The chapter
that first showed it is `docs/by-example/19-file-io/main.vr`, whose
`BufRead.read_until` walks `available.iter().enumerate()`.


## Field references passed as arguments (T1537)

Native `RefField` lowering can retain a loaded field value for existing value
consumers while also carrying the original field address. The address has a
runtime slot for each possible register alias, initialized at function entry,
cleared on every value definition and copied by `Mov`. This keeps branch joins,
loop backedges and register reuse from selecting another field's address.
The alias set is computed by a finite worklist before instruction lowering.

Calls whose declared parameter is a scalar reference or an unsafe reference use
that original address. Record references retain their existing object-handle
ABI. Raw-pointer casts use the existing `ToRawPtr` opcode, so they consume the
same address provenance. A spill of the loaded value cannot implement this
contract: writes and atomic waits must refer to the original cell.

`field_reference_arguments` checks source-to-native mutation, aliases, branch
selection, loops, method arguments, by-value controls and the nonblocking futex
mismatch path. `raw_field_reference_cast` checks the interpreter side. This
change does not unify unresolved generic, aggregate-carried or iterator element
reference representations, and does not implement native ownership/drop glue.


## Exact native-call authority prerequisite (2026-10-04)

T1573 remains open. The measured native failure crosses
`Maybe.as_ref -> Maybe.expect -> OnceLock.get_or_init -> root_supervisor`.
`GetVariantDataRef` produces the original payload cell ADDRESS, the fresh
variant stores that word, and the generic projection returns it. The final
method receiver applies a record layout to the cell instead of its contents.
The stage6 debugger observed successful initialization (37, 37, count 1)
before this failure; it did not establish successful supervisor access.

An isolated prototype at `f947cdd68` composed typed variant projections and
ordinary forwarding bodies. Its CFG analysis kept all reachable normal exits,
treated mixed/cyclic results conservatively, and emitted temporary value views
without replacing the original cell. Cached facts were limited to summaries
and relevant sites; register states stayed local to analysis. This prototype
is **not integrated**: a source control disproved its native-call authority.

| Evidence | Measured result | Limit |
| --- | --- | --- |
| Main production with the new actual core-source record-forwarding test | Existing Once tests pass; new record test fails its missing-load IR oracle before unsafe execution | Confirms the scalar Once controls did not cover a record receiver |
| Prototype with the same core-source test | Record method returns 73 twice; existing Once tests pass | Source-to-LLVM/JIT evidence, not fresh full native supervisor acceptance |
| Prototype with a differently named generic sum | Record/field read 73, scalar read 73, value return 73, mutation changes the original cell to 91, reference-valued field reads 37 | No claim about unknown field-carrier inference or legacy `RefMut` |
| Read before versus after an aggregate mutation | Saved payload word retains the original cell address; later read returns the replacement object pointer | Summary invalidates future storage projections, not already-read words |
| Value, mixed-return and recursive controls | No newly inferred speculative slot load | Incomplete legacy paths are not thereby repaired |
| Actual native-selection negative | Fails: emitted `verum_generic_hash` result receives the prototype's slot load | Blocks production integration |

The last control declares an ordinary source function whose readable VBC body
returns a reference through forwarding:

```verum
fn hash_value(p: &Parcel<Cell>) -> &Cell { forward(p) }
fn intercepted_read(p: &Parcel<Cell>) -> Int { hash_value(p).value }
```

The native call path replaces that call with `verum_generic_hash`. The prototype
trusted the VBC return summary and inserted a load through the replacement's
scalar result. The negative inspects emitted LLVM and rejects that load before
executing it. T1580 separately tracks this existing wrong-callee selection:
an opaque receipt prevents the new speculative load but does not make the
replacement honor the source body. See the
[Intrinsic Dispatch Contract](intrinsic-dispatch-contract.md) for the declaration
and intrinsic-authority boundary. The control demonstrates that a readable body
and an exact declared FunctionId are insufficient authority for representation
analysis.

T1578 records the required next unit. The three ordinary emission paths
(`lower_call`, the `CallG` arm, and `lower_call_m`) need one shared record of
the **actual selected native call**. A receipt or common call plan must carry:

- The emitted LLVM function/body identity after declaration replacement,
  FunctionId resolution and name/arity deduplication. An intrinsic/runtime
  replacement without a source-body contract is explicitly opaque.
- Argument positions after static-receiver omission, original field-address
  substitution, scalar spills, header adjustment and ABI coercion. A parameter
  summary cannot substitute the earlier, unadapted VBC register word.
- The result representation after any returned-slot normalization. Variant
  extraction must use the same authority as the legacy payload peel and
  pass-through marks, so a loaded word cannot subsequently be classified as
  the original address.

A missing, replaced, ambiguous or cyclic receipt yields Unknown. For a covered
site this must not fall back to a VBC-body-only returned-slot walk or Maybe peel:
that would restore the rejected inference through a second path. Declared `&T`,
callee spelling and a recursion-visited flag cannot prove a slot address.
The eventual consumer must meet facts from every reachable normal return,
distinguish Unknown from an unreachable exit, and invalidate future aggregate
reads after possible mutation through an alias. A previously extracted word
keeps its own fact. Analysis work, symbolic expression size and retained facts
need explicit finite budgets; exhausting one yields Unknown, not a positive
representation claim.

Receipts produced during emission are not available for every forward callee
when its caller starts lowering. The consumer therefore needs deferred views
or a sound dependency/fixed-point phase after actual selection. Such a phase
must account for previously emitted normalization and classification marks;
adding final IR loads alone is insufficient. Duplicating all dispatch branches
inside the analysis would create a second call router and is not the next unit.
The first bounded implementation should establish and test this shared call
authority without enabling new speculative loads; normalization follows it.

No wire change, ownership/drop rule, name blacklist or production source change
was landed from the prototype. T1573 still requires both-tier source controls
and the fresh supervisor result in its acceptance. The separate T1578 gate
covers native replacements, same-name different arities, static receivers,
post-adaptation original-cell arguments, value results and conservative cycles.


### Native emission receipts (implementation boundary, 5 October 2026)

The first T1578 implementation unit records the actual emitted `Call`, `CallG`,
and exact `CallM` sites. It keeps the selected native body's source FunctionId,
a deterministic seal of its LLVM type, calling convention, attributes and body,
and compact instruction positions for arguments and normalized results. Runtime
replacement, missing bodies, incompatible arity or calling convention invalidate
that evidence. Final resolution rebuilds handles from the live sealed body;
removed instructions are not retained across runtime emission. Linkage-only
internalization does not change the seal.

An argument records register passthrough, conditional original-field-cell
selection, temporary scalar-cell materialization, or an unclassified adjustment.
Coercions and unhandled parameter ABI attributes cannot reuse an unchanged
register fact. Result bridges, numeric storage promotion and unhandled return
attributes remain opaque. The existing eager returned-slot normalization is
recorded separately from the raw native result.

Facts are bounded to 131,072 sites and 524,288 argument words per compilation,
with at most 1,024 arguments per site and 262,144 native instructions per sealed
body. Function-local capture has the same site/argument limits; exceeding a
budget removes evidence without changing execution. Printed IR and register
states are not retained. Final body validation is linear in the emitted bodies
and recorded sites, not in the product of callers and callee-body lengths.

Focused source-to-LLVM controls cover forward calls, generic calls, static
receiver omission, original field addresses, cycles as finite edges, runtime
replacement, ABI changes and numeric coercions. The different-arity collision
control deliberately remains an opaque negative: its malformed native dispatch
is separately tracked by T1598 and rejected by LLVM verification.

This unit enables no new reference loads and does not close T1573. The next
consumer must use these receipts for the finite CFG/aggregate projection
analysis and remove body-only fallback at covered sites, with the mutation,
unknown-exit and original-cell rules above. Existing native reference semantics
are not certified merely by a valid call receipt.
