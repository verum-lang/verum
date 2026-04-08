//! Async program generator for Verum
//!
//! This module generates programs that test Verum's async/await system,
//! including concurrent execution, cancellation, and structured concurrency.
//!
//! # Async Model in Verum
//!
//! Verum uses structured concurrency with the following primitives:
//! - `async fn` - Async function definition
//! - `await` - Await a future
//! - `spawn` - Spawn a concurrent task
//! - `select` - Wait for first of multiple futures
//! - `join` - Wait for all futures
//! - `timeout` - Wrap future with timeout
//!
//! # Generated Patterns
//!
//! The generator creates programs testing:
//! - Simple async/await chains
//! - Concurrent spawn patterns
//! - Select/race conditions
//! - Timeout handling
//! - Cancellation propagation
//! - Async with context requirements

use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashSet;

/// Configuration for async generator
#[derive(Debug, Clone)]
pub struct AsyncConfig {
    /// Maximum depth of async call chains
    pub max_async_depth: usize,
    /// Maximum number of concurrent spawns
    pub max_concurrent_spawns: usize,
    /// Maximum number of select arms
    pub max_select_arms: usize,
    /// Probability of adding timeout wrapper
    pub timeout_probability: f64,
    /// Probability of adding cancellation point
    pub cancellation_probability: f64,
    /// Whether to generate channel operations
    pub enable_channels: bool,
    /// Whether to generate mutex/lock operations
    pub enable_sync_primitives: bool,
    /// Maximum number of async functions
    pub max_async_functions: usize,
}

impl Default for AsyncConfig {
    fn default() -> Self {
        Self {
            max_async_depth: 5,
            max_concurrent_spawns: 4,
            max_select_arms: 3,
            timeout_probability: 0.3,
            cancellation_probability: 0.2,
            enable_channels: true,
            enable_sync_primitives: true,
            max_async_functions: 8,
        }
    }
}

/// Represents async operation types for tracking
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AsyncOperation {
    Await,
    Spawn,
    Select,
    Join,
    Timeout,
    Sleep,
    ChannelSend,
    ChannelRecv,
    MutexLock,
}

/// Context for async generation
struct AsyncContext {
    /// Available async functions
    async_functions: Vec<String>,
    /// Available channel names
    channels: Vec<String>,
    /// Current nesting depth
    depth: usize,
    /// Operations used (for coverage tracking)
    operations_used: HashSet<AsyncOperation>,
    /// Counter for unique names
    counter: usize,
    /// Whether we're in an async context
    in_async: bool,
}

impl AsyncContext {
    fn new() -> Self {
        Self {
            async_functions: Vec::new(),
            channels: Vec::new(),
            depth: 0,
            operations_used: HashSet::new(),
            counter: 0,
            in_async: false,
        }
    }

    fn fresh_name(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{}_{}", prefix, self.counter)
    }

    fn record_operation(&mut self, op: AsyncOperation) {
        self.operations_used.insert(op);
    }
}

/// Generator for async programs
pub struct AsyncGenerator {
    config: AsyncConfig,
}

impl AsyncGenerator {
    /// Create a new async generator
    pub fn new(config: AsyncConfig) -> Self {
        Self { config }
    }

    /// Generate a complete async program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut ctx = AsyncContext::new();
        let mut program = String::new();

        program.push_str("// Async test program for Verum\n");
        program.push_str("// Tests async/await, spawn, select, and structured concurrency\n\n");

        // Imports
        program.push_str("use verum_std::core::{List, Text, Maybe, Result}\n");
        program.push_str("use verum_std::async::{spawn, sleep, timeout, select, join}\n");
        if self.config.enable_channels {
            program.push_str("use verum_std::channel::{channel, Sender, Receiver}\n");
        }
        if self.config.enable_sync_primitives {
            program.push_str("use verum_std::sync::{Mutex, RwLock, Semaphore}\n");
        }
        program.push('\n');

