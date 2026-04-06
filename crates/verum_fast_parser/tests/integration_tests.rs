#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Comprehensive integration tests for the Verum parser.
//
// This test suite focuses on:
// - Complete program parsing (full modules with multiple declarations)
// - Context system parsing (dependency injection with provide/using)
// - CBGR-specific constructs
// - Advanced refinement types
// - Edge cases and corner cases not covered in other tests
// - Real-world code patterns
// - Cross-feature interactions
//
// These tests complement the existing unit tests by testing how different
// features interact when combined in realistic scenarios.

use verum_ast::{FileId, Module};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse a complete module.
fn parse_module(source: &str) -> Result<Module, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|_| panic!("Failed to parse:\n{}", source));
}

/// Helper to check if parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_module(source).is_err(),
        "Expected parse failure for:\n{}",
        source
    );
}

// ============================================================================
// SECTION 1: COMPLETE PROGRAMS (~20 tests)
// ============================================================================

#[test]
fn test_minimal_program() {
    assert_parses("fn main() {}");
}

#[test]
fn test_hello_world() {
    assert_parses(
        r#"
        fn main() {
            print("Hello, World!");
        }
    "#,
    );
}

#[test]
fn test_fibonacci_function() {
    assert_parses(
        r#"
        fn fib(n: Int{>= 0}) -> Int{>= 0} {
            if n <= 1 {
                n
            } else {
                fib(n - 1) + fib(n - 2)
            }
        }
    "#,
    );
}

#[test]
fn test_module_with_multiple_functions() {
    assert_parses(
        r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn mul(x: Int, y: Int) -> Int {
            x * y
        }

        fn main() {
            let result = add(mul(2, 3), 4);
            print(result);
        }
    "#,
    );
}

#[test]
fn test_module_with_types_and_functions() {
    assert_parses(
        r#"
        type Point is {
            x: Float,
            y: Float
        };

        fn distance(p1: Point, p2: Point) -> Float{>= 0.0} {
            let dx = p2.x - p1.x;
            let dy = p2.y - p1.y;
            (dx * dx + dy * dy).sqrt()
        }
    "#,
    );
}

#[test]
fn test_module_with_protocols() {
    assert_parses(
        r#"
        type Drawable is protocol {
            fn draw(&self);
        };

        type Circle is {
            radius: Float{> 0.0}
        };

        implement Drawable for Circle {
            fn draw(&self) {
                print("Drawing circle");
            }
        }
    "#,
    );
}

#[test]
fn test_module_with_mounts() {
    assert_parses(
        r#"
        mount std.collections.List;
        mount std.io;

        fn main() {
            let v: List<Int> = List.new();
            io.print(v);
        }
    "#,
    );
}

#[test]
fn test_module_with_constants() {
    assert_parses(
        r#"
        const PI: Float = 3.14159;
        const MAX_SIZE: Int{> 0} = 1000;

        fn circle_area(radius: Float) -> Float {
            PI * radius * radius
        }
    "#,
    );
}

#[test]
fn test_module_with_generics() {
    assert_parses(
        r#"
        type Box<T> is { value: T };

        fn unbox<T>(b: Box<T>) -> T {
            b.value
        }

        fn main() {
            let b = Box { value: 42 };
            let x = unbox(b);
        }
    "#,
    );
}

#[test]
fn test_module_with_variants() {
    assert_parses(
        r#"
        type Option<T> is Some(T) | None;

        fn unwrap_or<T>(opt: Option<T>, default: T) -> T {
            match opt {
                Some(value) => value,
                None => default
            }
        }
    "#,
    );
}

#[test]
fn test_recursive_data_structure() {
    assert_parses(
        r#"
        type List<T> is
            Cons { head: T, tail: Box<List<T>> }
            | Nil;

        fn length<T>(list: List<T>) -> Int{>= 0} {
            match list {
                Cons { head: _, tail } => 1 + length(*tail),
                Nil => 0
            }
        }
    "#,
    );
}

#[test]
fn test_binary_tree() {
    assert_parses(
        r#"
        type Tree<T> is
            Leaf(T)
            | Branch {
                left: Box<Tree<T>>,
                right: Box<Tree<T>>
            };

        fn depth<T>(tree: Tree<T>) -> Int{>= 0} {
            match tree {
                Leaf(_) => 1,
                Branch { left, right } => {
                    let l = depth(*left);
                    let r = depth(*right);
                    1 + if l > r { l } else { r }
                }
            }
        }
    "#,
    );
}

#[test]
fn test_result_type_usage() {
    assert_parses(
        r#"
        type Result<T, E> is Ok(T) | Err(E);

        fn divide(x: Float, y: Float{!= 0.0}) -> Result<Float, String> {
            Ok(x / y)
        }

        fn main() {
            let result = divide(10.0, 2.0);
            match result {
                Ok(value) => print(value),
                Err(err) => print(err)
            }
        }
    "#,
    );
}

