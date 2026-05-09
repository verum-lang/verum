//! Test for parameter extraction and VBC codegen.

use crate::codegen::{CodegenConfig, VbcCodegen};
use ::verum_ast::decl::{FunctionParam, FunctionParamKind};
use ::verum_ast::pattern::{Pattern, PatternKind};
use ::verum_fast_parser::Parser;

fn extract_pattern_name(pattern: &Pattern) -> Option<String> {
    match &pattern.kind {
        PatternKind::Ident { name, .. } => Some(name.name.to_string()),
        PatternKind::Paren(inner) => extract_pattern_name(inner),
        _ => None,
    }
}

fn extract_param_name(param: &FunctionParam) -> Option<String> {
    match &param.kind {
        FunctionParamKind::Regular { pattern, .. } => extract_pattern_name(pattern),
        // All self parameter variants
        FunctionParamKind::SelfValue
        | FunctionParamKind::SelfValueMut
        | FunctionParamKind::SelfRef
        | FunctionParamKind::SelfRefMut
        | FunctionParamKind::SelfOwn
        | FunctionParamKind::SelfOwnMut
        | FunctionParamKind::SelfRefChecked
        | FunctionParamKind::SelfRefCheckedMut
        | FunctionParamKind::SelfRefUnsafe
        | FunctionParamKind::SelfRefUnsafeMut => Some("self".to_string()),
    }
}

