# Value-Copy Contract

A value copy preserves the source and gives the destination ownership of its
owned components. The declaration determines which components are owned,
shared or borrowed; a pointer-shaped runtime value cannot establish that
boundary.

## Rules

1. **Owned components copy according to their declared types.** Records,
   variants, tuples and supported collections preserve nested ownership.
2. **Places copy, temporaries transfer.** A named value (including `self`) or an explicit
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
| Eager by-value parameter | `fn f(value: Holder)` and `async fn` | selected call argument boundary |
| Projected parameter result | `fn f(value: Holder) -> Item { value.item }` | callable result boundary |
| Generic owned handoff | a place with a declared type parameter | value-use emitter |
| Checked implicit clone | a selected structural `value.clone()` | resolved-call emitter |

These boundaries use the source's declared type for both named values and
explicit dereferences. Owned array elements follow their element declaration.
Affinely owned locals transfer their obligation instead of invoking the copy
operation. References preserve their referent, and fresh temporaries become
the destination directly.

This is an implementation boundary, not a claim that every copy producer has
been migrated. Runtime-intercepted container arguments, repeat initialization
and other remaining untyped `Clone` sites still require their own integration.
An explicit `.clone()` follows method selection and its declared contract.
When no declared method applies, a structural clone admitted by the checker
carries an explicit value-copy selection into VBC. The source admission phase
retains this selection and receiver adjustments for every accepted module.
Admission follows the declared owned layout, including nested records, tuples,
sum payloads and fixed arrays. For a registered `List<T>` or `Deque<T>`, the
checker follows the element. For `Map<K, V>`, it follows both key and value.
A recursive declaration can close a cycle through the same instantiated
collection on its current path. Each sibling field and variant is still
checked. A bounded work list instantiates each nominal component and checks
its resource discipline. Borrowed fields stay borrowed;
affine payloads, mutable references, unknown captures and opaque storage do
not gain permission from the availability of the runtime copy operation.
Registered scalar storage is recognized through the exact builtin declaration,
never by a short type name.
Eager source functions, including `async fn`, with complete parameter declarations receive an
owned value at the selected call boundary. A copyable named or projected place
is copied through `CloneTyped`; an affine named place transfers its obligation;
a fresh temporary is received directly. References and raw addresses never
transfer their referent. The callee inventories immutable and mutable value
parameters alike, cleans them on normal and explicit returns, and preserves a
parameter transferred as its result. A returned field or element is copied
before its enclosing parameter is cleaned. Generic handoffs and cleanup use
the selected declaration's concrete witnesses.
An instance method reached through `CallM` receives its declared by-value
arguments under the same convention as a direct call. This includes source
helpers on collections whose other operations have runtime implementations.
The async callable allowance does not change this lifecycle. A synchronous
producer may satisfy an async-permitting callback contract; nested callable
parameters are contravariant and results covariant. Context requirements and
non-callable component identities remain exact. Both execution tiers use this
same compatibility check independently of environment ownership evidence.
Raw pointer aliases still require a complete source refinement target; the
calling-convention marker cannot manufacture that missing admission witness.

An owned receiver follows the same binding rules in local initializers, calls
and aggregate fields. Borrowed receivers retain their reference declaration.
Local nominal owners carry that declaration into `DropTyped` even without an
explicit destructor, so structural cleanup visits the active sum payload and
instantiated owned fields instead of relying on erased record fields.

VBC minor 60 carries this calling convention in function flag bit 4. Only a
completed body whose parameter inventory and cleanup were emitted receives the
flag. Forward declarations cannot issue it; body imports and specialization
carry it with the executable body, while declaration-only imports clear it.
The actual VM frame checks caller/callee compatibility once before execution;
TailCall checks before changing the frame. Mixed conventions are refused.
Native direct calls check their selected descriptors. A native empty-environment
callable also checks its selected producer, argument count and calling
convention when that identity survives local moves and immediate writes in one
basic block. Register overwrites, control-flow joins and unknown operations
discard this producer fact. Erased native indirect calls remain refused when
an owned-body target may occur. The flag declares an ABI, not an independent
ownership proof.

