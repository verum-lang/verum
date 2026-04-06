//! CBGR (Checked Borrow with Generational References) pattern generator
//!
//! Generates programs that stress test the CBGR memory safety system.
//! Tests:
//! - Reference creation and usage
//! - Borrowing patterns (shared, mutable, checked, unsafe)
//! - Lifetime-like scenarios
//! - Memory allocation patterns
//! - Reference invalidation scenarios
//! - Complex ownership transfers

use super::{Generate, GeneratorConfig, indent, random_identifier, random_type};
use rand::prelude::*;

/// Generator for CBGR stress test programs
pub struct CbgrGenerator {
    config: GeneratorConfig,
    current_depth: usize,
    variables: Vec<(String, String, RefKind)>, // (name, type, reference kind)
    heap_allocations: Vec<String>,
}

/// Kind of reference
#[derive(Clone, Copy, Debug)]
enum RefKind {
    Owned,
    SharedRef,
    MutRef,
    CheckedRef,
    UnsafeRef,
}

impl CbgrGenerator {
    /// Create a new CBGR generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            current_depth: 0,
            variables: Vec::new(),
            heap_allocations: Vec::new(),
        }
    }

    /// Reset state
    fn reset(&mut self) {
        self.current_depth = 0;
        self.variables.clear();
        self.heap_allocations.clear();
    }

    /// Generate a complete CBGR stress program
    fn generate_program<R: Rng>(&mut self, rng: &mut R) -> String {
        self.reset();
        let mut output = String::new();

        // Imports
        output.push_str("use verum_std::core::{List, Text, Map, Maybe, Set}\n");
        output.push_str("use verum_std::memory::{Heap, Arena, Shared}\n\n");

        // Generate types with various ownership patterns
        let num_types = rng.random_range(1..=3);
        for i in 0..num_types {
            output.push_str(&self.generate_owned_type(rng, &format!("Data{}", i)));
            output.push_str("\n\n");
        }

        // Generate functions with reference parameters
        let num_functions = rng.random_range(2..=5);
        for i in 0..num_functions {
            output.push_str(&self.generate_ref_function(rng, &format!("process_{}", i)));
            output.push_str("\n\n");
        }

        // Generate main with CBGR stress patterns
        output.push_str(&self.generate_cbgr_main(rng));

        output
    }

    /// Generate an owned type with fields
    fn generate_owned_type<R: Rng>(&self, rng: &mut R, name: &str) -> String {
        let num_fields = rng.random_range(2..=5);
        let fields: Vec<String> = (0..num_fields)
            .map(|i| {
                let field_type = match rng.random_range(0..6) {
                    0 => "Int".to_string(),
                    1 => "Text".to_string(),
                    2 => "List<Int>".to_string(),
                    3 => "Map<Text, Int>".to_string(),
                    4 => format!("Heap<{}>", random_type(rng, 0)),
                    _ => format!("Shared<{}>", random_type(rng, 0)),
                };
                format!("    field_{}: {}", i, field_type)
            })
            .collect();

        format!("type {} = {{\n{}\n}}", name, fields.join(",\n"))
    }

    /// Generate a function with reference parameters
    fn generate_ref_function<R: Rng>(&mut self, rng: &mut R, name: &str) -> String {
        self.variables.clear();

        // Generate parameters with various reference types
        let num_params = rng.random_range(1..=3);
        let mut params = Vec::new();

        for i in 0..num_params {
            let param_name = format!("p{}", i);
            let (param_type, ref_kind) = self.generate_ref_param_type(rng);
            self.variables
                .push((param_name.clone(), param_type.clone(), ref_kind));
            params.push(format!("{}: {}", param_name, param_type));
        }

        let return_type = if rng.random_bool(0.5) { " -> Int" } else { "" };

        let body = self.generate_ref_body(rng, 1);

        format!(
            "fn {}({}){}  {{\n{}\n}}",
            name,
            params.join(", "),
            return_type,
            body
        )
    }

    /// Generate a reference parameter type
    fn generate_ref_param_type<R: Rng>(&self, rng: &mut R) -> (String, RefKind) {
        match rng.random_range(0..5) {
            0 => ("&Int".to_string(), RefKind::SharedRef),
            1 => ("&mut Int".to_string(), RefKind::MutRef),
            2 => ("&checked Int".to_string(), RefKind::CheckedRef),
            3 if self.config.include_unsafe => ("&unsafe Int".to_string(), RefKind::UnsafeRef),
            _ => ("&List<Int>".to_string(), RefKind::SharedRef),
        }
    }

    /// Generate function body with reference operations
    fn generate_ref_body<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let mut body = String::new();
        let num_stmts = rng.random_range(3..=10);

        for _ in 0..num_stmts {
            body.push_str(&self.generate_ref_statement(rng, indent_level));
            body.push('\n');
        }

        // Return value
        if rng.random_bool(0.5) {
            body.push_str(&format!(
                "{}{}",
                indent(indent_level),
                rng.random_range(0..100)
            ));
        }

        body
    }

    /// Generate a statement involving references
    fn generate_ref_statement<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        if self.current_depth > self.config.max_depth {
            return format!("{}let x = 0;", indent(indent_level));
        }

        self.current_depth += 1;
        let result = match rng.random_range(0..15) {
            0 => self.generate_ref_create(rng, indent_level),
            1 => self.generate_ref_deref(rng, indent_level),
            2 => self.generate_ref_reborrow(rng, indent_level),
            3 => self.generate_heap_alloc(rng, indent_level),
            4 => self.generate_heap_dealloc(rng, indent_level),
            5 => self.generate_shared_create(rng, indent_level),
            6 => self.generate_shared_clone(rng, indent_level),
            7 => self.generate_ref_field_access(rng, indent_level),
            8 => self.generate_ref_scope(rng, indent_level),
            9 => self.generate_ref_if(rng, indent_level),
            10 => self.generate_move_semantics(rng, indent_level),
            11 => self.generate_borrow_check_edge(rng, indent_level),
            12 => self.generate_arena_alloc(rng, indent_level),
            13 => self.generate_checked_ref(rng, indent_level),
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

    /// Generate reference creation
    fn generate_ref_create<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        let ref_kind = match rng.random_range(0..4) {
            0 => "&",
            1 => "&mut ",
            2 => "&checked ",
            _ => "&",
        };

        self.variables.push((
            format!("ref_{}", name),
            "Int".to_string(),
            RefKind::SharedRef,
        ));

        format!(
            "{}let {} = {};\n{}let ref_{} = {}{};",
            indent(indent_level),
            name,
            rng.random_range(0..100),
            indent(indent_level),
            name,
            ref_kind,
            name
        )
    }

    /// Generate reference dereference
    fn generate_ref_deref<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let ref_name = if self.variables.is_empty() {
            "some_ref".to_string()
        } else {
            let (name, _, _) = &self.variables[rng.random_range(0..self.variables.len())];
            name.clone()
        };

        format!("{}let deref_val = *{};", indent(indent_level), ref_name)
    }

    /// Generate reborrow
    fn generate_ref_reborrow<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        format!(
            "{}let reborrow_{} = &*original_ref;",
            indent(indent_level),
            name
        )
    }

    /// Generate heap allocation
    fn generate_heap_alloc<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        let ty = match rng.random_range(0..3) {
            0 => "Int".to_string(),
            1 => "List<Int>".to_string(),
            _ => random_type(rng, 0),
        };

        self.heap_allocations.push(name.clone());
        self.variables
            .push((name.clone(), ty.clone(), RefKind::Owned));

        let value = match ty.as_str() {
            "Int" => format!("{}", rng.random_range(0..100)),
            "List<Int>" => format!(
                "[{}]",
                (0..rng.random_range(1..5))
                    .map(|_| format!("{}", rng.random_range(0..10)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => "()".to_string(),
        };

        format!(
            "{}let {} = Heap.new({});",
            indent(indent_level),
            name,
            value
        )
    }

    /// Generate heap deallocation
    fn generate_heap_dealloc<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        if self.heap_allocations.is_empty() {
            // Return a simple let statement instead of calling generate_heap_alloc
            let name = super::random_identifier(rng);
            return format!(
                "{}let {} = Heap.new({});",
                indent(indent_level),
                name,
                rng.random_range(0..100)
            );
        }

        let name = &self.heap_allocations[rng.random_range(0..self.heap_allocations.len())];
        format!("{}Heap.drop({});", indent(indent_level), name)
    }

    /// Generate shared reference creation
    fn generate_shared_create<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        self.variables
            .push((name.clone(), "Int".to_string(), RefKind::Owned));

        format!(
            "{}let {} = Shared.new({});",
            indent(indent_level),
            name,
            rng.random_range(0..100)
        )
    }

    /// Generate shared clone
    fn generate_shared_clone<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        format!("{}let {} = shared_val.clone();", indent(indent_level), name)
    }

    /// Generate reference field access
    fn generate_ref_field_access<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let fields = ["field_0", "field_1", "data", "value"];
        let field = fields[rng.random_range(0..fields.len())];

        format!(
            "{}let field_val = (*struct_ref).{};",
            indent(indent_level),
            field
        )
    }

    /// Generate scoped references
    fn generate_ref_scope<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let inner = self.generate_ref_statement(rng, indent_level + 1);
        format!(
            "{}{{\n{}let scoped_data = {};\n{}let scoped_ref = &scoped_data;\n{}\n{}}}",
            indent(indent_level),
            indent(indent_level + 1),
            rng.random_range(0..100),
            indent(indent_level + 1),
            inner,
            indent(indent_level)
        )
    }

    /// Generate conditional with references
    fn generate_ref_if<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let condition = format!("ref_val > {}", rng.random_range(0..50));
        let then_stmt = self.generate_ref_statement(rng, indent_level + 1);
        let else_stmt = self.generate_ref_statement(rng, indent_level + 1);

        format!(
            "{}if {} {{\n{}\n{}}} else {{\n{}\n{}}}",
            indent(indent_level),
            condition,
            then_stmt,
            indent(indent_level),
            else_stmt,
            indent(indent_level)
        )
    }

    /// Generate move semantics patterns
    fn generate_move_semantics<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        self.variables
            .push((name.clone(), "Int".to_string(), RefKind::Owned));

        format!(
            "{}let {} = owned_val;\n{}// owned_val is now moved",
            indent(indent_level),
            name,
            indent(indent_level)
        )
    }

    /// Generate edge cases for borrow checking
    fn generate_borrow_check_edge<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        match rng.random_range(0..5) {
            0 => {
                // Multiple borrows
                format!(
                    "{}let r1 = &data;\n{}let r2 = &data;\n{}let sum = *r1 + *r2;",
                    indent(indent_level),
                    indent(indent_level),
                    indent(indent_level)
                )
            }
            1 => {
                // Borrow then use
                format!(
                    "{}let r = &data;\n{}let val = *r;\n{}// borrow ends here",
                    indent(indent_level),
                    indent(indent_level),
                    indent(indent_level)
                )
            }
            2 => {
                // Nested borrows
                format!(
                    "{}{{\n{}let inner_ref = &outer_val;\n{}let inner_val = *inner_ref;\n{}}}",
                    indent(indent_level),
                    indent(indent_level + 1),
                    indent(indent_level + 1),
                    indent(indent_level)
                )
            }
            3 => {
                // Return reference (lifetime annotation needed)
                format!(
                    "{}let result_ref = get_ref(&container);",
                    indent(indent_level)
                )
            }
            _ => {
                // Mutable borrow exclusive
                format!(
                    "{}let mut_ref = &mut data;\n{}*mut_ref = {};",
                    indent(indent_level),
                    indent(indent_level),
                    rng.random_range(0..100)
                )
            }
        }
    }

    /// Generate arena allocation
    fn generate_arena_alloc<R: Rng>(&mut self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        self.variables
            .push((name.clone(), "Int".to_string(), RefKind::Owned));

        format!(
            "{}let {} = arena.alloc({});",
            indent(indent_level),
            name,
            rng.random_range(0..100)
        )
    }

    /// Generate checked reference operations
    fn generate_checked_ref<R: Rng>(&self, rng: &mut R, indent_level: usize) -> String {
        let name = random_identifier(rng);
        format!(
            "{}let {} = data;\n{}let checked = &checked {};\n{}// CBGR check: ~15ns overhead",
            indent(indent_level),
            name,
            indent(indent_level),
            name,
            indent(indent_level)
        )
    }

    /// Generate main function with CBGR patterns
    fn generate_cbgr_main<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut body = String::new();
        let indent_level = 1;

        // Create arena for arena allocations
        body.push_str(&format!(
            "{}let arena = Arena.new();\n\n",
            indent(indent_level)
        ));

        // Create some owned data
        body.push_str(&format!(
            "{}let mut data = {};\n",
            indent(indent_level),
            rng.random_range(0..100)
        ));
        self.variables
            .push(("data".to_string(), "Int".to_string(), RefKind::Owned));

        // Create various references
        body.push_str(&format!(
            "{}let shared_ref = &data;\n",
            indent(indent_level)
        ));
        body.push_str(&format!(
            "{}let checked_ref = &checked data;\n\n",
            indent(indent_level)
        ));

        // CBGR demonstration section
        body.push_str(&format!(
            "{}// CBGR reference patterns\n",
            indent(indent_level)
        ));

        // Add various CBGR operations
        let num_operations = rng.random_range(5..15);
        for _ in 0..num_operations {
            body.push_str(&self.generate_ref_statement(rng, indent_level));
            body.push('\n');
        }

        // Demonstrate generation checking
        body.push_str(&format!("\n{}// Generation checks\n", indent(indent_level)));
        body.push_str(&format!(
            "{}let gen_check = CBGR.check_generation(checked_ref);\n",
            indent(indent_level)
        ));
        body.push_str(&format!(
            "{}assert gen_check.is_valid();\n",
            indent(indent_level)
        ));

        // Cleanup
        body.push_str(&format!("\n{}arena.reset();\n", indent(indent_level)));

        format!("fn main() {{\n{}}}\n", body)
    }
}