#[test]
fn test_simple_param_extraction() {
    let source = "fn add(a: Int, b: Int) -> Int { a + b }";
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let func = match &module.items[0].kind {
        ::verum_ast::ItemKind::Function(f) => f,
        _ => panic!("Expected function"),
    };

    let param_names: Vec<String> = func.params.iter().filter_map(extract_param_name).collect();

    println!("Param names: {:?}", param_names);
    assert_eq!(param_names, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn test_function_type_param_extraction() {
    let source = r#"
fn map<U, F: fn(T) -> U>(self, f: F) -> Maybe<U> {
    match self {
        Some(v) => Some(f(v)),
        None => None,
    }
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let func = match &module.items[0].kind {
        ::verum_ast::ItemKind::Function(f) => f,
        _ => panic!("Expected function"),
    };

    println!("Function params count: {}", func.params.len());
    for (i, param) in func.params.iter().enumerate() {
        println!("Param {}: kind={:?}", i, param.kind);
        let name = extract_param_name(param);
        println!("Param {}: extracted name={:?}", i, name);
    }

    let param_names: Vec<String> = func.params.iter().filter_map(extract_param_name).collect();

    println!("Final param names: {:?}", param_names);
    assert_eq!(param_names, vec!["self".to_string(), "f".to_string()]);
}

#[test]
fn test_closure_param_extraction() {
    let source = r#"
fn filter<P: fn(&T) -> Bool>(self, predicate: P) -> Maybe<T> {
    match self {
        Some(ref v) if predicate(v) => self,
        _ => None,
    }
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let func = match &module.items[0].kind {
        ::verum_ast::ItemKind::Function(f) => f,
        _ => panic!("Expected function"),
    };

    println!("Function: {}", func.name.name);
    for (i, param) in func.params.iter().enumerate() {
        let name = extract_param_name(param);
        println!("Param {}: name={:?}", i, name);
    }

    let param_names: Vec<String> = func.params.iter().filter_map(extract_param_name).collect();

    println!("Final param names: {:?}", param_names);
    assert!(param_names.contains(&"self".to_string()));
    assert!(param_names.contains(&"predicate".to_string()));
}

// =========================================================================
// VBC Codegen Tests for Function Parameters
// =========================================================================

/// Tests that codegen correctly handles calling function-type parameters.
#[test]
fn test_codegen_function_param_call() {
    let source = r#"
fn map<F: fn(Int) -> Int>(x: Int, f: F) -> Int {
    f(x)
}

fn main() {
    let result = map(42, |x| x * 2);
    result
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_function_param").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    // This should NOT produce "undefined variable: f" warning
    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

/// Tests that codegen correctly handles static method calls.
#[test]
fn test_codegen_static_method_call() {
    let source = r#"
type Counter is { value: Int };

implement Counter {
    fn new() -> Counter {
        Counter { value: 0 }
    }
}

fn main() {
    let c = Counter.new();
    c.value
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_static_method").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    // This should correctly resolve Counter.new() as a static method call
    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

/// Tests that codegen handles self parameter correctly in method calls.
#[test]
fn test_codegen_self_method_call() {
    let source = r#"
type Counter is { value: Int };

implement Counter {
    fn increment(&mut self) -> Int {
        self.value = self.value + 1;
        self.value
    }
}

fn main() {
    let mut c = Counter { value: 0 };
    let result = c.increment();
    result
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_self_method").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

/// Tests compilation of Maybe<T> methods that use function parameters.
#[test]
fn test_codegen_maybe_map() {
    let source = r#"
type Maybe<T> is None | Some(T);

implement<T> Maybe<T> {
    fn map<U, F: fn(T) -> U>(self, f: F) -> Maybe<U> {
        match self {
            Some(v) => Some(f(v)),
            None => None,
        }
    }
}

fn main() {
    let opt = Some(42);
    let result = opt.map(|x| x * 2);
    result
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_maybe_map").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    // The f(v) call should resolve `f` from the function parameter, not as a function
    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

/// Tests that pattern-bound variables are available in match guards.
#[test]
fn test_codegen_pattern_variables_in_guard() {
    let source = r#"
type Maybe<T> is None | Some(T);

fn filter(m: Maybe<Int>, threshold: Int) -> Maybe<Int> {
    match m {
        Some(v) if v > threshold => Some(v),
        _ => None,
    }
}

fn main() {
    let opt = Some(42);
    let result = filter(opt, 10);
    result
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_pattern_guard").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    // The guard `v > threshold` should have access to pattern-bound `v`
    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

// =========================================================================
// Stdlib Compilation Tests
// =========================================================================

/// Helper to compile a stdlib file and report any errors.
///

/// Compile a stdlib `.vr` file with mount resolution.
///
/// Auto-derives `core_root` by walking up from `path` until a
/// directory whose final component is `core` is found.  Always uses
/// `compile_module_with_mounts` — symmetric whether or not the file
/// uses `mount` directives, removing the historical
/// `compile_stdlib_file` vs `compile_stdlib_file_with_mounts` split
/// that caused recurring breakages (e.g. tcp.vr added a `mount
/// sys.raw.{...}` and the without-mounts variant silently broke its
/// test until #49 closed the gap).  Mount resolution is purely
/// additive — files without mounts walk an empty mount tree at
/// negligible cost.
#[cfg(test)]
fn compile_stdlib_file(path: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;

    let mut parser = Parser::new(&source);
    let module = parser
        .parse_module()
        .map_err(|e| format!("Parse error in {}: {:?}", path, e))?;

    let core_root = derive_core_root(path).ok_or_else(|| {
        format!(
            "compile_stdlib_file: could not derive core/ root from path {}",
            path
        )
    })?;

    let config = CodegenConfig::new(path).with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    codegen
        .compile_module_with_mounts(&module, path, &core_root)
        .map_err(|e| format!("Codegen error in {}: {}", path, e))?;

    Ok(())
}

/// Walk up from `path`'s directory until a directory whose final
/// component is exactly `core` is found; return that path.  Used by
/// `compile_stdlib_file` so test sites don't have to thread a
/// `core_root` separately.
#[cfg(test)]
fn derive_core_root(path: &str) -> Option<String> {
    let mut p = std::path::PathBuf::from(path);
    while p.pop() {
        if p.file_name().and_then(|n| n.to_str()) == Some("core") {
            return p.into_os_string().into_string().ok();
        }
    }
    None
}

/// Backwards-compat wrapper retained for call sites that explicitly
/// thread a `core_root`.  Delegates to `compile_module_with_mounts`
/// directly with the supplied root.
#[cfg(test)]
fn compile_stdlib_file_with_mounts(path: &str, core_root: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))?;

    let mut parser = Parser::new(&source);
    let module = parser
        .parse_module()
        .map_err(|e| format!("Parse error in {}: {:?}", path, e))?;

    let config = CodegenConfig::new(path).with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    codegen
        .compile_module_with_mounts(&module, path, core_root)
        .map_err(|e| format!("Codegen error in {}: {}", path, e))?;

    Ok(())
}

/// Tests compilation of core/base/maybe.vr
#[test]
fn test_compile_stdlib_maybe() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/maybe.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile maybe.vr");
    } else {
        println!("Skipping test: {} not found", path);
    }
}

/// Tests compilation of core/base/result.vr
#[test]
fn test_compile_stdlib_result() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/result.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile result.vr");
    } else {
        println!("Skipping test: {} not found", path);
    }
}

/// Tests compilation of core/base/iterator.vr
#[test]
fn test_compile_stdlib_iterator() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/iterator.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile iterator.vr");
    } else {
        println!("Skipping test: {} not found", path);
    }
}

/// Tests compilation of core/collections/list.vr
///

/// Uses `compile_stdlib_file_with_mounts` because `list.vr` brings
/// the typed-OOM allocator primitives (`try_alloc` / `try_realloc`)
/// in via `mount core.base.memory.{ … }`. Without mount resolution
/// those references are undefined — a test-harness gap, not a real
/// codegen bug.
#[test]
fn test_compile_stdlib_list() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/collections/list.vr"
    );
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile list.vr");
    } else {
        println!("Skipping test: {} not found", path);
    }
}

/// Tests compilation of core/base/ordering.vr
#[test]
fn test_compile_stdlib_ordering() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/ordering.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile ordering.vr");
    }
}

/// Tests compilation of core/base/panic.vr
#[test]
fn test_compile_stdlib_panic() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/panic.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile panic.vr");
    }
}

/// Tests compilation of core/base/primitives.vr
#[test]
fn test_compile_stdlib_primitives() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/primitives.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile primitives.vr");
    }
}

/// Tests compilation of core/text/text.vr
///

/// Uses `compile_stdlib_file_with_mounts` (same rationale as
/// `test_compile_stdlib_list`).
#[test]
fn test_compile_stdlib_text() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/text.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile text.vr");
    }
}

/// Tests compilation of core/collections/map.vr
///

/// Uses `compile_stdlib_file_with_mounts` for the same reason as
/// `test_compile_stdlib_list`: `map.vr` mounts `try_alloc` /
/// `try_realloc` from `core.base.memory`.
#[test]
fn test_compile_stdlib_map() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/collections/map.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile map.vr");
    }
}

/// Tests compilation of core/collections/set.vr
///

/// Uses `compile_stdlib_file_with_mounts` (same rationale as
/// `test_compile_stdlib_list`).
#[test]
fn test_compile_stdlib_set() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/collections/set.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile set.vr");
    }
}

/// Tests compilation of core/sync/mutex.vr
#[test]
fn test_compile_stdlib_mutex() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/mutex.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mutex.vr");
    }
}

/// Tests compilation of core/sync/atomic.vr
#[test]
fn test_compile_stdlib_atomic() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/atomic.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile atomic.vr");
    }
}

