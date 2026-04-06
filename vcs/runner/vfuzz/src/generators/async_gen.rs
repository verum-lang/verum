//! Async/concurrent program generator for fuzzing
//!
//! Generates programs with async/await patterns and concurrency primitives.
//! Tests the runtime's handling of:
//! - Async functions and await points
//! - Spawning tasks
//! - Channels and message passing
//! - Select and join operations
//! - Timeouts and cancellation
//! - Race conditions (intentional edge cases)

use super::{Generate, GeneratorConfig, indent, random_identifier};
use rand::prelude::*;

/// Generator for async/concurrent programs
pub struct AsyncGenerator {
    config: GeneratorConfig,
    current_depth: usize,
    async_functions: Vec<String>,
    channels: Vec<String>,
}

impl AsyncGenerator {
    /// Create a new async generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            current_depth: 0,
            async_functions: Vec::new(),
            channels: Vec::new(),
        }
    }

    /// Reset state
    fn reset(&mut self) {
        self.current_depth = 0;
        self.async_functions.clear();
        self.channels.clear();
    }

    /// Generate a complete async program
    fn generate_program<R: Rng>(&mut self, rng: &mut R) -> String {
        self.reset();
        let mut output = String::new();

        // Imports
        output.push_str("use verum_std::core::{List, Text, Map, Maybe}\n");
        output.push_str("use verum_std::async::{spawn, sleep, join, select, timeout}\n");
        output.push_str("use verum_std::channel::{channel, Sender, Receiver}\n\n");

        // Generate async functions
        let num_functions = rng.random_range(2..=5);
        for i in 0..num_functions {
            let name = format!("async_task_{}", i);
            self.async_functions.push(name.clone());
            output.push_str(&self.generate_async_function(rng, &name));
            output.push_str("\n\n");
        }

        // Generate channel-based functions
        if rng.random_bool(0.7) {
            output.push_str(&self.generate_channel_producer(rng));
            output.push_str("\n\n");
            output.push_str(&self.generate_channel_consumer(rng));
            output.push_str("\n\n");
        }

        // Generate async main
        output.push_str(&self.generate_async_main(rng));

        output
    }

    /// Generate an async function
    fn generate_async_function<R: Rng>(&mut self, rng: &mut R, name: &str) -> String {
        let num_params = rng.random_range(0..=2);
        let params: Vec<String> = (0..num_params).map(|i| format!("p{}: Int", i)).collect();

        let return_type = if rng.random_bool(0.7) { " -> Int" } else { "" };

        let body = self.generate_async_body(rng, 1);

        format!(
            "async fn {}({}){}  {{\n{}\n}}",
            name,
            params.join(", "),
            return_type,
            body
        )
    }

    /// Generate async function body
    fn generate_async_body<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let mut body = String::new();
        let num_stmts = rng.random_range(2..=8);

        for _ in 0..num_stmts {
            body.push_str(&self.generate_async_statement(rng, indent_level));
            body.push('\n');
        }

        // Return value
        if rng.random_bool(0.7) {
            body.push_str(&format!(
                "{}{}",
                indent(indent_level),
                rng.random_range(0..100)
            ));
        }

        body
    }

    /// Generate an async statement
    fn generate_async_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        if self.current_depth > self.config.max_depth {
            return format!("{}let x = 0;", indent(indent_level));
        }

        self.current_depth += 1;
        let result = match rng.random_range(0..12) {
            0 => self.generate_await_call(rng, indent_level),
            1 => self.generate_spawn(rng, indent_level),
            2 => self.generate_sleep(rng, indent_level),
            3 => self.generate_join(rng, indent_level),
            4 => self.generate_select(rng, indent_level),
            5 => self.generate_timeout_wrapper(rng, indent_level),
            6 => self.generate_channel_send(rng, indent_level),
            7 => self.generate_channel_recv(rng, indent_level),
            8 => self.generate_async_if(rng, indent_level),
            9 => self.generate_async_loop(rng, indent_level),
            10 => self.generate_async_let(rng, indent_level),
            _ => format!(
                "{}let {} = {};",
                indent(indent_level),
                random_identifier(rng),
                rng.random_range(0..100)
            ),
        };
        self.current_depth -= 1;
        result
    }

    /// Generate an await call
    fn generate_await_call<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let func_name = if !self.async_functions.is_empty() && rng.random_bool(0.5) {
            self.async_functions[rng.random_range(0..self.async_functions.len())].clone()
        } else {
            format!("async_op_{}", rng.random_range(0..5))
        };

        let num_args = rng.random_range(0..3);
        let args: Vec<String> = (0..num_args)
            .map(|_| format!("{}", rng.random_range(0..100)))
            .collect();

        format!(
            "{}let result = {}({}).await;",
            indent(indent_level),
            func_name,
            args.join(", ")
        )
    }

    /// Generate spawn statement
    fn generate_spawn<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let task_name = random_identifier(rng);

        let task_body = if rng.random_bool(0.5) {
            // Spawn with closure
            format!(
                "async {{ sleep({}).await; {} }}",
                rng.random_range(10..1000),
                rng.random_range(0..100)
            )
        } else if !self.async_functions.is_empty() {
            // Spawn existing function
            let func = &self.async_functions[rng.random_range(0..self.async_functions.len())];
            format!("{}({})", func, rng.random_range(0..100))
        } else {
            format!("async {{ {} }}", rng.random_range(0..100))
        };

        format!(
            "{}let {} = spawn({});",
            indent(indent_level),
            task_name,
            task_body
        )
    }

    /// Generate sleep statement
    fn generate_sleep<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let duration = rng.random_range(1..1000);
        format!("{}sleep({}).await;", indent(indent_level), duration)
    }

    /// Generate join statement
    fn generate_join<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let num_tasks = rng.random_range(2..=4);
        let tasks: Vec<String> = (0..num_tasks).map(|i| format!("task_{}", i)).collect();

        format!(
            "{}let results = join!({}).await;",
            indent(indent_level),
            tasks.join(", ")
        )
    }

    /// Generate select statement
    fn generate_select<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let branches: Vec<String> = (0..rng.random_range(2..=4))
            .map(|i| {
                format!(
                    "{}    fut_{} => {{ {} }}",
                    indent(indent_level),
                    i,
                    rng.random_range(0..100)
                )
            })
            .collect();

        format!(
            "{}let result = select! {{\n{}\n{}}};",
            indent(indent_level),
            branches.join(",\n"),
            indent(indent_level)
        )
    }

    /// Generate timeout wrapper
    fn generate_timeout_wrapper<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let duration = rng.random_range(100..5000);
        let inner_op = if rng.random_bool(0.5) {
            format!("sleep({}).await", rng.random_range(10..1000))
        } else {
            format!("compute({})", rng.random_range(0..100))
        };

        format!(
            "{}let result = timeout({}, async {{ {} }}).await;",
            indent(indent_level),
            duration,
            inner_op
        )
    }

    /// Generate channel send
    fn generate_channel_send<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let channel = if !self.channels.is_empty() && rng.random_bool(0.5) {
            self.channels[rng.random_range(0..self.channels.len())].clone()
        } else {
            let name = format!("tx_{}", rng.random_range(0..10));
            self.channels.push(name.clone());
            name
        };

        format!(
            "{}{}.send({}).await;",
            indent(indent_level),
            channel,
            rng.random_range(0..100)
        )
    }

    /// Generate channel receive
    fn generate_channel_recv<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let channel = if !self.channels.is_empty() && rng.random_bool(0.5) {
            self.channels[rng.random_range(0..self.channels.len())].clone()
        } else {
            format!("rx_{}", rng.random_range(0..10))
        };

        format!(
            "{}let msg = {}.recv().await;",
            indent(indent_level),
            channel
        )
    }

    /// Generate async if statement
    fn generate_async_if<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let condition = if rng.random_bool(0.5) {
            "true".to_string()
        } else {
            format!("{} > 0", rng.random_range(0..10))
        };

        let then_body = self.generate_async_statement(rng, indent_level + 1);
        let else_body = if rng.random_bool(0.5) {
            format!(
                " else {{\n{}\n{}}}",
                self.generate_async_statement(rng, indent_level + 1),
                indent(indent_level)
            )
        } else {
            String::new()
        };

        format!(
            "{}if {} {{\n{}\n{}}}{}",
            indent(indent_level),
            condition,
            then_body,
            indent(indent_level),
            else_body
        )
    }

    /// Generate async loop
    fn generate_async_loop<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let iterations = rng.random_range(1..10);

        format!(
            "{}for i in 0..{} {{\n{}\n{}}}",
            indent(indent_level),
            iterations,
            self.generate_async_statement(rng, indent_level + 1),
            indent(indent_level)
        )
    }

    /// Generate async let binding
    fn generate_async_let<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        let value = if rng.random_bool(0.5) {
            format!("compute({}).await", rng.random_range(0..100))
        } else {
            format!("{}", rng.random_range(0..100))
        };

        format!("{}let {} = {};", indent(indent_level), name, value)
    }

    /// Generate channel producer function
    fn generate_channel_producer<R: Rng>(&self, rng: &mut R) -> String {
        let num_messages = rng.random_range(5..20);

        format!(
            r#"async fn producer(tx: Sender<Int>) {{
    for i in 0..{} {{
        tx.send(i * {}).await;
        sleep({}).await;
    }}
}}"#,
            num_messages,
            rng.random_range(1..10),
            rng.random_range(10..100)
        )
    }

    /// Generate channel consumer function
    fn generate_channel_consumer<R: Rng>(&self, rng: &mut R) -> String {
        format!(
            r#"async fn consumer(rx: Receiver<Int>) -> Int {{
    let mut sum = 0;
    while let Some(msg) = rx.recv().await {{
        sum = sum + msg;
        if sum > {} {{
            break;
        }}
    }}
    sum
}}"#,
            rng.random_range(100..1000)
        )
    }

    /// Generate async main function
    fn generate_async_main<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut body = String::new();
        let indent_level = 1;

        // Create channels
        if rng.random_bool(0.7) {
            body.push_str(&format!(
                "{}let (tx, rx) = channel::<Int>();\n",
                indent(indent_level)
            ));
            self.channels.push("tx".to_string());
        }

        // Spawn some tasks
        let num_tasks = rng.random_range(1..=4);
        for i in 0..num_tasks {
            if !self.async_functions.is_empty() {
                let func = &self.async_functions[rng.random_range(0..self.async_functions.len())];
                body.push_str(&format!(
                    "{}let task_{} = spawn({}({}));\n",
                    indent(indent_level),
                    i,
                    func,
                    rng.random_range(0..100)
                ));
            }
        }

        // Add some await operations
        for _ in 0..rng.random_range(1..4) {
            body.push_str(&self.generate_async_statement(rng, indent_level));
            body.push('\n');
        }

        // Join tasks if spawned
        if num_tasks > 0 && !self.async_functions.is_empty() {
            let task_names: Vec<String> = (0..num_tasks).map(|i| format!("task_{}", i)).collect();
            body.push_str(&format!(
                "\n{}let results = join!({}).await;\n",
                indent(indent_level),
                task_names.join(", ")
            ));
        }

        // Print result
        body.push_str(&format!("{}println(\"Done\");\n", indent(indent_level)));

        format!("async fn main() {{\n{}}}\n", body)
    }
}

