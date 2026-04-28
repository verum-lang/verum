# Production-Readiness Gates

Tracks #198. Each gate runner emits a green/red verdict + structured
JSON for the dashboard. `make production-readiness` (defined in
`vcs/Makefile`) runs all gates and fails the build on any red.

## Gates

| ID | Gate | Runner | Status |
|---|---|---|---|
| 1 | Tier-equivalence (#196) | `make diff-red-team` | partial — extend to full corpus |
| 2 | **Memory-safety** | `vcs/gates/memory_safety.sh` | **shipped** — zero skip/ignore/known-failure markers; currently GREEN |
| 3 | Soak (24h) | not shipped | needs CI infrastructure |
| 4 | Performance (±5%) | `cargo bench` baselines | not shipped |
| 5 | **Documentation** | `vcs/gates/documentation.sh` | **shipped** — every `public fn/type/axiom/theorem` has `///` doc; currently GREEN |
| 6 | **Diagnostic (#197)** | `vcs/gates/diagnostic.sh` | **shipped** — grep panic-without-context; currently RED |
| 7 | **Soundness obligation** | `vcs/gates/soundness.sh` | **shipped** — grep unsafe-without-SAFETY; currently RED |
| #176 | **AOT lenient-skip ratchet** | `vcs/gates/aot_lenient_skip.sh` | **shipped** — snapshot-vs-baseline comparator; needs CI snapshot pipeline |

## Running individual gates

```
bash vcs/gates/diagnostic.sh  # gate 6
bash vcs/gates/soundness.sh   # gate 7
```

Each gate emits:
- exit code 0 → green
- exit code 1 → red, with structured findings on stdout
- stderr → progress / status messages only

## Running all gates

```
make production-readiness
```

Fails on first red gate. Verbose mode emits all findings before
failing so the dashboard sees every gate's state.

## Adding a new gate

1. Add a script in this directory.
2. Update the table above.
3. Add a Make target in `vcs/Makefile` under
   `# Production-readiness gates`.
4. Add it to the `production-readiness` aggregator.
