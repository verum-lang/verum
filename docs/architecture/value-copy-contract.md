# Value-Copy Contract

A value copy preserves the source and gives the destination ownership of its
owned components. The declaration determines which components are owned,
shared or borrowed; a pointer-shaped runtime value cannot establish that
boundary.

## Rules

1. **Owned components copy according to their declared types.** Records,
   variants, tuples and supported collections preserve nested ownership.
2. **Places copy, temporaries transfer.** A named value or an explicit
   dereference remains available after a supported copy. A fresh temporary
   needs no additional copy.
3. **Shared ownership stays shared.** Copying a `Shared<T>` handle retains the
   same allocation through its declared retain operation.
4. **References stay references.** Copying a borrow does not duplicate or
   prolong the lifetime of its referent.
5. **Permission precedes copying.** An affine or linear owner, an unresolved
   declaration or an unsupported carrier does not acquire copy permission
   from its runtime representation.

## Declaration authority

`CloneTyped` carries a `TypeRef` through bytecode serialization, linking and
specialization. Both execution tiers use the graph built by `copy_plan` from
that declaration. The graph retains reference qualifiers, record field types,
sum alternatives and owned container arguments. Its explicit work list permits
recursive declarations without recursive host traversal.

The canonical runtime container registry establishes which declaration owns
which backing storage. A foreign type with the same short name gains no
special treatment. The declared `Shared<T>` Clone attachment must identify a
complete body whose checked forwarding contract reaches `RetainCopy` with the
same source, counter and result. Function identity, signature, field layout
and bytecode are part of that check. Merely publishing a Clone name or a
forward declaration is insufficient. Ordinary value copying does not execute
an arbitrary user-defined Clone method.

The resource query and the copy plan answer different questions. The first
establishes whether a value may be duplicated; the second specifies how to
preserve its ownership. A successful resource query does not compensate for a
missing copy plan. Function captures, unresolved dynamic ownership and raw
pointer fields without a semantic declaration are not guessed from memory.

## Source boundaries

The following emitters carry declaration authority into `CloneTyped`:

| Position | Example | Emitter |
|----------|---------|---------|
| Binding | `let b = a` | `compile_let` |
| Assignment | `b = a` | `compile_assignment` |
| Record field and shorthand | `Holder { value }` | `compile_record` |
| Sum payload and named variant field | `Choice.Value(value)` | variant emitters |
| By-value `mut` parameter | `fn f(mut value: Holder)` | function prologue |
| Generic owned handoff | a place with a declared type parameter | value-use emitter |

These boundaries use the source's declared type for both named values and
explicit dereferences. Owned array elements follow their element declaration.
Affinely owned locals transfer their obligation instead of invoking the copy
operation. References preserve their referent, and fresh temporaries become
the destination directly.

This is an implementation boundary, not a claim that every copy producer has
been migrated. Runtime-intercepted container arguments, repeat initialization
and other remaining untyped `Clone` sites still require their own integration.
An explicit `.clone()` follows method selection and its declared contract.
Neither the typed operation nor an available source body proves the complete
caller/callee ownership ABI; immutable value parameters, destruction and mixed
archives remain separately tracked by T1807. Native released-owner checks
remain separately tracked by T2719.

## Storage operations

| Declared shape | Operation |
|----------------|-----------|
| Scalar and immutable text | Preserve the value. |
| Reference | Preserve the reference and its qualifiers; do not traverse it. |
| Record or tuple | Allocate independent storage and copy each owned field. |
| Sum | Copy the active declared payload into independent storage. |
| Fixed array | Copy exactly the declared number of elements. |
| Registered list or deque | Copy initialized elements and independent backing storage. |
| Registered map | Copy occupied keys and values into independent backing storage. |
| Registered shared handle | Create the declared handle and retain the same allocation. |

Repeated owned children are copied independently. A cycle through owned
storage is refused; a cycle through a shared handle or reference is not
traversed. Therefore this operation is not a universal graph snapshot.