impl Generate for AsyncGenerator {
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "AsyncGenerator"
    }

    fn description(&self) -> &'static str {
        "Generates async/concurrent programs with channels and spawned tasks"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_async_generator() {
        let config = GeneratorConfig {
            include_async: true,
            ..Default::default()
        };
        let mut generator = AsyncGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.is_empty());
        assert!(program.contains("async fn main()"));
    }

    #[test]
    fn test_generates_async_constructs() {
        let config = GeneratorConfig {
            include_async: true,
            max_functions: 5,
            max_statements: 20,
            ..Default::default()
        };
        let mut generator = AsyncGenerator::new(config);

        let mut found_await = false;
        let mut found_spawn = false;
        let mut found_channel = false;

        for seed in 0..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let program = generator.generate(&mut rng);

            if program.contains(".await") {
                found_await = true;
            }
            if program.contains("spawn(") {
                found_spawn = true;
            }
            if program.contains("channel") {
                found_channel = true;
            }

            if found_await && found_spawn && found_channel {
                break;
            }
        }

        assert!(found_await, "Should generate await expressions");
        assert!(found_spawn, "Should generate spawn calls");
        assert!(found_channel, "Should generate channel operations");
    }

    #[test]
    fn test_generates_async_functions() {
        let config = GeneratorConfig {
            include_async: true,
            max_functions: 5,
            ..Default::default()
        };
        let mut generator = AsyncGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);

        // Count async function definitions
        let async_fn_count = program.matches("async fn").count();
        assert!(
            async_fn_count >= 2,
            "Should generate multiple async functions"
        );
    }
}
