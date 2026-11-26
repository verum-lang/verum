# Testing publication requirements

The publication corpus compares Verum requirement parsing and candidate matching with the version of the Rust `semver` crate pinned in `Cargo.lock`. The extension corpus separately exercises whitespace conjunctions and `||`.

Run `python3 scripts/ci/check_semver_publication.py prepare --help` for the offline inputs. Use `--api explicit` to generate fixtures for `parse_cargo` and `parse_extended`. Preparation checks the cached crate against the lockfile and validates the expected answers with that crate.

Execute each generated Verum program with the compiler and tier under test. Retain its exit status and stdout. To compare one run against its plan, use

```sh
python3 scripts/ci/check_semver_publication.py compare \
  --plan /path/to/plan.json --fixture publication-matching \
  --stdout /path/to/stdout.log --result /path/to/comparison.json
```

The comparator checks coverage and reports missing or duplicate cases. Both the program and the comparison must succeed. Run the comparator's own controls with `python3 -m unittest discover -s scripts/ci/tests -p test_semver_publication.py`.