#[test]
fn test_iterator_pattern() {
    assert_parses(
        r#"
        type Iterator is protocol {
            type Item;
            fn next(&mut self) -> Option<Self.Item>;
        };

        type Range is {
            start: Int,
            end: Int,
            current: Int
        };

        implement Iterator for Range {
            type Item is Int;

            fn next(&mut self) -> Option<Int> {
                if self.current < self.end {
                    let value = self.current;
                    self.current += 1;
                    Some(value)
                } else {
                    None
                }
            }
        }
    "#,
    );
}

#[test]
fn test_builder_pattern() {
    assert_parses(
        r#"
        type Config is {
            host: String,
            port: Int{1 <= it && it <= 65535},
            timeout: Int{> 0}
        };

        type ConfigBuilder is {
            host: Option<String>,
            port: Option<Int>,
            timeout: Option<Int>
        };

        implement ConfigBuilder {
            fn new() -> Self {
                ConfigBuilder {
                    host: None,
                    port: None,
                    timeout: None
                }
            }

            fn host(self, h: String) -> Self {
                ConfigBuilder { host: Some(h), ..self }
            }

            fn build(self) -> Option<Config> {
                match (self.host, self.port, self.timeout) {
                    (Some(h), Some(p), Some(t)) => Some(Config {
                        host: h,
                        port: p,
                        timeout: t
                    }),
                    _ => None
                }
            }
        }
    "#,
    );
}

#[test]
fn test_state_machine() {
    assert_parses(
        r#"
        type State is
            Idle
            | Processing { progress: Float{0.0 <= it && it <= 100.0} }
            | Complete { result: Int }
            | Error { message: String };

        fn transition(state: State, event: String) -> State {
            match (state, event) {
                (Idle, "start") => Processing { progress: 0.0 },
                (Processing { progress }, "update") => Processing { progress: progress + 10.0 },
                (Processing { .. }, "finish") => Complete { result: 42 },
                _ => Error { message: "Invalid transition" }
            }
        }
    "#,
    );
}

#[test]
fn test_functional_composition() {
    assert_parses(
        r#"
        fn compose<A, B, C>(f: fn(B) -> C, g: fn(A) -> B) -> fn(A) -> C {
            |x| f(g(x))
        }

        fn add_one(x: Int) -> Int { x + 1 }
        fn double(x: Int) -> Int { x * 2 }

        fn main() {
            let f = compose(double, add_one);
            let result = f(5);
        }
    "#,
    );
}

#[test]
fn test_map_filter_reduce() {
    assert_parses(
        r#"
        fn process_data(numbers: Vec<Int>) -> Int {
            numbers
                |> filter(|x| x > 0)
                |> map(|x| x * 2)
                |> fold(0, |acc, x| acc + x)
        }
    "#,
    );
}

#[test]
fn test_error_handling_chain() {
    assert_parses(
        r#"
        fn read_config(path: String) -> Result<Config, Error> {
            let contents = read_file(path)?;
            let parsed = parse_json(contents)?;
            validate_config(parsed)
        }
    "#,
    );
}

#[test]
fn test_async_await_pattern() {
    assert_parses(
        r#"
        async fn fetch_data(url: String) -> Result<Data, Error> using [IO, Network] {
            let response = http_get(url).await?;
            let data = parse_response(response).await?;
            Ok(data)
        }
    "#,
    );
}

// ============================================================================
// SECTION 2: CONTEXT SYSTEM TESTS (~15 tests)
// Verum uses Context System (provide/using) for dependency injection.
// This is NOT algebraic effects. Verum uses capability-based DI (context system)
// ============================================================================

#[test]
fn test_function_with_single_context() {
    assert_parses(
        r#"
        fn read_file(path: String) -> String using [IO] {
            ""
        }
    "#,
    );
}

#[test]
fn test_function_with_multiple_contexts() {
    assert_parses(
        r#"
        fn query_database(sql: String) -> Result<Data, Error> using [IO, Database, Logging] {
            Ok(Data {})
        }
    "#,
    );
}

#[test]
fn test_context_propagation() {
    assert_parses(
        r#"
        fn read() -> String using [IO] { "" }

        fn process() -> Int using [IO] {
            let data = read();
            data.len()
        }
    "#,
    );
}

#[test]
fn test_context_in_async() {
    assert_parses(
        r#"
        async fn fetch() -> Data using [IO, Network] {
            Data {}
        }
    "#,
    );
}

#[test]
fn test_context_in_closure() {
    assert_parses(
        r#"
        fn main() {
            let f = |x| using [IO] -> String { read_file(x) };
        }
    "#,
    );
}

#[test]
fn test_closure_with_multiple_contexts() {
    assert_parses(
        r#"
        fn main() {
            let f = |x, y| using [IO, Database, Logger] -> Result<String> {
                log("Starting");
                let data = db_query(x);
                write_file(y, data)
            };
        }
    "#,
    );
}

#[test]
fn test_closure_with_single_context() {
    assert_parses(
        r#"
        fn main() {
            let f = |x| using [State] -> Int {
                get_state() + x
            };
        }
    "#,
    );
}

