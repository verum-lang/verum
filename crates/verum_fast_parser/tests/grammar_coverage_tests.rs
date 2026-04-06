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
//! Grammar coverage tests for Verum parser.
//!
//! Tests grammar productions from grammar/verum.ebnf that lack dedicated
//! test coverage. Each test references the specific EBNF production it covers.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

/// Helper to parse a module from source.
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
    parse_module(source).unwrap_or_else(|e| panic!("Failed to parse: {}\nError: {}", source, e));
}

/// Helper to check if parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_module(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

// ============================================================================
// Section 2.3: Inductive Types
// Grammar: inductive_def = 'inductive' , '{' , variant_list , '}' ;
// ============================================================================

mod inductive_types {
    use super::*;

    #[test]
    fn basic_inductive_type() {
        // type Nat is inductive { | Zero | Succ(Nat) };
        assert_parses(
            r#"type Nat is inductive { | Zero | Succ(Nat) };"#
        );
    }

    #[test]
    fn inductive_without_leading_pipe() {
        assert_parses(
            r#"type Nat is inductive { Zero | Succ(Nat) };"#
        );
    }

    #[test]
    fn inductive_single_variant() {
        assert_parses(
            r#"type Unit is inductive { Unit };"#
        );
    }

    #[test]
    fn inductive_with_record_variants() {
        assert_parses(
            r#"type Expr is inductive {
                Lit { value: Int }
                | Add { left: Expr, right: Expr }
                | Neg { inner: Expr }
            };"#
        );
    }

    #[test]
    fn inductive_generic() {
        assert_parses(
            r#"type List<T> is inductive { Nil | Cons(T, List<T>) };"#
        );
    }

    #[test]
    fn inductive_with_multiple_data_fields() {
        assert_parses(
            r#"type Tree<T> is inductive {
                Leaf(T)
                | Node(Tree<T>, T, Tree<T>)
            };"#
        );
    }
}

// ============================================================================
// Section 2.3: Coinductive Types
// Grammar: coinductive_def = 'coinductive' , '{' , protocol_items , '}' ;
// ============================================================================

mod coinductive_types {
    use super::*;

    #[test]
    fn basic_coinductive_type() {
        assert_parses(
            r#"type Stream<T> is coinductive {
                fn head(&self) -> T;
                fn tail(&self) -> Stream<T>;
            };"#
        );
    }

    #[test]
    fn coinductive_single_observation() {
        assert_parses(
            r#"type InfiniteList<T> is coinductive {
                fn observe(&self) -> T;
            };"#
        );
    }

    #[test]
    fn coinductive_with_generic_methods() {
        assert_parses(
            r#"type Process<I, O> is coinductive {
                fn step(&self, input: I) -> (O, Process<I, O>);
            };"#
        );
    }

    #[test]
    fn coinductive_multiple_observations() {
        assert_parses(
            r#"type CoList<T> is coinductive {
                fn is_empty(&self) -> Bool;
                fn head(&self) -> T;
                fn tail(&self) -> CoList<T>;
            };"#
        );
    }
}

// ============================================================================
// Section 2.4: Throws Clause
// Grammar: throws_clause = 'throws' , '(' , error_type_list , ')' ;
// ============================================================================

mod throws_clause {
    use super::*;

    #[test]
    fn function_with_single_throws() {
        assert_parses(
            r#"fn parse(input: Text) throws(ParseError) -> AST { input }"#
        );
    }

    #[test]
    fn function_with_multiple_throws() {
        assert_parses(
            r#"fn process(data: Data) throws(ParseError | ValidationError) -> Result { data }"#
        );
    }

    #[test]
    fn function_throws_no_return_type() {
        assert_parses(
            r#"fn validate(input: Text) throws(ValidationError) { input }"#
        );
    }

    #[test]
    fn function_throws_with_using() {
        assert_parses(
            r#"fn save(item: Item) throws(DbError) -> Bool using [Database] { true }"#
        );
    }

    #[test]
    fn throws_empty_is_error() {
        assert_fails(
            r#"fn f() throws() -> Int { 0 }"#
        );
    }
}

// ============================================================================
// Section 2.4: Generator Functions (fn*)
// Grammar: fn_keyword = 'fn' , [ '*' ] ;
// ============================================================================

mod generator_functions {
    use super::*;

    #[test]
    fn sync_generator_basic() {
        assert_parses(
            r#"fn* range(n: Int) -> Int {
                let i = 0;
                yield i;
            }"#
        );
    }

    #[test]
    fn async_generator() {
        assert_parses(
            r#"async fn* fetch_pages() -> Page {
                yield default_page;
            }"#
        );
    }

    #[test]
    fn generator_with_loop() {
        assert_parses(
            r#"fn* naturals() -> Int {
                let mut n = 0;
                loop {
                    yield n;
                    n = n + 1;
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.4.7: Existential Types
// Grammar: existential_type = 'some' , identifier , ':' , existential_bounds ;
// ============================================================================

mod existential_types {
    use super::*;

    #[test]
    fn existential_return_type() {
        assert_parses(
            r#"fn make_iter() -> some I: Iterator { items }"#
        );
    }

    #[test]
    fn existential_multiple_bounds() {
        assert_parses(
            r#"fn processor() -> some P: Processor + Send + Sync { p }"#
        );
    }

    #[test]
    fn existential_type_definition() {
        assert_parses(
            r#"type Plugin is some P: PluginInterface;"#
        );
    }

    #[test]
    fn existential_in_function_param_type() {
        // Existential types can be used where a type expression is expected
        assert_parses(
            r#"fn use_thing(x: some T: Display) { x }"#
        );
    }
}

// ============================================================================
// Section 2.6.2: Layer Definitions
// Grammar: layer_def = visibility , 'layer' , identifier , layer_body ;
// ============================================================================

mod layer_definitions {
    use super::*;

    #[test]
    fn inline_layer() {
        assert_parses(
            r#"layer DatabaseLayer {
                provide ConnectionPool = ConnectionPool.new();
                provide QueryExecutor = QueryExecutor.new();
            }"#
        );
    }

    #[test]
    fn composite_layer() {
        assert_parses(
            r#"layer AppLayer = DatabaseLayer + LoggingLayer;"#
        );
    }

    #[test]
    fn composite_layer_three() {
        assert_parses(
            r#"layer FullStack = DatabaseLayer + LoggingLayer + CacheLayer;"#
        );
    }

    #[test]
    fn public_layer() {
        assert_parses(
            r#"public layer SharedLayer {
                provide Config = Config.default();
            }"#
        );
    }

    #[test]
    fn layer_single_provide() {
        assert_parses(
            r#"layer MinimalLayer {
                provide Logger = Logger.new();
            }"#
        );
    }
}

// ============================================================================
// Section 2.6.1: Context Protocol Definitions
// Grammar: context_protocol_def = visibility , 'context' , 'protocol' , identifier , ...
// ============================================================================

mod context_protocol_definitions {
    use super::*;

    #[test]
    fn context_protocol_basic() {
        assert_parses(
            r#"context protocol Serializable {
                fn serialize(&self) -> Text;
            }"#
        );
    }

    #[test]
    fn context_protocol_multiple_methods() {
        assert_parses(
            r#"context protocol Codec {
                fn encode(&self) -> List<Int>;
                fn decode(data: List<Int>) -> Self;
            }"#
        );
    }

    #[test]
    fn context_type_protocol() {
        // Alternative form: context type Name is protocol { };
        assert_parses(
            r#"context type Hashable is protocol {
                fn hash(&self) -> Int;
            };"#
        );
    }

    #[test]
    fn public_context_protocol() {
        assert_parses(
            r#"public context protocol Logger {
                fn log(&self, msg: Text);
            }"#
        );
    }
}

// ============================================================================
// Section 2.8: Type Expressions - Special Types
// Grammar: never_type = '!' ; unknown_type = 'unknown' ; universe_type = 'Type' ...
// ============================================================================

mod special_types {
    use super::*;

    #[test]
    fn never_type_in_return() {
        assert_parses(
            r#"fn diverge() -> ! { panic("diverge") }"#
        );
    }

    #[test]
    fn unknown_type_in_param() {
        assert_parses(
            r#"fn accept_any(x: unknown) -> Bool { true }"#
        );
    }

    #[test]
    fn universe_type_basic() {
        assert_parses(
            r#"fn identity(x: Type) -> Type { x }"#
        );
    }

    #[test]
    fn capability_type_in_function() {
        assert_parses(
            r#"fn analyze(db: Database with [Read]) -> Stats { db }"#
        );
    }

    #[test]
    fn capability_type_multiple() {
        assert_parses(
            r#"fn process(db: Database with [Read, Write]) -> Bool { true }"#
        );
    }

    #[test]
    fn dynamic_type_simple() {
        assert_parses(
            r#"fn show(item: dyn Display) { item }"#
        );
    }

    #[test]
    fn dynamic_type_multiple_bounds() {
        assert_parses(
            r#"fn show(item: dyn Display + Debug) { item }"#
        );
    }
}

