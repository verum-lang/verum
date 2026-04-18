# Differential Triage (2026-04-18)

Initial per-spec root-cause investigation for the 13 differential
tier-divergence failures.

## Investigated

### `diff_match_expr.vr` — AOT crashes before first assert

- Interpreter: exits 0, prints "diff_match_expr: ok"
- AOT: exits -1, no output

Bisection shows simple `assert_eq(day_name(1), "Monday")` works in
AOT (isolated case). `fizzbuzz(15)` which mixes `if/else if/else` +
f-string interpolation inside branches exits -1 silently. Root cause
appears to be AOT codegen of string-returning if-else chains with
f-string in one branch — the `else { f"{n}" }` branch emits code
that produces a different Value shape than the string-literal
branches.

### Common pattern across all 13

Every failing spec mixes:
- String/Text return from match/if
- f-string interpolation
- assert_eq between computed and literal Text

AOT's Text equality and/or f-string formatting diverges from the
interpreter's in at least one codegen path. Fix requires:

1. Audit `AsBytes` / `Text` equality LLVM lowering for the
   small-string vs heap-string branches (`crates/verum_codegen/
   src/llvm/instruction.rs`).
2. Verify f-string format sequence produces the same Value shape
   as literal strings under subsequent pattern matching.
3. Normalize Text comparisons to go through a single opcode in
   both tiers (today interpreter uses specialized dispatch,
   AOT uses a different path).

Each spec needs individual LLVM-IR dump + diff against the
interpreter's VBC + matching runtime assertion sequence. Estimate:
~2 days of focused AOT codegen work, with test-driven iteration on
each of the 13 specs.

## Not in scope this cycle

This is the single biggest remaining production-readiness gap after
closing the KNOWN_ISSUES, stdlib API gaps, and static-mut persistence.
Tracked as the next focus area after current iteration lands.

The differential baseline remains at **24/37 = 64.9%** with a
documented list of what fails and why.