#[test]
fn test_closure_context_without_return_type() {
    assert_parses(
        r#"
        fn main() {
            let f = |x| using [IO] { read_file(x) };
        }
    "#,
    );
}

#[test]
fn test_async_closure_with_context() {
    assert_parses(
        r#"
        fn main() {
            let f = async |x| using [IO] -> String { read_file(x).await };
        }
    "#,
    );
}

#[test]
fn test_move_closure_with_context() {
    assert_parses(
        r#"
        fn main() {
            let data = "test";
            let f = move |x| using [IO] -> String {
                format("{}{}", data, read_file(x))
            };
        }
    "#,
    );
}

#[test]
fn test_empty_closure_with_context() {
    assert_parses(
        r#"
        fn main() {
            let f = || using [IO] -> String { read_stdin() };
        }
    "#,
    );
}

#[test]
fn test_context_polymorphism() {
    assert_parses(
        r#"
        fn with_effect<E>(f: fn() -> Int using [E]) -> Int using [E] {
            f()
        }
    "#,
    );
}

// NOTE: Algebraic effect tests removed - Verum uses Context System (provide/using),
// NOT algebraic effects (effect/handle/resume). Verum contexts are dependency injection
// IMPORTANT: NEVER call Properties "Effects" - Verum has no algebraic effects.
// For context testing, see test_function_with_multiple_effects and context-specific tests.

#[test]
fn test_context_inference() {
    assert_parses(
        r#"
        fn main() {
            let f = || { read_file("test.txt") };
        }
    "#,
    );
}

#[test]
fn test_pure_functions() {
    assert_parses(
        r#"
        fn pure_add(x: Int, y: Int) -> Int {
            x + y
        }

        fn pure_map<T, U>(f: fn(T) -> U, xs: Vec<T>) -> Vec<U> {
            xs.map(f)
        }
    "#,
    );
}

#[test]
fn test_context_subtyping() {
    assert_parses(
        r#"
        fn with_io() -> Int using [IO] { 42 }
        fn with_io_and_net() -> Int using [IO, Network] { with_io() }
    "#,
    );
}

#[test]
fn test_context_in_types() {
    assert_parses(
        r#"
        type Handler is fn(String) -> Result<(), Error> using [IO];

        fn run(h: Handler) -> Result<(), Error> using [IO] {
            h("test")
        }
    "#,
    );
}

#[test]
fn test_context_abstraction() {
    assert_parses(
        r#"
        fn abstract_io<R>(f: fn() -> R using [IO]) -> R {
            f()
        }
    "#,
    );
}

// Parser limitation: context lists only support concrete type paths, not type
// parameters. Row polymorphism for contexts (using type parameters like [E] in
// `using [E]`) is not yet implemented.
// Context system: `context Name { }`, `provide Ctx = impl;`, `using [Ctx]`

#[test]
fn test_context_row_polymorphism_simple() {
    // Test with type parameter in inner context list
    assert_parses(
        r#"
        fn elevate<E>(f: fn() -> Int using [E]) -> Int {
            f()
        }
    "#,
    );
}

#[test]
fn test_context_row_polymorphism_outer() {
    // Test with type parameters in outer context list
    assert_parses(
        r#"
        fn elevate<E>() -> Int using [E] {
            42
        }
    "#,
    );
}

#[test]
fn test_context_row_polymorphism_full() {
    // Full row polymorphism test
    assert_parses(
        r#"
        fn elevate<E, R>(f: fn() -> R using [E]) -> Result<R, Error> using [E, Error] {
            Ok(f())
        }
    "#,
    );
}

// ============================================================================
// SECTION 3: CBGR (COMPILE-TIME BORROW & GEN REFS) TESTS (~15 tests)
// ============================================================================

#[test]
fn test_cbgr_basic_reference() {
    assert_parses(
        r#"
        fn borrow(x: &Int) -> Int {
            *x
        }
    "#,
    );
}

#[test]
fn test_cbgr_mutable_reference() {
    assert_parses(
        r#"
        fn increment(x: &mut Int) {
            *x = *x + 1;
        }
    "#,
    );
}

#[test]
fn test_cbgr_ownership() {
    assert_parses(
        r#"
        fn consume(x: %Int) -> Int {
            x
        }
    "#,
    );
}

#[test]
fn test_cbgr_mutable_ownership() {
    assert_parses(
        r#"
        fn consume_mut(x: %mut Int) -> Int {
            x
        }
    "#,
    );
}

#[test]
fn test_cbgr_multiple_borrows() {
    assert_parses(
        r#"
        fn sum(x: &Int, y: &Int) -> Int {
            *x + *y
        }
    "#,
    );
}

#[test]
fn test_cbgr_nested_references() {
    assert_parses(
        r#"
        fn nested(x: &&Int) -> Int {
            **x
        }
    "#,
    );
}

#[test]
fn test_cbgr_triple_nested_references() {
    assert_parses(
        r#"
        fn triple_nested(x: &&&Int) -> Int {
            ***x
        }
    "#,
    );
}

