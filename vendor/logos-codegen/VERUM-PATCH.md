# Logos code generation patch

This directory contains `logos-codegen` 0.16.1 from crates.io, under its
original MIT/Apache-2.0 licenses. The upstream crate checksum is
`58b3ffaa284e1350d017a57d04ada118c4583cf260c8fb01e0fe28a2e9cf8970`.

The changes in `src/generator/{mod,leaf,fork}.rs` move callback execution and
token emission out of individual DFA states into one shared terminal block,
and place immutable transition tables in static storage. A Rust `const`
array indexed at runtime can be materialized on the stack in an unoptimized
build; a `static` table is shared read-only storage in every build profile.
State transitions, offsets, match priorities, callbacks and trivia restart
semantics are unchanged. Both the DFA loop and trivia restart remain iterative.

With the upstream generator, Verum's unoptimized arm64 scanner allocates
`0x171780` bytes in a single native frame. Calling it from an interpolation
inside ordinary source exhausts a 2 MiB worker stack. Sharing the terminal
block removes the duplicated result temporaries instead of increasing stack
sizes or requiring compiler optimization for correctness. The resulting
frame on the same toolchain is `0xd240` bytes (from `0x171780`).

The Verum lexer conformance suite and ordinary-stack parser/Studio tests are
the consumer gates. Keep this patch until an upstream release provides the
same bounded allocation behavior, then remove the Cargo patch and this copy.