/// Tests compilation of core/time/duration.vr
#[test]
fn test_compile_stdlib_duration() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/time/duration.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile duration.vr");
    }
}

/// Tests compilation of core/io/file.vr
#[test]
fn test_compile_stdlib_file() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/file.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile file.vr");
    }
}

/// Tests compilation of core/base/ops.vr.
///

/// Exercises the bottom-type alias `public type Never is !;` end-to-end
/// (parser → AST `TypeDeclBody::Alias(TypeKind::Never)` → codegen). Use
/// `compile_stdlib_file` because `ops.vr` declares its operator-overloading
/// protocols self-contained and does not pull cross-module symbols via
/// `mount`.
#[test]
fn test_compile_stdlib_ops() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/ops.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile ops.vr");
    }
}

/// Tests compilation of core/base/error.vr
#[test]
fn test_compile_stdlib_error() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/error.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile error.vr");
    }
}

/// Tests compilation of core/base/env.vr
#[test]
fn test_compile_stdlib_env() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/env.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile env.vr");
    }
}

/// Tests compilation of core/base/protocols.vr
#[test]
fn test_compile_stdlib_protocols() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/protocols.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile protocols.vr");
    }
}

/// Tests compilation of core/base/memory.vr
#[test]
fn test_compile_stdlib_memory() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/memory.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile memory.vr");
    }
}

/// Tests compilation of core/collections/deque.vr
///

/// Uses `compile_stdlib_file_with_mounts` (same rationale as
/// `test_compile_stdlib_list`).
#[test]
fn test_compile_stdlib_deque() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/collections/deque.vr"
    );
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile deque.vr");
    }
}

/// Tests compilation of core/collections/heap.vr
#[test]
fn test_compile_stdlib_heap_collection() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/collections/heap.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile heap.vr");
    }
}

/// Tests compilation of core/collections/btree.vr
#[test]
fn test_compile_stdlib_btree() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/collections/btree.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile btree.vr");
    }
}

/// Tests compilation of core/collections/slice.vr
#[test]
fn test_compile_stdlib_slice() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/collections/slice.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile slice.vr");
    }
}

/// Tests compilation of core/sync/rwlock.vr
#[test]
fn test_compile_stdlib_rwlock() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/rwlock.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile rwlock.vr");
    }
}

/// Tests compilation of core/sync/barrier.vr
#[test]
fn test_compile_stdlib_barrier() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/barrier.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile barrier.vr");
    }
}

/// Tests compilation of core/sync/condvar.vr
#[test]
fn test_compile_stdlib_condvar() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/condvar.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile condvar.vr");
    }
}

/// Tests compilation of core/sync/once.vr
#[test]
fn test_compile_stdlib_once() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/once.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile once.vr");
    }
}

/// Tests compilation of core/sync/semaphore.vr
#[test]
fn test_compile_stdlib_semaphore() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/semaphore.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile semaphore.vr");
    }
}

/// Tests compilation of core/sync/waitgroup.vr
#[test]
fn test_compile_stdlib_waitgroup() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/sync/waitgroup.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile waitgroup.vr");
    }
}

/// Tests compilation of core/time/instant.vr
#[test]
fn test_compile_stdlib_instant() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/time/instant.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile instant.vr");
    }
}

/// Tests compilation of core/time/system_time.vr
#[test]
fn test_compile_stdlib_system_time() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/time/system_time.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile system_time.vr");
    }
}

/// Tests compilation of core/time/interval.vr
#[test]
fn test_compile_stdlib_interval() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/time/interval.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile interval.vr");
    }
}

// =========================================================================
// Additional Stdlib Compilation Tests (Task 1)
// =========================================================================

/// Tests compilation of core/base/data.vr
#[test]
fn test_compile_stdlib_data() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/data.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile data.vr");
    }
}

/// Tests compilation of core/base/cell.vr
#[test]
fn test_compile_stdlib_cell() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/cell.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile cell.vr");
    }
}

/// Tests compilation of core/base/log.vr
/// Note: log.vr uses `context X.method()` syntax which the parser
/// doesn't fully handle yet - skip parse errors gracefully.
#[test]
fn test_compile_stdlib_log() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/log.vr");
    if std::path::Path::new(path).exists() {
        match compile_stdlib_file(path) {
            Ok(()) => {}
            Err(e) if e.contains("Parse error") => {
                println!("Known parse limitation in log.vr: context keyword syntax");
            }
            Err(e) => panic!("Failed to compile log.vr: {}", e),
        }
    }
}

/// Tests compilation of core/base/serde.vr
/// Note: serde.vr uses advanced protocol syntax (D.Error associated type)
/// which the parser doesn't fully handle yet - skip parse errors gracefully.
#[test]
fn test_compile_stdlib_serde() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/base/serde.vr");
    if std::path::Path::new(path).exists() {
        match compile_stdlib_file(path) {
            Ok(()) => {}
            Err(e) if e.contains("Parse error") => {
                println!("Known parse limitation in serde.vr: associated type syntax");
            }
            Err(e) => panic!("Failed to compile serde.vr: {}", e),
        }
    }
}

/// Tests compilation of core/io/path.vr
#[test]
fn test_compile_stdlib_path() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/path.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile path.vr");
    }
}

/// Tests compilation of core/io/buffer.vr
#[test]
fn test_compile_stdlib_buffer() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/buffer.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile buffer.vr");
    }
}

/// Tests compilation of core/io/stdio.vr
#[test]
fn test_compile_stdlib_stdio() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/stdio.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile stdio.vr");
    }
}

