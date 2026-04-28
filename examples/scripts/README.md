# Verum example scripts

Each `.vr` file in this directory is a runnable, single-file Verum
script. Mark them executable (`chmod +x foo.vr`) and they run
directly via the kernel-level shebang dispatch on Unix; on every
platform they also work as `verum foo.vr` or `verum run foo.vr`.

| Script | What it shows |
|---|---|
| `hello.vr` | Minimal shebang invocation, top-level statement, void-tail exit code 0 |
| `exit_code.vr` | Tail-Int expression becomes the process exit code |
| `with_args.vr` | Explicit `fn main(args: List<Text>) -> Int` for command-line tools |
| `refinement.vr` | Refinement-typed `Port` validated at compile time inside a script |
| `decl_and_stmts.vr` | Mixing `fn` declarations with top-level statements; source-order preservation |
| `with_frontmatter.vr` | PEP-723-style `// /// script` metadata block (compiler-version pin) |

## Running

```bash
# Direct shebang exec (Unix)
chmod +x hello.vr
./hello.vr

# Bare invocation (any platform)
verum hello.vr

# Explicit form
verum run hello.vr

# With arguments
./with_args.vr Alice

# Inline expression (no file needed)
verum run -e '1 + 2'

# Stdin
echo 'print("piped");' | verum run -
```

## Caching

The first run of each script writes a compressed VBC artefact to
`~/.verum/script-cache/`. Subsequent runs of the same source skip the
front-end (parse, typecheck, verify, codegen) and hit the cache for
sub-second cold starts.