VBC minor 61 also carries an explicit closure environment convention and capture
count. Its native environment parameter has no source parameter type; the retained
semantic signature contains only the declared user arguments. Both execution
tiers check formation against the selected descriptor's capture count. The native
prologue and callable adapter use this metadata, never a generated name or the
unrelated stack-size field. Changing the convention invalidates a body-use receipt.

Callable copy and cleanup share an empty-environment producer check. The VM
accepts a known function ID only with its actual semantic signature, or a
registered empty environment with its exact live allocation and lexical
witnesses. The copy plan retains a callable check at every owned payload;
it does not turn a function type into unconditional identity-copy permission.
Native local copies also require the exact selected empty-environment producer
and semantic signature. Native aggregate copies containing callables remain
refused because their payloads do not yet retain this producer evidence.
A captured environment without an owned lifecycle is refused in either tier.
Native callable cleanup emits an executable refusal until it has an environment
ownership receipt. These checks
do not complete the closure lifecycle or native released-owner checks, which
remain separately tracked by T1807 and T2719. Taking a reference to a by-value
parameter is also a frontend lifetime question: the accepted parameter-store
case tracked by T2785 is not repaired by runtime cleanup.

## Storage operations

| Declared shape | Operation |
|----------------|-----------|
| Scalar or immediate Text codec | Preserve the carrier without allocating owned storage. |
| Text record | Allocate an independent record and byte backing for the initialized prefix; spare reservation is not copied. |
| Reference | Preserve the reference and its qualifiers; do not traverse it. |
| Callable | Check its actual empty-environment producer and signature before preserving the carrier. |
| Record or tuple | Allocate independent storage and copy each owned field. |
| Sum | Copy the active declared payload into independent storage. |
| Fixed array | Copy exactly the declared number of elements. |
| Registered list or deque | Copy initialized elements and independent backing storage. |
| Registered map | Copy occupied keys and values into independent backing storage. |
| Registered shared handle | Create the declared handle and retain the same allocation. |

Repeated owned children are copied independently. A cycle through owned
storage is refused; a cycle through a shared handle or reference is not
traversed. Therefore this operation is not a universal graph snapshot.

Text storage classification does not prove identity copying or trivial cleanup.
The VM checks live managed or allocator-backed byte extents. Native Text copy
requires a live flat record and live source byte range from its producer; its
length, capacity, null and overflow checks do not validate an arbitrary forged
pointer. Neither tier infers an allocator or exclusive ownership from capacity.
A physically encoded List with a declared `Drop` calls that source method with
its concrete element witness. Both tiers validate the initialized prefix.
Native execution checks the descriptor's allocation receipt before calling the
method and releases the descriptor afterward. A standalone module can also
instantiate the built-in `List<T>` without the source descriptor. Its
instantiated element type determines cleanup of the initialized prefix in
reverse order. Missing or unresolved element types are rejected.

Text producers issue a storage receipt separately from length and capacity.
Typed destruction releases the backing allocation and descriptor through their
recorded allocation domains; borrowed or pooled bytes are not freed. Replacing
an owned descriptor validates both receipts before releasing the old backing.
The source compiler emits cleanup for owned locals, parameters, aggregate
components and observed temporaries. Native lowering does not infer cleanup at
return or register overwrite from a table shared across control-flow branches.

These checks cover the named source and runtime paths below. They do not prove
complete reclamation for every container, closure, exceptional exit or discarded
expression. In particular, source cleanup for semantic containers still has
paths using `DropRef` instead of a fully instantiated element destruction plan.

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
- `parameter_ownership`, `parameter_callable_ownership` and `parameter_convention`
  retain the original call controls and check producer/signature refusal,
  mixed-generation calls, live-object preservation, and the versioned seal.
- Compiler `parameter_ownership_checked` exercises checker-selected methods;
  codegen `parameter_convention` checks native convention and callable refusals.
- `typed_copy_wire`, `retain_forward` and `owned_storage_resources` pin
  declaration identity and rejection boundaries.
- `text_storage_lifecycle` checks producer receipts, allocation domains and
  invalid-storage refusal. Compiler `text_source_lifecycle` executes selected
  unchanged standard-library bodies through serialized VBC and native lowering.
- `aggregate_storage_lifecycle`, `formatting_storage_lifecycle` and
  `value_replacement_lifecycle` repeat source and wire execution while checking
  that retired owners do not accumulate in the interpreter heap.

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
