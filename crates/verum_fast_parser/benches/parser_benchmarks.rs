//! Performance benchmarks for verum_fast_parser
//!
//! Run with: cargo bench -p verum_fast_parser
//!
//! Benchmark categories:
//! - Lexer throughput (tokenization speed)
//! - Parser throughput (AST construction speed)
//! - Real-world file parsing
//! - Specific construct parsing (expressions, types, declarations)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use verum_ast::span::FileId;
use verum_fast_parser::FastParser;
use verum_lexer::Lexer;

// =============================================================================
// Test Inputs - Varying Sizes and Complexity
// =============================================================================

const SMALL_FUNCTION: &str = r#"
fn add(a: Int, b: Int) -> Int {
    a + b
}
"#;

const MEDIUM_FUNCTION: &str = r#"
/// Calculate factorial with tail recursion optimization
@inline
pub fn factorial(n: Int{>= 0}) -> Int {
    fn go(n: Int, acc: Int) -> Int {
        match n {
            0 => acc,
            n => go(n - 1, n * acc)
        }
    }
    go(n, 1)
}
"#;

const COMPLEX_FUNCTION: &str = r#"
/// Process a stream of data with error handling and async operations
@requires(data.len() > 0)
@ensures(result.is_ok() => result.unwrap().len() <= data.len())
pub async fn process_stream<T, E>(
    data: List<T>,
    transformer: fn(T) -> Result<T, E>,
    filter: fn(&T) -> Bool
) -> Result<List<T>, E>
where
    T: Clone + Send + Sync,
    E: Debug + Display
using [Logger, Metrics] {
    let mut results = List.new();

    for item in data {
        let transformed = transformer(item)?;
        if filter(&transformed) {
            results.push(transformed);
        }
    }

    provide Logger = SilentLogger.new() in {
        log_info(f"Processed {results.len()} items");
    };

    Ok(results)
}
"#;

const TYPE_DEFINITIONS: &str = r#"
type Point is {
    x: Float,
    y: Float,
};

type Maybe<T> is None | Some(T);

type Result<T, E> is Ok(T) | Err(E);

type BinaryTree<T> is
    | Leaf(T)
    | Node {
        value: T,
        left: Heap<BinaryTree<T>>,
        right: Heap<BinaryTree<T>>,
    };

type Iterator is protocol {
    type Item;
    fn next(&mut self) -> Maybe<Self.Item>;
    fn size_hint(&self) -> (Int, Maybe<Int>) { (0, None) }
};

type AsyncIterator is protocol extends Iterator {
    async fn next_async(&mut self) -> Maybe<Self.Item>;
};
"#;

const EXPRESSION_HEAVY: &str = r#"
fn expression_test() {
    // Arithmetic
    let a = 1 + 2 * 3 - 4 / 5 % 6;
    let b = (1 + 2) * (3 - 4) / (5 + 6);

    // Comparison and logical
    let c = a > b && b < 10 || a == b;
    let d = !c && (a >= 5 || b <= 3);

    // Pipeline
    let e = data
        |> filter(x => x > 0)
        |> map(x => x * 2)
        |> fold(0, |acc, x| acc + x);

    // Match expression
    let f = match value {
        Some(x) if x > 0 => x * 2,
        Some(x) => x,
        None => 0,
    };

    // Closures
    let g = |x: Int, y: Int| -> Int { x + y };
    let h = |x| x * x;

    // Collection literals
    let arr = [1, 2, 3, 4, 5];
    let tuple = (1, "hello", true);
    let record = Point { x: 1.0, y: 2.0 };

    // Method chains
    let result = list
        .iter()
        .filter(|x| x.is_valid())
        .map(|x| x.transform())
        .collect();
}
"#;

const PATTERN_MATCHING: &str = r#"
fn pattern_test(value: Value) -> Int {
    match value {
        // Literal patterns
        0 => 0,
        1 | 2 | 3 => 1,

        // Range patterns
        4..10 => 2,
        10..=20 => 3,

        // Tuple patterns
        (a, b) => a + b,
        (_, _, c) => c,

        // Record patterns
        Point { x: 0, y } => y,
        Point { x, y: 0 } => x,
        Point { x, y } if x == y => x,
        Point { .. } => 0,

        // Variant patterns
        Some(inner) => inner,
        None => 0,
        Ok(value) => value,
        Err(e) => handle_error(e),

        // Nested patterns
        Some(Point { x, y }) if x > 0 && y > 0 => x + y,

        // Array patterns
        [first, .., last] => first + last,
        [single] => single,
        [] => 0,

        // Reference patterns
        &x => x,
        &mut y => y,

        // Wildcard
        _ => -1,
    }
}
"#;

