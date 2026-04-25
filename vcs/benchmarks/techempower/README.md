# TechEmpower R23 benchmark rig — Verum / weft

Six canonical TechEmpower scenarios run against the pure-Verum
weft + warp stack. Closes `internal/specs/net-framework.md` §1.1
performance gates and §12 verification + benchmarks.

## Scenarios

| ID | Scenario | What it exercises |
|----|----------|-------------------|
| 1  | [Plaintext](./plaintext/) | Ceiling RPS — 16-byte body, no DB, no allocation. |
| 2  | [JSON](./json/) | Json serialisation hot path; one allocation per request. |
| 3  | [Single Query](./db_query/) | One indexed-row SELECT through `core.database.sqlite.native`. |
| 4  | [Multiple Queries](./db_queries/) | N parallel SELECTs (N parsed from query string), tests scheduler under load. |
| 5  | [Fortunes](./fortunes/) | Random-row SELECT + HTML template render — exercises text-build hot path. |
| 6  | [DB Updates](./db_updates/) | UPDATE-after-SELECT; tests transaction throughput. |

Each scenario is a real weft service that can be served by:

```bash
cd vcs/benchmarks/techempower
verum run --release --bin plaintext  &
./scripts/run_loadgen.sh plaintext 30s
```

## Acceptance gates

Per net-framework.md §1.1:

| Metric | Target | Reference |
|--------|--------|-----------|
| Plaintext RPS / core | ≥ 4M | actix-web 4.x ceiling on c5n.4xlarge |
| Plaintext RPS aggregate (56 cores, SO_REUSEPORT) | ≥ 20M | top-quartile R23 |
| p99.9 echo latency | < 200 µs | net-framework.md §1.1 |
| TLS 1.3 handshake p50 | < 2 ms with 0-RTT | net-framework.md §1.1 |
| Memory per idle conn | < 8 KiB | BEAM-equivalent |
| Graceful shutdown drop rate at 1M req/s | 0 | SO_REUSEPORT FD-pass |

Numbers are produced by `scripts/run_loadgen.sh` in CI under
`.github/workflows/techempower.yml`. Wrk2 with calibrated rate
control is the canonical load gen — coordinated-omission corrected.

## Repro

Local single-machine run (laptop-class — for sanity, not record):

```bash
cd vcs/benchmarks/techempower
./scripts/local_smoke.sh    # 30s @ 100 conn against each scenario
```

CI multi-machine run (real numbers):

```bash
gh workflow run techempower.yml --ref main
```

The CI matrix uses two boxes — `app` runs the verum binary,
`loadgen` runs wrk2. They share a 25 GbE link; numbers are what
you'd see on EC2 c5n.4xlarge × 2.

## Comparison baseline

The rig committed here is *infrastructure*. Numbers from the same
methodology against rustls/quiche + axum land in
`vcs/benchmarks/techempower/baselines.json` for direct comparison.
The CI workflow regenerates this file on every release tag so the
acceptance gates above are always against current peer-stack
performance.
