# Verum

**A memory-safe systems programming language with semantic types, generation-based references, structured concurrency, and compile-time verification.**

Verum combines the performance of systems languages with the safety of managed languages. Its type system uses meaningful names (List, Text, Map) instead of implementation-oriented names (Vec, String, HashMap). Memory safety is achieved through CBGR (Counter-Based Generation References) — a three-tier reference system that provides configurable safety/performance tradeoffs. The context system provides first-class dependency injection. Refinement types enable compile-time verification of invariants via SMT solving.

The compiler is written in Rust. The standard library (289 modules) is written in Verum itself.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Key Features](#2-key-features)
3. [Syntax Quick Reference](#3-syntax-quick-reference)
4. [Standard Library](#4-standard-library-core)
5. [Memory Safety Model (CBGR)](#5-memory-safety-model-cbgr)
6. [Compilation Pipeline](#6-compilation-pipeline)
7. [Building](#7-building)
8. [Testing](#8-testing)
9. [Project Structure](#9-project-structure)
10. [Reserved Keywords](#10-reserved-keywords)
11. [Built-in Functions](#11-built-in-functions)

---

## 1. Overview

Verum is a statically typed, compiled systems programming language designed around four core principles:

- **Semantic Honesty** — Types describe meaning (`List`, `Text`, `Map`, `Maybe`), not implementation details (`Vec`, `String`, `HashMap`, `Option`).
- **No Magic** — All dependencies are explicit via `using [...]`. No hidden state, no implicit allocators, no invisible runtime behavior.
- **Gradual Safety** — A three-tier reference system lets you choose between full runtime validation (~15ns), compiler-proven zero-cost references, and manual unsafe references.
- **Zero-Cost Abstractions** — CBGR memory safety, the context system, and computational properties are designed to optimize away in release builds.

Verum has only **three reserved keywords**: `let`, `fn`, `is`. All type definitions use the unified `type Name is ...` syntax. There is no `struct`, `enum`, `trait`, or `impl` keyword. All compile-time constructs use the `@` prefix — there is no `!` suffix syntax anywhere in the language.

---

## 2. Key Features

### 2.1 Semantic Types

Verum uses types whose names reflect their purpose, not their implementation.

| Semantic Type | Rust Equivalent | Purpose |
|---------------|----------------|---------|
| `List<T>` | `Vec<T>` | Dynamic array |
| `Text` | `String` | UTF-8 string |
| `Map<K, V>` | `HashMap<K, V>` | Key-value mapping |
| `Set<T>` | `HashSet<T>` | Unique collection |
| `Maybe<T>` | `Option<T>` | Optional value |
| `Result<T, E>` | `Result<T, E>` | Success or error |
| `Heap<T>` | `Box<T>` | Heap-allocated value |
| `Shared<T>` | `Arc<T>` | Shared ownership |

```verum
mount core.collections.{List, Map, Set};
mount core.base.{Maybe, Some, None, Result, Ok, Err};

fn process_names(names: List<Text>) -> Map<Text, Int> {
    let mut counts = Map.new();
    for name in names {
        let current = counts.get(&name) ?? 0;
        counts.insert(name, current + 1);
    }
    counts
}
```

### 2.2 Type Definitions

All types in Verum are defined using the unified `type Name is ...;` syntax.

#### Record Types (like structs)

```verum
type Point is { x: Float, y: Float };

type User is {
    name: Text,
    email: Text,
    age: Int,
};
```

#### Sum Types (like enums)

```verum
type Color is Red | Green | Blue;

type Shape is
    Circle { radius: Float }
    | Rectangle { width: Float, height: Float }
    | Triangle { a: Float, b: Float, c: Float };

type Option<T> is None | Some(T);

type Tree<T> is
    Leaf(T)
    | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>> };
```

#### Protocols (like traits)

```verum
type Display is protocol {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError>;
};

type Iterator is protocol {
    type Item;
    fn next(&mut self) -> Maybe<Self.Item>;
};

type Numeric is protocol extends Eq + Ord {
    fn zero() -> Self;
    fn one() -> Self;
    fn add(self, other: Self) -> Self;
    fn mul(self, other: Self) -> Self;
};
```

#### Newtypes

```verum
type UserId is (Int);
type Email is (Text);
type Meters is (Float);
```

#### Unit Types

```verum
type Marker is ();
```

### 2.3 Three-Tier References (CBGR)

Verum's memory safety model provides three tiers of references with different safety/performance tradeoffs.

| Tier | Syntax | Overhead | Description |
|------|--------|----------|-------------|
| 0 | `&T` | ~15ns | Managed reference with full CBGR validation |
| 1 | `&checked T` | 0ns | Compiler-proven safe via escape analysis |
| 2 | `&unsafe T` | 0ns | Manual safety proof required |

```verum
// Tier 0: Managed references (default, safe)
fn sum(values: &List<Int>) -> Int {
    let mut total = 0;
    for v in values {
        total += v;
    }
    total
}

// Tier 1: Compiler-verified zero-cost references
fn fast_lookup(data: &checked List<Int>, idx: Int) -> Int {
    data[idx]
}

// Tier 2: Unsafe references for performance-critical paths
fn raw_copy(src: &unsafe Byte, dst: &unsafe mut Byte, len: Int) {
    // SAFETY: caller guarantees non-overlapping, valid memory
    unsafe {
        mem_copy(dst, src, len);
    }
}
```

### 2.4 Context System (Dependency Injection)

The context system provides compile-time-tracked, runtime dependency injection. Functions declare their context requirements with `using`, and callers provide contexts with `provide`.

```verum
// Define a context
context Database {
    fn query(&self, sql: Text) -> Result<Rows, DbError>;
    fn execute(&self, sql: Text) -> Result<Int, DbError>;
}

context Logger {
    fn info(&self, msg: Text);
    fn error(&self, msg: Text);
}

// Functions declare required contexts
fn get_user(id: Int) -> Result<User, AppError> using [Database, Logger] {
    Logger.info(f"Fetching user {id}");
    let rows = Database.query(f"SELECT * FROM users WHERE id = {id}")?;
    parse_user(rows)
}

// Caller provides contexts
fn main() {
    provide Database = PostgresDb.connect("postgres://localhost/mydb");
    provide Logger = ConsoleLogger.new();

    let user = get_user(42);
    print(f"Found: {user}");
}
```

Advanced context features include aliasing, conditional contexts, transforms, negative contexts, and layers:

```verum
// Multiple instances of the same context type
fn replicate() using [Database as primary, Database as replica] {
    let data = replica.query("SELECT * FROM table")?;
    primary.execute(f"INSERT INTO backup ...")?;
}

// Negative contexts (compiler-verified absence)
fn pure_compute(x: Int) -> Int using [!Database, !IO] {
    x * x + 1
}

// Context layers for composition
layer DatabaseLayer {
    provide ConnectionPool = ConnectionPool.new(Config.get_url());
    provide QueryExecutor = QueryExecutor.new(ConnectionPool);
}

layer AppLayer = DatabaseLayer + LoggingLayer;

fn main() {
    provide AppLayer;
    run_server();
}
```

### 2.5 Pattern Matching

Verum supports comprehensive pattern matching with guards, or-patterns, and-patterns, active patterns, view patterns, and destructuring.

```verum
type Shape is
    Circle(Float)
    | Rectangle(Float, Float)
    | Triangle { a: Float, b: Float, c: Float };

fn area(shape: Shape) -> Float {
    match shape {
        Circle(r) => 3.14159 * r ** 2,
        Rectangle(w, h) => w * h,
        Triangle { a, b, c } => {
            let s = (a + b + c) / 2.0;
            (s * (s - a) * (s - b) * (s - c)) ** 0.5
        },
    }
}

// Guards
fn classify(n: Int) -> Text {
    match n {
        x if x < 0 => "negative",
        0 => "zero",
        x if x <= 100 => "small positive",
        _ => "large positive",
    }
}

// Or-patterns
fn is_whitespace(c: Char) -> Bool {
    c is (' ' | '\t' | '\n' | '\r')
}

// And-patterns with active patterns
pattern Even(n: Int) -> Bool = n % 2 == 0;
pattern Positive(n: Int) -> Bool = n > 0;

fn describe(n: Int) -> Text {
    match n {
        Even() & Positive() => "positive even",
        Even() => "non-positive even",
        Positive() => "positive odd",
        _ => "non-positive odd",
    }
}

// Active patterns with extraction (partial patterns)
pattern ParseInt(s: Text) -> Maybe<Int> = s.parse_int();

fn process_input(input: Text) -> Result<Int, Error> {
    match input {
        ParseInt()(n) => Ok(n * 2),
        _ => Err(Error.new("not a number")),
    }
}

// View patterns
fn process(data: List<Int>) -> Text {
    match data {
        data.len -> 0 => "empty",
        data.len -> 1 => "singleton",
        _ => "multiple",
    }
}

// Destructuring assignment
let (a, b) = (b, a);          // parallel swap
let Point { x, y } = origin;  // record destructuring
let [first, ..rest] = items;   // array destructuring
```

### 2.6 Async/Await and Structured Concurrency

Verum requires explicit `async` marking — there is no implicit async infection. Structured concurrency is enforced through nurseries that guarantee all spawned tasks complete before the scope exits.

```verum
// Async functions must be explicitly marked
async fn fetch_data(url: Text) -> Result<Data, HttpError> using [Http] {
    let response = Http.get(url).await?;
    response.json().await
}

// Nursery: structured concurrency scope
async fn fetch_all(urls: List<Text>) -> List<Data> using [Http] {
    let mut results = List.new();

    nursery(timeout: 30000, on_error: cancel_all) {
        for url in urls {
            let data = spawn fetch_data(url);
            results.push(data);
        }
    }

    results
}

// Select: race multiple futures
async fn fetch_with_fallback() -> Data using [Http] {
    select {
        data = fetch_primary().await => data,
        data = fetch_secondary().await => data,
        else => Data.default(),
    }
}

// Channels
async fn producer_consumer() {
    let (tx, rx) = channel.new();

    nursery {
        spawn async {
            for i in 0..100 {
                tx.send(i).await;
            }
        };

        spawn async {
            for await value in rx {
                print(f"Received: {value}");
            }
        };
    }
}

// Async generators
async fn* paginated_fetch(url: Text) -> Page using [Http] {
    let mut page_num = 1;
    loop {
        let page = Http.get(f"{url}?page={page_num}").await?;
        if page.is_empty() { return; }
        yield page;
        page_num += 1;
    }
}

// Consuming async generators
async fn process_pages(url: Text) using [Http] {
    for await page in paginated_fetch(url) {
        process(page);
    }
}
```

### 2.7 Metaprogramming

Verum supports staged metaprogramming with `meta fn`, `quote { }` blocks, and `@` attributes. All compile-time constructs use the `@` prefix.

```verum
// Derive attribute
@derive(Eq, Hash, Display)
type Color is Red | Green | Blue;

// Meta function (executes at compile time)
meta fn generate_getters(fields: List<FieldInfo>) -> TokenStream {
    quote {
        $[for field in fields {
            public fn $field.name(&self) -> $field.ty {
                self.$field.name
            }
        }]
    }
}

// Multi-stage metaprogramming
meta fn derive_eq() -> TokenStream {
    // Stage 1: generates runtime code (stage 0)
    quote {
        fn eq(&self, other: &Self) -> Bool {
            $[for field in @fields_of(Self) {
                if self.$field.name != other.$field.name {
                    return false;
                }
            }]
            true
        }
    }
}

// User-defined macros use @ prefix
meta sql_query(input: tt) {
    // Parse SQL at compile time, generate type-safe query
    ...
}

let users = @sql_query("SELECT name, age FROM users WHERE active = true");

// Compile-time meta functions
let type_name = @type_name(User);       // "User"
let field_count = @fields_of(User).len; // number of fields
let is_int = @implements(Int, Eq);      // true
```

### 2.8 Rank-2 Polymorphism

Verum supports rank-2 polymorphic function types where the quantified type parameters scope within the function type itself. The function must work for **all** choices of the quantified type, rather than the caller choosing a specific type.

```verum
// Regular function type (rank-1): caller chooses T
type Processor<T> is fn(T) -> T;

// Rank-2: fn<R>(...) — function works for ALL R
type Reducer<A, R> is fn(R, A) -> R;

type Transducer<A, B> is {
    transform: fn<R>(Reducer<B, R>) -> Reducer<A, R>,
};

// Stateful rank-2 transducer
type StatefulTransducer<A, B, S> is {
    initial_state: S,
    transform: fn<R>(Reducer<B, R>, &mut S) -> Reducer<A, R>,
};

// Usage: the transform function must work for ANY accumulator type R
fn mapping<A, B>(f: fn(A) -> B) -> Transducer<A, B> {
    Transducer {
        transform: fn<R>(reducer: Reducer<B, R>) -> Reducer<A, R> {
            fn(acc: R, item: A) -> R {
                reducer(acc, f(item))
            }
        },
    }
}
```

### 2.9 Computational Properties

Verum tracks computational properties (Pure, IO, Async, Fallible, Mutates) at compile time. These are **not** algebraic effects — they are static properties inferred from code.

```verum
// Pure function: compiler verifies no side effects
pure fn add(a: Int, b: Int) -> Int {
    a + b
}

// Pure functions enable memoization and parallel execution
pure fn fibonacci(n: Int) -> Int {
    if n <= 1 { n }
    else { fibonacci(n - 1) + fibonacci(n - 2) }
}

// Impure functions are inferred from code that performs IO, mutation, etc.
fn greet(name: Text) {
    print(f"Hello, {name}!");  // IO property inferred
}
```

### 2.10 Refinement Types

Refinement types allow you to attach logical predicates to types. These predicates are verified at compile time using Z3 SMT solving.

```verum
// Refined integer types
type PositiveInt is Int { self > 0 };
type Percentage is Int { self >= 0, self <= 100 };
type NonEmptyText is Text { self.len() > 0 };

// Functions with refinement type parameters
fn divide(a: Int, b: Int { b != 0 }) -> Float {
    a as Float / b as Float
}

// Refinements on record fields
type Config is {
    port: Int { self > 0, self <= 65535 },
    max_connections: Int { self > 0 },
    timeout_ms: Int { self >= 100 },
};

// Formal verification with requires/ensures
fn binary_search(arr: &[Int], target: Int) -> Maybe<Int>
    requires forall i: Int in 0..arr.len() - 1. arr[i] <= arr[i + 1]
    ensures result is Some(idx) => arr[idx] == target
{
    // implementation...
}
```

### 2.11 Algebraic Data Types

Verum supports the full spectrum of algebraic data types: product types, sum types, inductive types, and coinductive types.

```verum
// Product type (record)
type Point3D is { x: Float, y: Float, z: Float };

// Sum type
type JsonValue is
    Null
    | Bool(Bool)
    | Number(Float)
    | Str(Text)
    | Array(List<JsonValue>)
    | Object(Map<Text, JsonValue>);

// Inductive type (well-founded recursion with termination checking)
type Nat is inductive {
    | Zero
    | Succ(Nat)
};

// Coinductive type (infinite structures with productivity checking)
type Stream<A> is coinductive {
    fn head(&self) -> A;
    fn tail(&self) -> Stream<A>;
};
```

### 2.12 Protocols

Protocols are Verum's mechanism for polymorphism, analogous to traits but with `extends` for inheritance, default method implementations, and associated types.

```verum
type Hashable is protocol extends Eq {
    fn hash(&self) -> Int;
};

type Serializable is protocol {
    type Format;
    fn serialize(&self) -> Result<Self.Format, SerError>;
    fn deserialize(data: Self.Format) -> Result<Self, SerError>;
};

// Protocol with default implementations
type Collection is protocol {
    type Item;
    fn len(&self) -> Int;
    fn is_empty(&self) -> Bool { self.len() == 0 }
};

// Higher-Kinded Types (HKT)
type Functor is protocol {
    type F<_>;
    fn map<A, B>(self: Self.F<A>, f: fn(A) -> B) -> Self.F<B>;
};

// Implement a protocol
implement Display for Point {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> {
        f.write(f"({self.x}, {self.y})")
    }
}

// Specialization
@specialize
implement Display for List<Text> {
    fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> {
        f.write(self.join(", "))
    }
}
```

### 2.13 Generics

Verum generics support bounds, where clauses, const generics, higher-kinded types, and existential types.

```verum
// Basic generics with bounds
fn max<T: Ord>(a: T, b: T) -> T {
    if a >= b { a } else { b }
}

// Where clauses
fn serialize_all<T>(items: List<T>) -> List<Text>
    where type T: Serializable + Display
{
    items.map(|item| item.to_string())
}

// Const generics (type-level literals)
type Matrix<const ROWS: Int, const COLS: Int> is {
    data: [[Float; COLS]; ROWS],
};

fn multiply<const M: Int, const N: Int, const P: Int>(
    a: Matrix<M, N>,
    b: Matrix<N, P>,
) -> Matrix<M, P> {
    // ...
}

// Existential types
fn make_iterator() -> some I: Iterator<Item = Int> {
    [1, 2, 3].iter()
}

// Higher-kinded type parameters
fn lift<F<_>: Functor, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B> {
    Functor.map(fa, f)
}
```

### 2.14 Error Handling

Verum uses `Result<T, E>` and `Maybe<T>` for error handling, with `try`/`recover`/`finally` blocks and the `?` operator for propagation. Functions can also declare typed throws clauses.

```verum
// The ? operator propagates errors
fn read_config(path: Text) -> Result<Config, AppError> {
    let content = fs.read_to_string(path)?;
    let config = parse_toml(content)?;
    Ok(config)
}

// try/recover/finally
fn safe_divide(a: Int, b: Int) -> Int {
    try {
        divide(a, b) as Int
    } recover {
        DivisionByZero => 0,
        Overflow(msg) => {
            log.error(f"Overflow: {msg}");
            Int.max
        },
    } finally {
        cleanup();
    }
}

// Recover with closure syntax
fn parse_or_default(input: Text) -> Int {
    try {
        parse_int(input)?
    } recover |e| {
        print(f"Parse failed: {e}");
        0
    }
}

// Typed throws clause
fn parse(input: Text) throws(ParseError) -> AST {
    if input.is_empty() {
        throw ParseError.Empty;
    }
    // ...
}

// errdefer: cleanup only on error path
fn open_and_process(path: Text) -> Result<Data, Error> {
    let file = File.open(path)?;
    errdefer file.close();  // only runs if an error occurs below

    let data = file.read_all()?;
    process(data)
}
```

### 2.15 Closures and Higher-Order Functions

```verum
// Basic closure
let double = |x: Int| -> Int { x * 2 };
let add = |a, b| a + b;

// Higher-order functions
fn apply<A, B>(f: fn(A) -> B, value: A) -> B {
    f(value)
}

fn compose<A, B, C>(f: fn(A) -> B, g: fn(B) -> C) -> fn(A) -> C {
    |x| g(f(x))
}

// Closures with capture
fn make_counter() -> fn() -> Int {
    let mut count = 0;
    || {
        count += 1;
        count
    }
}

// Async closures
let fetch = async |url: Text| -> Data {
    Http.get(url).await?.json().await
};
```

### 2.16 Iterators

Verum iterators are lazy and support the full suite of functional combinators.

```verum
type Iterator is protocol {
    type Item;
    fn next(&mut self) -> Maybe<Self.Item>;

    // Default adapter methods
    fn map<B>(self, f: fn(Self.Item) -> B) -> MappedIter<Self, fn(Self.Item) -> B>;
    fn filter(self, pred: fn(&Self.Item) -> Bool) -> FilterIter<Self, fn(&Self.Item) -> Bool>;
    fn fold<B>(self, init: B, f: fn(B, Self.Item) -> B) -> B;
    fn collect<C: FromIterator<Self.Item>>(self) -> C;
    fn take(self, n: Int) -> TakeIter<Self>;
    fn skip(self, n: Int) -> SkipIter<Self>;
    fn enumerate(self) -> EnumerateIter<Self>;
    fn zip<U: Iterator>(self, other: U) -> ZipIter<Self, U>;
    fn chain<U: Iterator<Item = Self.Item>>(self, other: U) -> ChainIter<Self, U>;
    fn flat_map<B, U: Iterator<Item = B>>(self, f: fn(Self.Item) -> U) -> FlatMapIter<Self, U, fn(Self.Item) -> U>;
    // ... and many more
};

// Usage
let result = numbers
    .iter()
    .filter(|n| n > 0)
    .map(|n| n * 2)
    .take(10)
    .collect::<List<Int>>();

// Comprehensions (alternative syntax)
let doubled = [n * 2 for n in numbers if n > 0];

// Map comprehension
let lengths = {name: name.len() for name in names};

// Set comprehension
let unique_lengths = set{name.len() for name in names};

// Generator expression (lazy)
let squares = gen{x * x for x in 0..1000};
```

### 2.17 Loops

```verum
// For loop
for item in collection {
    process(item);
}

// For loop with index
for (i, item) in collection.enumerate() {
    print(f"Item {i}: {item}");
}

// For await (async iteration)
for await msg in channel {
    handle(msg);
}

// While loop
while condition {
    step();
}

// Loop with verification annotations
for i in 0..n
    invariant total >= 0
    decreases n - i
{
    total += arr[i];
}

// Infinite loop
loop {
    let event = poll_event();
    if event is Quit { break; }
    handle(event);
}
```

### 2.18 Format Strings

Verum uses `f"..."` literals for string interpolation. There is no `format!()` macro.

```verum
let name = "world";
let greeting = f"Hello, {name}!";

let x = 42;
let y = 3.14;
print(f"x = {x}, y = {y:.2}");

// Debug format
print(f"Config: {config:?}");

// Padding and alignment
print(f"{value:>10}");   // right-aligned, width 10
print(f"{value:<10}");   // left-aligned, width 10
print(f"{value:^10}");   // centered, width 10
print(f"{value:06}");    // zero-padded, width 6
```

### 2.19 Module System

Verum uses `mount` (not `use`) for imports and `cog` (not `crate`) for the package unit.

```verum
// Import specific items
mount core.collections.{List, Map, Set};
mount core.base.{Maybe, Some, None};

// Import with alias
mount core.io.File as IoFile;

// Glob import
mount core.base.*;

// Nested import tree
mount core.{
    collections.{List, Map},
    base.{Result, Ok, Err},
    io.File,
};

// Module path (dot-separated, not ::)
mount my_cog.utils.helpers;

// Relative imports
mount self.submodule;
mount super.sibling;

// Module definition
module my_module;

// Nested module
module my_module {
    public fn helper() -> Int { 42 }
}
```

### 2.20 Tagged Literals

Verum provides compile-time validated tagged literals for common data formats. The content is validated at compilation, not runtime.

```verum
// JSON (relaxed syntax, validated at compile time)
let config = json#"""{ "port": 8080, "host": "localhost" }""";

// SQL (compile-time syntax checking)
let query = sql#"""SELECT name, age FROM users WHERE active = true""";

// Regex
let pattern = rx#"^\d{3}-\d{4}$";

// URL
let endpoint = url#"https://api.example.com/v2/users";

// DateTime
let deadline = d#"2026-12-31T23:59:59Z";

// Tagged literals with interpolation
let table = "users";
let q = sql#"""SELECT * FROM ${table} WHERE id = ${user_id}""";

// Byte strings
let bytes = b"Hello\x00World";
```

### 2.21 Formal Verification

Verum supports formal proofs, theorems, lemmas, and calculational proofs, backed by Z3 SMT solving.

```verum
// Theorem with proof
theorem sum_positive(a: Int, b: Int)
    requires a > 0, b > 0
    ensures result > 0
{
    proof by auto
}

// Lemma
lemma list_append_length<T>(xs: List<T>, ys: List<T>)
    ensures xs.append(ys).len() == xs.len() + ys.len()
{
    proof by induction(xs)
}

// Axiom (unproven assumption)
axiom commutativity_of_addition(a: Int, b: Int) -> Bool;

// Calculational proof
calc {
    x + y
    == { by associativity } y + x
    == { by simplify } result
}

// Quantifiers in specifications
fn sort(arr: &mut [Int])
    ensures forall i: Int in 0..arr.len() - 1. arr[i] <= arr[i + 1]
{
    // ...
}
```

### 2.22 Streams and Generators

```verum
// Sync generator (fn*)
fn* fibonacci() -> Int {
    let (mut a, mut b) = (0, 1);
    loop {
        yield a;
        (a, b) = (b, a + b);
    }
}

// Consuming a generator
for n in fibonacci().take(20) {
    print(f"{n}");
}

// Stream comprehension
let evens = stream[x * 2 for x in 0..];

// Stream literals
let cycle = stream[1, 2, 3, ...];     // infinite cycle
let range = stream[0..1000];           // lazy range

// Stream pattern matching
match my_stream {
    stream[first, second, ...rest] => process(first, second, rest),
    stream[] => handle_empty(),
}
```

### 2.23 FFI (Foreign Function Interface)

```verum
// Extern block for C functions
extern "C" {
    fn malloc(size: Int) -> &unsafe Byte;
    fn free(ptr: &unsafe Byte);
}

// FFI boundary with contracts
ffi LibSqlite {
    @extern("sqlite3_open", calling_convention = "C")
    fn open(filename: Text) -> Result<DbHandle, SqliteError>;

    requires filename.len() > 0;
    errors_via = ReturnCode(SQLITE_OK);
    thread_safe = false;
    memory_effects = Allocates;
}
```

### 2.24 Capability Types

Verum supports capability-restricted types for fine-grained access control.

```verum
type Database.ReadOnly is Database with [Read];
type Database.Full is Database with [Read, Write, Admin];

// Functions accept capability-restricted types
fn analyze(db: Database with [Read]) -> Stats {
    db.query("SELECT count(*) FROM events")
}

// Capability attenuation is automatic via subtyping
// Database.Full <: Database.ReadOnly because superset of capabilities
fn main() {
    let db = Database.Full.connect("...");
    let stats = analyze(db);  // Full automatically narrows to Read
}
```

---

## 3. Syntax Quick Reference

### Verum vs Rust

| Concept | Rust | Verum |
|---------|------|-------|
| Struct | `struct Point { x: f64, y: f64 }` | `type Point is { x: Float, y: Float };` |
| Enum | `enum Color { Red, Green, Blue }` | `type Color is Red \| Green \| Blue;` |
| Trait | `trait Display { ... }` | `type Display is protocol { ... };` |
| Impl | `impl Display for Point { ... }` | `implement Display for Point { ... }` |
| Inherent impl | `impl Point { ... }` | `implement Point { ... }` |
| Box | `Box::new(x)` | `Heap(x)` |
| Vec | `Vec<T>` | `List<T>` |
| String | `String` | `Text` |
| HashMap | `HashMap<K, V>` | `Map<K, V>` |
| HashSet | `HashSet<T>` | `Set<T>` |
| Option | `Option<T>` | `Maybe<T>` |
| Derive | `#[derive(Debug)]` | `@derive(Debug)` |
| Attribute | `#[repr(C)]` | `@repr(C)` |
| Import | `use std::collections::HashMap;` | `mount core.collections.Map;` |
| Crate | `crate` | `cog` |
| Path sep | `foo::bar::Baz` | `foo.bar.Baz` |
| println! | `println!("hello")` | `print("hello")` |
| format! | `format!("x={}", x)` | `f"x={x}"` |
| panic! | `panic!("error")` | `panic("error")` |
| assert! | `assert!(cond)` | `assert(cond)` |
| matches! | `matches!(x, Some(_))` | `x is Some(_)` |
| Macro call | `my_macro!(...)` | `@my_macro(...)` |

### Operators

| Operator | Description |
|----------|-------------|
| `+`, `-`, `*`, `/`, `%` | Arithmetic |
| `**` | Exponentiation |
| `==`, `!=`, `<`, `>`, `<=`, `>=` | Comparison |
| `&&`, `\|\|`, `!` | Logical |
| `&`, `\|`, `^`, `~`, `<<`, `>>` | Bitwise |
| `..`, `..=` | Range (exclusive, inclusive) |
| `\|>` | Pipeline |
| `>>`, `<<` | Function composition |
| `?.` | Optional chaining |
| `??` | Null coalescing |
| `?` | Error propagation |
| `is` | Pattern test |
| `as` | Type cast |

---

## 4. Standard Library (core/)

The standard library consists of 289 modules written in Verum (`.vr` files), organized into 19 subsystems.

| Subsystem | Path | Modules | Description |
|-----------|------|---------|-------------|
| **base** | `core/base/` | 16 | Core types and protocols: Eq, Ord, Hash, Iterator, Maybe, Result, Error, primitives, serialization |
| **collections** | `core/collections/` | 8 | Data structures: List, Map, Set, Deque, BTree, Heap, Slice |
| **mem** | `core/mem/` | 13 | CBGR memory system: ThinRef, FatRef, allocator, arena, epoch tracking, hazard pointers, segments |
| **async** | `core/async/` | 17 | Concurrency: Future, Task, Channel, Executor, Nursery, Select, Stream, Generator, Timer, Waker |
| **io** | `core/io/` | 10 | I/O system: File, FileSystem, Path, Process, Stdio, Buffer, async protocols, engine |
| **math** | `core/math/` | 31 | Mathematics: elementary, algebra, calculus, linear algebra, complex numbers, tensor, autodiff, random, statistics, topology, category theory |
| **text** | `core/text/` | 7 | Text processing: Text type, StringBuilder, Char utilities, Regex, format strings, tagged literals |
| **time** | `core/time/` | 5 | Time: Instant, Duration, SystemTime, Interval |
| **net** | `core/net/` | 7 | Networking: TCP, UDP, TLS, HTTP, DNS, address types |
| **sync** | `core/sync/` | 9 | Synchronization: Mutex, RwLock, Atomic, Barrier, Semaphore, CondVar, Once, WaitGroup |
| **context** | `core/context/` | 6 | Context system runtime: Provider, Scope, Layer, error handling |
| **meta** | `core/meta/` | 7 | Metaprogramming: quote, token, span, reflection, attribute, context info |
| **runtime** | `core/runtime/` | 18 | Runtime support: CBGR bridge, thread pool, task queue, supervisor, recovery, syscalls, stack allocation, TLS |
| **sys** | `core/sys/` | 16+ | OS interface: platform-specific modules for Darwin, Linux, Windows; file ops, net ops, process ops, signals, I/O engine, MMIO |
| **simd** | `core/simd/` | 2 | SIMD and GPU: vectorized operations |
| **intrinsics** | `core/intrinsics/` | 13+ | Compiler intrinsics: arithmetic, atomic, bitwise, conversion, control flow, float, GPU, memory, platform, SIMD, tensor, type info |
| **term** | `core/term/` | 7+ | Terminal: app framework, events, layout, raw mode, rendering, styling, widgets |

---

## 5. Memory Safety Model (CBGR)

CBGR (Counter-Based Generation References) is Verum's memory safety system. It prevents use-after-free, dangling pointers, and data races through generation-based validation.

### How It Works

Every heap allocation has a **generation counter** in its allocation header. Every reference stores the expected generation at the time of creation. On dereference, the reference's stored generation is compared against the allocation's current generation. If they differ, the allocation has been freed and reallocated — the reference is dangling, and the access is trapped.

### Reference Types

| Type | Size | Fields | Description |
|------|------|--------|-------------|
| `ThinRef<T>` | 16 bytes | ptr (8) + generation (4) + epoch_caps (4) | For sized types, fits in two registers |
| `FatRef<T>` | 24 bytes | ptr (8) + generation (4) + epoch_caps (4) + len (8) | For slices and dynamically-sized types |

### Three Tiers

**Tier 0 — Managed (`&T`):** Full CBGR validation on every dereference. Approximately 15ns overhead. This is the default and catches all use-after-free bugs at runtime.

**Tier 1 — Checked (`&checked T`):** Zero overhead. The compiler proves via escape analysis that the reference cannot outlive its referent. If the proof fails, compilation fails. No runtime checks needed.

**Tier 2 — Unsafe (`&unsafe T`):** Zero overhead. The programmer asserts safety manually, documented with `// SAFETY:` comments. No compiler proof, no runtime checks.

### Epoch System

The epoch_caps field in each reference packs a 16-bit epoch counter and 16-bit capability flags. The epoch prevents ABA problems (where a generation counter wraps around). Capabilities control access rights (read, write, mutable, revoke).

### Validation Tiers by Execution Mode

| Mode | Overhead | Strategy |
|------|----------|----------|
| Interpreter (Tier 0) | ~15ns | Full validation with hazard pointers |
| Baseline JIT (Tier 1) | ~10ns | Standard validation |
| Optimizing JIT (Tier 2) | ~5ns | Selective validation (hot path elimination) |
| AOT (Tier 3) | ~3ns | Minimal validation (compiler-proven paths skipped) |

### Performance Targets

| Metric | Target |
|--------|--------|
| CBGR check latency | < 15ns |
| Runtime vs native C | 0.85--0.95x |
| Memory overhead | < 5% |

---

## 6. Compilation Pipeline

```
Source (.vr)
    |
    v
[Lexer] (logos-based DFA tokenization)
    |
    v
[Parser] (recursive-descent, error-recovering)
    |
    v
[AST]
    |
    v
[Type Checker] (Hindley-Milner inference + refinement types)
    |       \
    |        v
    |    [SMT Solver] (Z3 — verifies refinement predicates)
    |
    v
[CBGR Analysis] (reference tier assignment, escape analysis)
    |
    v
[VBC Codegen] (Verum Bytecode — register-based IR)
    |
    +-----------+-----------+
    |                       |
    v                       v
[Interpreter]         [LLVM Backend]
(Tier 0: dev/debug)   (AOT: release)
    |                       |
    v                       v
  Output              Native Binary
```

### Execution Tiers

| Tier | Mode | Use Case |
|------|------|----------|
| 0 | VBC Interpreter | Development, debugging, REPL |
| 1 | VBC to LLVM IR | Release builds, production deployment |

---

## 7. Building

### Prerequisites

- **Rust** (nightly, specified in `rust-toolchain.toml`)
- **LLVM 21.x** (for AOT compilation)
- **Z3 4.x** (for SMT-based refinement type verification)

### Build the Compiler

```bash
cargo build --release
```

### Run a Verum Program

```bash
# Interpreter mode (Tier 0)
./target/release/verum run hello.vr

# AOT compile to native binary
./target/release/verum build hello.vr -o hello
./hello
```

### REPL

```bash
./target/release/verum repl
```

### Hello World

```verum
fn main() {
    print("Hello, Verum!");
}
```

### Fibonacci

```verum
fn fib(n: Int) -> Int {
    if n <= 1 { n }
    else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    let result = fib(10);
    print(f"fib(10) = {result}");  // fib(10) = 55
}
```

---

## 8. Testing

### Verum Conformance Suite (VCS)

The VCS contains 1000+ specification tests organized into five levels.

| Level | Name | Purpose | Status |
|-------|------|---------|--------|
| **L0** | Critical | Lexer, parser, ownership, memory safety | 2918 tests |
| **L1** | Core | Type inference, refinements, generics | 415/415 (100%) |
| **L2** | Standard | Async/await, context system, modules | 354/354 (100%) |
| **L3** | Extended | Dependent types, FFI, GPU, metaprogramming | 310/312 (99.4%) |
| **L4** | Performance | CBGR latency, compilation speed, throughput | 80/80 (100%) |

### Running Tests

```bash
cd vcs
make test          # Run all tests
make test-l0       # Run L0-critical only
make test-l1       # Run L1-core only
make bench         # Run benchmarks
make fuzz          # Run fuzzer
make differential  # Run differential tests (interpreter vs AOT)
```

### Test File Format

Each `.vr` test file includes metadata annotations:

```verum
// @test: unit
// @tier: 0
// @level: L1
// @tags: type-inference, generics
// @timeout: 5000
// @expect: pass

fn main() {
    let x = 42;
    assert_eq(x, 42);
}
```

---

## 9. Project Structure

### Crate Architecture

```
                         LAYER 4: TOOLS
  verum_cli ---------- verum_compiler ---------- verum_lsp
                             |
                     verum_interactive
                     (Playbook TUI, REPL)

                    LAYER 3: EXECUTION (VBC-First)
  verum_vbc <------------- verum_codegen
  (bytecode, interpreter,   (VBC -> LLVM for AOT)
   codegen, intrinsics)
        |                        |
  verum_verification       verum_modules

                     LAYER 2: TYPE SYSTEM
  verum_types <------- verum_smt <------- verum_cbgr
        |                 | (Z3)               |
  verum_diagnostics    verum_error

                      LAYER 1: PARSING
  verum_fast_parser <-- verum_lexer <------ verum_ast
  (main parser)         (logos DFA)

                    LAYER 0: FOUNDATION
                      verum_common
                    (List, Text, Map, Maybe)

                    core/ (Verum stdlib in .vr)
```

### Crate Descriptions

| Crate | Purpose |
|-------|---------|
| `verum_common` | Semantic types (List, Text, Map, Maybe) with zero external dependencies |
| `verum_ast` | AST node definitions: expressions, types, patterns, declarations |
| `verum_lexer` | Tokenization via logos DFA-based lexer generation |
| `verum_fast_parser` | Production recursive-descent parser with error recovery |
| `verum_types` | Type checking: inference, unification, refinement types |
| `verum_smt` | SMT verification backend (Z3): refinement predicate solving, tactics |
| `verum_cbgr` | Memory safety system: managed, checked, and unsafe references |
| `verum_vbc` | VBC bytecode: codegen, interpreter, intrinsics |
| `verum_codegen` | VBC to LLVM IR lowering for AOT native compilation |
| `verum_verification` | Gradual verification: levels, verification condition generation, passes |
| `verum_modules` | Module resolution: loader, resolver, dependency graph |
| `verum_diagnostics` | Diagnostic messages, error rendering, LSP integration |
| `verum_error` | Error type definitions and error catalog |
| `verum_compiler` | Compilation pipeline orchestration: sessions, phases, parallel compilation |
| `verum_lsp` | Language Server Protocol: completions, hover, go-to-definition, script parsing |
| `verum_interactive` | REPL and Playbook TUI (re-exports from verum_lsp) |
| `verum_cli` | Command-line interface: build, run, test, playbook commands |

### Key External Dependencies

| Library | Version | Used By | Purpose |
|---------|---------|---------|---------|
| Z3 | 0.19.5 | verum_smt | SMT solving for refinement type verification |
| LLVM | 21.x | verum_codegen | Native code generation (AOT path) |
| logos | 0.15.1 | verum_lexer | High-performance DFA-based lexer generation |
| rayon | 1.11 | verum_compiler | Parallel compilation of independent modules |

---

## 10. Reserved Keywords

Verum has only **three reserved keywords**:

| Keyword | Purpose |
|---------|---------|
| `let` | Variable binding |
| `fn` | Function definition |
| `is` | Type definition body, pattern testing, type test |

All other keywords (`type`, `if`, `else`, `match`, `for`, `while`, `loop`, `return`, `break`, `continue`, `async`, `await`, `pub`, `mut`, `const`, `unsafe`, `pure`, `meta`, `mount`, `module`, `implement`, `context`, `protocol`, `extends`, `spawn`, `yield`, `try`, `recover`, `finally`, `provide`, `defer`, `errdefer`, `select`, `nursery`, `where`, `using`, `throw`, `throws`, `forall`, `exists`, `theorem`, `lemma`, `axiom`, `proof`, etc.) are non-reserved — they are contextual keywords recognized by the parser in specific positions.

---

## 11. Built-in Functions

Verum built-in functions use **standard call syntax** with no `!` suffix. All compile-time constructs use the `@` prefix.

### I/O

| Function | Description |
|----------|-------------|
| `print(msg)` | Print to stdout |
| `eprint(msg)` | Print to stderr |

### Assertions

| Function | Description |
|----------|-------------|
| `assert(condition)` | Assert a condition is true |
| `assert(condition, message)` | Assert with custom message |
| `assert_eq(a, b)` | Assert equality |
| `assert_ne(a, b)` | Assert inequality |
| `debug_assert(condition)` | Debug-only assertion (removed in release) |

### Control Flow

| Function | Description |
|----------|-------------|
| `panic(message)` | Abort with error message |
| `unreachable()` | Mark unreachable code (aborts if reached) |
| `unimplemented()` | Mark unimplemented code |
| `todo(message)` | Mark work in progress |

### Async

| Function | Description |
|----------|-------------|
| `join(a, b)` | Concurrently execute two futures |
| `try_join(a, b)` | Concurrent execution with error propagation |
| `join_all(futures)` | Wait for all futures |
| `select_any(futures)` | Wait for first future to complete |

### Compile-Time Meta Functions (`@` prefix)

| Function | Description |
|----------|-------------|
| `@const(expr)` | Force compile-time evaluation |
| `@cfg(condition)` | Configuration check |
| `@error("msg")` | Compile-time error |
| `@warning("msg")` | Compile-time warning |
| `@stringify(tokens)` | Convert tokens to string |
| `@concat(a, b)` | Token concatenation |
| `@file` | Current file path |
| `@line` | Current line number |
| `@column` | Current column number |
| `@type_name(T)` | Name of type T as Text |
| `@fields_of(T)` | List of fields of type T |
| `@variants_of(T)` | List of variants of sum type T |
| `@implements(T, P)` | Check if T implements protocol P |

### Pattern Testing

Instead of a `matches!()` macro, Verum uses the `is` operator:

```verum
if value is Some(x) { use(x); }
while status is Pending { poll(); }
let valid = input is ParseInt()(n) && n > 0;
```

### Format Strings

Instead of `format!()`, Verum uses `f"..."` literals:

```verum
let msg = f"Hello, {name}! You have {count} items.";
```

---

## License

See the LICENSE file for details.