const ASYNC_CODE: &str = r#"
async fn async_operations() -> Result<Data, Error> using [Network, Database] {
    // Spawn concurrent tasks
    let task1 = spawn fetch_data("url1");
    let task2 = spawn fetch_data("url2");

    // Select from multiple channels
    let result = select {
        data = task1.await => process(data),
        data = task2.await => process(data),
        _ = timeout(5000).await => return Err(Error.timeout()),
    };

    // Structured concurrency with nursery
    nursery {
        spawn worker1();
        spawn worker2();
        spawn worker3();
    };

    // Async iteration
    for await item in stream {
        process_item(item).await?;
    }

    // Try with async
    let value = try {
        fetch().await?;
        parse().await?;
        validate().await?
    };

    Ok(result)
}
"#;

const PROOF_CONSTRUCTS: &str = r#"
theorem list_length_non_negative<T>:
    forall(list: List<T>) => list.len() >= 0
{
    proof by induction on list {
        case Nil => {
            assert(Nil.len() == 0);
            assert(0 >= 0);
        }
        case Cons(_, tail) => {
            have tail.len() >= 0 by induction_hypothesis;
            calc {
                Cons(_, tail).len()
                    == 1 + tail.len()
                    >= 1 + 0
                    >= 0
            }
        }
    }
}

lemma addition_commutative:
    forall(a: Int, b: Int) => a + b == b + a
{
    proof by axiom arithmetic_commutativity;
}

@requires(n >= 0)
@ensures(result >= 1)
@ensures(n > 0 => result == n * factorial(n - 1))
fn factorial(n: Int) -> Int {
    match n {
        0 => 1,
        n => n * factorial(n - 1)
    }
}
"#;

// Generate a large file with many items
fn generate_large_module(num_functions: usize) -> String {
    let mut result = String::new();

    for i in 0..num_functions {
        result.push_str(&format!(
            r#"
fn function_{i}(x: Int, y: Int) -> Int {{
    let a = x + y;
    let b = x * y;
    let c = match a {{
        0 => b,
        n if n > 0 => n + b,
        _ => b - a,
    }};
    c
}}
"#
        ));
    }

    result
}

// =============================================================================
// Benchmarks
// =============================================================================

fn lexer_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer_throughput");

    let inputs = [
        ("small", SMALL_FUNCTION),
        ("medium", MEDIUM_FUNCTION),
        ("complex", COMPLEX_FUNCTION),
        ("types", TYPE_DEFINITIONS),
        ("expressions", EXPRESSION_HEAVY),
        ("patterns", PATTERN_MATCHING),
        ("async", ASYNC_CODE),
        ("proofs", PROOF_CONSTRUCTS),
    ];

    for (name, source) in inputs {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("tokenize", name), source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let tokens: Vec<_> = lexer.collect();
                black_box(tokens)
            });
        });
    }

    // Large file benchmark
    let large_source = generate_large_module(100);
    group.throughput(Throughput::Bytes(large_source.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("tokenize", "large_100_functions"),
        &large_source,
        |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let tokens: Vec<_> = lexer.collect();
                black_box(tokens)
            });
        },
    );

    group.finish();
}

fn parser_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_throughput");

    let inputs = [
        ("small", SMALL_FUNCTION),
        ("medium", MEDIUM_FUNCTION),
        ("complex", COMPLEX_FUNCTION),
        ("types", TYPE_DEFINITIONS),
        ("expressions", EXPRESSION_HEAVY),
        ("patterns", PATTERN_MATCHING),
        ("async", ASYNC_CODE),
        ("proofs", PROOF_CONSTRUCTS),
    ];

    let parser = FastParser::new();

    for (name, source) in inputs {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        });
    }

    // Large file benchmark
    let large_source = generate_large_module(100);
    group.throughput(Throughput::Bytes(large_source.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("parse", "large_100_functions"),
        &large_source,
        |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        },
    );

    group.finish();
}