impl Generate for CbgrGenerator {
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "CbgrGenerator"
    }

    fn description(&self) -> &'static str {
        "Generates programs with CBGR memory safety patterns"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_cbgr_generator() {
        let config = GeneratorConfig {
            include_cbgr: true,
            ..Default::default()
        };
        let mut generator = CbgrGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.is_empty());
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_generates_references() {
        let config = GeneratorConfig {
            include_cbgr: true,
            max_statements: 30,
            ..Default::default()
        };
        let mut generator = CbgrGenerator::new(config);

        let mut found_shared = false;
        let mut found_checked = false;
        let mut found_heap = false;

        for seed in 0..50 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let program = generator.generate(&mut rng);

            if program.contains("&") || program.contains("shared_ref") {
                found_shared = true;
            }
            if program.contains("&checked") || program.contains("checked_ref") {
                found_checked = true;
            }
            if program.contains("Heap.new") {
                found_heap = true;
            }

            if found_shared && found_checked && found_heap {
                break;
            }
        }

        assert!(found_shared, "Should generate shared references");
        assert!(found_checked, "Should generate checked references");
        assert!(found_heap, "Should generate heap allocations");
    }

    #[test]
    fn test_generates_memory_patterns() {
        let config = GeneratorConfig {
            include_cbgr: true,
            ..Default::default()
        };
        let mut generator = CbgrGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);

        // Should have Arena usage
        assert!(program.contains("Arena"), "Should include Arena patterns");
    }
}
