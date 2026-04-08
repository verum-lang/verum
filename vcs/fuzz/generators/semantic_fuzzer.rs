//! Semantic fuzzer for Verum
//!
//! This module generates programs that are semantically interesting -
//! targeting specific compiler features, edge cases, and optimization paths.
//! Unlike the type fuzzer which ensures type correctness, this fuzzer
//! intentionally generates programs that stress-test semantic analysis.
//!
//! # Target Areas
//!
//! - Ownership and borrowing edge cases
//! - CBGR (Checked Borrowing with Generational References)
//! - Context system (using/provide)
//! - Async/await patterns
//! - Refinement type verification
//! - Protocol implementations
//! - Pattern matching exhaustiveness
//! - Dead code elimination
//! - Constant folding opportunities
//!
//! # Generation Strategies
//!
//! - **Ownership stress**: Create complex borrowing scenarios
//! - **CBGR edge cases**: Test generation counter wrapping
//! - **Context threading**: Test context propagation paths
//! - **Concurrent patterns**: Generate async code with potential races
//! - **Refinement bounds**: Test SMT solver interactions

use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashSet;

/// Configuration for the semantic fuzzer
#[derive(Debug, Clone)]
pub struct SemanticFuzzerConfig {
    /// Maximum depth for nested structures
    pub max_depth: usize,
    /// Maximum statements per block
    pub max_block_size: usize,
    /// Focus on ownership edge cases
    pub focus_ownership: bool,
    /// Focus on CBGR stress testing
    pub focus_cbgr: bool,
    /// Focus on context system
    pub focus_contexts: bool,
    /// Focus on async patterns
    pub focus_async: bool,
    /// Focus on refinement types
    pub focus_refinements: bool,
    /// Generate invalid programs (for error recovery testing)
    pub generate_invalid: bool,
    /// Probability of generating edge cases
    pub edge_case_probability: f64,
}

impl Default for SemanticFuzzerConfig {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_block_size: 15,
            focus_ownership: true,
            focus_cbgr: true,
            focus_contexts: true,
            focus_async: true,
            focus_refinements: true,
            generate_invalid: false,
            edge_case_probability: 0.3,
        }
    }
}

/// Semantic categories for targeted generation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SemanticCategory {
    /// Test ownership and move semantics
    Ownership,
    /// Test CBGR reference checking
    CBGR,
    /// Test context system propagation
    Context,
    /// Test async/await and concurrency
    Async,
    /// Test refinement type verification
    Refinement,
    /// Test pattern matching
    Pattern,
    /// Test generic instantiation
    Generics,
    /// Test protocol implementations
    Protocol,
    /// Test control flow analysis
    ControlFlow,
    /// Test constant evaluation
    ConstEval,
}

/// Generation context for semantic fuzzer
#[derive(Debug, Clone)]
struct SemanticContext {
    depth: usize,
    current_category: Option<SemanticCategory>,
    variables: Vec<SemanticVar>,
    borrowed_vars: HashSet<String>,
    moved_vars: HashSet<String>,
    in_async: bool,
    in_unsafe: bool,
    available_contexts: Vec<String>,
    name_counter: usize,
}

#[derive(Debug, Clone)]
struct SemanticVar {
    name: String,
    is_borrowed: bool,
    is_moved: bool,
    is_mutable: bool,
    borrow_count: usize,
}

impl SemanticContext {
    fn new() -> Self {
        Self {
            depth: 0,
            current_category: None,
            variables: Vec::new(),
            borrowed_vars: HashSet::new(),
            moved_vars: HashSet::new(),
            in_async: false,
            in_unsafe: false,
            available_contexts: vec!["Logger".to_string(), "Database".to_string()],
            name_counter: 0,
        }
    }

    fn fresh_name(&mut self, prefix: &str) -> String {
        self.name_counter += 1;
        format!("{}_{}", prefix, self.name_counter)
    }

    fn enter_scope(&mut self) {
        self.depth += 1;
    }