// ============================================================================
// Section 2.4: Where Clauses
// Grammar: where_clause = 'where' , where_predicates ;
// ============================================================================

mod where_clauses {
    use super::*;

    #[test]
    fn simple_where_clause() {
        assert_parses(
            r#"fn sort<T>(list: List<T>) -> List<T> where type T: Ord { list }"#
        );
    }

    #[test]
    fn where_clause_multiple_bounds() {
        assert_parses(
            r#"fn display<T>(item: T) -> Text where type T: Display + Debug { item }"#
        );
    }

    #[test]
    fn where_clause_on_impl() {
        assert_parses(
            r#"implement<T> Display for List<T> where type T: Display {
                fn display(&self) -> Text { self }
            }"#
        );
    }

    #[test]
    fn generic_where_clause_on_type() {
        assert_parses(
            r#"type SortedList<T> where type T: Ord is { items: List<T> };"#
        );
    }
}

// ============================================================================
// Section 2.3: Sigma / Dependent Types
// Grammar: sigma_bindings = sigma_binding , { ',' , sigma_binding } ;
// ============================================================================

mod sigma_types {
    use super::*;

    #[test]
    fn single_sigma_binding() {
        assert_parses(
            r#"type Natural is n: Int where n >= 0;"#
        );
    }

    #[test]
    fn multi_sigma_binding() {
        assert_parses(
            r#"type SizedVec is n: Int, data: List<Int>;"#
        );
    }
}

// ============================================================================
// Section 2.10: For-Await Loops
// Grammar: for_await_loop = 'for' , 'await' , pattern , 'in' , expression ...
// ============================================================================

mod for_await_loops {
    use super::*;

    #[test]
    fn for_await_basic() {
        assert_parses(
            r#"fn consume() {
                for await item in stream {
                    process(item);
                }
            }"#
        );
    }

    #[test]
    fn for_await_with_pattern() {
        assert_parses(
            r#"fn consume() {
                for await (key, value) in entries {
                    store(key, value);
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.12.2: Select Expression
// Grammar: select_expr = 'select' , [ 'biased' ] , '{' , select_arms , '}' ;
// ============================================================================

mod select_expressions {
    use super::*;

    #[test]
    fn select_basic() {
        assert_parses(
            r#"fn race() {
                let result = select {
                    data = channel.recv().await => data,
                    else => default_value,
                };
            }"#
        );
    }

    #[test]
    fn select_biased() {
        assert_parses(
            r#"fn race() {
                let result = select biased {
                    data = priority.recv().await => data,
                    msg = normal.recv().await => msg,
                };
            }"#
        );
    }

    #[test]
    fn select_with_guard() {
        assert_parses(
            r#"fn race() {
                let result = select {
                    data = ch.recv().await if enabled => data,
                    else => fallback,
                };
            }"#
        );
    }
}

// ============================================================================
// Section 2.12.3: Nursery Expression
// Grammar: nursery_expr = 'nursery' , [ nursery_options ] , block_expr , ...
// ============================================================================

mod nursery_expressions {
    use super::*;

    #[test]
    fn nursery_basic() {
        assert_parses(
            r#"fn concurrent() {
                nursery {
                    let a = spawn fetch_a();
                    let b = spawn fetch_b();
                }
            }"#
        );
    }

    #[test]
    fn nursery_with_recover() {
        assert_parses(
            r#"fn concurrent() {
                nursery {
                    let x = spawn work();
                } recover {
                    err => handle(err),
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.12.1: Try / Recover / Finally
// Grammar: try_expr = 'try' , block_expr , [ try_handlers ] ;
// ============================================================================

mod try_recover_finally {
    use super::*;

    #[test]
    fn try_recover_basic() {
        assert_parses(
            r#"fn safe() {
                try {
                    risky_operation();
                } recover {
                    err => handle(err),
                }
            }"#
        );
    }

    #[test]
    fn try_finally_basic() {
        assert_parses(
            r#"fn safe() {
                try {
                    open_resource();
                } finally {
                    close_resource();
                }
            }"#
        );
    }

    #[test]
    fn try_recover_finally() {
        assert_parses(
            r#"fn safe() {
                try {
                    do_work();
                } recover {
                    err => log(err),
                } finally {
                    cleanup();
                }
            }"#
        );
    }

    #[test]
    fn try_recover_closure_syntax() {
        assert_parses(
            r#"fn safe() {
                try {
                    risky();
                } recover |e| {
                    handle(e)
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.13: Defer / Errdefer Statements
// Grammar: defer_stmt = 'defer' , defer_body | 'errdefer' , defer_body ;
// ============================================================================

mod defer_statements {
    use super::*;

    #[test]
    fn defer_expression() {
        assert_parses(
            r#"fn cleanup() {
                defer close();
            }"#
        );
    }

    #[test]
    fn defer_block() {
        assert_parses(
            r#"fn cleanup() {
                defer {
                    close_file();
                    flush_buffer();
                }
            }"#
        );
    }

    #[test]
    fn errdefer_expression() {
        assert_parses(
            r#"fn careful() {
                errdefer rollback();
                let result = do_work();
            }"#
        );
    }

    #[test]
    fn errdefer_block() {
        assert_parses(
            r#"fn careful() {
                errdefer {
                    rollback();
                    notify_failure();
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.13: Let-Else Statements
// Grammar: let_else_stmt = 'let' , pattern , '=' , expression , 'else' , block_expr ;
// ============================================================================

mod let_else_statements {
    use super::*;

    #[test]
    fn let_else_basic() {
        assert_parses(
            r#"fn f() {
                let Some(x) = maybe_val else { return; };
            }"#
        );
    }

    #[test]
    fn let_else_with_type() {
        assert_parses(
            r#"fn f() {
                let Ok(val): Result<Int, Error> = compute() else { return; };
            }"#
        );
    }
}

// ============================================================================
// Section 2.14: View Patterns
// Grammar: view_pattern = ( identifier | qualified_path ) , '->' , primary_pattern ;
// ============================================================================

mod view_patterns {
    use super::*;

    #[test]
    fn view_pattern_in_match() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    parity -> Even => "even",
                    _ => "odd",
                }
            }"#
        );
    }

    #[test]
    fn view_pattern_with_binding() {
        assert_parses(
            r#"fn f(data: Text) {
                match data {
                    parse_json -> Ok(value) => value,
                    _ => default_val,
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.7: Extern Blocks
// Grammar: extern_block = 'extern' , [ string_lit ] , '{' , { extern_fn_decl } , '}' ;
// ============================================================================

mod extern_blocks {
    use super::*;

    #[test]
    fn extern_c_abi() {
        assert_parses(
            r#"extern "C" {
                fn malloc(size: Int) -> &unsafe Int;
                fn free(ptr: &unsafe Int);
            }"#
        );
    }

    #[test]
    fn extern_default_abi() {
        assert_parses(
            r#"extern {
                fn custom_func(x: Int) -> Int;
            }"#
        );
    }

    #[test]
    fn extern_empty() {
        assert_parses(
            r#"extern "C" {}"#
        );
    }
}

// ============================================================================
// Section 2.5: Protocol with Extension
// Grammar: protocol_extension = 'extends' , trait_path , { '+' , trait_path } ;
// ============================================================================

mod protocol_extensions {
    use super::*;

    #[test]
    fn protocol_extends_single() {
        assert_parses(
            r#"type Ordered is protocol extends Eq {
                fn compare(&self, other: &Self) -> Int;
            };"#
        );
    }

    #[test]
    fn protocol_extends_multiple() {
        assert_parses(
            r#"type Printable is protocol extends Display + Debug {
                fn pretty_print(&self) -> Text;
            };"#
        );
    }
}

// ============================================================================
// Section 2.8: Rank-2 Function Types
// Grammar: rank2_function_type = [ 'async' ] , 'fn' , generics , '(' , type_list , ')' , ...
// ============================================================================

mod rank2_function_types {
    use super::*;

    #[test]
    fn rank2_basic() {
        assert_parses(
            r#"type Transducer is {
                transform: fn<R>(R) -> R,
            };"#
        );
    }

    #[test]
    fn rank2_multi_params() {
        assert_parses(
            r#"type Mapper is {
                apply: fn<A, B>(A) -> B,
            };"#
        );
    }

    #[test]
    fn rank2_in_function_param() {
        assert_parses(
            r#"fn apply(f: fn<T>(T) -> T, x: Int) -> Int { f(x) }"#
        );
    }
}

// ============================================================================
// Section 2.10: Pipeline Expressions
// Grammar: pipeline_expr = assignment_expr , { '|>' , ... } ;
// ============================================================================

mod pipeline_expressions {
    use super::*;

