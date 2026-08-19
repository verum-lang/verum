# Value-Copy Contract

Pinned architectural rules for the one question a language with value
semantics has to answer the same way everywhere: **what does it mean to
copy a value?**

A record in Verum is a value. `let b = a` gives `b` a record of its own;
`a` keeps its. Nothing in the surface language hints that these two names
might denote one object — and yet, until T0832, they did, because a
record lives in a heap object and a register holds the pointer to it.

The defect this document closes was never one bug. It was one *unstated*
contract implemented three times, each implementation wrong exactly where
the others were right.

---

## 0. TL;DR — the five rules

1. **A copy is one level deep.** The copy owns its own object and its own
   backing store; the values inside are copied bit-wise. That is the same
   ownership depth reading an element hands out.
2. **Places copy, temporaries do not.** A value that keeps living under
   its own name (a variable) is duplicated. A value nobody else can name
   (a literal, a call result, an arithmetic expression) is taken as is.
3. **`Shared<T>` copies by refcount bump**, never by forking the cell.
4. **References copy as references.** Copying an alias yields an alias;
   deep-copying the pointee would silently turn it into a snapshot.
5. **One implementation.** `value_copy` is the single carrier. Every
   spelling of "copy this" routes through it.

---

## 1. Where the copy happens

Seven syntactic positions duplicate a value. Each was measured against a
probe before and after; each now copies when — and only when — its source
is a place.

| Position | Example | Emitter |
|----------|---------|---------|
| Binding | `let b = a` | `compile_let` |
| Assignment | `b = a` | `compile_assignment` |
| Record field | `Mat3 { r0: z, r1: z, r2: z }` | `compile_record` |
| Record shorthand | `Point { x, y }` | `compile_record` |
| By-value `mut` parameter | `fn f(mut h: Holder)` | `compile_function` prologue |
| Container insert | `xs.push(item)` | `compile_method_call` |
| Repeat literal | `[slot; 256]` | `compile_array_repeat` |

Two of these deserve their reasons stated, because both were once
"fixed" in the wrong direction:

* **The parameter copies in the PROLOGUE, not at the call site.** The
  callee's declaration is what decides whether the argument is taken by
  value or by reference, and under dynamic dispatch the call site cannot
  see which body it will reach. Only `mut` parameters copy: an immutable
  binding can neither write through itself nor lend a `&mut`, so no
  program can observe whether it shares.
* **A container copies what it is given.** `push`/`insert`/`append`/
  `send` store their argument; the name that supplied it goes on living.
  These are runtime-intercepted and have no Verum body to carry a
  prologue, so the copy is made at the call site.

## 2. What a copy means, by shape

`value_copy` (`verum_vbc::interpreter::…::memory_collections`) is the
only place that answers this. Five shapes answer differently:

| Shape | Copy | Why not otherwise |
|-------|------|-------------------|
| Primitive | itself | nothing to share |
| `Text`, immutable values | itself | sharing what cannot be written is unobservable |
| `Shared<T>` | refcount bump, same carrier | forking gives the copy a private inner value; later writes are lost to the original (SHARED-CLONE-IDENTITY-1, T0107) |
| `List`/`Map`/`Set`/`Deque` | header slots + a fresh backing array | copying only the header leaves two containers writing into one store (T0499) |
| Record / variant / tuple | fresh object, slots copied bit-wise | the slot must be the copy's own |
| Untracked pointer (raw `alloc` buffer, CBGR cell) | itself | its bytes are not an `ObjectHeader`; reading one there fabricates an object out of the user's payload |

## 3. The three implementations that disagreed

Stated plainly, because the shape of this defect recurs:

* `Instruction::Clone` (Tier-0 opcode) — gave a record a fresh object,
  but copied a `List`'s three header slots and left the backing store
  shared.
* `.clone()` (method dispatch) — copied a container's spine correctly,
  and handed a **record straight back as an alias**.
* Tier-1 (`verum_codegen`) — `memcpy` when the allocation size was
  statically known, plain register copy otherwise. So `Shared<T>` was
  **forked**, `List` shared its backing, and anything of unknown size
  aliased outright.

Which spelling a program used decided whether a later write stayed
private. That is the signature of a contract with no owner.

## 4. Verification

* Unit: `value_copy_contract` (5 tests, inline — CI runs `--lib`) pins
  record separation, `Shared` identity + refcount, container backing,
  untracked-pointer passthrough, primitive identity.
* Spec: `vcs/specs/L0-critical/vbc/e2e/628_value_semantics_binding.vr`
  pins **both** sides — records copy AND references alias. The reference
  half is not decoration: a fix that copies indiscriminately passes the
  record half while turning every alias into a snapshot.
* Probe: seven forms, expected `0 1` / `0 2` / `0 3` / `3 4` / `0 5` /
  `0 7` / `4`.

## 4b. Tier coherence

The contract is one contract, so both tiers must answer alike. Tier-1
decides by the register's STATIC type and allocation size, which is the
same information the interpreter reads from the object header:

* `Shared<T>` returns the same carrier — a memcpy would fork the cell.
* A `List` object copies through the list-clone helper, and only where
  the size is statically known: that is the path a flat memcpy of the
  three header slots would otherwise have taken, sharing the backing
  store.
* Anything else with a known size gets `malloc` + `memcpy`.

Two facts have to survive for that to work, and neither did:

1. `New` recorded the allocation size BEFORE `set_register`, which
   clears it. The mechanism was inert from the day it landed.
2. `propagate_value_type_facts` — "ONE authority for copying every
   value-describing register fact" — carried the type name but not the
   size, so a binding's `Mov` dropped it anyway.

`VERUM_AOT_CLONE_COPY=0` restores the historic aliasing behaviour, and
`VERUM_TRACE_CLONE=1` prints what each lowering site knows about the
register it is copying. Both exist so a Tier-1 failure can be
ATTRIBUTED rather than guessed at.

## 5. Forbidden patterns

* A second implementation of "what does copying mean". Route through
  `value_copy`.
* Copying a value whose source is a temporary — that is cost with no
  observable effect.
* Copying a reference-typed binding's pointee. `reference_bindings`
  carries which names are references; consult it.
* Deep-copying through an untracked pointer.