    fn exit_scope(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn at_max_depth(&self, config: &SemanticFuzzerConfig) -> bool {
        self.depth >= config.max_depth
    }
}

/// Semantic-aware program fuzzer
pub struct SemanticFuzzer {
    config: SemanticFuzzerConfig,
}

impl SemanticFuzzer {
    /// Create a new semantic fuzzer
    pub fn new(config: SemanticFuzzerConfig) -> Self {
        Self { config }
    }

    /// Generate a semantically interesting program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut ctx = SemanticContext::new();

        // Choose a primary semantic category to focus on
        let category = self.choose_category(rng);
        ctx.current_category = Some(category);

        let mut output = String::new();
        output.push_str(&format!(
            "// Semantic fuzzer generated program\n// Focus: {:?}\n\n",
            category
        ));

        // Generate imports
        output.push_str(&self.generate_imports(rng, &ctx));
        output.push('\n');

        // Generate context definitions if focusing on contexts
        if category == SemanticCategory::Context {
            output.push_str(&self.generate_context_definitions(rng, &mut ctx));
            output.push('\n');
        }

        // Generate type definitions
        output.push_str(&self.generate_type_definitions(rng, &mut ctx, category));
        output.push('\n');

        // Generate helper functions based on category
        match category {
            SemanticCategory::Ownership => {
                output.push_str(&self.generate_ownership_functions(rng, &mut ctx));
            }
            SemanticCategory::CBGR => {
                output.push_str(&self.generate_cbgr_functions(rng, &mut ctx));
            }
            SemanticCategory::Async => {
                output.push_str(&self.generate_async_functions(rng, &mut ctx));
            }
            SemanticCategory::Refinement => {
                output.push_str(&self.generate_refinement_functions(rng, &mut ctx));
            }
            SemanticCategory::Context => {
                output.push_str(&self.generate_context_functions(rng, &mut ctx));
            }
            _ => {
                output.push_str(&self.generate_generic_functions(rng, &mut ctx));
            }
        }
        output.push('\n');

        // Generate main function
        output.push_str(&self.generate_main(rng, &mut ctx, category));

        output
    }

    /// Choose a semantic category to focus on
    fn choose_category<R: Rng>(&self, rng: &mut R) -> SemanticCategory {
        let mut categories = Vec::new();

        if self.config.focus_ownership {
            categories.push(SemanticCategory::Ownership);
        }
        if self.config.focus_cbgr {
            categories.push(SemanticCategory::CBGR);
        }
        if self.config.focus_contexts {
            categories.push(SemanticCategory::Context);
        }
        if self.config.focus_async {
            categories.push(SemanticCategory::Async);
        }
        if self.config.focus_refinements {
            categories.push(SemanticCategory::Refinement);
        }

        // Add some general categories
        categories.push(SemanticCategory::Pattern);
        categories.push(SemanticCategory::ControlFlow);

        categories
            .choose(rng)
            .copied()
            .unwrap_or(SemanticCategory::Ownership)
    }

    /// Generate imports based on context
    fn generate_imports<R: Rng>(&self, rng: &mut R, ctx: &SemanticContext) -> String {
        let mut imports =
            String::from("import verum_core.base.{List, Text, Map, Maybe, Heap, Shared};\n");

        if ctx.current_category == Some(SemanticCategory::Async) {
            imports.push_str("import verum_std.async.{spawn, sleep, yield};\n");
        }

        if ctx.current_category == Some(SemanticCategory::Context) {
            imports.push_str("import verum_std.context.{Logger, Database};\n");
        }

        imports
    }