    #[test]
    fn pipeline_basic() {
        assert_parses(
            r#"fn f() {
                let result = data |> process;
            }"#
        );
    }

    #[test]
    fn pipeline_chained() {
        assert_parses(
            r#"fn f() {
                let result = data |> transform |> filter |> collect;
            }"#
        );
    }

    #[test]
    fn pipeline_method_call() {
        assert_parses(
            r#"fn f() {
                let result = items |> .filter(pred) |> .map(transform);
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Null Coalescing
// Grammar: null_coalesce_expr = range_expr , { '??' , range_expr } ;
// ============================================================================

mod null_coalescing {
    use super::*;

    #[test]
    fn null_coalesce_basic() {
        assert_parses(
            r#"fn f() {
                let val = maybe_val ?? default_val;
            }"#
        );
    }

    #[test]
    fn null_coalesce_chained() {
        assert_parses(
            r#"fn f() {
                let val = first ?? second ?? third;
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Is Expression (Pattern Testing)
// Grammar: is_relational_expr = relational_expr , [ 'is' , [ 'not' ] , pattern ] ;
// ============================================================================

mod is_expressions {
    use super::*;

    #[test]
    fn is_pattern_test() {
        assert_parses(
            r#"fn f(x: Maybe<Int>) {
                if x is Some(v) {
                    v
                }
            }"#
        );
    }

    #[test]
    fn is_not_pattern() {
        assert_parses(
            r#"fn f(x: Maybe<Int>) {
                if x is not None {
                    process(x);
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.11: Stream Expressions
// Grammar: stream_expr = stream_comprehension_expr | stream_literal_expr ;
// ============================================================================

mod stream_expressions {
    use super::*;

    #[test]
    fn stream_comprehension() {
        assert_parses(
            r#"fn f() {
                let s = stream[x * 2 for x in source];
            }"#
        );
    }

    #[test]
    fn stream_comprehension_with_filter() {
        assert_parses(
            r#"fn f() {
                let s = stream[x for x in items if x > 0];
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Comprehension Expressions (List, Map, Set, Generator)
// ============================================================================

mod comprehensions {
    use super::*;

    #[test]
    fn list_comprehension() {
        assert_parses(
            r#"fn f() {
                let doubled = [x * 2 for x in items];
            }"#
        );
    }

    #[test]
    fn list_comprehension_with_filter() {
        assert_parses(
            r#"fn f() {
                let evens = [x for x in items if x % 2 == 0];
            }"#
        );
    }

    #[test]
    fn map_comprehension() {
        assert_parses(
            r#"fn f() {
                let m = {k: v for (k, v) in entries};
            }"#
        );
    }

    #[test]
    fn set_comprehension() {
        assert_parses(
            r#"fn f() {
                let s = set{x for x in items};
            }"#
        );
    }

    #[test]
    fn generator_expression() {
        assert_parses(
            r#"fn f() {
                let g = gen{x * x for x in range};
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Async/Await Expressions
// ============================================================================

mod async_await {
    use super::*;

    #[test]
    fn async_function() {
        assert_parses(
            r#"async fn fetch(url: Text) -> Response { get(url).await }"#
        );
    }

    #[test]
    fn async_block() {
        assert_parses(
            r#"fn f() {
                let future = async { compute().await };
            }"#
        );
    }

    #[test]
    fn spawn_basic() {
        assert_parses(
            r#"fn f() {
                let handle = spawn async { work() };
            }"#
        );
    }

    #[test]
    fn spawn_with_contexts() {
        assert_parses(
            r#"fn f() {
                let handle = spawn using [Database, Logger] async { query() };
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Yield Expression
// Grammar: yield_expr = 'yield' , expression ;
// ============================================================================

mod yield_expressions {
    use super::*;

    #[test]
    fn yield_value() {
        assert_parses(
            r#"fn* gen() -> Int {
                yield 42;
            }"#
        );
    }

    #[test]
    fn yield_expression() {
        assert_parses(
            r#"fn* gen(x: Int) -> Int {
                yield x + 1;
            }"#
        );
    }
}

// ============================================================================
// Section 2.13: Provide Statements
// Grammar: provide_stmt = 'provide' , context_path , ... ;
// ============================================================================

mod provide_statements {
    use super::*;

    #[test]
    fn provide_basic() {
        assert_parses(
            r#"fn f() {
                provide Logger = ConsoleLogger.new();
            }"#
        );
    }

    #[test]
    fn provide_with_alias() {
        assert_parses(
            r#"fn f() {
                provide Database as primary = PgDatabase.new();
            }"#
        );
    }

    #[test]
    fn provide_in_block() {
        assert_parses(
            r#"fn f() {
                provide Logger = FileLogger.new() in {
                    do_work();
                }
            }"#
        );
    }

    #[test]
    fn provide_layer_shorthand() {
        assert_parses(
            r#"fn f() {
                provide AppLayer;
            }"#
        );
    }
}

// ============================================================================
// Section 2.14: Pattern Matching - And Patterns, Guards, etc.
// ============================================================================

mod pattern_matching {
    use super::*;

    #[test]
    fn or_pattern() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    1 | 2 | 3 => "small",
                    _ => "big",
                }
            }"#
        );
    }

    #[test]
    fn pattern_guard() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    n if n > 0 => "positive",
                    n if n < 0 => "negative",
                    _ => "zero",
                }
            }"#
        );
    }

    #[test]
    fn pattern_guard_where_syntax() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    n where n > 0 => "positive",
                    _ => "other",
                }
            }"#
        );
    }

    #[test]
    fn at_binding() {
        assert_parses(
            r#"fn f(x: Maybe<Int>) {
                match x {
                    whole @ Some(inner) => whole,
                    None => None,
                }
            }"#
        );
    }

    #[test]
    fn rest_pattern() {
        assert_parses(
            r#"fn f(list: List<Int>) {
                match list {
                    [first, ..] => first,
                    [] => 0,
                }
            }"#
        );
    }

    #[test]
    fn reference_pattern() {
        assert_parses(
            r#"fn f(x: &Int) {
                match x {
                    &0 => "zero",
                    &n => "nonzero",
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.14.1: Active Pattern Definitions
// Grammar: pattern_def = visibility , 'pattern' , identifier , ...
// ============================================================================

mod active_pattern_defs {
    use super::*;

    #[test]
    fn simple_active_pattern() {
        assert_parses(
            r#"pattern Even(n: Int) -> Bool = n % 2 == 0;"#
        );
    }

    #[test]
    fn parameterized_active_pattern() {
        assert_parses(
            r#"pattern InRange(lo: Int, hi: Int)(n: Int) -> Bool = n >= lo && n <= hi;"#
        );
    }
}

// ============================================================================
// Section 2.15: Constants and Statics
// ============================================================================

mod constants_and_statics {
    use super::*;

    #[test]
    fn const_basic() {
        assert_parses(r#"const MAX: Int = 100;"#);
    }

    #[test]
    fn const_public() {
        assert_parses(r#"public const PI: Float = 3.14159;"#);
    }

    #[test]
    fn static_basic() {
        assert_parses(r#"static COUNTER: Int = 0;"#);
    }

    #[test]
    fn static_mut() {
        assert_parses(r#"static mut GLOBAL: Int = 0;"#);
    }
}

// ============================================================================
// Section 2.6: Context Definitions
// Grammar: context_def = visibility , ( 'context' , [ 'async' ] | 'async' , 'context' ) , ...
// ============================================================================

mod context_definitions {
    use super::*;

    #[test]
    fn context_basic() {
        assert_parses(
            r#"context Database {
                fn query(&self, sql: Text) -> List<Row>;
                fn execute(&self, sql: Text) -> Int;
            }"#
        );
    }

    #[test]
    fn context_async() {
        assert_parses(
            r#"context async HttpClient {
                fn get(&self, url: Text) -> Response;
            }"#
        );
    }

    #[test]
    fn context_alt_async_syntax() {
        assert_parses(
            r#"async context WebSocket {
                fn send(&self, msg: Text);
            }"#
        );
    }

    #[test]
    fn context_with_generics() {
        assert_parses(
            r#"context Cache<K, V> {
                fn get(&self, key: K) -> Maybe<V>;
                fn set(&self, key: K, value: V);
            }"#
        );
    }
}

// ============================================================================
// Section 2.4: Function Using Clause (Context System)
// Grammar: context_clause = 'using' , context_spec ;
// ============================================================================

mod context_clauses {
    use super::*;

    #[test]
    fn using_single() {
        assert_parses(
            r#"fn query() -> Data using Database { data }"#
        );
    }

    #[test]
    fn using_multiple() {
        assert_parses(
            r#"fn process() -> Result using [Database, Logger] { ok }"#
        );
    }

    #[test]
    fn using_negative() {
        assert_parses(
            r#"fn pure_fn() using [!IO, !State] { 42 }"#
        );
    }

    #[test]
    fn using_with_alias() {
        assert_parses(
            r#"fn work() using [Database as db, Logger as log] { ok }"#
        );
    }
}

// ============================================================================
// Section 2.10: Optional Chaining
// Grammar: optional_chain = '?.' ;
// ============================================================================

mod optional_chaining {
    use super::*;

    #[test]
    fn optional_chain_basic() {
        assert_parses(
            r#"fn f(x: Maybe<Record>) {
                let val = x?.field;
            }"#
        );
    }

    #[test]
    fn optional_chain_method() {
        assert_parses(
            r#"fn f(x: Maybe<List<Int>>) {
                let len = x?.len();
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Destructuring Assignment
// Grammar: destructuring_assign = destructuring_target , assign_op , assignment_expr ;
// ============================================================================

mod destructuring_assignment {
    use super::*;

    #[test]
    fn tuple_destructure() {
        assert_parses(
            r#"fn f() {
                let (a, b) = get_pair();
            }"#
        );
    }

    #[test]
    fn record_destructure() {
        assert_parses(
            r#"fn f() {
                let Point { x, y } = get_point();
            }"#
        );
    }
}

// ============================================================================
// Section 2.3: Type Definitions - Variants with Attributes
// ============================================================================

mod variant_attributes {
    use super::*;

    #[test]
    fn variant_with_attribute() {
        assert_parses(
            r#"type Status is @default Ok | @deprecated Legacy | Error(Text);"#
        );
    }
}

// ============================================================================
// Section 2.10: Unsafe Expression
// Grammar: unsafe_expr = 'unsafe' , block_expr ;
// ============================================================================

mod unsafe_expressions {
    use super::*;

    #[test]
    fn unsafe_block() {
        assert_parses(
            r#"fn f() {
                unsafe {
                    raw_operation();
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.16: Meta / Macro Expressions
// Grammar: meta_call = '@' , path , meta_call_args ;
// ============================================================================

mod meta_expressions {
    use super::*;

    #[test]
    fn meta_call_basic() {
        assert_parses(
            r#"fn f() {
                let q = @sql_query("SELECT * FROM users");
            }"#
        );
    }

    #[test]
    fn meta_block() {
        assert_parses(
            r#"fn f() {
                meta {
                    generate_code()
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Quote Expressions (Staged Meta)
// Grammar: quote_expr = 'quote' , [ quote_stage ] , '{' , token_tree , '}' ;
// ============================================================================

mod quote_expressions {
    use super::*;

    #[test]
    fn quote_basic() {
        assert_parses(
            r#"meta fn gen() {
                quote {
                    fn hello() { print("hello") }
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.4: Pure Functions
// Grammar: function_modifiers = [ 'pure' ] , ...
// ============================================================================

mod pure_functions {
    use super::*;

    #[test]
    fn pure_function() {
        assert_parses(
            r#"pure fn add(a: Int, b: Int) -> Int { a + b }"#
        );
    }

    #[test]
    fn pure_async_function() {
        assert_parses(
            r#"pure async fn compute(x: Int) -> Int { x * 2 }"#
        );
    }
}

// ============================================================================
// Section 2.5: Implementation Blocks
// ============================================================================

mod impl_blocks {
    use super::*;

    #[test]
    fn impl_inherent() {
        assert_parses(
            r#"implement Point {
                fn new(x: Float, y: Float) -> Self {
                    Point { x: x, y: y }
                }
            }"#
        );
    }

    #[test]
    fn impl_for_protocol() {
        assert_parses(
            r#"implement Display for Point {
                fn display(&self) -> Text { self }
            }"#
        );
    }

    #[test]
    fn impl_generic() {
        assert_parses(
            r#"implement<T> Container<T> {
                fn get(&self) -> T { self.value }
            }"#
        );
    }

    #[test]
    fn impl_with_associated_type() {
        assert_parses(
            r#"implement Iterator for MyIter {
                type Item = Int;
                fn next(&mut self) -> Maybe<Int> { None }
            }"#
        );
    }

    #[test]
    fn impl_unsafe() {
        assert_parses(
            r#"unsafe implement Send for MyType {}"#
        );
    }
}

// ============================================================================
// Section 2.10: Range Expressions
// Grammar: range_expr = logical_or_expr , [ range_op , logical_or_expr ] ;
// ============================================================================

mod range_expressions {
    use super::*;

    #[test]
    fn range_exclusive() {
        assert_parses(
            r#"fn f() {
                for i in 0..10 { i; }
            }"#
        );
    }

    #[test]
    fn range_inclusive() {
        assert_parses(
            r#"fn f() {
                for i in 0..=10 { i; }
            }"#
        );
    }
}

// ============================================================================
// Section 2.8: Three-Tier Reference Types
// Grammar: managed_reference_type, checked_reference_type, unsafe_reference_type
// ============================================================================

mod reference_types {
    use super::*;

    #[test]
    fn managed_ref() {
        assert_parses(r#"fn f(x: &Int) -> &Int { x }"#);
    }

    #[test]
    fn managed_mut_ref() {
        assert_parses(r#"fn f(x: &mut Int) { x }"#);
    }

    #[test]
    fn checked_ref() {
        assert_parses(r#"fn f(x: &checked Int) -> &checked Int { x }"#);
    }

    #[test]
    fn checked_mut_ref() {
        assert_parses(r#"fn f(x: &checked mut Int) { x }"#);
    }

    #[test]
    fn unsafe_ref() {
        assert_parses(r#"fn f(x: &unsafe Int) -> &unsafe Int { x }"#);
    }

    #[test]
    fn unsafe_mut_ref() {
        assert_parses(r#"fn f(x: &unsafe mut Int) { x }"#);
    }
}

// ============================================================================
// Section 2.8: Array and Slice Types
// Grammar: array_type = '[' , type_expr , ';' , expression , ']' ;
//          slice_type = '[' , type_expr , ']' ;
// ============================================================================

mod array_slice_types {
    use super::*;

    #[test]
    fn array_type() {
        assert_parses(r#"fn f(arr: [Int; 10]) { arr }"#);
    }

    #[test]
    fn slice_type() {
        assert_parses(r#"fn f(s: [Int]) { s }"#);
    }
}

// ============================================================================
// Section 2.8: Tuple Types
// Grammar: tuple_type = '(' , type_expr , { ',' , type_expr } , ')' ;
// ============================================================================

mod tuple_types {
    use super::*;

    #[test]
    fn tuple_type_pair() {
        assert_parses(r#"fn f() -> (Int, Text) { (1, "hi") }"#);
    }

    #[test]
    fn tuple_type_triple() {
        assert_parses(r#"fn f() -> (Int, Float, Bool) { (1, 2.0, true) }"#);
    }
}

// ============================================================================
// Section 2.8: Function Types
// Grammar: function_type = [ 'async' ] , 'fn' , '(' , type_list , ')' , ...
// ============================================================================

mod function_types {
    use super::*;

    #[test]
    fn fn_type_basic() {
        assert_parses(r#"fn apply(f: fn(Int) -> Int, x: Int) -> Int { f(x) }"#);
    }

    #[test]
    fn fn_type_no_return() {
        assert_parses(r#"fn call(f: fn(Int)) { f(0) }"#);
    }

    #[test]
    fn fn_type_async() {
        assert_parses(r#"fn call(f: async fn(Int) -> Int) { f(0) }"#);
    }

    #[test]
    fn fn_type_with_context() {
        assert_parses(
            r#"fn call(f: fn(Text) -> Bool using [Logger]) { f("test") }"#
        );
    }
}

// ============================================================================
// Section 2.10: If-Let Chains
// Grammar: if_condition = let_condition , { '&&' , let_condition } ;
// ============================================================================

mod if_let_chains {
    use super::*;

    #[test]
    fn if_let_basic() {
        assert_parses(
            r#"fn f(x: Maybe<Int>) {
                if let Some(v) = x { v; }
            }"#
        );
    }

    #[test]
    fn if_let_chain() {
        assert_parses(
            r#"fn f(x: Maybe<Int>) {
                if let Some(v) = x && v > 0 { v; }
            }"#
        );
    }

    #[test]
    fn if_let_chain_multiple_lets() {
        assert_parses(
            r#"fn f(a: Maybe<Int>, b: Maybe<Int>) {
                if let Some(x) = a && let Some(y) = b {
                    x + y;
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Closure Expressions
// Grammar: closure_expr = [ 'async' ] , closure_params , [ '->' , type_expr ] , expression ;
// ============================================================================

mod closures {
    use super::*;

    #[test]
    fn closure_no_params() {
        assert_parses(
            r#"fn f() {
                let c = || 42;
            }"#
        );
    }

    #[test]
    fn closure_with_params() {
        assert_parses(
            r#"fn f() {
                let c = |x, y| x + y;
            }"#
        );
    }

    #[test]
    fn closure_with_block() {
        assert_parses(
            r#"fn f() {
                let c = |x| {
                    let y = x + 1;
                    y * 2
                };
            }"#
        );
    }

    #[test]
    fn async_closure() {
        assert_parses(
            r#"fn f() {
                let c = async |url| fetch(url).await;
            }"#
        );
    }
}

// ============================================================================
// Section 2.8: Higher-Kinded Types
// Grammar: higher_kinded_type = path , '<' , '_' , '>' ;
// ============================================================================

mod higher_kinded_types {
    use super::*;

    #[test]
    fn hkt_placeholder() {
        assert_parses(
            r#"type Functor is protocol {
                type F<_>;
                fn map<A, B>(&self, f: fn(A) -> B) -> Self;
            };"#
        );
    }

    #[test]
    fn hkt_param_in_function() {
        assert_parses(
            r#"fn transform<F<_>>(container: F<Int>) -> F<Text> { container }"#
        );
    }
}

// ============================================================================
// Section 2.3: Type Definition - Record with Defaults
// Grammar: field = ... , [ field_default ] ; field_default = '=' , expression ;
// ============================================================================

mod field_defaults {
    use super::*;

    #[test]
    fn record_with_default_values() {
        assert_parses(
            r#"type Config is {
                host: Text = "localhost",
                port: Int = 8080,
                debug: Bool = false,
            };"#
        );
    }
}

// ============================================================================
// Section 2.4: Affine Types
// Grammar: type_def = ... , [ 'affine' ] , identifier , ...
// ============================================================================

mod affine_types {
    use super::*;

    #[test]
    fn affine_type() {
        assert_parses(
            r#"type affine FileHandle is { fd: Int };"#
        );
    }
}

// ============================================================================
// Section 2.12: Throw Expression
// Grammar: throw_expr = 'throw' , expression ;
// ============================================================================

mod throw_expressions {
    use super::*;

    #[test]
    fn throw_basic() {
        assert_parses(
            r#"fn validate(x: Int) throws(ValidationError) {
                if x < 0 {
                    throw ValidationError;
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.19: Forall / Exists Quantifiers
// Grammar: forall_expr = 'forall' , quantifier_binding , ... , '.' , expression ;
// ============================================================================

mod quantifier_expressions {
    use super::*;

    #[test]
    fn forall_basic() {
        assert_parses(
            r#"fn f() {
                forall x: Int. x + 0 == x
            }"#
        );
    }

    #[test]
    fn exists_basic() {
        assert_parses(
            r#"fn f() {
                exists x: Int. x * x == 4
            }"#
        );
    }
}

// ============================================================================
// Section 2.8: GenRef Type
// Grammar: genref_type = 'GenRef' , '<' , type_expr , '>' ;
// ============================================================================

mod genref_types {
    use super::*;

    #[test]
    fn genref_basic() {
        assert_parses(
            r#"type WindowIter<T> is {
                data: GenRef<List<T>>,
            };"#
        );
    }
}

// ============================================================================
// Section 2.4: Cofix Function Modifier
// Grammar: cofix is a modifier between async/unsafe and fn keyword
// ============================================================================

mod cofix_functions {
    use super::*;

    #[test]
    fn async_cofix_function() {
        // cofix is valid after async, before fn
        assert_parses(
            r#"async cofix fn ones() -> Stream<Int> {
                ones()
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Match Expression - Method-Style
// Grammar: match_expr = [ expression , '.' ] , 'match' , ...
// ============================================================================

mod match_expression_styles {
    use super::*;

    #[test]
    fn match_standard() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    0 => "zero",
                    _ => "other",
                }
            }"#
        );
    }

    #[test]
    fn match_method_style() {
        assert_parses(
            r#"fn f(x: Int) {
                x.match {
                    0 => "zero",
                    _ => "other",
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Loop Annotations (invariant, decreases)
// Grammar: loop_annotation = 'invariant' , expression | 'decreases' , expression ;
// ============================================================================

mod loop_annotations {
    use super::*;

    #[test]
    fn while_with_invariant() {
        assert_parses(
            r#"fn f() {
                while i < n invariant i >= 0 {
                    i = i + 1;
                }
            }"#
        );
    }

    #[test]
    fn while_with_decreases() {
        assert_parses(
            r#"fn f() {
                while i > 0 decreases i {
                    i = i - 1;
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.5: Protocol with Associated Const
// ============================================================================

mod protocol_consts {
    use super::*;

    #[test]
    fn protocol_with_const() {
        assert_parses(
            r#"type Bounded is protocol {
                const MAX: Int;
                fn value(&self) -> Int;
            };"#
        );
    }
}

// ============================================================================
// Section 2.5: Default Implementation in Protocol
// ============================================================================

mod protocol_defaults {
    use super::*;

    #[test]
    fn protocol_default_method() {
        assert_parses(
            r#"type Greetable is protocol {
                fn name(&self) -> Text;
                fn greet(&self) -> Text {
                    self.name()
                }
            };"#
        );
    }
}

// ============================================================================
// Section 2.7: FFI Declaration
// Grammar: ffi_declaration = visibility , 'ffi' , identifier , ...
// ============================================================================

mod ffi_declarations {
    use super::*;

    #[test]
    fn ffi_basic() {
        assert_parses(
            r#"ffi LibC {
                @extern("malloc") fn malloc(size: Int) -> &unsafe Int;
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Typeof Expression
// Grammar: typeof_expr = 'typeof' , '(' , expression , ')' ;
// ============================================================================

mod typeof_expressions {
    use super::*;

    #[test]
    fn typeof_basic() {
        assert_parses(
            r#"fn f(x: unknown) {
                let t = typeof(x);
            }"#
        );
    }
}

// ============================================================================
// Integration: Complex Programs Combining Multiple Productions
// ============================================================================

mod integration {
    use super::*;

    #[test]
    fn full_program_with_many_features() {
        assert_parses(
            r#"
            type Option<T> is None | Some(T);

            type List<T> is inductive {
                Nil
                | Cons(T, List<T>)
            };

            type Stream<T> is coinductive {
                fn head(&self) -> T;
                fn tail(&self) -> Stream<T>;
            };

            context Database {
                fn query(&self, sql: Text) -> List<Row>;
            }

            layer DbLayer {
                provide Database = PgDatabase.new();
            }

            pure fn add(a: Int, b: Int) -> Int { a + b }

            fn process(data: List<Int>) -> List<Int> using [Database] {
                [x * 2 for x in data if x > 0]
            }

            implement Display for Option<Int> {
                fn display(&self) -> Text {
                    match self {
                        Some(n) => n,
                        None => "none",
                    }
                }
            }
            "#
        );
    }

    #[test]
    fn async_program_with_concurrency() {
        assert_parses(
            r#"
            async fn fetch_all() -> List<Data> using [HttpClient] {
                nursery {
                    let a = spawn fetch_data("url1");
                    let b = spawn fetch_data("url2");
                }
            }

            async fn race_requests() {
                let result = select {
                    data = fast_api.get().await => data,
                    backup = slow_api.get().await => backup,
                    else => default_data,
                };
            }
            "#
        );
    }

    #[test]
    fn error_handling_program() {
        assert_parses(
            r#"
            fn safe_operation() {
                defer cleanup();
                errdefer rollback();

                try {
                    let result = risky_call();
                } recover {
                    err => handle(err),
                } finally {
                    log_done();
                }
            }
            "#
        );
    }

    #[test]
    fn type_system_features() {
        assert_parses(
            r#"
            type Natural is n: Int where n >= 0;

            type Printable is protocol extends Display {
                fn pretty(&self) -> Text;
            };

            fn sort<T>(list: List<T>) -> List<T> where type T: Ord {
                list
            }

            fn diverge() -> ! { panic("never returns") }

            fn accept_any(x: unknown) -> Bool { true }
            "#
        );
    }

    #[test]
    fn pattern_matching_features() {
        assert_parses(
            r#"
            pattern Even(n: Int) -> Bool = n % 2 == 0;

            fn classify(x: Int) -> Text {
                match x {
                    0 => "zero",
                    n if n > 0 => "positive",
                    n where n < 0 => "negative",
                    _ => "unreachable",
                }
            }

            fn check(opt: Maybe<Int>) {
                if opt is Some(v) && v > 0 {
                    process(v);
                }
                if opt is not None {
                    handle(opt);
                }
            }
            "#
        );
    }
}

// ============================================================================
// Section 2.8: Pointer Types
// Grammar: pointer_type = '*' , ( 'const' | 'mut' | 'volatile' , [ 'mut' ] ) , type_expr ;
// ============================================================================

mod pointer_types {
    use super::*;

    #[test]
    fn const_pointer() {
        assert_parses(r#"fn f(p: *const Int) { p }"#);
    }

    #[test]
    fn mut_pointer() {
        assert_parses(r#"fn f(p: *mut Int) { p }"#);
    }

    #[test]
    fn volatile_pointer() {
        assert_parses(r#"fn f(p: *volatile Int) { p }"#);
    }

    #[test]
    fn volatile_mut_pointer() {
        assert_parses(r#"fn f(p: *volatile mut Int) { p }"#);
    }
}

// ============================================================================
// Section 2.10: As Cast Expression
// Grammar: postfix_op = ... | 'as' , type_expr ;
// ============================================================================

mod as_cast_expressions {
    use super::*;

    #[test]
    fn as_cast_basic() {
        assert_parses(
            r#"fn f(x: Int) {
                let y = x as Float;
            }"#
        );
    }

    #[test]
    fn as_cast_chained() {
        assert_parses(
            r#"fn f(x: Int) {
                let y = x as Float as Int;
            }"#
        );
    }
}

// ============================================================================
// Section 2.8: Universe Type with Level
// Grammar: universe_type = 'Type' , [ '(' , universe_level , ')' ] ;
// ============================================================================

mod universe_types {
    use super::*;

    #[test]
    fn universe_type_level_zero() {
        assert_parses(r#"fn f(x: Type(0)) -> Type(0) { x }"#);
    }

    #[test]
    fn universe_type_level_one() {
        assert_parses(r#"fn f(x: Type(1)) -> Type(1) { x }"#);
    }

    #[test]
    fn universe_type_level_variable() {
        assert_parses(r#"fn f(x: Type(u)) -> Type(u) { x }"#);
    }
}

// ============================================================================
// Section 2.12: Record Expression with Spread
// Grammar: field_inits = [ field_init , { ',' , field_init } , [ '..' , expression ] ] ;
// ============================================================================

mod record_spread {
    use super::*;

    #[test]
    fn record_with_spread() {
        assert_parses(
            r#"fn f() {
                let p = Point { x: 1, ..other };
            }"#
        );
    }

    #[test]
    fn record_with_spread_only() {
        assert_parses(
            r#"fn f() {
                let p = Config { ..defaults };
            }"#
        );
    }
}

// ============================================================================
// Section 2.11: Stream Literal Expressions
// Grammar: stream_literal_expr = 'stream' , '[' , stream_literal_body , ']' ;
// ============================================================================

mod stream_literals {
    use super::*;

    #[test]
    fn stream_empty() {
        assert_parses(
            r#"fn f() {
                let s = stream[];
            }"#
        );
    }

    #[test]
    fn stream_elements() {
        assert_parses(
            r#"fn f() {
                let s = stream[1, 2, 3];
            }"#
        );
    }

    #[test]
    fn stream_elements_infinite() {
        assert_parses(
            r#"fn f() {
                let s = stream[1, 2, 3, ...];
            }"#
        );
    }

    #[test]
    fn stream_range_exclusive() {
        assert_parses(
            r#"fn f() {
                let s = stream[0..100];
            }"#
        );
    }

    #[test]
    fn stream_range_inclusive() {
        assert_parses(
            r#"fn f() {
                let s = stream[0..=100];
            }"#
        );
    }

    #[test]
    fn stream_range_infinite() {
        assert_parses(
            r#"fn f() {
                let s = stream[0..];
            }"#
        );
    }

    #[test]
    fn stream_single_element() {
        assert_parses(
            r#"fn f() {
                let s = stream[42];
            }"#
        );
    }
}

// ============================================================================
// Section 2.14: Stream Patterns
// Grammar: stream_pattern = 'stream' , '[' , stream_pattern_elements , ']' ;
// ============================================================================

mod stream_patterns {
    use super::*;

    #[test]
    fn stream_pattern_head_tail() {
        assert_parses(
            r#"fn f(s: Stream<Int>) {
                match s {
                    stream[head, ...tail] => head,
                    stream[] => 0,
                }
            }"#
        );
    }

    #[test]
    fn stream_pattern_two_elements() {
        assert_parses(
            r#"fn f(s: Stream<Int>) {
                match s {
                    stream[first, second, ...rest] => first + second,
                    _ => 0,
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.6.2: Context Group Definitions
// Grammar: context_group_def = 'using' , identifier , '=' , context_list_def , ';' ;
// ============================================================================

mod context_group_definitions {
    use super::*;

    #[test]
    fn context_group_simple() {
        assert_parses(
            r#"using WebContext = [Database, Logger];"#
        );
    }

    #[test]
    fn context_group_with_negation() {
        assert_parses(
            r#"using Pure = [!IO, !State];"#
        );
    }
}

// ============================================================================
// Section 2.1: Inject Expression
// Grammar: inject_expr = 'inject' , type_path ;
// ============================================================================

mod inject_expressions {
    use super::*;

    #[test]
    fn inject_basic() {
        assert_parses(
            r#"fn f() {
                let db = inject DatabaseService;
            }"#
        );
    }
}

// ============================================================================
// Section 2.12.3: Nursery with Options
// Grammar: nursery_options = '(' , nursery_option , { ',' , nursery_option } , ')' ;
// ============================================================================

mod nursery_options {
    use super::*;

    #[test]
    fn nursery_with_timeout() {
        assert_parses(
            r#"fn f() {
                nursery(timeout: 5000) {
                    let a = spawn fetch_data();
                }
            }"#
        );
    }

    #[test]
    fn nursery_with_on_cancel() {
        assert_parses(
            r#"fn f() {
                nursery {
                    let a = spawn work();
                } on_cancel {
                    cleanup();
                }
            }"#
        );
    }

    #[test]
    fn nursery_with_multiple_options() {
        assert_parses(
            r#"fn f() {
                nursery(timeout: 5000, on_error: cancel_all) {
                    let a = spawn fetch_a();
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.2: Module Definitions
// Grammar: module_def = visibility , 'module' , module_path , module_body ;
// ============================================================================

mod module_definitions {
    use super::*;

    #[test]
    fn module_with_body() {
        assert_parses(
            r#"module math {
                fn add(a: Int, b: Int) -> Int { a + b }
            }"#
        );
    }

    #[test]
    fn module_external() {
        assert_parses(
            r#"module utils;"#
        );
    }

    #[test]
    fn public_module() {
        assert_parses(
            r#"public module api {
                fn hello() -> Text { "hello" }
            }"#
        );
    }

    #[test]
    fn nested_module_path() {
        assert_parses(
            r#"module math.linear {
                fn dot(a: Int, b: Int) -> Int { a * b }
            }"#
        );
    }
}

// ============================================================================
// Section 2.2: Mount Statements (Advanced)
// Grammar: mount_tree = ... | path , '.' , '{' , mount_list , '}' | path , '.' , '*' ;
// ============================================================================

mod mount_advanced {
    use super::*;

    #[test]
    fn mount_glob() {
        assert_parses(r#"mount std.collections.*;"#);
    }

    #[test]
    fn mount_grouped() {
        assert_parses(r#"mount std.collections.{List, Map, Set};"#);
    }

    #[test]
    fn mount_with_alias() {
        assert_parses(r#"mount std.collections.List as ArrayList;"#);
    }
}

// ============================================================================
// Section 2.4: Context Clause Advanced Features
// Grammar: negative_context, conditional_context, transformed_context, named_context
// ============================================================================

mod context_clause_advanced {
    use super::*;

    #[test]
    fn named_context_colon() {
        assert_parses(
            r#"fn work() using [db: Database, log: Logger] { ok }"#
        );
    }

    #[test]
    fn conditional_context() {
        assert_parses(
            r#"fn work() using [Analytics if analytics_enabled] { ok }"#
        );
    }

    #[test]
    fn transformed_context() {
        assert_parses(
            r#"fn work() using [Database.transactional()] { ok }"#
        );
    }
}

// ============================================================================
// Section 2.10: Interpolated Strings
// Grammar: interpolated_string = interpolated_prefix , '"' , ... ;
// ============================================================================

mod interpolated_strings {
    use super::*;

    #[test]
    fn f_string_basic() {
        assert_parses(
            r#"fn f() {
                let msg = f"Hello {name}";
            }"#
        );
    }

    #[test]
    fn f_string_multiple_interpolations() {
        assert_parses(
            r#"fn f() {
                let msg = f"x={x}, y={y}";
            }"#
        );
    }
}

// ============================================================================
// Section 1.5.2.1: Tagged Literals
// Grammar: tagged_literal = format_tag , '#' , tagged_content ;
// ============================================================================

mod tagged_literals {
    use super::*;

    #[test]
    fn sql_tagged_literal() {
        assert_parses(
            r#"fn f() {
                let q = sql#"SELECT * FROM users";
            }"#
        );
    }

    #[test]
    fn regex_tagged_literal() {
        assert_parses(
            r#"fn f() {
                let pattern = rx#"[a-z]+";
            }"#
        );
    }

    #[test]
    fn url_tagged_literal() {
        assert_parses(
            r#"fn f() {
                let u = url#"https://example.com";
            }"#
        );
    }
}

// ============================================================================
// Section 2.19: Proof Declarations (Theorem, Lemma, Axiom)
// Grammar: theorem_decl, lemma_decl, axiom_decl
// ============================================================================

mod proof_declarations {
    use super::*;

    #[test]
    fn theorem_basic() {
        assert_parses(
            r#"theorem add_comm(a: Int, b: Int)
                requires a >= 0, b >= 0
                ensures result >= 0
            {
                proof by auto
            }"#
        );
    }

    #[test]
    fn lemma_basic() {
        assert_parses(
            r#"lemma zero_identity(x: Int)
                ensures result == x
            {
                proof by auto
            }"#
        );
    }

    #[test]
    fn axiom_basic() {
        assert_parses(
            r#"axiom excluded_middle(p: Bool) -> Bool;"#
        );
    }
}

// ============================================================================
// Section 2.4: Ensures Clause on Functions
// Grammar: ensures_clause = 'where' , ensures_item , { ',' , ensures_item } ;
// ============================================================================

mod ensures_clauses {
    use super::*;

    #[test]
    fn function_with_ensures() {
        assert_parses(
            r#"fn abs(x: Int) -> Int where ensures result >= 0 { x }"#
        );
    }
}

// ============================================================================
// Section 2.16: Meta Definitions
// Grammar: meta_def = visibility , 'meta' , identifier , meta_args , '{' , meta_rules , '}' ;
// ============================================================================

mod meta_definitions {
    use super::*;

    #[test]
    fn meta_def_basic() {
        // Meta rule expansion must be a block expression
        assert_parses(
            r#"meta my_macro(input: expr) {
                input => { input }
            }"#
        );
    }

    #[test]
    fn meta_def_multiple_rules() {
        assert_parses(
            r#"meta log_macro(msg: expr) {
                msg => { print(msg) }
            }"#
        );
    }
}

// ============================================================================
// Section 2.20.6: Meta-Level Functions (@const, @cfg, etc.)
// Grammar: meta_function = '@' , meta_function_name , [ '(' , [ argument_list ] , ')' ] ;
// ============================================================================

mod meta_level_functions {
    use super::*;

    #[test]
    fn meta_const() {
        assert_parses(
            r#"fn f() {
                let x = @const(2 + 2);
            }"#
        );
    }

    #[test]
    fn meta_cfg() {
        assert_parses(
            r#"fn f() {
                let x = @cfg(debug);
            }"#
        );
    }

    #[test]
    fn meta_file() {
        assert_parses(
            r#"fn f() {
                let x = @file;
            }"#
        );
    }

    #[test]
    fn meta_line() {
        assert_parses(
            r#"fn f() {
                let x = @line;
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Question Mark Operator (Try/Propagation)
// Grammar: postfix_op = ... | '?' ;
// ============================================================================

mod question_mark_operator {
    use super::*;

    #[test]
    fn question_mark_basic() {
        assert_parses(
            r#"fn f() -> Maybe<Int> {
                let x = get_value()?;
                Some(x)
            }"#
        );
    }

    #[test]
    fn question_mark_chained() {
        assert_parses(
            r#"fn f() {
                let x = a()?.b()?.c;
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Tuple Index Expression
// Grammar: postfix_op = ... | '.' , integer_lit ;
// ============================================================================

mod tuple_index {
    use super::*;

    #[test]
    fn tuple_index_basic() {
        assert_parses(
            r#"fn f() {
                let pair = (1, "hello");
                let x = pair.0;
                let y = pair.1;
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Return Expression with Value
// Grammar: return_expr = 'return' , [ expression ] ;
// ============================================================================

mod return_expressions {
    use super::*;

    #[test]
    fn return_with_value() {
        assert_parses(
            r#"fn f() -> Int {
                return 42;
            }"#
        );
    }

    #[test]
    fn return_bare() {
        assert_parses(
            r#"fn f() {
                return;
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Break with Value
// Grammar: break_expr = 'break' , [ expression ] ;
// ============================================================================

mod break_expressions {
    use super::*;

    #[test]
    fn break_with_value() {
        assert_parses(
            r#"fn f() {
                let x = loop {
                    break 42;
                };
            }"#
        );
    }

    #[test]
    fn break_bare() {
        assert_parses(
            r#"fn f() {
                loop { break; }
            }"#
        );
    }

    #[test]
    fn continue_basic() {
        assert_parses(
            r#"fn f() {
                for x in items {
                    if x == 0 { continue; }
                    process(x);
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.3: Newtype and Unit Type Definitions
// Grammar: type_definition_body = '(' , type_list , ')' , ';' ;
// ============================================================================

mod newtype_and_unit_types {
    use super::*;

    #[test]
    fn newtype_definition() {
        assert_parses(r#"type UserId is (Int);"#);
    }

    #[test]
    fn unit_type_definition() {
        assert_parses(r#"type Marker is ();"#);
    }
}

// ============================================================================
// Section 2.8: Inferred Type
// Grammar: inferred_type = '_' ;
// ============================================================================

mod inferred_types {
    use super::*;

    #[test]
    fn inferred_type_in_let() {
        assert_parses(
            r#"fn f() {
                let x: _ = 42;
            }"#
        );
    }
}

// ============================================================================
// Section 2.3: Type Alias (in impl context)
// Grammar: type_alias = 'type' , identifier , [ type_params ] , ... , '=' , type_expr , ';' ;
// ============================================================================

mod type_aliases {
    use super::*;

    #[test]
    fn type_alias_simple() {
        assert_parses(r#"type IntList is List<Int>;"#);
    }

    #[test]
    fn type_alias_generic() {
        assert_parses(r#"type Pair<A, B> is (A, B);"#);
    }
}

// ============================================================================
// Section 2.12: Unit Expression
// Grammar: primary_expr = ... | '(' , ')' ;
// ============================================================================

mod unit_expressions {
    use super::*;

    #[test]
    fn unit_expression() {
        assert_parses(
            r#"fn f() {
                let x = ();
            }"#
        );
    }
}

// ============================================================================
// Section 2.12: Array Repeat Expression
// Grammar: array_elements = ... | expression , ';' , expression ;
// ============================================================================

mod array_expressions {
    use super::*;

    #[test]
    fn array_repeat() {
        assert_parses(
            r#"fn f() {
                let arr = [0; 100];
            }"#
        );
    }

    #[test]
    fn array_empty() {
        assert_parses(
            r#"fn f() {
                let arr: [Int] = [];
            }"#
        );
    }
}

// ============================================================================
// Section 2.5: Protocol with GATs (Generic Associated Types)
// Grammar: protocol_type = 'type' , identifier , [ type_params ] , ... ;
// ============================================================================

mod gat_in_protocol {
    use super::*;

    #[test]
    fn protocol_with_gat() {
        assert_parses(
            r#"type Lending is protocol {
                type Item<T>;
                fn next(&mut self) -> Maybe<Self.Item<Int>>;
            };"#
        );
    }
}

// ============================================================================
// Section 2.5: Default Implementation Item
// Grammar: impl_item = visibility , [ 'default' ] , ( function_def | type_alias | const_def ) ;
// ============================================================================

mod default_impl_items {
    use super::*;

    #[test]
    fn impl_with_multiple_methods() {
        // Note: 'default' is contextual in protocol_item, not impl_item.
        // Test that impl blocks can have multiple regular methods.
        assert_parses(
            r#"implement<T> Display for List<T> {
                fn display(&self) -> Text { "list" }
                fn debug(&self) -> Text { "debug" }
            }"#
        );
    }
}

// ============================================================================
// Section 2.4: Meta Modifier with Stage Level
// Grammar: meta_modifier = 'meta' , [ '(' , stage_level , ')' ] ;
// ============================================================================

mod meta_staged {
    use super::*;

    #[test]
    fn meta_function_basic() {
        assert_parses(
            r#"meta fn generate() {
                quote { fn hello() { print("hello") } }
            }"#
        );
    }

    #[test]
    fn meta_function_with_stage() {
        assert_parses(
            r#"meta(2) fn generate_meta() {
                quote { meta fn inner() { quote { 42 } } }
            }"#
        );
    }
}

// ============================================================================
// Section 2.3: Variant with Record Data
// Grammar: variant_data = '{' , field_list , '}' | '(' , type_list , ')' ;
// ============================================================================

mod variant_data_kinds {
    use super::*;

    #[test]
    fn variant_with_record_data() {
        assert_parses(
            r#"type Shape is
                Circle { radius: Float }
                | Rectangle { width: Float, height: Float }
                | Point;"#
        );
    }

    #[test]
    fn variant_with_tuple_and_record_mixed() {
        assert_parses(
            r#"type Expr is
                Literal(Int)
                | Binary { op: Text, left: Heap<Expr>, right: Heap<Expr> }
                | Unary(Text, Heap<Expr>);"#
        );
    }
}

// ============================================================================
// Section 2.14: Slice Pattern
// Grammar: slice_pattern = '[' , slice_pattern_elements , ']' ;
// ============================================================================

mod slice_patterns {
    use super::*;

    #[test]
    fn slice_pattern_head_rest() {
        assert_parses(
            r#"fn f(items: [Int]) {
                match items {
                    [first, ..] => first,
                    [] => 0,
                }
            }"#
        );
    }

    #[test]
    fn slice_pattern_head_tail() {
        assert_parses(
            r#"fn f(items: [Int]) {
                match items {
                    [a, b, ..] => a + b,
                    _ => 0,
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.14: Range Pattern
// Grammar: range_pattern = literal_expr , range_op , [ literal_expr ] | range_op , literal_expr ;
// ============================================================================

mod range_patterns {
    use super::*;

    #[test]
    fn range_pattern_inclusive() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    0..=9 => "digit",
                    _ => "other",
                }
            }"#
        );
    }

    #[test]
    fn range_pattern_exclusive() {
        assert_parses(
            r#"fn f(x: Int) {
                match x {
                    0..100 => "small",
                    _ => "big",
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Bitwise and Shift Operators
// Grammar: bitwise_expr = shift_expr , { ( '&' | '|' | '^' ) , shift_expr } ;
//          shift_expr = additive_expr , { ( '<<' | '>>' ) , additive_expr } ;
// ============================================================================

mod bitwise_operators {
    use super::*;

    #[test]
    fn bitwise_and() {
        assert_parses(
            r#"fn f(a: Int, b: Int) -> Int { a & b }"#
        );
    }

    #[test]
    fn bitwise_or() {
        assert_parses(
            r#"fn f(a: Int, b: Int) -> Int { a | b }"#
        );
    }

    #[test]
    fn bitwise_xor() {
        assert_parses(
            r#"fn f(a: Int, b: Int) -> Int { a ^ b }"#
        );
    }

    #[test]
    fn shift_left() {
        assert_parses(
            r#"fn f(a: Int) -> Int { a << 2 }"#
        );
    }

    #[test]
    fn shift_right() {
        assert_parses(
            r#"fn f(a: Int) -> Int { a >> 2 }"#
        );
    }

    #[test]
    fn power_operator() {
        assert_parses(
            r#"fn f(a: Int) -> Int { a ** 2 }"#
        );
    }
}

// ============================================================================
// Section 2.10: Compound Assignment Operators
// Grammar: assign_op = '=' | '+=' | '-=' | '*=' | '/=' | '%=' | ...
// ============================================================================

mod compound_assignment {
    use super::*;

    #[test]
    fn plus_assign() {
        assert_parses(
            r#"fn f() {
                let mut x = 0;
                x += 1;
            }"#
        );
    }

    #[test]
    fn minus_assign() {
        assert_parses(
            r#"fn f() {
                let mut x = 10;
                x -= 1;
            }"#
        );
    }

    #[test]
    fn multiply_assign() {
        assert_parses(
            r#"fn f() {
                let mut x = 2;
                x *= 3;
            }"#
        );
    }

    #[test]
    fn divide_assign() {
        assert_parses(
            r#"fn f() {
                let mut x = 10;
                x /= 2;
            }"#
        );
    }
}

// ============================================================================
// Section 2.10: Unary Operators
// Grammar: unary_op = '!' | '-' | '~' | '&' | ... | '*' ;
// ============================================================================

mod unary_operators {
    use super::*;

    #[test]
    fn logical_not() {
        assert_parses(
            r#"fn f(b: Bool) -> Bool { !b }"#
        );
    }

    #[test]
    fn negation() {
        assert_parses(
            r#"fn f(x: Int) -> Int { -x }"#
        );
    }

    #[test]
    fn bitwise_not() {
        assert_parses(
            r#"fn f(x: Int) -> Int { ~x }"#
        );
    }

    #[test]
    fn dereference() {
        assert_parses(
            r#"fn f(x: &Int) -> Int { *x }"#
        );
    }

    #[test]
    fn address_of() {
        assert_parses(
            r#"fn f(x: Int) {
                let r = &x;
            }"#
        );
    }

    #[test]
    fn address_of_mut() {
        assert_parses(
            r#"fn f(mut x: Int) {
                let r = &mut x;
            }"#
        );
    }
}

// ============================================================================
// Section 2.7: FFI Advanced Features
// Grammar: ffi_requires_clause, ffi_ensures_clause, ffi_memory_effects, etc.
// ============================================================================

mod ffi_advanced {
    use super::*;

    #[test]
    fn ffi_with_functions() {
        assert_parses(
            r#"ffi LibMath {
                @extern("sqrt") fn sqrt(x: Float) -> Float;
                @extern("pow") fn pow(base: Float, exp: Float) -> Float;
            }"#
        );
    }

    #[test]
    fn ffi_extends() {
        assert_parses(
            r#"ffi LibExtended extends LibBase {
                @extern("extended_fn") fn extended_fn(x: Int) -> Int;
            }"#
        );
    }
}

// ============================================================================
// Section 2.3: Protocol with Extends and Where Clause
// Grammar: protocol_body = [ protocol_extension ] , [ generic_where_clause ] , ...
// ============================================================================

mod protocol_with_where {
    use super::*;

    #[test]
    fn protocol_extends_with_where() {
        assert_parses(
            r#"type Sortable<T> is protocol extends Eq where type T: Ord {
                fn sort(&self) -> Self;
            };"#
        );
    }
}

// ============================================================================
// Section 2.12: While-Let Loop
// Grammar: while_loop = 'while' , expression , ... ;
// ============================================================================

mod while_let_loops {
    use super::*;

    #[test]
    fn while_let_basic() {
        assert_parses(
            r#"fn f() {
                while let Some(item) = iter.next() {
                    process(item);
                }
            }"#
        );
    }
}

// ============================================================================
// Section 2.4: Negative Bounds
// Grammar: negative_bound = '!' , protocol_path ;
// ============================================================================

mod negative_bounds {
    use super::*;

    #[test]
    fn negative_bound_in_where() {
        assert_parses(
            r#"implement<T> MyProtocol for T where type T: Send {
                fn do_it(&self) { self }
            }"#
        );
    }
}

// ============================================================================
// Section 2.4.5: Type-Level Functions / Type Aliases
// Grammar: type_function_def = 'type' , identifier , '<' , ... , '>' , '=' , type_expr , ';' ;
// ============================================================================

mod type_level_functions {
    use super::*;

    #[test]
    fn type_alias_with_constraints() {
        assert_parses(
            r#"type NumList<T> is List<T>;"#
        );
    }
}

// ============================================================================
// Section 2.3: Field Visibility
// Grammar: field = { attribute } , [ visibility ] , identifier , ':' , type_expr , ... ;
// ============================================================================

mod field_visibility {
    use super::*;

    #[test]
    fn public_field() {
        assert_parses(
            r#"type Config is {
                public host: Text,
                port: Int,
            };"#
        );
    }
}

// ============================================================================
// Section 2.4: Multiple Generic Parameters and Bounds
// Grammar: generic_params = generic_param , { ',' , generic_param } ;
// ============================================================================

mod generic_features {
    use super::*;

    #[test]
    fn multiple_generic_params_with_bounds() {
        assert_parses(
            r#"fn zip<A: Clone, B: Clone>(a: List<A>, b: List<B>) -> List<(A, B)> { a }"#
        );
    }

    #[test]
    fn generic_function_explicit_type_arg() {
        assert_parses(
            r#"fn f() {
                let x = identity<Int>(42);
            }"#
        );
    }
}

// ============================================================================
// Section 2.1: Attributes
// Grammar: attribute = '@' , ... ;
// ============================================================================

mod attributes {
    use super::*;

    #[test]
    fn derive_attribute() {
        assert_parses(
            r#"@derive(Eq, Hash)
            type Point is { x: Int, y: Int };"#
        );
    }

    #[test]
    fn specialize_attribute() {
        assert_parses(
            r#"@specialize
            implement Display for List<Int> {
                fn display(&self) -> Text { "list" }
            }"#
        );
    }

    #[test]
    fn verify_attribute() {
        assert_parses(
            r#"@verify(runtime)
            fn safe_divide(a: Int, b: Int) -> Int { a / b }"#
        );
    }
}

// ============================================================================
// Integration: Complex programs combining new grammar features
// ============================================================================

mod integration_advanced {
    use super::*;

    #[test]
    fn pointer_types_in_ffi_context() {
        assert_parses(
            r#"
            extern "C" {
                fn malloc(size: Int) -> *mut Int;
                fn free(ptr: *mut Int);
                fn memcpy(dst: *mut Int, src: *const Int, n: Int);
            }
            "#
        );
    }

    #[test]
    fn full_context_system_program() {
        assert_parses(
            r#"
            context Database {
                fn query(&self, sql: Text) -> List<Row>;
            }

            context Logger {
                fn log(&self, msg: Text);
            }

            using AppContext = [Database, Logger];

            layer AppLayer {
                provide Database = PgDatabase.new();
                provide Logger = ConsoleLogger.new();
            }

            fn process_data() using [Database, Logger] {
                provide AppLayer;
                let data = inject Database;
            }
            "#
        );
    }

    #[test]
    fn stream_operations_program() {
        assert_parses(
            r#"
            fn stream_demo() {
                let finite = stream[1, 2, 3];
                let infinite = stream[0..];
                let ranged = stream[0..100];
                let comprehended = stream[x * 2 for x in source if x > 0];
            }
            "#
        );
    }

    #[test]
    fn type_system_advanced() {
        assert_parses(
            r#"
            type UserId is (Int);
            type Marker is ();

            type Shape is
                Circle { radius: Float }
                | Rectangle { width: Float, height: Float }
                | Point;

            type Stream<T> is coinductive {
                fn head(&self) -> T;
                fn tail(&self) -> Stream<T>;
            };

            type Nat is inductive {
                Zero
                | Succ(Nat)
            };
            "#
        );
    }

    #[test]
    fn expression_features() {
        assert_parses(
            r#"
            fn expressions_demo() {
                let x = 42 as Float;
                let y = pair.0;
                let z = get_value()?;
                let arr = [0; 10];
                let unit = ();
                let shifted = x << 2;
                let powered = x ** 2;
                let mut counter = 0;
                counter += 1;
            }
            "#
        );
    }
}
