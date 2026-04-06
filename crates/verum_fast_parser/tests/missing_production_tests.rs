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
//! Tests for 12 previously untested grammar productions.
//!
//! Each production gets at least 3 tests:
//!   - Happy path (valid syntax)
//!   - Error case (invalid syntax produces error)
//!   - Edge case (complex/nested usage)
//!
//! Productions tested:
//!   1. provide_stmt
//!   2. context_expr (context_def)
//!   3. nursery_block (nursery_expr)
//!   4. spawn_expr
//!   5. defer_stmt
//!   6. errdefer_stmt
//!   7. pipe_expr
//!   8. lambda_expr
//!   9. mount_stmt
//!  10. rank2_fn_type
//!  11. ffi_block (extern_block)
//!  12. newtype_def

use verum_ast::{FileId, ItemKind, Module, FunctionBody, Spanned, StmtKind, ExprKind};
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::{VerumParser, ParseError};

// =============================================================================
// HELPERS
// =============================================================================

fn parse_module(source: &str) -> Result<Module, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

fn assert_parses(source: &str) {
    parse_module(source).unwrap_or_else(|e| {
        let msgs: Vec<String> = e.iter().map(|err| format!("{:?}", err)).collect();
        panic!("Failed to parse:\n{}\nErrors: {}", source, msgs.join("\n"))
    });
}

fn assert_fails(source: &str) {
    if parse_module(source).is_ok() {
        panic!("Expected parse failure but succeeded:\n{}", source);
    }
}

/// Parse source as a function body and return the statements.
fn parse_stmts(source: &str) -> Module {
    let wrapped = format!("fn __test__() {{ {} }}", source);
    parse_module(&wrapped).unwrap_or_else(|e| {
        let msgs: Vec<String> = e.iter().map(|err| format!("{:?}", err)).collect();
        panic!("Failed to parse:\n{}\nErrors: {}", source, msgs.join("\n"))
    })
}

// =============================================================================
// 1. PROVIDE_STMT
// Grammar: provide_stmt = 'provide' context_path ['as' identifier] '=' expression (';' | 'in' block_expr)
//                        | 'provide' identifier ';' ;
// =============================================================================

#[test]
fn test_provide_stmt_basic() {
    assert_parses("fn test() { provide Logger = ConsoleLogger.new(); }");
}

#[test]
fn test_provide_stmt_with_alias() {
    assert_parses("fn test() { provide Database as PrimaryDb = PostgresDb.new(); }");
}

#[test]
fn test_provide_stmt_layer_shorthand() {
    assert_parses("fn test() { provide AppLayer; }");
}

#[test]
fn test_provide_stmt_missing_equals() {
    // Missing `=` should fail
    assert_fails("fn test() { provide Logger ConsoleLogger.new(); }");
}

