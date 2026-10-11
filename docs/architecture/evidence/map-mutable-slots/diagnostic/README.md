# Mutable Map instruction diagnostic

The unchanged Map consumer at `abfb5fc88` still fails with exit 1 after
86.079 seconds. It prints only `map-read: ok`; the original `map-mutate: ok`
success oracle remains unmet. The [review](review.json) binds the original
source, executable, build receipts and logs. Stage23 artifacts are reused;
no new standard-library bake is represented by this run.

The scoped bytecode dump and instruction trace select the production
`Map.get_mut` body. At byte offset 89 it reads `self.entries` into register
16. Offsets 93–100 calculate a slot address with stride 32, offset 104
applies `Spec`, and offsets 108–111 move the result into register 20.
The last executed instruction, `GetF` at offset 114, reads field 2 from
that register. The backtrace reports 118, the cursor after its operands.
The existing guard rejects a zero-size `Unit` object.

The runtime constructor, insertion and owned lookup use two `Value` words
per entry, while the selected source method expects four-field
`Slot { key, value, hash, psl }` storage. This trace confirms the selected
source path; it does not justify relaxing pointer or allocation guards.
Only registers 1–3 have their values printed by this trace facility, so
the dump is not a complete record of the failing base pointer's provenance.

T1196 retains its broader mutable-entry acceptance requirements, including
`entry`/`or_insert_with` and both execution tiers. T1245 tracks consolidation
of the two representations. This diagnostic establishes neither fix.