#[test]
fn test_cbgr_nested_reference_with_mut() {
    assert_parses(
        r#"
        fn nested_mut(x: &&mut Int) -> Int {
            **x
        }
    "#,
    );
}

#[test]
fn test_cbgr_nested_checked_references() {
    assert_parses(
        r#"
        fn nested_checked(x: &checked &checked Int) -> Int {
            **x
        }
    "#,
    );
}

#[test]
fn test_cbgr_nested_mixed_references() {
    assert_parses(
        r#"
        fn nested_mixed(x: &checked &Int) -> Int {
            **x
        }
    "#,
    );
}

#[test]
fn test_cbgr_double_reference_expression() {
    assert_parses(
        r#"
        fn test(x: Int) {
            let y = &&x;
            let z = **y;
        }
    "#,
    );
}

#[test]
fn test_cbgr_reference_in_struct() {
    assert_parses(
        r#"
        type Borrowed<'a> is {
            data: &'a Int
        };
    "#,
    );
}

#[test]
fn test_cbgr_lifetime_annotations() {
    assert_parses(
        r#"
        fn longest<'a>(x: &'a String, y: &'a String) -> &'a String {
            if x.len() > y.len() { x } else { y }
        }
    "#,
    );
}

#[test]
fn test_cbgr_lifetime_elision() {
    assert_parses(
        r#"
        fn first(s: &String) -> &String {
            s
        }
    "#,
    );
}

#[test]
fn test_cbgr_generational_reference() {
    assert_parses(
        r#"
        type ThinRef<T> is {
            index: Int,
            generation: Int
        };

        fn deref<T>(r: ThinRef<T>, arena: &Arena<T>) -> Option<&T> {
            arena.get(r)
        }
    "#,
    );
}

#[test]
fn test_cbgr_arena_allocator() {
    assert_parses(
        r#"
        type Arena<T> is {
            data: Vec<Option<T>>,
            generations: Vec<Int>
        };

        implement<T> Arena<T> {
            fn alloc(&mut self, value: T) -> ThinRef<T> {
                ThinRef { index: 0, generation: 0 }
            }

            fn free(&mut self, r: ThinRef<T>) {
                self.data[r.index] = None;
                self.generations[r.index] += 1;
            }
        }
    "#,
    );
}

#[test]
fn test_cbgr_borrow_checker_interaction() {
    assert_parses(
        r#"
        fn test() {
            let mut x = 42;
            let r1 = &x;
            let r2 = &x;
            let sum = *r1 + *r2;
        }
    "#,
    );
}

#[test]
fn test_cbgr_reference_counting() {
    assert_parses(
        r#"
        type Rc<T> is {
            ptr: %T,
            count: Int
        };

        fn clone_rc<T>(rc: &Rc<T>) -> Rc<T> {
            Rc { ptr: rc.ptr, count: rc.count + 1 }
        }
    "#,
    );
}

#[test]
fn test_cbgr_weak_references() {
    assert_parses(
        r#"
        type Weak<T> is {
            index: Int,
            generation: Int
        };

        fn upgrade<T>(weak: Weak<T>) -> Option<ThinRef<T>> {
            None
        }
    "#,
    );
}

#[test]
fn test_cbgr_pattern_matching_refs() {
    assert_parses(
        r#"
        fn match_ref(opt: &Option<Int>) -> Int {
            match opt {
                &Some(x) => x,
                &None => 0
            }
        }
    "#,
    );
}

// ============================================================================
// SECTION 4: ADVANCED REFINEMENT TYPES (~20 tests)
// ============================================================================

#[test]
fn test_refinement_positive_int() {
    assert_parses(
        r#"
        fn factorial(n: Int{>= 0}) -> Int{> 0} {
            if n == 0 { 1 } else { n * factorial(n - 1) }
        }
    "#,
    );
}

#[test]
fn test_refinement_bounded_range() {
    assert_parses(
        r#"
        fn percentage(value: Float{0.0 <= it && it <= 100.0}) -> String {
            "ok"
        }
    "#,
    );
}

#[test]
fn test_refinement_non_empty_string() {
    assert_parses(
        r#"
        fn process(s: String{len(it) > 0}) -> Char {
            s.chars().first()
        }
    "#,
    );
}

#[test]
fn test_refinement_email_validation() {
    assert_parses(
        r#"
        type Email is String{it.contains('@') && it.len() > 3};

        fn send_email(to: Email, message: String) {
            print(to);
        }
    "#,
    );
}

#[test]
fn test_refinement_port_number() {
    assert_parses(
        r#"
        type Port is Int{1 <= it && it <= 65535};

        fn connect(host: String, port: Port) {
            print(port);
        }
    "#,
    );
}

#[test]
fn test_refinement_sorted_list() {
    assert_parses(
        r#"
        type SortedList<T> is Vec<T>{is_sorted(it)};

        fn binary_search<T>(list: SortedList<T>, target: T) -> Option<Int> {
            None
        }
    "#,
    );
}

