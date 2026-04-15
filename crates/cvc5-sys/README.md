# cvc5-sys

Low-level FFI bindings for the [CVC5](https://cvc5.github.io/) SMT solver,
with support for **static linking** into Rust binaries.

This crate is part of the [Verum language platform](https://github.com/verum-lang/verum)
and provides the foundation for Verum's complementary Z3 + CVC5 SMT architecture.

## Quick Start

```toml
[dependencies]
cvc5-sys = { version = "0.1", features = ["vendored"] }
```

Then rebuild your project. First build takes 3–5 minutes (CVC5 is compiled
from source). Subsequent builds use cached artifacts.

## Build Modes

| Feature      | Behavior                                        | Use Case               |
|--------------|-------------------------------------------------|-----------------------|
| `vendored`   | Build CVC5 from source, static link             | Distribution, CI       |
| `static`     | Alias for `vendored`                            | —                      |
| `system`     | Link against system-installed libcvc5           | Fast dev builds        |
| *(none)*     | Stub bindings (functions return null)           | Docs, portable builds  |
| `regen-bindings` | Regenerate `bindings.rs` from `cvc5.h`       | Maintenance            |
| `gpl`        | Enable GPL-licensed components (CLN, CryptoMiniSat) | Internal testing only  |

## Vendored Build Requirements

- **CMake** ≥ 3.16
- **C++17-capable compiler**: GCC ≥ 9, Clang ≥ 10, or MSVC 2019+
- **GMP** (≥ 6.2): `apt install libgmp-dev` / `brew install gmp`
- **Python 3** (for CVC5's build scripts)
- **~4 GB disk space** for build artifacts
- **3–5 minutes** for the first build

## Environment Variables

- `CVC5_ROOT`: Path to a pre-built CVC5 installation (overrides all features).
- `CVC5_NO_VENDOR`: Disable vendored builds even when the feature is enabled.
- `CVC5_JOBS`: Parallel build jobs (default: CPU count).
- `GMP_STATIC_LIB`: Path to `libgmp.a` for fully static linking.

## Setting Up the Vendored Source

The CVC5 source must be present at `crates/cvc5-sys/cvc5/` as a git submodule:

```bash
cd path/to/verum
git submodule add https://github.com/cvc5/cvc5.git crates/cvc5-sys/cvc5
cd crates/cvc5-sys/cvc5
git checkout cvc5-1.2.0  # pinned version
```

## Licensing

- **cvc5-sys (this crate)**: Apache-2.0
- **CVC5 itself**: 3-clause BSD license
- **GMP**: LGPL (dynamically linked by default; statically linked when
  `GMP_STATIC_LIB` is set)
- **Optional `gpl` feature** pulls in CLN/CryptoMiniSat, which are GPL. Do
  not enable this feature for Apache-2.0 distributions.

## Safety

All FFI functions are `unsafe`. Use [`verum_smt::Cvc5Backend`] for a safe,
idiomatic wrapper. CVC5 is not thread-safe; each solver instance must be
used from a single thread.

## Runtime Availability

Even when CVC5 is not linked (stub mode), this crate compiles successfully
and `cvc5_sys::init()` returns `false`. This lets downstream code use
feature detection at runtime:

```rust
if cvc5_sys::init() {
    // Use CVC5
} else {
    // Fall back to Z3
}
```

## See Also

- [CVC5 documentation](https://cvc5.github.io/doc/)
- [CVC5 C API reference](https://cvc5.github.io/doc/cvc5-main/c/cvc5__c_8h.html)
- [SMT-LIB standard](https://smtlib.cs.uiowa.edu/)