        // Generate helper async functions
        let num_funcs = rng.random_range(3..=self.config.max_async_functions);
        for i in 0..num_funcs {
            let func = self.generate_async_function(rng, &mut ctx, i);
            program.push_str(&func);
            program.push('\n');
        }

        // Generate concurrent patterns
        program.push_str(&self.generate_concurrent_patterns(rng, &mut ctx));
        program.push('\n');

        // Generate main async function
        program.push_str(&self.generate_async_main(rng, &mut ctx));

        program
    }

    /// Generate an async function
    fn generate_async_function<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut AsyncContext,
        idx: usize,
    ) -> String {
        let patterns = [
            AsyncPattern::SimpleComputation,
            AsyncPattern::IoOperation,
            AsyncPattern::ChainedAwaits,
            AsyncPattern::ConcurrentSpawn,
            AsyncPattern::SelectPattern,
            AsyncPattern::TimeoutWrapped,
        ];

        let pattern = patterns.choose(rng).unwrap();
        self.generate_function_for_pattern(rng, ctx, idx, *pattern)
    }

    fn generate_function_for_pattern<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut AsyncContext,
        idx: usize,
        pattern: AsyncPattern,
    ) -> String {
        let name = format!("async_op_{}", idx);
        ctx.async_functions.push(name.clone());
        ctx.in_async = true;

        let mut result = String::new();

        match pattern {
            AsyncPattern::SimpleComputation => {
                result.push_str(&format!("/// Simple async computation\n"));
                result.push_str(&format!("async fn {}(input: Int) -> Int {{\n", name));
                result.push_str("    // Simulate async work\n");
                result.push_str("    await sleep(Duration::from_millis(10));\n");
                result.push_str("    input * 2 + 1\n");
                result.push_str("}\n");
            }

            AsyncPattern::IoOperation => {
                result.push_str(&format!("/// Async I/O simulation\n"));
                result.push_str(&format!(
                    "async fn {}(path: Text) -> Result<Text, IoError> using [FileSystem] {{\n",
                    name
                ));
                result.push_str("    // Simulate file read\n");
                result.push_str("    let content = await FileSystem.read_async(path)?;\n");
                result.push_str("    await sleep(Duration::from_millis(5));\n");
                result.push_str("    Ok(content)\n");
                result.push_str("}\n");
            }

            AsyncPattern::ChainedAwaits => {
                let depth = rng.random_range(2..=4);
                result.push_str(&format!(
                    "/// Chained async operations (depth: {})\n",
                    depth
                ));
                result.push_str(&format!("async fn {}(start: Int) -> Int {{\n", name));

                result.push_str("    let mut value = start;\n");
                for i in 0..depth {
                    result.push_str(&format!("    value = await process_step_{}(value);\n", i));
                }
                result.push_str("    value\n");
                result.push_str("}\n");

                // Generate helper functions
                for i in 0..depth {
                    result.push_str(&format!(
                        "\nasync fn process_step_{}(x: Int) -> Int {{\n",
                        i
                    ));
                    result.push_str("    await sleep(Duration::from_millis(1));\n");
                    result.push_str(&format!("    x + {}\n", i + 1));
                    result.push_str("}\n");
                }
            }

            AsyncPattern::ConcurrentSpawn => {
                let num_spawns = rng.random_range(2..=self.config.max_concurrent_spawns);
                result.push_str(&format!("/// Concurrent spawn ({} tasks)\n", num_spawns));
                result.push_str(&format!(
                    "async fn {}(inputs: List<Int>) -> List<Int> {{\n",
                    name
                ));
                result.push_str("    let mut handles = [];\n\n");

                result.push_str("    // Spawn concurrent tasks\n");
                result.push_str("    for input in inputs {\n");
                result.push_str("        let handle = spawn async {\n");
                result.push_str("            await sleep(Duration::from_millis(input as u64));\n");
                result.push_str("            input * 2\n");
                result.push_str("        };\n");
                result.push_str("        handles.push(handle);\n");
                result.push_str("    }\n\n");

                result.push_str("    // Collect results\n");
                result.push_str("    let mut results = [];\n");
                result.push_str("    for handle in handles {\n");
                result.push_str("        results.push(await handle);\n");
                result.push_str("    }\n");
                result.push_str("    results\n");
                result.push_str("}\n");
            }

            AsyncPattern::SelectPattern => {
                let num_arms = rng.random_range(2..=self.config.max_select_arms);
                result.push_str(&format!("/// Select pattern ({} arms)\n", num_arms));
                result.push_str(&format!("async fn {}() -> Text {{\n", name));
                result.push_str("    select {\n");

                for i in 0..num_arms {
                    let delay = rng.random_range(10..100);
                    result.push_str(&format!(
                        "        _ = sleep(Duration::from_millis({})) => \"branch_{}\",\n",
                        delay, i
                    ));
                }

                result.push_str("    }\n");
                result.push_str("}\n");
            }

            AsyncPattern::TimeoutWrapped => {
                let timeout_ms = rng.random_range(100..1000);
                result.push_str(&format!(
                    "/// Timeout wrapped operation ({}ms)\n",
                    timeout_ms
                ));
                result.push_str(&format!(
                    "async fn {}(input: Int) -> Result<Int, TimeoutError> {{\n",
                    name
                ));
                result.push_str(&format!(
                    "    await timeout(Duration::from_millis({}), async {{\n",
                    timeout_ms
                ));
                result.push_str("        // Potentially long operation\n");
                result.push_str("        let mut result = input;\n");
                result.push_str("        for i in 0..10 {\n");
                result.push_str("            await sleep(Duration::from_millis(10));\n");
                result.push_str("            result = result + i;\n");
                result.push_str("        }\n");
                result.push_str("        result\n");
                result.push_str("    })\n");
                result.push_str("}\n");
            }
        }

        ctx.in_async = false;
        result
    }

    /// Generate concurrent patterns demonstrations
    fn generate_concurrent_patterns<R: Rng>(&self, rng: &mut R, ctx: &mut AsyncContext) -> String {
        let mut result = String::new();

        // Join pattern
        result.push_str("/// Parallel join pattern\n");
        result.push_str("async fn parallel_join(a: Int, b: Int, c: Int) -> (Int, Int, Int) {\n");
        result.push_str("    let (ra, rb, rc) = await join(\n");
        result.push_str("        async { await process_a(a) },\n");
        result.push_str("        async { await process_b(b) },\n");
        result.push_str("        async { await process_c(c) },\n");
        result.push_str("    );\n");
        result.push_str("    (ra, rb, rc)\n");
        result.push_str("}\n\n");

        // Helper functions for join
        result.push_str(
            "async fn process_a(x: Int) -> Int { await sleep(Duration::from_millis(10)); x + 1 }\n",
        );
        result.push_str(
            "async fn process_b(x: Int) -> Int { await sleep(Duration::from_millis(20)); x * 2 }\n",
        );
        result.push_str("async fn process_c(x: Int) -> Int { await sleep(Duration::from_millis(15)); x - 1 }\n\n");

        // Race pattern
        result.push_str("/// Race pattern - first result wins\n");
        result.push_str("async fn race_first() -> Text {\n");
        result.push_str("    select {\n");
        result.push_str("        result = fast_source() => result,\n");
        result.push_str("        result = slow_source() => result,\n");
        result.push_str("        _ = sleep(Duration::from_secs(1)) => \"timeout\",\n");
        result.push_str("    }\n");
        result.push_str("}\n\n");

        result.push_str("async fn fast_source() -> Text {\n");
        result.push_str("    await sleep(Duration::from_millis(50));\n");
        result.push_str("    \"fast\"\n");
        result.push_str("}\n\n");

        result.push_str("async fn slow_source() -> Text {\n");
        result.push_str("    await sleep(Duration::from_millis(200));\n");
        result.push_str("    \"slow\"\n");
        result.push_str("}\n\n");

        // Channel pattern
        if self.config.enable_channels {
            result.push_str(&self.generate_channel_pattern(rng, ctx));
        }

        // Mutex pattern
        if self.config.enable_sync_primitives {
            result.push_str(&self.generate_mutex_pattern(rng, ctx));
        }

        // Cancellation pattern
        if rng.random_bool(self.config.cancellation_probability) {
            result.push_str(&self.generate_cancellation_pattern(rng, ctx));
        }

        result
    }

    /// Generate channel-based producer-consumer pattern
    fn generate_channel_pattern<R: Rng>(&self, rng: &mut R, ctx: &mut AsyncContext) -> String {
        let buffer_size = rng.random_range(1..10);
        let num_items = rng.random_range(5..20);

        let mut result = String::new();
        result.push_str(&format!(
            "/// Channel producer-consumer (buffer: {}, items: {})\n",
            buffer_size, num_items
        ));

        result.push_str(&format!("async fn channel_demo() -> List<Int> {{\n"));
        result.push_str(&format!(
            "    let (tx, rx) = channel::<Int>({});\n\n",
            buffer_size
        ));

        result.push_str("    // Producer task\n");
        result.push_str("    let producer = spawn async {\n");
        result.push_str(&format!("        for i in 0..{} {{\n", num_items));
        result.push_str("            await tx.send(i);\n");
        result.push_str("            await sleep(Duration::from_millis(5));\n");
        result.push_str("        }\n");
        result.push_str("        drop(tx); // Close channel\n");
        result.push_str("    };\n\n");

        result.push_str("    // Consumer collects all\n");
        result.push_str("    let mut results = [];\n");
        result.push_str("    while let Some(item) = await rx.recv() {\n");
        result.push_str("        results.push(item * 2);\n");
        result.push_str("    }\n\n");

        result.push_str("    await producer;\n");
        result.push_str("    results\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate mutex-protected shared state pattern
    fn generate_mutex_pattern<R: Rng>(&self, rng: &mut R, _ctx: &mut AsyncContext) -> String {
        let num_tasks = rng.random_range(2..5);
        let increments_per_task = rng.random_range(10..50);

        let mut result = String::new();
        result.push_str(&format!(
            "/// Mutex shared counter ({} tasks, {} increments each)\n",
            num_tasks, increments_per_task
        ));

        result.push_str("async fn mutex_demo() -> Int {\n");
        result.push_str("    let counter = Mutex::new(0);\n");
        result.push_str("    let mut handles = [];\n\n");

        result.push_str(&format!("    for _ in 0..{} {{\n", num_tasks));
        result.push_str("        let handle = spawn async {\n");
        result.push_str(&format!(
            "            for _ in 0..{} {{\n",
            increments_per_task
        ));
        result.push_str("                let mut guard = await counter.lock();\n");
        result.push_str("                *guard = *guard + 1;\n");
        result.push_str("            }\n");
        result.push_str("        };\n");
        result.push_str("        handles.push(handle);\n");
        result.push_str("    }\n\n");

        result.push_str("    for handle in handles {\n");
        result.push_str("        await handle;\n");
        result.push_str("    }\n\n");

        result.push_str("    let final_guard = await counter.lock();\n");
        result.push_str("    *final_guard\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate cancellation pattern
    fn generate_cancellation_pattern<R: Rng>(
        &self,
        _rng: &mut R,
        _ctx: &mut AsyncContext,
    ) -> String {
        let mut result = String::new();

        result.push_str("/// Cancellation propagation pattern\n");
        result.push_str(
            "async fn cancellation_demo(token: CancellationToken) -> Result<Int, Cancelled> {\n",
        );
        result.push_str("    let mut total = 0;\n\n");

        result.push_str("    for i in 0..100 {\n");
        result.push_str("        // Check for cancellation\n");
        result.push_str("        if token.is_cancelled() {\n");
        result.push_str("            return Err(Cancelled);\n");
        result.push_str("        }\n\n");

        result.push_str("        // Do some work\n");
        result.push_str("        await sleep(Duration::from_millis(10));\n");
        result.push_str("        total = total + i;\n\n");

        result.push_str("        // Cooperative cancellation point\n");
        result.push_str("        await token.check()?;\n");
        result.push_str("    }\n\n");

        result.push_str("    Ok(total)\n");
        result.push_str("}\n\n");

        result.push_str("/// Parent task that may cancel child\n");
        result.push_str("async fn cancellation_parent() -> Result<Int, Text> {\n");
        result.push_str("    let token = CancellationToken::new();\n");
        result.push_str("    let child_token = token.child();\n\n");

        result.push_str("    let child = spawn async {\n");
        result.push_str("        await cancellation_demo(child_token)\n");
        result.push_str("    };\n\n");

        result.push_str("    // Simulate deciding to cancel\n");
        result.push_str("    await sleep(Duration::from_millis(250));\n\n");

        result.push_str("    select {\n");
        result.push_str("        result = child => result.map_err(|_| \"cancelled\"),\n");
        result.push_str("        _ = sleep(Duration::from_millis(100)) => {\n");
        result.push_str("            token.cancel();\n");
        result.push_str("            Err(\"timeout, cancelled child\")\n");
        result.push_str("        }\n");
        result.push_str("    }\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate async main function
    fn generate_async_main<R: Rng>(&self, rng: &mut R, ctx: &mut AsyncContext) -> String {
        let mut result = String::new();

        result.push_str("/// Entry point - runs all async demos\n");
        result.push_str("async fn async_main() {\n");

        // Call some of the generated functions
        for (i, func) in ctx.async_functions.iter().take(3).enumerate() {
            result.push_str(&format!("    println!(\"Running {}...\");\n", func));
            result.push_str(&format!(
                "    let result_{} = await {}({});\n",
                i,
                func,
                i * 10
            ));
            result.push_str(&format!(
                "    println!(\"Result: {{}}\", result_{});\n\n",
                i
            ));
        }

        // Run concurrent demos
        result.push_str("    // Run concurrent patterns\n");
        result.push_str("    let (a, b, c) = await parallel_join(1, 2, 3);\n");
        result.push_str("    println!(\"Parallel join: ({}, {}, {})\", a, b, c);\n\n");

        result.push_str("    let race_result = await race_first();\n");
        result.push_str("    println!(\"Race winner: {}\", race_result);\n\n");

        if self.config.enable_channels {
            result.push_str("    let channel_results = await channel_demo();\n");
            result.push_str("    println!(\"Channel results: {:?}\", channel_results);\n\n");
        }

        if self.config.enable_sync_primitives {
            result.push_str("    let counter_result = await mutex_demo();\n");
            result.push_str("    println!(\"Final counter: {}\", counter_result);\n\n");
        }

        result.push_str("    println!(\"All async demos completed!\");\n");
        result.push_str("}\n\n");

        // Non-async main that runs the runtime
        result.push_str("fn main() {\n");
        result.push_str("    Runtime::new().block_on(async_main());\n");
        result.push_str("}\n");

        result
    }
}

/// Patterns for async function generation
#[derive(Debug, Clone, Copy)]
enum AsyncPattern {
    SimpleComputation,
    IoOperation,
    ChainedAwaits,
    ConcurrentSpawn,
    SelectPattern,
    TimeoutWrapped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_async_generator() {
        let config = AsyncConfig::default();
        let generator = AsyncGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate_program(&mut rng);
        assert!(!program.is_empty());

        // Should contain async keywords
        assert!(program.contains("async fn"));
        assert!(program.contains("await"));

        // Should have main
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_async_patterns() {
        let config = AsyncConfig {
            enable_channels: true,
            enable_sync_primitives: true,
            ..Default::default()
        };
        let generator = AsyncGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(123);

        let program = generator.generate_program(&mut rng);

        // Should contain structured concurrency primitives
        assert!(
            program.contains("spawn") || program.contains("join") || program.contains("select")
        );
    }

    #[test]
    fn test_async_with_channels() {
        let config = AsyncConfig {
            enable_channels: true,
            ..Default::default()
        };
        let generator = AsyncGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(456);

        let program = generator.generate_program(&mut rng);
        assert!(program.contains("channel"));
    }
}