#[test]
fn test_refinement_non_empty_vec() {
    assert_parses(
        r#"
        fn first<T>(v: Vec<T>{len(it) > 0}) -> T {
            v[0]
        }
    "#,
    );
}

#[test]
fn test_refinement_matrix_dimensions() {
    assert_parses(
        r#"
        type Matrix is Vec<Vec<Float>>{
            len(it) > 0 &&
            it.all(|row| row.len() == it[0].len())
        };
    "#,
    );
}

#[test]
fn test_refinement_division_safety() {
    assert_parses(
        r#"
        fn safe_divide(x: Float, y: Float{it != 0.0}) -> Float {
            x / y
        }
    "#,
    );
}

#[test]
fn test_refinement_array_bounds() {
    assert_parses(
        r#"
        fn get_element<T>(arr: Vec<T>, index: Int{0 <= it && it < len(arr)}) -> T {
            arr[index]
        }
    "#,
    );
}

#[test]
fn test_refinement_dependent_types() {
    assert_parses(
        r#"
        fn take<T>(n: Int{>= 0}, vec: Vec<T>{len(it) >= n}) -> Vec<T> {
            vec[0..n]
        }
    "#,
    );
}

#[test]
fn test_refinement_state_invariants() {
    assert_parses(
        r#"
        type Counter is {
            value: Int{>= 0},
            max: Int{> 0},
            valid: Bool{value <= max}
        };
    "#,
    );
}

#[test]
fn test_refinement_type_level_arithmetic() {
    assert_parses(
        r#"
        fn add_positive(x: Int{> 0}, y: Int{> 0}) -> Int{> 0} {
            x + y
        }
    "#,
    );
}

#[test]
fn test_refinement_string_length_bounds() {
    assert_parses(
        r#"
        type Username is String{3 <= len(it) && len(it) <= 20};
        type Password is String{8 <= len(it) && len(it) <= 128};

        fn register(username: Username, password: Password) {}
    "#,
    );
}

#[test]
fn test_refinement_file_path_validation() {
    assert_parses(
        r#"
        type AbsolutePath is String{it.starts_with('/')};
        type RelativePath is String{!it.starts_with('/')};
    "#,
    );
}

#[test]
fn test_refinement_rgb_color() {
    assert_parses(
        r#"
        type RGB is {
            r: Int{0 <= it && it <= 255},
            g: Int{0 <= it && it <= 255},
            b: Int{0 <= it && it <= 255}
        };
    "#,
    );
}

#[test]
fn test_refinement_temperature() {
    assert_parses(
        r#"
        type Celsius is Float{it >= -273.15};
        type Kelvin is Float{it >= 0.0};

        fn celsius_to_kelvin(c: Celsius) -> Kelvin {
            c + 273.15
        }
    "#,
    );
}

#[test]
fn test_refinement_probability() {
    assert_parses(
        r#"
        type Probability is Float{0.0 <= it && it <= 1.0};

        fn complement(p: Probability) -> Probability {
            1.0 - p
        }
    "#,
    );
}

#[test]
fn test_refinement_complex_predicate() {
    assert_parses(
        r#"
        type ValidDate is {
            year: Int{1900 <= it && it <= 2100},
            month: Int{1 <= it && it <= 12},
            day: Int{1 <= it && it <= days_in_month(month, year)}
        };
    "#,
    );
}

#[test]
fn test_refinement_graph_acyclicity() {
    assert_parses(
        r#"
        type AcyclicGraph<T> is {
            nodes: Vec<T>,
            edges: Vec<(Int, Int)>,
            is_acyclic: Bool{is_dag(edges)}
        };
    "#,
    );
}

// ============================================================================
// SECTION 5: EDGE CASES AND CORNER CASES (~25 tests)
// ============================================================================

#[test]
fn test_empty_module() {
    assert_parses("");
}

#[test]
fn test_whitespace_only() {
    assert_parses("   \n\n\t\t  ");
}

#[test]
fn test_comments_only() {
    assert_parses(
        r#"
        // This is a comment
        /* This is a block comment */
    "#,
    );
}

#[test]
fn test_deeply_nested_expressions() {
    assert_parses(
        r#"
        fn main() {
            let x = ((((((1 + 2) * 3) - 4) / 5) % 6) ** 7);
        }
    "#,
    );
}

#[test]
fn test_deeply_nested_types() {
    assert_parses(
        r#"
        type Deep is Option<Result<Vec<HashMap<String, Box<Option<Int>>>>, Error>>;
    "#,
    );
}

#[test]
fn test_long_identifier_chains() {
    assert_parses(
        r#"
        fn main() {
            let x = very.long.chain.of.method.calls.that.goes.on.forever();
        }
    "#,
    );
}

#[test]
fn test_maximum_tuple_size() {
    assert_parses(
        r#"
        fn main() {
            let t = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
        }
    "#,
    );
}

#[test]
fn test_unicode_identifiers() {
    assert_parses(
        r#"
        fn 函数() {
            let 变量 = 42;
        }
    "#,
    );
}

#[test]
fn test_unicode_strings() {
    assert_parses(
        r#"
        fn main() {
            let s = "Hello, 世界! 🌍";
        }
    "#,
    );
}

