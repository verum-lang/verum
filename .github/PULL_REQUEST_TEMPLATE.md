# Pull Request

## Description

<!-- Provide a clear and concise description of your changes -->

## Type of Change

<!-- Check all that apply -->

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to change)
- [ ] Documentation update
- [ ] Performance improvement
- [ ] Code refactoring
- [ ] Test addition/improvement
- [ ] CI/CD improvement

## Related Issues

<!-- Link related issues using keywords like "Fixes #123" or "Relates to #456" -->

Fixes #
Relates to #

## Changes Made

<!-- List the main changes in your PR -->

-
-
-

## Testing

<!-- Describe the tests you ran and how to reproduce them -->

### Test Environment

- OS: <!-- e.g., Ubuntu 22.04, macOS 14, Windows 11 -->
- Rust version: <!-- e.g., 1.93.0 -->
- LLVM version: <!-- e.g., 21.1.0 -->
- Z3 version: <!-- e.g., 4.13.0 -->

### Tests Run

- [ ] All unit tests pass (`cargo test --workspace`)
- [ ] All integration tests pass (`cargo test --workspace --test '*'`)
- [ ] All documentation tests pass (`cargo test --workspace --doc`)
- [ ] Benchmarks run without regression
- [ ] Manual testing performed

### Test Output

```
<!-- Paste relevant test output here -->
```

## Semantic Type Compliance

<!-- For Rust code changes only -->

- [ ] Used `List<T>` instead of `Vec<T>`
- [ ] Used `Text` instead of `String`
- [ ] Used `Map<K,V>` instead of `HashMap<K,V>`
- [ ] Used `Set<T>` instead of `HashSet<T>`
- [ ] Used `Heap<T>` instead of `Box<T>`
- [ ] All dependencies explicit via `using [...]`
- [ ] No hidden state or magic
- [ ] Semantic type compliance verified with `scripts/verify_compliance.py`

## Code Quality Checklist

- [ ] Code follows the project's style guidelines
- [ ] Self-review of code performed
- [ ] Code is well-commented, especially in complex areas
- [ ] Documentation updated (if applicable)
- [ ] No new warnings introduced
- [ ] All clippy warnings addressed
- [ ] Code formatted with `cargo fmt`
- [ ] No unsafe code added without proper documentation
- [ ] All unsafe blocks have `// SAFETY:` comments

## Performance Impact

<!-- If applicable, describe the performance impact of your changes -->

- [ ] No performance impact expected
- [ ] Performance improvement (provide benchmarks)
- [ ] Potential performance regression (provide justification)

### Benchmark Results

```
<!-- Paste benchmark comparison here -->
```

## Breaking Changes

<!-- If this is a breaking change, describe the impact and migration path -->

- [ ] No breaking changes
- [ ] Breaking changes documented in CHANGELOG.md
- [ ] Migration guide provided

## Documentation

- [ ] README updated (if needed)
- [ ] API documentation updated
- [ ] CHANGELOG.md updated
- [ ] Examples updated (if needed)
- [ ] Migration guide provided (for breaking changes)

## CI/CD

- [ ] All CI checks pass
- [ ] Coverage threshold met (95%+)
- [ ] Security audit passes
- [ ] Cross-platform tests pass (Linux, macOS, Windows)
- [ ] No new dependency vulnerabilities

## VCS (Verum Compliance Suite) Checklist

<!-- Complete if your changes affect VCS or language semantics -->

### Test Level Status

- [ ] L0 Critical: 100% pass rate
- [ ] L1 Core: 100% pass rate
- [ ] L2 Standard: 95%+ pass rate
- [ ] L3 Extended: 90%+ pass rate (nightly)
- [ ] L4 Performance: Advisory only

### Additional VCS Checks

- [ ] Differential tests pass (Tier 0 == Tier 3)
- [ ] No performance regressions exceed 10% threshold
- [ ] Fuzzing did not discover new crashes
- [ ] Proof stability tests pass (if verification changes)

### VCS Test Changes

<!-- If you added/modified VCS tests, describe them -->

- [ ] New tests added to appropriate level (L0-L4)
- [ ] Tests follow VCS specification format (.vr files)
- [ ] Tests include appropriate directives (EXPECT, PARSE_ERROR, etc.)
- [ ] Edge cases covered

### VCS Workflow Runs

<!-- Link to relevant VCS workflow runs -->

- Tests: <!-- https://github.com/.../actions/runs/... -->
- Benchmarks: <!-- https://github.com/.../actions/runs/... -->
- Differential: <!-- https://github.com/.../actions/runs/... -->

## Additional Context

<!-- Add any other context about the PR here -->

## Screenshots/Videos

<!-- If applicable, add screenshots or videos to demonstrate changes -->

## Reviewer Notes

<!-- Any specific areas you'd like reviewers to focus on? -->

---

## Pre-Merge Checklist

Before merging, ensure:

- [ ] All required CI checks pass
- [ ] At least one approval from a maintainer
- [ ] No unresolved review comments
- [ ] Branch is up to date with main
- [ ] Commit messages follow conventional format
- [ ] No merge conflicts
