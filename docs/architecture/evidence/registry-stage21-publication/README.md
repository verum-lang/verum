# Publication preparation and module boundaries

These controls use the captured ordinary CLI from `49559a042` and its automatic
standard-library bake. The [review](review.json) binds every case to exact
inputs and output, and retains the original failed receipts without rewriting
their verdicts. Component inputs whose temporary directories had been removed
are reconstructed from the pinned candidate and original assembly rules;
every byte matches the hashes captured during execution.

The public consuming `PreparedPublication.into_parts` API checks when its type
is explicitly mounted. Reusing the affine value is correctly refused with
E310 and the first/second call offsets. The diagnostic's displayed excerpt
points at the first call, so this record does not claim correct second-use
source highlighting.

Four negative controls incorrectly check successfully: reading the private
credential-store field, constructing that store through private fields,
constructing a prepared publication through private fields, and reading its
private publication field. T1713 tracks field visibility. The separate
private-state isolation runtime fixture passes, but deliberately appends a
test helper and cannot establish external privacy.

The credential-file fixture stops before runtime because a method on a
return-inferred public type is missing (T1711). Preparation has eight such
method errors plus a borrowed fixed-array versus List mismatch. Its runtime
and text-slicing behavior remain unmeasured. The bounds fixture also stops
before runtime: its mounted TextBuilder constructor resolves to a foreign
same-named type (T1212).

Two additional unchanged narrow-width endian fixtures pass. The website
receipt retains all five successful gates and the review of four rendered
pages at `73390a4`. These results do not establish general cryptography,
timing resistance, native AOT, bounded HTTP receive, durable publication or a
deployable registry. The authentication candidate remains separate from main.