#[test]
fn test_raw_multiline_string_literals() {
    // Note: The old r"..." syntax has been removed.
    // Use triple-quoted """...""" for raw strings.
    assert_parses(
        r#"
        fn main() {
            let s = """This is a raw string with \n not interpreted""";
        }
    "#,
    );
}

#[test]
fn test_multiline_strings() {
    assert_parses(
        r#"
        fn main() {
            let s = "This is a
                     multiline
                     string";
        }
    "#,
    );
}

#[test]
fn test_empty_blocks_everywhere() {
    assert_parses(
        r#"
        fn main() {
            if true {} else {}
            match x { _ => {} }
            loop {}
        }
    "#,
    );
}

#[test]
fn test_trailing_commas_everywhere() {
    assert_parses(
        r#"
        fn f(x: Int, y: Int,) {
            let t = (1, 2, 3,);
            let a = [1, 2, 3,];
            foo(1, 2, 3,);
        }
    "#,
    );
}

#[test]
fn test_underscore_in_numbers() {
    assert_parses(
        r#"
        fn main() {
            let x = 1_000_000;
            let y = 3.141_592_653;
        }
    "#,
    );
}

#[test]
fn test_hex_oct_bin_literals() {
    assert_parses(
        r#"
        fn main() {
            let h = 0xFF;
            let o = 0o77;
            let b = 0b1010;
        }
    "#,
    );
}

#[test]
fn test_scientific_notation() {
    assert_parses(
        r#"
        fn main() {
            let x = 1e10;
            let y = 2.5e-3;
        }
    "#,
    );
}

#[test]
fn test_expression_as_statement_everywhere() {
    assert_parses(
        r#"
        fn main() {
            1 + 2;
            foo();
            x.y;
            if true { 1 } else { 2 };
        }
    "#,
    );
}

#[test]
fn test_redundant_parentheses() {
    assert_parses(
        r#"
        fn main() {
            let x = (((((42)))));
        }
    "#,
    );
}

#[test]
fn test_pattern_in_closure_params() {
    assert_parses(
        r#"
        fn main() {
            let f = |(x, y)| x + y;
            let g = |Point { x, y }| x + y;
        }
    "#,
    );
}

#[test]
fn test_nested_closures() {
    assert_parses(
        r#"
        fn main() {
            let f = |x| |y| |z| x + y + z;
        }
    "#,
    );
}

#[test]
fn test_closure_with_move() {
    assert_parses(
        r#"
        fn main() {
            let x = 42;
            let f = move |y| x + y;
        }
    "#,
    );
}

#[test]
fn test_move_closures_various_forms() {
    assert_parses(
        r#"
        fn main() {
            let x = 42;

            // Simple move closure with one parameter
            let f1 = move |y| x + y;

            // Move closure with multiple parameters
            let f2 = move |a, b| a + b + x;

            // Move closure with no parameters
            let f3 = move || x * 2;

            // Move closure with type annotations
            let f4 = move |y: Int| -> Int { x + y };

            // Async move closure
            let f5 = async move |x| { x + 1 };
        }
    "#,
    );
}

#[test]
fn test_generic_where_clauses() {
    assert_parses(
        r#"
        fn foo<T, U>(x: T, y: U) where
            T: Clone + Display,
            U: Debug + Default
        {}
    "#,
    );
}

#[test]
fn test_associated_type_bounds() {
    assert_parses(
        r#"
        fn foo<T>(x: T) where
            T: Iterator,
            T.Item: Display
        {}
    "#,
    );
}

// Note: Higher-ranked trait bounds (for<'a>) are NOT part of Verum grammar.
// Verum does not have Rust-style lifetimes.

#[test]
fn test_verum_style_macro_invocations() {
    // Verum uses @macro_name(...) syntax, not Rust's macro!(...) syntax
    // Standard I/O uses print(...) not println!(...)
    assert_parses(
        r#"
        fn main() {
            print("Hello");
            let items = [1, 2, 3];
            assert(true);
        }
    "#,
    );
}

// ============================================================================
// SECTION 6: ERROR RECOVERY (~15 tests)
// ============================================================================

#[test]
fn test_missing_semicolon_is_error() {
    // VCS E010: Missing semicolon after let statement is an error
    // Semicolons are only optional at block end
    assert_fails(
        r#"
        fn main() {
            let x = 1
            let y = 2;
        }
    "#,
    );
}

#[test]
fn test_semicolon_optional_at_block_end() {
    // Semicolon is optional at end of block
    assert_parses(
        r#"
        fn main() {
            let x = 1
        }
    "#,
    );
}

#[test]
fn test_unmatched_delimiter_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x = [1, 2, 3;
        }
    "#,
    );
}

#[test]
fn test_invalid_token_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x = @ 42;
        }
    "#,
    );
}

#[test]
fn test_incomplete_expression_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x = 1 +;
        }
    "#,
    );
}

#[test]
fn test_missing_type_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x: = 42;
        }
    "#,
    );
}

