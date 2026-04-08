# Differential Testing Scripts

Automation scripts for running differential tests and comparing outputs.

## Scripts

### run_differential.sh

Main script for running differential tests between Tier 0 (interpreter) and Tier 3 (AOT).

```bash
# Run all tier-oracle tests
./run_differential.sh tier-oracle/

# Run specific test with verbose output
./run_differential.sh -v tier-oracle/arithmetic_oracle.vr

# Run with 8 parallel jobs
./run_differential.sh -j 8 tier-oracle/

# Only show failed tests
./run_differential.sh --only-failed tier-oracle/

# Generate markdown report
./run_differential.sh -f markdown tier-oracle/

# Save all outputs for debugging
./run_differential.sh --save-outputs tier-oracle/

# Use semantic comparison
./run_differential.sh --compare-py tier-oracle/
```

#### Options

| Option | Description |
|--------|-------------|
| `-v, --verbose` | Show detailed output |
| `-q, --quiet` | Minimal output |
| `-j, --jobs N` | Number of parallel jobs (default: CPU count) |
| `-t, --timeout N` | Timeout per test in seconds (default: 30) |
| `-o, --output DIR` | Output directory for reports (default: ./reports) |
| `-f, --format FMT` | Report format: text, json, markdown |
| `--tier0 PATH` | Path to interpreter binary |
| `--tier3 PATH` | Path to AOT binary |
| `--only-failed` | Only show failed tests |
| `--save-outputs` | Save stdout/stderr to files |
| `--compare-py` | Use Python semantic comparison |

#### Environment Variables

| Variable | Description |
|----------|-------------|
| `VERUM_INTERPRET` | Path to interpreter binary |
| `VERUM_RUN` | Path to AOT runner binary |

### compare_outputs.py

Python script for semantic comparison of tier outputs. Handles acceptable differences like:
- Float precision variations
- Collection ordering differences
- Memory addresses
- Timestamps
- Whitespace

```bash
# Basic comparison
./compare_outputs.py tier0.txt tier3.txt

# With relaxed float tolerance
./compare_outputs.py tier0.txt tier3.txt --float-epsilon 1e-6

# Allow unordered collections
./compare_outputs.py tier0.txt tier3.txt --allow-unordered

# Verbose output with diff
./compare_outputs.py tier0.txt tier3.txt -v --diff

# JSON output for scripting
./compare_outputs.py tier0.txt tier3.txt --json
```

#### Options

| Option | Description |
|--------|-------------|
| `--float-epsilon N` | Float tolerance (default: 1e-10) |
| `--allow-unordered` | Allow unordered collection comparison |
| `--normalize-ws` | Normalize whitespace |
| `--strip-ansi` | Strip ANSI codes (default: true) |
| `-v, --verbose` | Show detailed comparison |
| `--json` | Output as JSON |
| `--diff` | Show unified diff |

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Outputs are semantically equivalent |
| 1 | Outputs differ |
| 2 | Error during comparison |

## Integration with CI

Example GitHub Actions workflow:

```yaml
name: Differential Tests

on:
  push:
    branches: [main]
  pull_request:

jobs:
  differential:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build Verum
        run: cargo build --release

      - name: Run differential tests
        run: |
          ./vcs/differential/scripts/run_differential.sh \
            --tier0 ./target/release/verum-interpret \
            --tier3 ./target/release/verum-run \
            --format markdown \
            --output ./differential-report \
            vcs/differential/tier-oracle/

      - name: Upload report
        uses: actions/upload-artifact@v3
        with:
          name: differential-report
          path: ./differential-report/
```

## Development

### Adding New Tests

1. Create `.vr` file in appropriate directory
2. Add test annotations:
   ```verum
   // @test: differential
   // @tier: 0, 3
   // @level: L1
   // @tags: your, tags
   ```
3. Run with verbose mode to verify

### Debugging Failures

1. Run with `--save-outputs` to capture raw output
2. Use `compare_outputs.py --verbose --diff` for details
3. Check float precision with `--float-epsilon`
4. Verify test is deterministic (no random values)

### Performance Testing

To measure speedup between tiers:

```bash
./run_differential.sh -v tier-oracle/ | grep "T0:" | \
  awk -F'[,:]' '{sum_t0+=$3; sum_t3+=$5; count++}
  END {print "Avg T0:", sum_t0/count, "ms, Avg T3:", sum_t3/count, "ms, Speedup:", sum_t0/sum_t3, "x"}'
```
