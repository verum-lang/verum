# Verum Release Policy

This document is the contract between the Verum project and its users. It
defines which surfaces are stable, how versions are bumped, when
deprecations take effect, and which release lines receive long-term
support. Every release ships under these terms.

Spec: tracked under task #183.

---

## 1. Versioning — Semver 2.0.0

Verum uses [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html)
on **stable surfaces**. The version number `MAJOR.MINOR.PATCH` is bumped
according to the change made:

| Change                                                  | Bump      |
| ------------------------------------------------------- | --------- |
| Backwards-incompatible change to a stable surface       | **MAJOR** |
| Backwards-compatible feature added to a stable surface  | **MINOR** |
| Backwards-compatible fix to a stable surface            | **PATCH** |
| Internal refactor, performance work, doc edits          | (no bump) |

Pre-1.0 releases are exempt from the major-version stability guarantee:
`0.x → 0.y` may include breaking changes per [semver §4](https://semver.org/spec/v2.0.0.html#spec-item-4).
Once Verum hits 1.0.0, the rules above apply unconditionally.

### 1.1 Stable surfaces

The following surfaces are part of the stability contract. Breaking
changes require a MAJOR bump.

- **Language grammar** — `grammar/verum.ebnf`. Productions present in
  the EBNF as of a release are guaranteed parseable in subsequent
  same-major releases.
- **Public stdlib API** — every `public` symbol in `core/`. Removal,
  signature change, or semantic change of a public stdlib symbol is
  breaking.
- **VBC bytecode format** — `verum_vbc` instruction encoding,
  metadata layout, and module file format. Bytecode produced by
  version `N.x.y` must be loadable by every `N.*.*` runtime. Tracked
  under #175 (bytecode versioning + migration path).
- **CLI subcommand surface** — `verum build`, `verum run`, `verum
  test`, `verum check`, `verum playbook`, etc. Subcommand names,
  flag names, and exit-code semantics. New flags / subcommands are
  MINOR; removals are MAJOR.
- **LSP protocol** — JSON-RPC method names, parameter shapes, and
  semantic tokens emitted to clients. `lsp/` request/response types
  are stable.
- **Verification proof terms** — kernel rule names (`KernelRule`
  enum) and certificate format. A proof valid under version `N` must
  remain valid under every `N+x` release; removing a kernel rule is
  breaking.

### 1.2 Internal surfaces (NOT stable)

Internal compiler crates (`verum_compiler`, `verum_vbc`,
`verum_types`, `verum_codegen`, `verum_smt`, etc.) bump with the
project version, but their internal Rust APIs are NOT stable.
Re-exports through the `verum` umbrella crate (when one exists) are
stable per §1.1.

This means:
- A `verum_vbc::codegen::*` API change does **not** require a MAJOR bump
  if no public Verum-language behaviour observably changes.
- A change to compiler-internal data structures, intermediate
  representations, or query plans is treated as patch-level work.
- Cargo workspace consumers depending on internal crates do so at
  their own risk.

### 1.3 Experimental surfaces

Modules and features explicitly marked `@experimental` may break
between MINOR releases. Examples:
- `core.theory_interop`
- `core.cognitive` (when it ships)
- Anything under `core/experimental/` (when it exists)

Experimental status is documented in the module-level doc comment
and ships behind `@cfg(feature = "experimental_<name>")`.

---

## 2. Deprecation Policy

When a stable-surface symbol must be removed, Verum follows a
**two-cycle deprecation window**:

| Phase   | Version    | Behaviour                                                  |
| ------- | ---------- | ---------------------------------------------------------- |
| Notice  | `N.x`      | `@deprecated("reason; replacement = …")` warning at use    |
| Grace   | `N+1.x`    | Warning continues; replacement is the documented path      |
| Removal | `N+2.0`    | Removal lands in the next MAJOR after Notice               |

The minimum window is **two minor releases** between deprecation notice
and removal. This gives users at least one full release cycle to
migrate.

### 2.1 Deprecation attribute

Every deprecation MUST be machine-readable:

```verum
@deprecated(
    since = "0.5",
    reason = "use Foo.new_v2() — same behaviour with capability checks",
    remove_in = "0.7",
)
public fn old_api(...) { ... }
```

The compiler emits a warning at every use site; `verum check
--deny-warnings` upgrades it to an error so CI catches it.

### 2.2 Pre-1.0 exception

Until Verum ships 1.0.0, deprecation windows may be shortened to one
minor cycle (`N.x` notice → `N+1.0` removal) to keep iteration speed
high. The two-cycle minimum re-applies once 1.0.0 ships.

---

## 3. LTS — Long-Term Support

Verum maintains **one LTS line at any time**. The current LTS line
gets:

- **Security patches** — every CVE-class defect (memory safety,
  capability bypass, cryptographic weakness, hostile-input DoS).
- **Correctness patches** — every soundness defect in the type
  system, kernel, or stdlib that was present in the LTS version.
- **Build / toolchain compat patches** — fixes needed to keep the LTS
  line buildable on supported platforms (current cargo, current
  LLVM, current Z3).

LTS lines do **NOT** get:
- New features.
- Performance optimisations (unless they are also a correctness fix).
- Breaking changes of any kind.

### 3.1 LTS support window

Each LTS line is supported for **18 months** from its initial
release. After 18 months the line moves to "best-effort
community-maintained" status — patches are accepted but not
guaranteed.

### 3.2 LTS cadence

A new LTS line is designated approximately every **12 months**, so at
any given time there are typically 1-2 lines under official support
(the current LTS, plus the previous LTS in its final 6 months of
overlap).

### 3.3 LTS designation

A release line is designated LTS via the `RELEASES.md` table
maintained at the head of `main`:

| Line | Initial release | LTS until    | Status        |
| ---- | --------------- | ------------ | ------------- |
| 0.1  | 2026-XX-XX      | 2027-XX-XX   | LTS (current) |

(Table populated as releases ship.)

### 3.4 Pre-1.0 exception

Until 1.0.0 ships, LTS commitments are aspirational. The first LTS
line will be 1.0; pre-1.0 lines get best-effort support for security
and severe-correctness defects only.

---

## 4. Backward-compatibility CI Gates

To make the contract above enforceable rather than aspirational, the
project maintains automated gates:

### 4.1 Bytecode forward-compat (depends on #175)

A `vbc-forward-compat` CI job:
1. Compiles a fixed corpus of `.vr` programs to VBC under each shipped
   version since the current MAJOR's first release.
2. Loads each archived bytecode artifact under the head-of-`main`
   runtime and runs the corpus.
3. Fails CI if any archived bytecode fails to load or produces
   semantically different output.

### 4.2 Stdlib API stability

A `stdlib-api-stability` CI job runs a tool equivalent to
`cargo-semver-checks` against the Verum stdlib:
1. Diff the public-surface set between the head-of-`main` `core/` and
   the most recent release.
2. Categorise each diff as Breaking / Minor / Patch.
3. Fail CI if a Breaking diff lands without a corresponding MAJOR bump
   in `Cargo.toml` / `verum.toml`.

### 4.3 Grammar stability

A `grammar-forward-compat` CI job parses a corpus of every `.vr` file
in the workspace under both head-of-`main` and the most recent shipped
parser. Any file that parses cleanly under the shipped parser but
fails under head-of-`main` indicates a breaking grammar change.

---

## 5. Migration Tooling

### 5.1 `verum migrate`

Mechanical breaking changes ship with a migration script:

```bash
verum migrate --from 0.5 --to 0.6 ./src
```

The migrator reads a per-version `migrations.toml` produced by the
release that introduced the breaking change. Each migration is a
matched pair of (parse-pattern, rewrite). Pure rename / signature
shift / module-move migrations are mechanical; semantic changes that
require human judgment are reported with file:line:col plus the
documented manual fix.

### 5.2 Migration notes

Every release with a breaking change ships:
- A "Breaking changes" section in the changelog with a 1-paragraph
  explanation per change.
- A migrations.toml entry for every mechanical migration.
- Reference documentation at `docs/migrations/<from>-<to>.md` for
  semantic migrations the tool can't automate.

---

## 6. Release Cadence

| Track    | Cadence              | Purpose                                       |
| -------- | -------------------- | --------------------------------------------- |
| `main`   | Continuous           | Active development; no stability guarantees   |
| Patch    | As needed            | Correctness / security fixes for active lines |
| Minor    | ~6 weeks (target)    | Feature additions (semver-compatible)         |
| Major    | ~12 months (target)  | Breaking changes, accumulated cleanup         |
| LTS      | ~12 months (target)  | Designated stable line per §3                 |

Cadences are **targets**, not guarantees — releases ship when the
release-criteria checklist (CI green, docs updated, changelog
written, deprecations documented) is satisfied.

---

## 7. Release Branches

| Branch              | Purpose                                         |
| ------------------- | ----------------------------------------------- |
| `main`              | Next major's active development                 |
| `release-v0.1`      | Patches to the 0.1 line (until end-of-life)     |
| `release-v0.2`      | Patches to the 0.2 line                         |
| `release-v0.X`      | One per supported / LTS line                    |

Patches land on `release-v0.X` first, then forward-port to `main` if
still applicable. Every patch fix on a release branch ships under a
PATCH bump (`0.X.Y → 0.X.Y+1`).

---

## 8. Acknowledging Breaking Changes

When a PR introduces a breaking change to a stable surface, it MUST:
1. Update `CHANGELOG.md` with a "Breaking" line under the next-major
   section.
2. Update `RELEASES.md` (this file) if the contract itself changes.
3. Add a deprecation entry on the old API per §2 (deprecation must
   precede removal — direct removal without prior deprecation is
   reserved for security defects).
4. Add an entry to the migration tool per §5 if the change is
   mechanically rewritable.

CI rejects PRs touching stable surfaces without these updates.

---

## 9. Open Items

This file is the **policy** ; the **infrastructure** that enforces it
lands across:

- **#175** — VBC bytecode versioning + migration path (prerequisite
  for §1.1 bytecode stability and §4.1 forward-compat CI).
- **#176** — Drive full-corpus AOT lenient-skip count to zero (so
  releases ship without silent codegen drops).
- **#198** — Production-readiness gates: regression-detection +
  soak-test discipline (§4 CI gates require this baseline).

When all three land, this policy becomes a hard contract instead of
an aspirational one.