#[test]
fn test_invalid_pattern_recovery() {
    assert_fails(
        r#"
        fn main() {
            let 1 + 2 = 3;
        }
    "#,
    );
}

#[test]
fn test_unclosed_string_recovery() {
    // The lexer's parse_string function (token.rs:1192) returns None for unclosed strings,
    // which logos converts to an error token. The parser should fail when encountering
    // invalid tokens from the lexer.
    assert_fails(
        r#"
        fn main() {
            let s = "unclosed;
        }
    "#,
    );
}

#[test]
fn test_unexpected_token_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x = 42 43;
        }
    "#,
    );
}

#[test]
fn test_missing_function_body_allowed() {
    // NOTE: Per VCS spec, function declarations without bodies require extern keyword.
    // This is valid for:
    // - Extern function declarations (FFI)
    // - Protocol method signatures (within protocol bodies)
    // The body is represented as Maybe.None in the AST.
    assert_parses(
        r#"
        extern fn foo();
    "#,
    );
}

#[test]
fn test_invalid_generic_syntax_recovery() {
    assert_fails(
        r#"
        fn foo<T<U>>() {}
    "#,
    );
}

#[test]
fn test_mismatched_delimiter_types() {
    assert_fails(
        r#"
        fn main() {
            let x = [1, 2, 3);
        }
    "#,
    );
}

#[test]
fn test_double_operator_recovery() {
    assert_fails(
        r#"
        fn main() {
            let x = 1 ++ 2;
        }
    "#,
    );
}