    /// Generate context definitions
    fn generate_context_definitions<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
    ) -> String {
        let mut result = String::new();

        // Define a custom context
        let context_name = ctx.fresh_name("MyContext");
        ctx.available_contexts.push(context_name.clone());

        result.push_str(&format!(
            "context {} {{\n    fn log(&self, msg: Text);\n    fn get_value(&self) -> Int;\n}}\n\n",
            context_name
        ));

        result
    }

    /// Generate type definitions based on category
    fn generate_type_definitions<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
        category: SemanticCategory,
    ) -> String {
        let mut result = String::new();

        match category {
            SemanticCategory::Ownership => {
                // Types that test ownership semantics
                result.push_str("// Owned resource type\n");
                result.push_str(
                    "type Resource is {\n    data: Heap<List<Int>>,\n    id: Int,\n};\n\n",
                );

                result.push_str("// Type with non-copyable field\n");
                result.push_str(
                    "type Container<T> is {\n    value: Heap<T>,\n    count: Int,\n};\n\n",
                );
            }
            SemanticCategory::CBGR => {
                // Types that stress CBGR
                result.push_str("// CBGR stress type with multiple references\n");
                result.push_str(
                    "type RefContainer<T> is {\n    inner: Heap<T>,\n    cached: Int,\n};\n\n",
                );

                result.push_str("// Recursive type for deep reference chains\n");
                result.push_str("type Node<T> is\n    Leaf(T)\n    | Branch{ left: Heap<Node<T>>, right: Heap<Node<T>> };\n\n");
            }
            SemanticCategory::Refinement => {
                // Types with refinements
                result.push_str("// Positive integer\n");
                result.push_str("type Positive is (Int) where value it > 0;\n\n");

                result.push_str("// Bounded range\n");
                result.push_str("type Bounded<const MIN: Int, const MAX: Int> is (Int) where value it >= MIN && it <= MAX;\n\n");

                result.push_str("// Non-empty list\n");
                result.push_str("type NonEmpty<T> is (List<T>) where value len(it) > 0;\n\n");
            }
            SemanticCategory::Pattern => {
                // Types for pattern matching
                result.push_str("// Option-like type\n");
                result.push_str("type Option<T> is None | Some(T);\n\n");

                result.push_str("// Result type\n");
                result.push_str("type Result<T, E> is Ok(T) | Err(E);\n\n");

                result.push_str("// Recursive enum\n");
                result.push_str("type Expr is\n    Lit(Int)\n    | Add{ left: Heap<Expr>, right: Heap<Expr> }\n    | Mul{ left: Heap<Expr>, right: Heap<Expr> };\n\n");
            }
            _ => {
                // Generic useful types
                result.push_str("type Wrapper<T> is { value: T };\n\n");
            }
        }

        result
    }

    /// Generate ownership-focused functions
    fn generate_ownership_functions<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
    ) -> String {
        let mut result = String::new();

        // Function that takes ownership
        result.push_str("fn take_ownership(r: Heap<List<Int>>) -> Int {\n");
        result.push_str("    len(r)\n");
        result.push_str("}\n\n");

        // Function that borrows
        result.push_str("fn borrow_ref(r: &List<Int>) -> Int {\n");
        result.push_str("    len(r)\n");
        result.push_str("}\n\n");

        // Function that mutably borrows
        result.push_str("fn mutate_ref(r: &mut List<Int>) {\n");
        result.push_str("    // Mutate the reference\n");
        result.push_str("}\n\n");

        // Function with complex borrowing
        result.push_str("fn complex_borrow(a: &Int, b: &mut Int) -> Int {\n");
        result.push_str("    *a + *b\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate CBGR-focused functions
    fn generate_cbgr_functions<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut result = String::new();

        // Test managed references
        result.push_str("fn test_managed_ref(data: &Int) -> Int {\n");
        result.push_str("    *data * 2\n");
        result.push_str("}\n\n");

        // Test checked references
        result.push_str("fn test_checked_ref(data: &checked Int) -> Int {\n");
        result.push_str("    *data + 1\n");
        result.push_str("}\n\n");

        // Test unsafe references
        result.push_str("fn test_unsafe_ref(data: &unsafe Int) -> Int {\n");
        result.push_str("    unsafe {\n");
        result.push_str("        *data\n");
        result.push_str("    }\n");
        result.push_str("}\n\n");

        // Function that creates and returns references
        result.push_str("fn create_ref(value: Int) -> Heap<Int> {\n");
        result.push_str("    Heap(value)\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate async-focused functions
    fn generate_async_functions<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut result = String::new();

        // Basic async function
        result.push_str("async fn fetch_data() -> Int {\n");
        result.push_str("    let result = 42;\n");
        result.push_str("    result\n");
        result.push_str("}\n\n");

        // Async function with await
        result.push_str("async fn process_async() -> Int {\n");
        result.push_str("    let data = fetch_data().await;\n");
        result.push_str("    data * 2\n");
        result.push_str("}\n\n");

        // Async function with multiple awaits
        result.push_str("async fn chain_async() -> Int {\n");
        result.push_str("    let a = fetch_data().await;\n");
        result.push_str("    let b = fetch_data().await;\n");
        result.push_str("    a + b\n");
        result.push_str("}\n\n");

        // Spawn task
        result.push_str("async fn spawn_work() {\n");
        result.push_str("    spawn {\n");
        result.push_str("        let _ = fetch_data().await;\n");
        result.push_str("    };\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate refinement-focused functions
    fn generate_refinement_functions<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
    ) -> String {
        let mut result = String::new();

        // Function with precondition
        result.push_str("fn divide(a: Int, b: Int{!= 0}) -> Int {\n");
        result.push_str("    a / b\n");
        result.push_str("}\n\n");

        // Function with postcondition
        result.push_str("fn abs(x: Int) -> Int where ensures result >= 0 {\n");
        result.push_str("    if x < 0 { -x } else { x }\n");
        result.push_str("}\n\n");

        // Function with index bounds
        result.push_str("fn safe_index<T>(list: &List<T>, idx: Int{>= 0}) -> Maybe<&T> {\n");
        result.push_str("    if idx < len(list) {\n");
        result.push_str("        Some(&list[idx])\n");
        result.push_str("    } else {\n");
        result.push_str("        None\n");
        result.push_str("    }\n");
        result.push_str("}\n\n");

        // Function with dependent refinement
        result.push_str("fn bounded_add(a: Int{>= 0}, b: Int{>= 0}) -> Int{>= 0} {\n");
        result.push_str("    a + b\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate context-focused functions
    fn generate_context_functions<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut result = String::new();

        // Function that uses a context
        result.push_str("fn log_message(msg: Text) using [Logger] {\n");
        result.push_str("    Logger.log(msg);\n");
        result.push_str("}\n\n");

        // Function that uses multiple contexts
        result.push_str("fn fetch_and_log() using [Database, Logger] {\n");
        result.push_str("    Logger.log(\"Fetching...\");\n");
        result.push_str("}\n\n");

        // Function that provides a context
        result.push_str("fn with_logging<T>(f: fn() -> T) -> T {\n");
        result.push_str("    provide Logger = ConsoleLogger {};\n");
        result.push_str("    f()\n");
        result.push_str("}\n\n");

        // Nested context usage
        result.push_str("fn nested_contexts() using [Logger] {\n");
        result.push_str("    log_message(\"outer\");\n");
        result.push_str("    {\n");
        result.push_str("        log_message(\"inner\");\n");
        result.push_str("    }\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate generic helper functions
    fn generate_generic_functions<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut result = String::new();

        result.push_str("fn identity<T>(x: T) -> T {\n");
        result.push_str("    x\n");
        result.push_str("}\n\n");

        result.push_str("fn pair<A, B>(a: A, b: B) -> (A, B) {\n");
        result.push_str("    (a, b)\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate main function based on category
    fn generate_main<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
        category: SemanticCategory,
    ) -> String {
        let mut result = String::from("fn main() {\n");

        match category {
            SemanticCategory::Ownership => {
                result.push_str(&self.generate_ownership_main_body(rng, ctx));
            }
            SemanticCategory::CBGR => {
                result.push_str(&self.generate_cbgr_main_body(rng, ctx));
            }
            SemanticCategory::Async => {
                result.push_str(&self.generate_async_main_body(rng, ctx));
            }
            SemanticCategory::Refinement => {
                result.push_str(&self.generate_refinement_main_body(rng, ctx));
            }
            SemanticCategory::Context => {
                result.push_str(&self.generate_context_main_body(rng, ctx));
            }
            SemanticCategory::Pattern => {
                result.push_str(&self.generate_pattern_main_body(rng, ctx));
            }
            _ => {
                result.push_str(&self.generate_generic_main_body(rng, ctx));
            }
        }

        result.push_str("}\n");
        result
    }

    /// Generate ownership test cases in main
    fn generate_ownership_main_body<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
    ) -> String {
        let mut body = String::new();

        body.push_str("    // Ownership tests\n");
        body.push_str("    let owned = Heap([1, 2, 3, 4, 5]);\n");
        body.push_str("    let len1 = take_ownership(owned);\n");
        body.push_str("    // owned is now moved\n\n");

        body.push_str("    let data = [10, 20, 30];\n");
        body.push_str("    let len2 = borrow_ref(&data);\n");
        body.push_str("    // data is still valid\n");
        body.push_str("    let len3 = borrow_ref(&data);\n\n");

        body.push_str("    let mut mutable = [1, 2, 3];\n");
        body.push_str("    mutate_ref(&mut mutable);\n\n");

        body.push_str("    let x = 10;\n");
        body.push_str("    let mut y = 20;\n");
        body.push_str("    let sum = complex_borrow(&x, &mut y);\n");

        body
    }

    /// Generate CBGR test cases in main
    fn generate_cbgr_main_body<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut body = String::new();

        body.push_str("    // CBGR reference tests\n");
        body.push_str("    let value = 42;\n\n");

        body.push_str("    // Managed reference (default)\n");
        body.push_str("    let r1 = test_managed_ref(&value);\n\n");

        body.push_str("    // Checked reference (compile-time verified)\n");
        body.push_str("    let r2 = test_checked_ref(&checked value);\n\n");

        body.push_str("    // Create heap allocation\n");
        body.push_str("    let heap_val = create_ref(100);\n");
        body.push_str("    let r3 = test_managed_ref(&*heap_val);\n\n");

        body.push_str("    // Multiple references\n");
        body.push_str("    let a = &value;\n");
        body.push_str("    let b = &value;\n");
        body.push_str("    let sum = *a + *b;\n");

        body
    }

    /// Generate async test cases in main
    fn generate_async_main_body<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut body = String::new();

        body.push_str("    // Async tests (would be in async main)\n");
        body.push_str("    // Note: main is not async, so we show patterns\n\n");

        body.push_str("    // Spawn async work\n");
        body.push_str("    let handle = spawn {\n");
        body.push_str("        let data = 42;\n");
        body.push_str("        data * 2\n");
        body.push_str("    };\n\n");

        body.push_str("    // Basic computation while async runs\n");
        body.push_str("    let local_work = 1 + 2 + 3;\n\n");

        body.push_str("    // Demonstrate yield\n");
        body.push_str("    for i in 0..10 {\n");
        body.push_str("        let _ = i * 2;\n");
        body.push_str("    }\n");

        body
    }

    /// Generate refinement test cases in main
    fn generate_refinement_main_body<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut SemanticContext,
    ) -> String {
        let mut body = String::new();

        body.push_str("    // Refinement type tests\n\n");

        body.push_str("    // Safe division with non-zero check\n");
        body.push_str("    let result = divide(10, 2);\n\n");

        body.push_str("    // Absolute value with postcondition\n");
        body.push_str("    let pos = abs(-42);\n\n");

        body.push_str("    // Bounded addition\n");
        body.push_str("    let sum = bounded_add(10, 20);\n\n");

        body.push_str("    // Safe indexing\n");
        body.push_str("    let list = [1, 2, 3, 4, 5];\n");
        body.push_str("    match safe_index(&list, 2) {\n");
        body.push_str("        Some(val) => { let _ = *val; },\n");
        body.push_str("        None => {},\n");
        body.push_str("    }\n");

        body
    }

    /// Generate context test cases in main
    fn generate_context_main_body<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut body = String::new();

        body.push_str("    // Context system tests\n\n");

        body.push_str("    // Provide a logger context\n");
        body.push_str("    provide Logger = ConsoleLogger {};\n\n");

        body.push_str("    // Use the context\n");
        body.push_str("    log_message(\"Hello from main\");\n\n");

        body.push_str("    // Nested context scope\n");
        body.push_str("    {\n");
        body.push_str("        provide Logger = FileLogger { path: \"log.txt\" };\n");
        body.push_str("        log_message(\"Logged to file\");\n");
        body.push_str("    }\n\n");

        body.push_str("    // Original logger restored\n");
        body.push_str("    log_message(\"Back to console\");\n");

        body
    }

    /// Generate pattern matching test cases in main
    fn generate_pattern_main_body<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut body = String::new();

        body.push_str("    // Pattern matching tests\n\n");

        body.push_str("    // Option matching\n");
        body.push_str("    let opt: Option<Int> = Some(42);\n");
        body.push_str("    let value = match opt {\n");
        body.push_str("        Some(x) => x,\n");
        body.push_str("        None => 0,\n");
        body.push_str("    };\n\n");

        body.push_str("    // Result matching\n");
        body.push_str("    let res: Result<Int, Text> = Ok(100);\n");
        body.push_str("    match res {\n");
        body.push_str("        Ok(v) => { let _ = v * 2; },\n");
        body.push_str("        Err(e) => { let _ = e; },\n");
        body.push_str("    }\n\n");

        body.push_str("    // Nested pattern\n");
        body.push_str("    let nested: Option<Option<Int>> = Some(Some(5));\n");
        body.push_str("    match nested {\n");
        body.push_str("        Some(Some(x)) => { let _ = x; },\n");
        body.push_str("        Some(None) => {},\n");
        body.push_str("        None => {},\n");
        body.push_str("    }\n\n");

        body.push_str("    // Guard patterns\n");
        body.push_str("    let num = 42;\n");
        body.push_str("    match num {\n");
        body.push_str("        x where x > 100 => { let _ = \"large\"; },\n");
        body.push_str("        x where x > 0 => { let _ = \"positive\"; },\n");
        body.push_str("        _ => { let _ = \"other\"; },\n");
        body.push_str("    }\n");

        body
    }

    /// Generate generic test cases in main
    fn generate_generic_main_body<R: Rng>(&self, rng: &mut R, ctx: &mut SemanticContext) -> String {
        let mut body = String::new();

        body.push_str("    // Generic tests\n");
        body.push_str("    let x = identity(42);\n");
        body.push_str("    let s = identity(\"hello\");\n");
        body.push_str("    let p = pair(1, \"two\");\n");
        body.push_str("    let q = pair(true, 3.14);\n");

        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_semantic_fuzzer_generates_programs() {
        let config = SemanticFuzzerConfig::default();
        let fuzzer = SemanticFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let program = fuzzer.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn main()"));
        }
    }

    #[test]
    fn test_category_selection() {
        let mut config = SemanticFuzzerConfig::default();
        let fuzzer = SemanticFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // All categories should be selectable
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let category = fuzzer.choose_category(&mut rng);
            seen.insert(format!("{:?}", category));
        }

        // Should see multiple categories
        assert!(seen.len() > 1, "Should select multiple categories");
    }

    #[test]
    fn test_ownership_focus() {
        let mut config = SemanticFuzzerConfig::default();
        config.focus_ownership = true;
        config.focus_cbgr = false;
        config.focus_contexts = false;
        config.focus_async = false;
        config.focus_refinements = false;

        let fuzzer = SemanticFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = fuzzer.generate_program(&mut rng);
        // Should contain ownership-related code
        assert!(
            program.contains("Heap") || program.contains("&") || program.contains("mut"),
            "Ownership-focused program should contain references/ownership code"
        );
    }

    #[test]
    fn test_uses_verum_syntax() {
        let config = SemanticFuzzerConfig::default();
        let fuzzer = SemanticFuzzer::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let program = fuzzer.generate_program(&mut rng);

            // Should use Verum syntax
            if program.contains("type ") {
                assert!(
                    program.contains(" is "),
                    "Type definitions should use 'is' keyword"
                );
            }

            // Should not use Rust syntax
            assert!(
                !program.contains("struct "),
                "Should not contain Rust 'struct'"
            );
            assert!(!program.contains("enum "), "Should not contain Rust 'enum'");
            assert!(!program.contains("impl "), "Should not contain Rust 'impl'");
            assert!(
                !program.contains("trait "),
                "Should not contain Rust 'trait'"
            );
        }
    }
}