## Failure and resource limits

Execution uses explicit work storage on ordinary thread stacks. Both tiers
use `CopyLimits` for work and allocated payload/backing bytes; the byte budget
excludes backend headers and is not a whole-process memory limit.

The transaction validates storage and prepares allocations before retaining
shared owners. It publishes the result only after all required retains
succeed. On failure it releases newly allocated storage and rolls back only
its own retain increments. Invalid storage, owned cycles, work/allocation
limits, allocation failure, refused retains and failed rollback have distinct
failure classifications. An unsupported shape never falls back to shallow
aliasing.

The native backend emits the operation and its bookkeeping in LLVM IR using
the target runtime. It does not call a project-authored C/C++ copying shim.

## Verification

- `nested_copy_boundaries` exercises actual source bindings, assignments,
  fields, sum payloads, explicit dereferences and mutable parameters across
  both source entry points and serialized VBC, including lexical destruction.
- `typed_value_copy` checks nested mutation isolation, borrowed fields,
  repeated children, arrays, wire remapping and refusal of unknown or affine
  ownership.
- `nested_value_copy` executes generated native images and the validated
  interpreter entry on equivalent source programs.
- `typed_copy_transactions` checks nested runtime operations, retain failures,
  rollback, budgets, cycles and deep owned chains on ordinary thread stacks.
- `typed_copy_wire`, `retain_forward` and `owned_storage_resources` pin
  declaration identity and rejection boundaries.

These gates establish the named producer and runtime paths. They do not
qualify a fresh full standard-library archive, configuration parsing or
Registry publishing; those require their actual source and execution paths.

## Forbidden shortcuts

- Infer ownership from a short type name, an object header or a raw address.
- Traverse a reference or arbitrary raw pointer as an owned component.
- Accept an absent declaration or an unproved retain body as a shallow copy.
- Copy a fresh temporary merely because it has a heap representation.
- Publish a partial result or leave increments from a failed transaction.
- Replace bounded work storage with growing or enlarged host stacks.

## 6. Block-tail cleanup demand (T1539)

A block first copies its result into a safe register so register recycling
cannot overwrite it. If a used tail names a local binding in that block,
its scope cleanup must not destroy the same value before the consumer can
use it. The exact source register identifies this handoff; neither a type
name nor a pointer-shaped value establishes ownership.

`ResultDemand` records whether enclosing syntax uses or discards the
result. Blocks, parentheses, unsafe blocks and value-producing branch arms
forward that demand. So do recover/finally result paths, scoped contexts,
select/nursery result arms, stage forwarding and the right branch of
coalescing. Conditions, call arguments and other operands remain used even
when their parent's result is discarded. Loop bodies, deferred/finally
cleanup, cancel handlers and expression statements discard their values.
A loop break value follows the loop result's demand.

This distinction is required at the producer: skipping every local-tail
`DropRef` would leak `{ let value = Watch { ... }; value };`. An ignored
result has no receiving binding, so its local retains ordinary cleanup.
The compiler does not issue an extra guessed `DropRef` on arbitrary
expression temporaries; in particular borrowed and raw aliases do not gain
an ownership obligation.

`crates/verum_vbc/tests/block_result_lifetime.rs` pins direct/nested return,
reverse cleanup order, explicit drop, used and discarded forwarding paths,
loop body/break values, deferred/finally/recover cleanup and borrowed/raw
negative controls. These are source-to-VBC/interpreter checks. Native owned
Drop remains T1538/T1540.

The bound is exact local-tail handoff. It does not prove arbitrary ownership
transfer through calls or aggregates, nor selection of an outer local by a
branch merge: `let a = ...; let b = ...; if flag { a } else { b }` needs a
shared lifecycle plan identifying which obligation moved. Implicit copies
of exclusive resources and general temporary-result destruction also
remain outside this rule. The demand flag is not a replacement for that
ownership model.