#[test]
fn test_invalid_refinement_syntax() {
    assert_fails(
        r#"
        fn foo(x: Int{{{}>>) {}
    "#,
    );
}

#[test]
fn test_incomplete_match_arm() {
    // Note: With `=>` as an infix operator for logical implication,
    // `1 => 2 => 3` is now valid: pattern `1` with body expression `2 => 3`.
    // Test a genuinely incomplete arm that has no body at all.
    assert_fails(
        r#"
        fn main() {
            match x {
                1 =>
            }
        }
    "#,
    );
}

#[test]
fn test_invalid_lifetime_syntax() {
    assert_fails(
        r#"
        fn foo<'a 'b>(x: &'a Int) {}
    "#,
    );
}

// ============================================================================
// SECTION 7: REAL-WORLD PATTERNS (~10 tests)
// ============================================================================

#[test]
fn test_web_server_route() {
    assert_parses(
        r#"
        type Request is {
            method: String,
            path: String,
            body: Option<String>
        };

        type Response is {
            status: Int{100 <= it && it <= 599},
            body: String
        };

        fn handle_request(req: Request) -> Response using [IO] {
            match req.path {
                "/api/users" => get_users(),
                "/api/posts" => get_posts(),
                _ => Response { status: 404, body: "Not Found" }
            }
        }
    "#,
    );
}

#[test]
fn test_json_parser() {
    assert_parses(
        r#"
        type Json is
            Null
            | Bool(Bool)
            | Number(Float)
            | String(String)
            | Array(Vec<Json>)
            | Object(HashMap<String, Json>);

        fn parse_json(input: String) -> Result<Json, ParseError> {
            Ok(Json.Null)
        }
    "#,
    );
}

#[test]
fn test_database_query_builder() {
    // Updated to use Verum semantic types (List instead of Vec) and list literal [] instead of vec![]
    // Note: Using `choose` instead of `select` because `select` is a reserved keyword in Verum (async select)
    assert_parses(
        r#"
        type Query is {
            table: String,
            columns: List<String>,
            conditions: List<Condition>,
            limit: Option<Int{> 0}>
        };

        implement Query {
            fn choose(columns: List<String>) -> Self {
                Query {
                    table: "",
                    columns,
                    conditions: [],
                    limit: None
                }
            }

            fn from(self, table: String) -> Self {
                Query { table, ..self }
            }

            fn where_clause(self, cond: Condition) -> Self {
                let mut conditions = self.conditions;
                conditions.push(cond);
                Query { conditions, ..self }
            }
        }
    "#,
    );
}

#[test]
fn test_lexer_implementation() {
    assert_parses(
        r#"
        type Token is
            Number(Int)
            | Ident(String)
            | Plus
            | Minus
            | Star
            | Slash
            | Eof;

        type Lexer is {
            input: String,
            pos: Int{>= 0},
            current: Option<Char>
        };

        implement Lexer {
            fn new(input: String) -> Self {
                Lexer { input, pos: 0, current: None }
            }

            fn next_token(&mut self) -> Token {
                Token.Eof
            }
        }
    "#,
    );
}

#[test]
fn test_config_parser() {
    assert_parses(
        r#"
        type Config is {
            server: ServerConfig,
            database: DatabaseConfig,
            logging: LoggingConfig
        };

        fn load_config(path: String) -> Result<Config, Error> using [IO] {
            let contents = read_file(path)?;
            parse_toml(contents)
        }
    "#,
    );
}

#[test]
fn test_cache_implementation() {
    assert_parses(
        r#"
        type Cache<K, V> is {
            data: HashMap<K, CacheEntry<V>>,
            max_size: Int{> 0},
            ttl: Int{> 0}
        };

        type CacheEntry<V> is {
            value: V,
            timestamp: Int,
            hits: Int{>= 0}
        };

        implement<K, V> Cache<K, V> where K: Hash + Eq {
            fn get(&mut self, key: K) -> Option<V> {
                None
            }

            fn put(&mut self, key: K, value: V) {
                if self.data.len() >= self.max_size {
                    self.evict();
                }
            }
        }
    "#,
    );
}

#[test]
fn test_validation_framework() {
    assert_parses(
        r#"
        type Validate is protocol {
            fn validate(&self) -> Result<(), ValidationError>;
        };

        type User is {
            username: String,
            email: String,
            age: Int
        };

        implement Validate for User {
            fn validate(&self) -> Result<(), ValidationError> {
                if self.username.len() < 3 {
                    return Err(ValidationError.new("Username too short"));
                }
                if !self.email.contains('@') {
                    return Err(ValidationError.new("Invalid email"));
                }
                if self.age < 18 {
                    return Err(ValidationError.new("Must be 18+"));
                }
                Ok(())
            }
        }
    "#,
    );
}

#[test]
fn test_event_sourcing() {
    assert_parses(
        r#"
        type Event is
            UserCreated { id: String, username: String }
            | UserUpdated { id: String, field: String, value: String }
            | UserDeleted { id: String };

        type Aggregate is {
            id: String,
            version: Int{>= 0},
            state: UserState
        };

        fn apply_event(aggregate: Aggregate, event: Event) -> Aggregate {
            match event {
                Event.UserCreated { id, username } => Aggregate {
                    id,
                    version: aggregate.version + 1,
                    state: UserState { username }
                },
                _ => aggregate
            }
        }
    "#,
    );
}

#[test]
fn test_command_pattern() {
    // Context requirements in protocol/impl methods are now supported
    // Syntax: `fn foo() -> T using [Ctx]` works in protocol and impl method signatures
    // Context inheritance: spawned tasks inherit parent context environment
    assert_parses(
        r#"
        type Command is protocol {
            fn execute(&self) -> Result<(), Error> using [IO];
            fn undo(&self) -> Result<(), Error> using [IO];
        };

        type CreateFileCommand is {
            path: String,
            content: String
        };

        implement Command for CreateFileCommand {
            fn execute(&self) -> Result<(), Error> using [IO] {
                write_file(self.path, self.content)
            }

            fn undo(&self) -> Result<(), Error> using [IO] {
                delete_file(self.path)
            }
        }
    "#,
    );
}

#[test]
fn test_dependency_injection() {
    // Context requirements in protocol/impl methods are now supported
    // Syntax: `fn foo() -> T using [Ctx]` works in protocol and impl method signatures
    // Verum uses Context System for dependency injection
    assert_parses(
        r#"
        type Logger is protocol {
            fn log(&self, message: String) using [IO];
        };

        type Database is protocol {
            fn query(&self, sql: String) -> Result<Data, Error> using [IO, Database];
        };

        type UserService<L: Logger, D: Database> is {
            logger: L,
            database: D
        };

        implement<L: Logger, D: Database> UserService<L, D> {
            fn create_user(&self, username: String) -> Result<User, Error> using [IO, Database] {
                self.logger.log("Creating user");
                self.database.query("INSERT INTO users...")
            }
        }
    "#,
    );
}

// ============================================================================
// Loop Annotations in Module Context
// ============================================================================

#[test]
fn test_while_loop_with_invariant_in_function() {
    assert_parses(
        r#"
        fn test_loop_invariant_while() {
            let mut i = 0;

            while i < 10
                invariant i >= 0
            {
                i = i + 1;
            }
        }
    "#,
    );
}

#[test]
fn test_for_loop_with_invariant_in_function() {
    assert_parses(
        r#"
        fn test_for_invariant() {
            for i in 0..10
                invariant i >= 0
                decreases 10 - i
            {
                print(i);
            }
        }
    "#,
    );
}

#[test]
fn test_multiple_invariants_in_function() {
    assert_parses(
        r#"
        fn test_multiple_invariants() {
            let mut i = 0;
            let mut sum = 0;

            while i < 10
                invariant i >= 0
                invariant i <= 10
                invariant sum >= 0
            {
                sum = sum + i;
                i = i + 1;
            }
        }
    "#,
    );
}

// ============================================================================
// SUMMARY
// ============================================================================

// Total new tests: ~150 tests
// - Complete programs: 20 tests
// - Context system: 15 tests
// - CBGR (Compile-time Borrow & Gen Refs): 15 tests
// - Advanced refinement types: 20 tests
// - Edge cases and corner cases: 25 tests
// - Error recovery: 15 tests
// - Real-world patterns: 10 tests
//
// Combined with existing tests (~8500 lines), this brings comprehensive
// coverage to the parser with emphasis on integration testing and
// real-world usage patterns.