#[test]
fn test_provide_stmt_in_block_scoped() {
    // provide X = expr in { block } form
    assert_parses(r#"
        fn test() {
            provide Logger = ConsoleLogger.new() in {
                do_work();
            };
        }
    "#);
}

#[test]
fn test_provide_stmt_nested() {
    assert_parses(r#"
        fn test() {
            provide Logger = ConsoleLogger.new();
            provide Database = PostgresDb.new();
        }
    "#);
}

// =============================================================================
// 2. CONTEXT_DEF
// Grammar: context_def = visibility ('context' ['async'] | 'async' 'context') identifier
//                         [generics] '{' {context_item} '}'
// =============================================================================

#[test]
fn test_context_def_basic() {
    assert_parses(r#"
        context Database {
            fn query(sql: Text) -> Result<Rows, Error>;
        }
    "#);
}

#[test]
fn test_context_def_async() {
    assert_parses(r#"
        context async Database {
            async fn query(sql: Text) -> Result<Rows, Error>;
        }
    "#);
}

#[test]
fn test_context_def_alt_async_syntax() {
    assert_parses(r#"
        async context Network {
            async fn fetch(url: Text) -> Result<Response, Error>;
        }
    "#);
}

#[test]
fn test_context_def_empty_body() {
    assert_parses("context Empty {}");
}

#[test]
fn test_context_def_multiple_methods() {
    assert_parses(r#"
        context FileSystem {
            fn read(path: Text) -> Result<Text, Error>;
            fn write(path: Text, data: Text) -> Result<(), Error>;
            fn exists(path: Text) -> Bool;
        }
    "#);
}

#[test]
fn test_context_def_missing_braces() {
    assert_fails("context Database fn query();");
}

// =============================================================================
// 3. NURSERY_EXPR
// Grammar: nursery_expr = 'nursery' [nursery_options] block_expr [nursery_handlers]
// =============================================================================

#[test]
fn test_nursery_basic() {
    assert_parses(r#"
        fn test() {
            nursery {
                spawn fetch_data();
                spawn process_data();
            };
        }
    "#);
}

#[test]
fn test_nursery_with_recover() {
    assert_parses(r#"
        fn test() {
            nursery {
                spawn task1();
                spawn task2();
            } recover {
                err => default_value()
            };
        }
    "#);
}

#[test]
fn test_nursery_missing_block() {
    assert_fails("fn test() { nursery; }");
}

#[test]
fn test_nursery_nested() {
    assert_parses(r#"
        fn test() {
            nursery {
                nursery {
                    spawn inner_task();
                };
                spawn outer_task();
            };
        }
    "#);
}

// =============================================================================
// 4. SPAWN_EXPR
// Grammar: spawn_expr = 'spawn' ['using' '[' identifier_list ']'] expression
// =============================================================================

#[test]
fn test_spawn_basic() {
    assert_parses("fn test() { spawn fetch_data(); }");
}

#[test]
fn test_spawn_with_context_forwarding() {
    assert_parses("fn test() { spawn using [Database, Logger] process_request(); }");
}

#[test]
fn test_spawn_async_block() {
    assert_parses(r#"
        fn test() {
            spawn async {
                let data = fetch();
                process(data);
            };
        }
    "#);
}

#[test]
fn test_spawn_missing_expr() {
    assert_fails("fn test() { spawn; }");
}

#[test]
fn test_spawn_complex_expr() {
    assert_parses("fn test() { let handle = spawn compute(x + y * 2); }");
}

// =============================================================================
// 5. DEFER_STMT
// Grammar: defer_stmt = 'defer' defer_body | 'errdefer' defer_body
//          defer_body = expression ';' | block_expr
// =============================================================================

#[test]
fn test_defer_basic() {
    assert_parses("fn test() { defer cleanup(); }");
}

#[test]
fn test_defer_block() {
    assert_parses(r#"
        fn test() {
            defer {
                file.close();
                log("done");
            };
        }
    "#);
}

#[test]
fn test_defer_missing_body() {
    assert_fails("fn test() { defer; }");
}

#[test]
fn test_defer_multiple() {
    assert_parses(r#"
        fn test() {
            defer cleanup1();
            defer cleanup2();
            defer cleanup3();
        }
    "#);
}

// =============================================================================
// 6. ERRDEFER_STMT
// Grammar: defer_stmt = ... | 'errdefer' defer_body
// =============================================================================

#[test]
fn test_errdefer_basic() {
    assert_parses("fn test() { errdefer rollback(); }");
}

#[test]
fn test_errdefer_block() {
    assert_parses(r#"
        fn test() {
            errdefer {
                log("Error occurred");
                rollback_transaction();
            };
        }
    "#);
}

#[test]
fn test_errdefer_missing_body() {
    assert_fails("fn test() { errdefer; }");
}

#[test]
fn test_errdefer_combined_with_defer() {
    assert_parses(r#"
        fn test() {
            defer file.close();
            errdefer log("failed");
            do_work();
        }
    "#);
}

// =============================================================================
// 7. PIPE_EXPR
// Grammar: pipe_op = '|>'
// =============================================================================

#[test]
fn test_pipe_basic() {
    assert_parses("fn test() { x |> f; }");
}

#[test]
fn test_pipe_chained() {
    assert_parses("fn test() { data |> transform |> filter |> collect; }");
}

#[test]
fn test_pipe_with_method_calls() {
    assert_parses("fn test() { values |> sort() |> take(10); }");
}

#[test]
fn test_pipe_incomplete() {
    assert_fails("fn test() { x |>; }");
}

#[test]
fn test_pipe_complex_lhs() {
    assert_parses("fn test() { (a + b) |> process; }");
}

#[test]
fn test_pipe_in_let() {
    assert_parses("fn test() { let result = data |> transform |> collect; }");
}

// =============================================================================
// 8. LAMBDA_EXPR
// Grammar: lambda_expr = '|' param_list_lambda '|' expression
//          (also: \x -> expr syntax may exist)
// =============================================================================

#[test]
fn test_lambda_closure_basic() {
    assert_parses("fn test() { let f = |x| x + 1; }");
}

#[test]
fn test_lambda_closure_multi_param() {
    assert_parses("fn test() { let f = |x, y| x + y; }");
}

#[test]
fn test_lambda_closure_no_params() {
    assert_parses("fn test() { let f = || 42; }");
}

#[test]
fn test_lambda_closure_block_body() {
    assert_parses(r#"
        fn test() {
            let f = |x| {
                let y = x * 2;
                y + 1
            };
        }
    "#);
}

#[test]
fn test_lambda_closure_nested() {
    assert_parses("fn test() { let f = |x| |y| x + y; }");
}

#[test]
fn test_lambda_closure_with_pipe() {
    assert_parses("fn test() { data |> |x| x + 1; }");
}

// =============================================================================
// 9. MOUNT_STMT
// Grammar: mount_stmt = 'mount' mount_tree ['as' identifier] ';'
//          mount_tree = mount_item ['as' identifier]
//                     | path '.' '{' mount_list '}'
//                     | path '.' '*'
// =============================================================================

#[test]
fn test_mount_simple_path() {
    assert_parses("mount std.io.File;");
}

#[test]
fn test_mount_group_import() {
    assert_parses("mount core.collections.{List, Map, Set};");
}

#[test]
fn test_mount_glob() {
    assert_parses("mount std.io.*;");
}

#[test]
fn test_mount_with_alias() {
    assert_parses("mount std.collections.HashMap as Map;");
}

#[test]
fn test_mount_invalid_syntax() {
    // Double dot in path is invalid
    assert_fails("mount std..io..File;");
}

#[test]
fn test_mount_deeply_nested() {
    assert_parses("mount cog.vendor.lib.module.submodule.Type;");
}

#[test]
fn test_mount_multiple() {
    assert_parses(r#"
        mount core.collections.List;
        mount core.collections.Map;
        mount core.io.File;
    "#);
}

// =============================================================================
// 10. RANK2_FN_TYPE
// Grammar: rank2_function_type = ['async'] 'fn' generics '(' type_list ')'
//                                ['->' type_expr] [context_clause] [generic_where_clause]
// =============================================================================

#[test]
fn test_rank2_fn_type_basic() {
    assert_parses("type Transformer is { transform: fn<R>(R) -> R };");
}

#[test]
fn test_rank2_fn_type_multi_params() {
    assert_parses("type Reducer is { reduce: fn<R>(R, Int) -> R };");
}

#[test]
fn test_rank2_fn_type_in_function_param() {
    assert_parses("fn apply(f: fn<T>(T) -> T, x: Int) -> Int { f(x) }");
}

#[test]
fn test_rank2_fn_type_complex() {
    // Transducer pattern: fn<R>(Reducer<B, R>) -> Reducer<A, R>
    assert_parses(r#"
        type Transducer<A, B> is {
            transform: fn<R>(fn(R, B) -> R) -> fn(R, A) -> R,
        };
    "#);
}

#[test]
fn test_rank2_fn_type_missing_generics() {
    // Regular fn type (not rank-2) should still work
    assert_parses("type Callback is fn(Int) -> Int;");
}

// =============================================================================
// 11. FFI_BLOCK (extern_block)
// Grammar: extern_block = 'extern' [string_lit] '{' {extern_fn_decl} '}'
//          extern_fn_decl = [visibility] 'fn' identifier [generics]
//                           '(' param_list ')' ['->' type_expr] ';'
// =============================================================================

#[test]
fn test_extern_block_c_abi() {
    assert_parses(r#"
        extern "C" {
            fn malloc(size: Int) -> &unsafe Void;
            fn free(ptr: &unsafe Void);
        }
    "#);
}

#[test]
fn test_extern_block_no_abi() {
    assert_parses(r#"
        extern {
            fn custom_func(x: Int) -> Int;
        }
    "#);
}

#[test]
fn test_extern_block_empty() {
    assert_parses(r#"extern "C" {}"#);
}

#[test]
fn test_extern_block_unclosed_brace() {
    // Unclosed brace should fail
    assert_fails(r#"extern "C" { fn malloc(size: Int) -> &unsafe Void;"#);
}

#[test]
fn test_extern_block_multiple_functions() {
    assert_parses(r#"
        extern "C" {
            fn open(path: &Text, flags: Int) -> Int;
            fn read(fd: Int, buf: &unsafe Void, count: Int) -> Int;
            fn write(fd: Int, buf: &Void, count: Int) -> Int;
            fn close(fd: Int) -> Int;
        }
    "#);
}

#[test]
fn test_extern_block_with_pub() {
    assert_parses(r#"
        extern "C" {
            pub fn printf(fmt: &Text) -> Int;
        }
    "#);
}

// =============================================================================
// 12. NEWTYPE_DEF
// Grammar: type_definition_body = ... | '(' type_list ')' ';'
//          (newtype: type Name is (InnerType);)
// =============================================================================

#[test]
fn test_newtype_basic() {
    assert_parses("type UserId is (Int);");
}

#[test]
fn test_newtype_text() {
    assert_parses("type Email is (Text);");
}

#[test]
fn test_newtype_unit() {
    assert_parses("type Marker is ();");
}

#[test]
fn test_newtype_alias() {
    // Type alias: type X is Y; (not wrapped in parens)
    assert_parses("type Count is Int;");
}

#[test]
fn test_newtype_with_generics() {
    assert_parses("type Wrapper<T> is (T);");
}

#[test]
fn test_newtype_missing_semicolon() {
    assert_fails("type UserId is (Int)");
}

#[test]
fn test_newtype_multiple_fields_tuple() {
    // Tuple newtype with multiple fields
    assert_parses("type Point is (Float, Float);");
}

// =============================================================================
// CROSS-PRODUCTION INTEGRATION TESTS
// =============================================================================

#[test]
fn test_full_program_with_many_productions() {
    assert_parses(r#"
        mount core.io.File;
        mount core.collections.{List, Map};

        context Logger {
            fn log(msg: Text);
        }

        type UserId is (Int);

        extern "C" {
            fn get_time() -> Int;
        }

        fn process(data: List<Int>) -> Int using [Logger] {
            provide Logger = ConsoleLogger.new();
            defer cleanup();
            let result = data |> filter(|x| x > 0) |> sum;
            result
        }
    "#);
}

#[test]
fn test_async_spawn_with_nursery() {
    assert_parses(r#"
        async fn run_tasks() {
            nursery {
                spawn fetch_a();
                spawn fetch_b();
            };
        }
    "#);
}

#[test]
fn test_defer_errdefer_combined_in_function() {
    assert_parses(r#"
        fn risky_operation() -> Result<Int, Error> {
            let file = open("data.txt");
            defer file.close();
            errdefer log("Operation failed");
            let data = file.read();
            process(data)
        }
    "#);
}