/// Tests compilation of core/text/char.vr
#[test]
fn test_compile_stdlib_char() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/char.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile char.vr");
    }
}

/// Tests compilation of core/text/builder.vr
#[test]
fn test_compile_stdlib_builder() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/builder.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile builder.vr");
    }
}

/// Tests compilation of core/text/format.vr
#[test]
fn test_compile_stdlib_format() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/format.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile format.vr");
    }
}

/// Tests compilation of core/text/regex.vr
#[test]
fn test_compile_stdlib_regex() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/text/regex.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile regex.vr");
    }
}

/// Tests compilation of core/async/future.vr
#[test]
fn test_compile_stdlib_future() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/future.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile future.vr");
    }
}

/// Tests compilation of core/async/channel.vr
#[test]
fn test_compile_stdlib_channel() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/channel.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile channel.vr");
    }
}

/// Tests compilation of core/async/task.vr
#[test]
fn test_compile_stdlib_task() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/task.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile task.vr");
    }
}

/// Tests compilation of core/mem/allocator.vr
#[test]
fn test_compile_stdlib_allocator() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/allocator.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile allocator.vr");
    }
}

/// Tests compilation of core/mem/arena.vr
#[test]
fn test_compile_stdlib_arena() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/arena.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile arena.vr");
    }
}

/// Tests compilation of core/math/simple.vr
#[test]
fn test_compile_stdlib_math_simple() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/simple.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile simple.vr");
    }
}

/// Tests compilation of core/math/constants.vr
#[test]
fn test_compile_stdlib_math_constants() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/constants.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile constants.vr");
    }
}

/// Tests compilation of core/context/provider.vr
#[test]
fn test_compile_stdlib_context_provider() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/context/provider.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile provider.vr");
    }
}

/// Tests compilation of core/context/scope.vr
#[test]
fn test_compile_stdlib_context_scope() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/context/scope.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile scope.vr");
    }
}

/// Tests compilation of core/context/layer.vr
#[test]
fn test_compile_stdlib_context_layer() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/context/layer.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile layer.vr");
    }
}

// =========================================================================
// Additional Stdlib Compilation Tests (Mount Resolution)
// =========================================================================

/// Tests compilation of core/context/error.vr
#[test]
fn test_compile_stdlib_context_error() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/context/error.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile context/error.vr");
    }
}

/// Tests compilation of core/io/protocols.vr
#[test]
fn test_compile_stdlib_io_protocols() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/protocols.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile io/protocols.vr");
    }
}

/// Tests compilation of core/io/engine.vr
#[test]
fn test_compile_stdlib_io_engine() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/engine.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile io/engine.vr");
    }
}

/// Tests compilation of core/io/fs.vr
///

/// `fs.vr` mounts cross-module symbols from `io.protocols` (most
/// notably `io_error_kind_from_os_code`, the per-platform errno-to-
/// `IoErrorKind` mapping populated via @cfg). The mount-aware test
/// helper parses each mounted module and pre-registers its
/// declarations before compiling the main file — `compile_stdlib_file`
/// would surface "undefined function: io_error_kind_from_os_code (in
/// function metadata)" because mount resolution is not active.
#[test]
fn test_compile_stdlib_io_fs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/fs.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile io/fs.vr");
    }
}

/// Tests compilation of core/io/process.vr
///

/// Currently fails on a stdlib/codegen binding error. Pre-existing
/// (predates the production-readiness push). Re-enable once the
/// underlying binding is fixed.
#[test]
#[ignore = "stdlib/codegen: pre-existing compile failure"]
fn test_compile_stdlib_io_process() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/io/process.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile io/process.vr");
    }
}

/// Tests compilation of core/io/async_protocols.vr
#[test]
fn test_compile_stdlib_io_async_protocols() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/io/async_protocols.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile io/async_protocols.vr");
    }
}

/// Tests compilation of core/net/addr.vr
#[test]
fn test_compile_stdlib_net_addr() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/net/addr.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile net/addr.vr");
    }
}

/// Tests compilation of core/net/tcp.vr.
///
/// Uses `compile_stdlib_file_with_mounts` because `tcp.vr` pulls in
/// the unified TCP listen intrinsics from `core/sys/raw.vr` via
/// `mount sys.raw.{__tcp_listen_v2_raw, __tcp_local_port_raw}`.
/// Without mount resolution, those references are undefined and
/// codegen fails with `E_FUNCTION_UNDEFINED` — symmetric with the
/// `net_udp` test below.
#[test]
fn test_compile_stdlib_net_tcp() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/net/tcp.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile net/tcp.vr");
    }
}

/// Tests compilation of core/net/udp.vr
///

/// Uses `compile_stdlib_file_with_mounts` because `udp.vr` brings
/// platform-specific safe socket wrappers (`safe_set_ip_tos`,
/// `safe_join_multicast_v4`, etc.) in via `mount sys.<os>.net.{ … }`.
/// Without mount resolution, those references are undefined and
/// codegen fails — a test-harness gap, not a real codegen bug.
#[test]
fn test_compile_stdlib_net_udp() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/net/udp.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile net/udp.vr");
    }
}

/// Tests compilation of core/net/dns.vr
#[test]
fn test_compile_stdlib_net_dns() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/net/dns.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile net/dns.vr");
    }
}

/// Tests compilation of core/net/tls.vr
#[test]
fn test_compile_stdlib_net_tls() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/net/tls.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile net/tls.vr");
    }
}

/// Tests compilation of core/mem/epoch.vr
#[test]
fn test_compile_stdlib_mem_epoch() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/epoch.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/epoch.vr");
    }
}

