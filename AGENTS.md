# Verum implementation rules

Do not use `stacker` or any stack-growing library, dynamic stack switching,
or increased thread stacks to repair compiler/interpreter recursion. Use
explicit bounded work queues or small, structurally bounded operations.
Validate nested inputs and rejection paths on ordinary thread stacks.

Implement language and standard-library primitives as VBC intrinsics. The
registry owns their identity; the interpreter executes their VBC operation and
the native backend lowers that operation to target instructions or LLVM IR.
Do not implement either tier by calling project-authored C/C++, compiling a
shim to an object or bitcode, or embedding it in Rust. A static executable
without libc is not evidence that this source boundary was respected.

Use the documented OS ABI at the platform boundary. Compiler-host bindings to
LLVM, LLD and TableGen, and independent benchmark baselines, do not implement
Verum operations. See [the intrinsic contract](docs/architecture/intrinsic-dispatch-contract.md)
and the [project instructions](CLAUDE.md).
