# Example 01 — hello-world

## What it does

Prints `Hello, Verum!` to stdout and exits with status 0.

## What's new

- `fn main() { ... }` — program entry point. Every Verum executable
  has exactly one `main` in its top module.
- `print(text)` — built-in I/O. No `import` / `mount` needed; the
  prelude pulls it in.
- Single-line comments start with `//`.

## The code

```verum
fn main() {
    print("Hello, Verum!");
}
```

- `fn main()` — `fn` declares a function, `main` is the name, `()`
  is an empty parameter list. The implicit return type is `Unit`
  (the empty tuple).
- `print(...)` — call the built-in `print`. The argument is a
  string literal of type `Text`.
- `;` — terminates the statement.

The function body executes top-to-bottom. When it reaches the end,
the program exits with status 0.

## Things to try

1. Print on multiple lines:

   ```verum
   fn main() {
       print("Line 1");
       print("Line 2");
   }
   ```

2. Use an f-string to interpolate a value:

   ```verum
   fn main() {
       let name = "World";
       print(f"Hello, {name}!");
   }
   ```

3. Add a second function and call it from `main`:

   ```verum
   fn greet() {
       print("Hello, Verum!");
   }

   fn main() {
       greet();
   }
   ```

## Reference

- Function declarations: `grammar/verum.ebnf` §2 (`function_def`).
- Comments: `grammar/verum.ebnf` §1.
- I/O built-ins: `core/io/stdio.vr`.
- `Text` literals: `grammar/verum.ebnf` §1.5.2.