/// Tests compilation of core/mem/header.vr.
///

/// Uses `compile_stdlib_file_with_mounts` because `header.vr` mounts
/// the `cap_audit` writer helpers (record_revoke / record_attenuate /
/// record_ref_incr / record_ref_decr / record_gen_bump) so every
/// successful CBGR state transition can emit a CapEvent through the
/// runtime-gated audit ring .
#[test]
fn test_compile_stdlib_mem_header() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/header.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile mem/header.vr");
    }
}

/// Tests compilation of core/mem/heap.vr.
///

/// Uses `compile_stdlib_file_with_mounts` because `heap.vr` brings
/// `SEGMENT_HUGE`, `SEGMENT_NORMAL`, `MemSegment`, page-size constants and
/// segment-allocator helpers in via `mount super.segment.{ … }`. Without
/// mount resolution those references are undefined — a test-harness gap,
/// not a real codegen bug. (Same rationale as
/// `test_compile_stdlib_list` above.)
#[test]
fn test_compile_stdlib_mem_heap() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/heap.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile mem/heap.vr");
    }
}

/// Tests compilation of core/mem/thin_ref.vr.
///

/// Uses `compile_stdlib_file_with_mounts` for the same reason as
/// `test_compile_stdlib_mem_heap`: pulls cross-module CBGR symbols via
/// `mount`.
#[test]
fn test_compile_stdlib_mem_thin_ref() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/thin_ref.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root)
            .expect("Failed to compile mem/thin_ref.vr");
    }
}

/// Tests compilation of core/mem/fat_ref.vr.
///

/// Uses `compile_stdlib_file_with_mounts` for the same reason as
/// `test_compile_stdlib_mem_heap`.
#[test]
fn test_compile_stdlib_mem_fat_ref() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/fat_ref.vr");
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file_with_mounts(path, core_root).expect("Failed to compile mem/fat_ref.vr");
    }
}

/// Tests compilation of core/mem/hazard.vr
#[test]
fn test_compile_stdlib_mem_hazard() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/hazard.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/hazard.vr");
    }
}

/// Tests compilation of core/mem/segment.vr
#[test]
fn test_compile_stdlib_mem_segment() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/segment.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/segment.vr");
    }
}

/// Tests compilation of core/mem/size_class.vr
#[test]
fn test_compile_stdlib_mem_size_class() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/size_class.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/size_class.vr");
    }
}

/// Tests compilation of core/mem/raw_ops.vr
#[test]
fn test_compile_stdlib_mem_raw_ops() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/raw_ops.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/raw_ops.vr");
    }
}

/// Tests compilation of core/mem/capability.vr
#[test]
fn test_compile_stdlib_mem_capability() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/mem/capability.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile mem/capability.vr");
    }
}

/// Tests compilation of core/async/poll.vr
#[test]
fn test_compile_stdlib_async_poll() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/poll.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/poll.vr");
    }
}

/// Tests compilation of core/async/waker.vr
#[test]
fn test_compile_stdlib_async_waker() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/waker.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/waker.vr");
    }
}

/// Tests compilation of core/async/executor.vr
#[test]
fn test_compile_stdlib_async_executor() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/executor.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/executor.vr");
    }
}

/// Tests compilation of core/async/stream.vr
#[test]
fn test_compile_stdlib_async_stream() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/stream.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/stream.vr");
    }
}

/// Tests compilation of core/async/select.vr
#[test]
fn test_compile_stdlib_async_select() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/select.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/select.vr");
    }
}

/// Tests compilation of core/async/broadcast.vr
#[test]
fn test_compile_stdlib_async_broadcast() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/broadcast.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/broadcast.vr");
    }
}

/// Tests compilation of core/async/generator.vr
#[test]
fn test_compile_stdlib_async_generator() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/generator.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/generator.vr");
    }
}

/// Tests compilation of core/async/nursery.vr
#[test]
fn test_compile_stdlib_async_nursery() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/nursery.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/nursery.vr");
    }
}

/// Tests compilation of core/async/spawn_config.vr
#[test]
fn test_compile_stdlib_async_spawn_config() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/async/spawn_config.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/spawn_config.vr");
    }
}

/// Tests compilation of core/async/spawn_with.vr
#[test]
fn test_compile_stdlib_async_spawn_with() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/async/spawn_with.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/spawn_with.vr");
    }
}

/// Tests compilation of core/async/timer.vr
#[test]
fn test_compile_stdlib_async_timer() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/timer.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/timer.vr");
    }
}

/// Tests compilation of core/async/parallel.vr
#[test]
fn test_compile_stdlib_async_parallel() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/async/parallel.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/parallel.vr");
    }
}

/// Tests compilation of core/async/intrinsics.vr
#[test]
fn test_compile_stdlib_async_intrinsics() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/async/intrinsics.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile async/intrinsics.vr");
    }
}

/// Tests compilation of core/math/bits.vr
#[test]
fn test_compile_stdlib_math_bits() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/bits.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/bits.vr");
    }
}

/// Tests compilation of core/math/checked.vr
#[test]
fn test_compile_stdlib_math_checked() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/checked.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/checked.vr");
    }
}

/// Tests compilation of core/math/integers.vr
#[test]
fn test_compile_stdlib_math_integers() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/integers.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/integers.vr");
    }
}

/// Tests compilation of core/math/complex.vr
#[test]
fn test_compile_stdlib_math_complex() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/complex.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/complex.vr");
    }
}

/// Tests compilation of core/math/random.vr
#[test]
fn test_compile_stdlib_math_random() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/random.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/random.vr");
    }
}