fn parse_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_expressions");

    let expressions = [
        ("simple_arithmetic", "1 + 2 * 3"),
        ("complex_arithmetic", "((1 + 2) * (3 - 4)) / ((5 + 6) % 7)"),
        ("pipeline_short", "x |> f |> g"),
        ("pipeline_long", "x |> filter(a => a > 0) |> map(a => a * 2) |> fold(0, |a, b| a + b)"),
        ("match_simple", "match x { 0 => a, _ => b }"),
        ("match_complex", "match x { Some(v) if v > 0 => v, Some(v) => -v, None => 0 }"),
        ("closure_simple", "|x| x + 1"),
        ("closure_typed", "|x: Int, y: Int| -> Int { x + y }"),
        ("method_chain", "obj.method1().method2().method3().result"),
        ("array_literal", "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"),
        ("record_literal", "Point { x: 1.0, y: 2.0, z: 3.0 }"),
        ("if_expr", "if cond { then_val } else { else_val }"),
        ("nested_if", "if a { if b { c } else { d } } else { e }"),
    ];

    let parser = FastParser::new();

    for (name, expr) in expressions {
        // Wrap expression in a function for parsing
        let source = format!("fn test() {{ let _ = {expr}; }}");
        group.bench_with_input(BenchmarkId::new("expr", name), &source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn parse_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_types");

    let types = [
        ("primitive", "fn test(x: Int) {}"),
        ("generic_simple", "fn test(x: List<Int>) {}"),
        ("generic_nested", "fn test(x: Map<Text, List<Int>>) {}"),
        ("function_type", "fn test(f: fn(Int, Int) -> Int) {}"),
        ("function_with_context", "fn test(f: fn(Int) -> Int using [Logger]) {}"),
        ("reference", "fn test(x: &Int, y: &mut Int) {}"),
        ("tuple", "fn test(x: (Int, Text, Bool)) {}"),
        ("array", "fn test(x: [Int; 10]) {}"),
        ("refinement", "fn test(x: Int{x > 0 && x < 100}) {}"),
        ("where_clause", "fn test<T>(x: T) where T: Clone + Send + Sync {}"),
        ("complex", "fn test<T, E>(x: Result<List<T>, E>) -> Maybe<T> where T: Clone, E: Debug {}"),
    ];

    let parser = FastParser::new();

    for (name, source) in types {
        group.bench_with_input(BenchmarkId::new("type", name), &source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn parse_declarations(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_declarations");

    let declarations = [
        ("fn_simple", "fn add(a: Int, b: Int) -> Int { a + b }"),
        (
            "fn_async",
            "pub async fn fetch() -> Result<Data, Error> using [Network] { Ok(data) }",
        ),
        ("type_record", "type Point is { x: Float, y: Float };"),
        ("type_variant", "type Maybe<T> is None | Some(T);"),
        (
            "type_protocol",
            "type Iterator is protocol { type Item; fn next(&mut self) -> Maybe<Self.Item>; };",
        ),
        ("impl_simple", "implement Point { fn new(x: Float, y: Float) -> Point { Point { x, y } } }"),
        (
            "impl_protocol",
            "implement Iterator for Range { type Item = Int; fn next(&mut self) -> Maybe<Int> { None } }",
        ),
        ("const_def", "const MAX_SIZE: Int = 1024;"),
        ("static_def", "static mut COUNTER: Int = 0;"),
        ("module_def", "module utils { pub fn helper() {} }"),
    ];

    let parser = FastParser::new();

    for (name, source) in declarations {
        group.bench_with_input(BenchmarkId::new("decl", name), &source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn scaling_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");

    // Test how parser scales with file size
    for num_functions in [10, 50, 100, 200, 500] {
        let source = generate_large_module(num_functions);
        let lines = source.lines().count();

        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(
            BenchmarkId::new("functions", num_functions),
            &source,
            |b, source| {
                let file_id = FileId::new(0);
                let parser = FastParser::new();
                b.iter(|| {
                    let lexer = Lexer::new(black_box(source), file_id);
                    let result = parser.parse_module(lexer, file_id);
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn error_recovery_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_recovery");

    // Code with various errors to test recovery performance
    let error_inputs = [
        (
            "missing_semicolon",
            r#"fn test() { let x = 1 let y = 2; }"#,
        ),
        (
            "unclosed_brace",
            r#"fn test() { if true { let x = 1; }"#,
        ),
        (
            "invalid_token",
            r#"fn test() { let @ = 1; let y = 2; }"#,
        ),
        (
            "multiple_errors",
            r#"fn test() { let x = let y = 2 match { } }"#,
        ),
    ];

    let parser = FastParser::new();

    for (name, source) in error_inputs {
        group.bench_with_input(BenchmarkId::new("recover", name), &source, |b, source| {
            let file_id = FileId::new(0);
            b.iter(|| {
                let lexer = Lexer::new(black_box(source), file_id);
                let result = parser.parse_module(lexer, file_id);
                black_box(result)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    lexer_throughput,
    parser_throughput,
    parse_expressions,
    parse_types,
    parse_declarations,
    scaling_benchmark,
    error_recovery_benchmark,
);

criterion_main!(benches);