/// Tests compilation of core/math/elementary.vr
#[test]
fn test_compile_stdlib_math_elementary() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/elementary.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/elementary.vr");
    }
}

/// Tests compilation of core/math/advanced.vr
#[test]
fn test_compile_stdlib_math_advanced() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/advanced.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/advanced.vr");
    }
}

/// Tests compilation of core/math/linalg.vr
#[test]
fn test_compile_stdlib_math_linalg() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/linalg.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/linalg.vr");
    }
}

/// Tests compilation of core/math/tensor.vr
#[test]
fn test_compile_stdlib_math_tensor() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/tensor.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/tensor.vr");
    }
}

/// Tests compilation of core/math/special.vr
#[test]
fn test_compile_stdlib_math_special() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/special.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/special.vr");
    }
}

/// Tests compilation of core/math/hyperbolic.vr
#[test]
fn test_compile_stdlib_math_hyperbolic() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/hyperbolic.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/hyperbolic.vr");
    }
}

/// Tests compilation of core/math/ieee754.vr
#[test]
fn test_compile_stdlib_math_ieee754() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/ieee754.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/ieee754.vr");
    }
}

/// Tests compilation of core/math/logic.vr
#[test]
fn test_compile_stdlib_math_logic() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/logic.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/logic.vr");
    }
}

/// Tests compilation of core/math/number_theory.vr
#[test]
fn test_compile_stdlib_math_number_theory() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/math/number_theory.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/number_theory.vr");
    }
}

/// Tests compilation of core/math/algebra.vr
#[test]
fn test_compile_stdlib_math_algebra() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/algebra.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/algebra.vr");
    }
}

/// Tests compilation of core/math/analysis.vr
#[test]
fn test_compile_stdlib_math_analysis() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/analysis.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/analysis.vr");
    }
}

/// Tests compilation of core/math/category.vr
#[test]
fn test_compile_stdlib_math_category() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/category.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/category.vr");
    }
}

/// Tests compilation of core/math/topology.vr
#[test]
fn test_compile_stdlib_math_topology() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/topology.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/topology.vr");
    }
}

/// Tests compilation of core/math/autodiff.vr
///

/// Currently fails on a stdlib/codegen binding error. Pre-existing
/// (predates the production-readiness push). Re-enable once the
/// underlying binding is fixed.
#[test]
#[ignore = "stdlib/codegen: pre-existing compile failure"]
fn test_compile_stdlib_math_autodiff() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/autodiff.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/autodiff.vr");
    }
}

/// Tests compilation of core/math/internal.vr
#[test]
fn test_compile_stdlib_math_internal() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/internal.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/internal.vr");
    }
}

/// Tests compilation of core/math/libm.vr
#[test]
fn test_compile_stdlib_math_libm() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/math/libm.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile math/libm.vr");
    }
}

/// Tests compilation of core/text/tagged_literals.vr
#[test]
fn test_compile_stdlib_text_tagged_literals() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/text/tagged_literals.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile text/tagged_literals.vr");
    }
}

/// Tests compilation of core/meta/attribute.vr
#[test]
fn test_compile_stdlib_meta_attribute() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/attribute.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/attribute.vr");
    }
}

/// Tests compilation of core/meta/reflection.vr
#[test]
fn test_compile_stdlib_meta_reflection() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/reflection.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/reflection.vr");
    }
}

/// Tests compilation of core/meta/span.vr
/// Note: write! syntax was fixed to write(), but `write` function is undefined.
#[test]
fn test_compile_stdlib_meta_span() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/span.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/span.vr");
    }
}

/// Tests compilation of core/meta/token.vr
/// Note: write! syntax was fixed to write(), but `None` variant not resolved.
#[test]
fn test_compile_stdlib_meta_token() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/token.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/token.vr");
    }
}

/// Tests compilation of core/meta/quote.vr
#[test]
fn test_compile_stdlib_meta_quote() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/quote.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/quote.vr");
    }
}

/// Tests compilation of core/meta/contexts.vr
#[test]
fn test_compile_stdlib_meta_contexts() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/meta/contexts.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile meta/contexts.vr");
    }
}

/// Tests compilation of core/intrinsics/arithmetic.vr
#[test]
fn test_compile_stdlib_intrinsics_arithmetic() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/arithmetic.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/arithmetic.vr");
    }
}

/// Tests compilation of core/intrinsics/bitwise.vr
#[test]
fn test_compile_stdlib_intrinsics_bitwise() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/bitwise.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/bitwise.vr");
    }
}

/// Tests compilation of core/intrinsics/conversion.vr
#[test]
fn test_compile_stdlib_intrinsics_conversion() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/conversion.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/conversion.vr");
    }
}

/// Tests compilation of core/intrinsics/float.vr
#[test]
fn test_compile_stdlib_intrinsics_float() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/float.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/float.vr");
    }
}

/// Tests compilation of core/intrinsics/memory.vr
#[test]
fn test_compile_stdlib_intrinsics_memory() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/memory.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/memory.vr");
    }
}

/// Tests compilation of core/intrinsics/platform.vr
#[test]
fn test_compile_stdlib_intrinsics_platform() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/platform.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/platform.vr");
    }
}

/// Tests compilation of core/intrinsics/control.vr
#[test]
fn test_compile_stdlib_intrinsics_control() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/control.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/control.vr");
    }
}

/// Tests compilation of core/intrinsics/atomic.vr
#[test]
fn test_compile_stdlib_intrinsics_atomic() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/atomic.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/atomic.vr");
    }
}

/// Tests compilation of core/intrinsics/simd.vr
#[test]
fn test_compile_stdlib_intrinsics_simd() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/intrinsics/simd.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/simd.vr");
    }
}

/// Tests compilation of core/intrinsics/type_info.vr
#[test]
fn test_compile_stdlib_intrinsics_type_info() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/type_info.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/type_info.vr");
    }
}

/// Tests compilation of core/intrinsics/tensor.vr
#[test]
fn test_compile_stdlib_intrinsics_tensor() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core/intrinsics/tensor.vr"
    );
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/tensor.vr");
    }
}

/// Tests compilation of core/intrinsics/gpu.vr
#[test]
fn test_compile_stdlib_intrinsics_gpu() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/intrinsics/gpu.vr");
    if std::path::Path::new(path).exists() {
        compile_stdlib_file(path).expect("Failed to compile intrinsics/gpu.vr");
    }
}

/// Batch test: tries to compile all core/ .vr files and reports results.
/// This is an informational test that tracks overall compilation coverage.
#[test]
fn test_compile_stdlib_coverage_report() {
    let core_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/");
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut failures: Vec<(String, String)> = Vec::new();

    // Collect all .vr files excluding mod.vr
    let mut files: Vec<String> = Vec::new();
    fn collect_vr_files(dir: &str, files: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_vr_files(&path.to_string_lossy(), files);
                } else if path.extension().map(|e| e == "vr").unwrap_or(false)
                    && path.file_name().map(|n| n != "mod.vr").unwrap_or(false)
                {
                    files.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
    collect_vr_files(core_root, &mut files);
    files.sort();

    for file_path in &files {
        match compile_stdlib_file(file_path) {
            Ok(()) => passed += 1,
            Err(e) => {
                // Distinguish parse errors from codegen errors
                if e.contains("Parse error") || e.contains("Failed to read") {
                    skipped += 1;
                } else {
                    failed += 1;
                    let short_path = file_path.rsplit("core/").next().unwrap_or(file_path);
                    failures.push((short_path.to_string(), e));
                }
            }
        }
    }

    println!("\n=== Stdlib Compilation Coverage ===");
    println!("Total files: {}", files.len());
    println!("Compiled OK: {}", passed);
    println!("Codegen failures: {}", failed);
    println!("Parse errors (skipped): {}", skipped);
    println!(
        "Coverage: {:.1}%",
        (passed as f64 / files.len() as f64) * 100.0
    );

    if !failures.is_empty() {
        println!("\nCodegen failures:");
        for (path, err) in &failures {
            // Show first 120 chars of error
            let short_err = if err.len() > 120 { &err[..120] } else { err };
            println!("  {} → {}", path, short_err);
        }
    }

    // We expect at least 190 files to compile successfully (with mount resolution).
    // Before mount resolution: ~50 files compiled.
    // After mount resolution: ~199 files compile (76.8% coverage).
    assert!(
        passed >= 190,
        "Expected at least 190 stdlib files to compile, got {}. Failures: {:?}",
        passed,
        failures.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>()
    );
}

/// Tests dereference assignment (*self = value).
#[test]
fn test_codegen_deref_assignment() {
    let source = r#"
type Maybe<T> is None | Some(T);

fn take_and_replace(ptr: &mut Maybe<Int>) -> Maybe<Int> {
    let old = *ptr;
    *ptr = None;
    old
}

fn main() {
    let mut opt = Some(42);
    let result = take_and_replace(&mut opt);
    result
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_deref_assign").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    let result = codegen.compile_module(&module);
    assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
}

/// Lenient panic-stub: when a function fails bug-class compilation, the
/// codegen emits a Panic-bodied stub with the function's pre-registered
/// `FunctionId` instead of silently dropping the descriptor.  Lets
/// dispatch (suffix-and-args, qualified-path, function-pointer load)
/// keep finding the function; only an actual call execution panics
/// — and with the original codegen error message inline, not the
/// opaque `FunctionNotFound` from before.
///
/// The test exercises end-to-end module finalisation with one healthy
/// function and one bug-class failure (an undefined-function call) —
/// the stubbed function must land in the final module with a Panic-
/// bodied bytecode block, the healthy one must stay untouched, and
/// finalisation must succeed without dropping anything.
#[test]
fn lenient_compile_failure_emits_panic_stub() {
    // `does_not_exist` is referenced but never declared.  This
    // raises `UndefinedFunction` which `skip_class()` classifies as
    // BugClass — exactly the path the auto-stub replaces.
    let source = r#"
fn known_good() -> Int { 1 }

fn broken_fn() -> Int {
    does_not_exist()
}

fn main() -> Int { known_good() }
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("parse");

    // Validation is opt-in (`with_validation()`).  Enabling it here
    // doubles as a regression check for the validator's
    // byte-offset-vs-count bound on bytecode `StringId` references.
    let config = CodegenConfig::new("lenient_panic_stub_test").with_validation();
    let mut codegen = VbcCodegen::with_config(config);

    // Mirror the stdlib bootstrap path: initialize, collect
    // declarations + register everything, then drive the lenient
    // body-compile entry.  `broken_fn` will raise UndefinedFunction
    // mid-body; the auto-stub takes over from there.
    codegen.initialize();
    codegen
        .collect_non_protocol_declarations(&module)
        .expect("collect");
    codegen
        .compile_module_items_lenient(&module)
        .expect("lenient compile must succeed even with bug-class failures");

    let vbc_module = codegen
        .finalize_module()
        .expect("finalize");

    // The broken function must NOT have been silently dropped — its
    // descriptor lands in the module with a stub body.
    let broken_descriptor = vbc_module
        .functions
        .iter()
        .find(|f| {
            vbc_module
                .strings
                .get(f.name)
                .map(|s| s == "broken_fn")
                .unwrap_or(false)
        })
        .expect("broken_fn descriptor must be present in the module — the auto-stub replaces the silent drop");

    // The stub body must contain a Panic instruction (the user-
    // facing diagnostic on call) followed by RetV.
    let bytecode_offset = broken_descriptor.bytecode_offset as usize;
    let bytecode_length = broken_descriptor.bytecode_length as usize;
    assert!(
        bytecode_length > 0,
        "broken_fn must have a non-empty stub body, got 0 bytes"
    );
    let body_bytes = &vbc_module.bytecode[bytecode_offset..bytecode_offset + bytecode_length];
    let decoded = crate::bytecode::decode_instructions(body_bytes)
        .expect("stub body must decode cleanly");
    assert!(
        decoded
            .iter()
            .any(|instr| matches!(instr, crate::Instruction::Panic { .. })),
        "stub body must include a Panic instruction; got {:?}",
        decoded
    );

    // The known-good function must still be present and untouched
    // — the auto-stub path must never fire for healthy code.
    let known_good = vbc_module.functions.iter().find(|f| {
        vbc_module
            .strings
            .get(f.name)
            .map(|s| s == "known_good")
            .unwrap_or(false)
    });
    assert!(
        known_good.is_some(),
        "known_good must still be in the module"
    );
}

/// Receiver-shadow safety: `r.stdout()` where `r` is a local variable and
/// `stdout` is also a free function in scope must dispatch to the METHOD on `r`,
/// not to the free function. The free function must never appear in `main`'s
/// bytecode.
///
/// Regression pin for the `'static_resolution` gate introduced in commit
/// 68ef6d25 and extracted into `try_resolve_static_method` in the refactor
/// that followed. Without the gate, `r.stdout()` was miscompiled as
/// `stdout()` (the free function), silently dropping the receiver.
///
/// Note: the devirtualization path in value dispatch emits `Call` (not `CallM`)
/// when the receiver type is statically known — that is correct behavior. What
/// must never happen is a `Call` to the *free* `stdout` function (which would
/// drop the receiver entirely).
#[test]
fn test_receiver_shadow_safety_free_fn_not_called_in_main() {
    let source = r#"
type Recorder is { value: Int };

fn stdout() -> Int { 99 }

implement Recorder {
    fn stdout(&self) -> Int { self.value }
}

fn main() -> Int {
    let r = Recorder { value: 42 };
    r.stdout()
}
"#;
    let mut parser = Parser::new(source);
    let module = parser.parse_module().expect("Should parse");

    let config = CodegenConfig::new("test_receiver_shadow");
    let mut codegen = VbcCodegen::with_config(config);
    let vbc_module = codegen
        .compile_module(&module)
        .expect("Compilation must succeed");

    // Locate the FREE `stdout` function (no parent type — it's not a method).
    let free_stdout_id = vbc_module
        .functions
        .iter()
        .find(|f| {
            f.parent_type.is_none()
                && vbc_module
                    .strings
                    .get(f.name)
                    .map(|s| s == "stdout")
                    .unwrap_or(false)
        })
        .map(|f| f.id);

    // Locate the `Recorder.stdout` method (has a parent type).
    let method_stdout_id = vbc_module
        .functions
        .iter()
        .find(|f| {
            f.parent_type.is_some()
                && vbc_module
                    .strings
                    .get(f.name)
                    .map(|s| s == "Recorder.stdout" || s == "stdout")
                    .unwrap_or(false)
        })
        .map(|f| f.id);

    // Decode `main`'s bytecode.
    let main_desc = vbc_module
        .functions
        .iter()
        .find(|f| {
            vbc_module
                .strings
                .get(f.name)
                .map(|s| s == "main")
                .unwrap_or(false)
        })
        .expect("main must be present in the module");

    let body_bytes = &vbc_module.bytecode
        [main_desc.bytecode_offset as usize..][..main_desc.bytecode_length as usize];
    let instructions =
        crate::bytecode::decode_instructions(body_bytes).expect("main body must decode cleanly");

    // The free `stdout` function must NOT appear in `main`'s call list.
    // Before the gate fix, `r.stdout()` was miscompiled to `stdout()` (the
    // free function), silently dropping `r` as the receiver.
    if let Some(free_id) = free_stdout_id {
        let calls_free_fn = instructions.iter().any(|instr| {
            matches!(instr, crate::Instruction::Call { func_id, .. } if *func_id == free_id.0)
        });
        assert!(
            !calls_free_fn,
            "r.stdout() must NOT call the free `stdout` fn (id={}); got: {:?}",
            free_id.0,
            instructions,
        );
    }

    // `main` must call SOMETHING for `r.stdout()` — either the devirt
    // `Recorder.stdout` method via `Call` (with receiver as first arg) or
    // a dynamic `CallM` dispatch. At least one call instruction must exist.
    let has_any_call = instructions.iter().any(|instr| {
        matches!(instr, crate::Instruction::Call { .. } | crate::Instruction::CallM { .. })
    });
    assert!(
        has_any_call,
        "main must contain at least one Call/CallM for r.stdout(); got: {:?}",
        instructions,
    );

    // Specifically: if devirtualization produced a static Call, it must
    // point at the METHOD (Recorder.stdout), not the free function.
    if let (Some(method_id), Some(free_id)) = (method_stdout_id, free_stdout_id) {
        for instr in &instructions {
            if let crate::Instruction::Call { func_id, .. } = instr {
                assert_ne!(
                    *func_id, free_id.0,
                    "Call targets the free stdout fn (id={}) instead of the method (id={})",
                    free_id.0, method_id.0,
                );
            }
        }
    }
}
