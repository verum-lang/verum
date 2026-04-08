//! Complete program generator for fuzz testing
//!
//! This module provides random program generation with Arbitrary trait
//! implementations for property-based testing. It combines expression and
//! type generators to create complete, valid Verum programs.
//!
//! # Features
//!
//! - Complete program structure (imports, types, functions, main)
//! - Type-aware generation for valid programs
//! - Configurable complexity and features
//! - Shrinking for minimal counterexamples
//! - Property-based testing support
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::generators::program_generator::{ProgramGenerator, ArbitraryProgram};
//! use rand::rng;
//!
//! let generator = ProgramGenerator::new(Default::default());
//! let program = generator.generate(&mut rng());
//! ```

use super::config::GeneratorConfig;
use super::expr_generator::ExprGenerator;
use super::type_generator::{ArbitraryType, TypeGenerator, TypeKind};
use rand::Rng;
use std::fmt;

/// Generated program with source representation
#[derive(Clone)]
pub struct ArbitraryProgram {
    /// Source code representation
    pub source: String,
    /// Program structure for shrinking
    pub structure: ProgramStructure,
    /// Estimated complexity score
    pub complexity: usize,
}

impl fmt::Debug for ArbitraryProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitraryProgram")
            .field("source_len", &self.source.len())
            .field("num_functions", &self.structure.functions.len())
            .field("num_types", &self.structure.types.len())
            .finish()
    }
}

impl fmt::Display for ArbitraryProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl ArbitraryProgram {
    /// Create a new program
    pub fn new(source: String, structure: ProgramStructure) -> Self {
        let complexity = Self::calculate_complexity(&source, &structure);
        Self {
            source,
            structure,
            complexity,
        }
    }

    /// Calculate complexity score for a program
    fn calculate_complexity(source: &str, structure: &ProgramStructure) -> usize {
        let mut score = source.len();
        score += structure.functions.len() * 50;
        score += structure.types.len() * 30;
        score += source.matches("fn ").count() * 20;
        score += source.matches("if ").count() * 5;
        score += source.matches("match ").count() * 8;
        score += source.matches("async ").count() * 10;
        score += source.matches("using").count() * 5;
        score
    }

    /// Generate shrunk versions of this program
    pub fn shrink(&self) -> Vec<ArbitraryProgram> {
        let mut shrunk = Vec::new();

        // Try removing functions (except main)
        for i in 0..self.structure.functions.len() {
            if self.structure.functions[i].name != "main" {
                let mut new_structure = self.structure.clone();
                new_structure.functions.remove(i);
                if let Some(prog) = self.rebuild(&new_structure)
                    && prog.complexity < self.complexity
                {
                    shrunk.push(prog);
                }
            }
        }

        // Try removing type definitions
        for i in 0..self.structure.types.len() {
            let mut new_structure = self.structure.clone();
            new_structure.types.remove(i);
            if let Some(prog) = self.rebuild(&new_structure)
                && prog.complexity < self.complexity
            {
                shrunk.push(prog);
            }
        }

        // Try removing imports
        for i in 0..self.structure.imports.len() {
            let mut new_structure = self.structure.clone();
            new_structure.imports.remove(i);
            if let Some(prog) = self.rebuild(&new_structure)
                && prog.complexity < self.complexity
            {
                shrunk.push(prog);
            }
        }

        // Try simplifying function bodies
        for (i, func) in self.structure.functions.iter().enumerate() {
            for simpler_body in func.body.shrink() {
                let mut new_func = func.clone();
                new_func.body = simpler_body;

                let mut new_structure = self.structure.clone();
                new_structure.functions[i] = new_func;
                if let Some(prog) = self.rebuild(&new_structure)
                    && prog.complexity < self.complexity
                {
                    shrunk.push(prog);
                }
            }
        }

        shrunk
    }

    /// Rebuild program from structure
    fn rebuild(&self, structure: &ProgramStructure) -> Option<ArbitraryProgram> {
        let mut source = String::new();

        // Imports
        for import in &structure.imports {
            source.push_str(&import.source);
            source.push('\n');
        }
        if !structure.imports.is_empty() {
            source.push('\n');
        }

        // Type definitions
        for type_def in &structure.types {
            source.push_str(&type_def.source);
            source.push_str("\n\n");
        }

        // Functions
        for func in &structure.functions {
            source.push_str(&func.source);
            source.push_str("\n\n");
        }

        Some(ArbitraryProgram::new(source, structure.clone()))
    }
}

/// Program structure for shrinking
#[derive(Debug, Clone)]
pub struct ProgramStructure {
    pub imports: Vec<ImportDef>,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
}

/// Import definition
#[derive(Debug, Clone)]
pub struct ImportDef {
    pub source: String,
    pub path: String,
}

/// Type definition
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub source: String,
    pub name: String,
    pub kind: TypeDefKind,
}

#[derive(Debug, Clone)]
pub enum TypeDefKind {
    Struct {
        fields: Vec<(String, ArbitraryType)>,
    },
    Enum {
        variants: Vec<(String, Option<ArbitraryType>)>,
    },
    Alias {
        target: ArbitraryType,
    },
}

/// Function definition
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub source: String,
    pub name: String,
    pub params: Vec<(String, ArbitraryType)>,
    pub return_type: Option<ArbitraryType>,
    pub body: FunctionBody,
    pub is_async: bool,
    pub is_pub: bool,
    pub contexts: Vec<String>,
}

/// Function body for shrinking
#[derive(Debug, Clone)]
pub struct FunctionBody {
    pub statements: Vec<String>,
    pub trailing_expr: Option<String>,
}

impl FunctionBody {
    /// Generate shrunk versions of this function body
    pub fn shrink(&self) -> Vec<FunctionBody> {
        let mut shrunk = Vec::new();

        // Try removing statements
        for i in 0..self.statements.len() {
            let mut new_stmts = self.statements.clone();
            new_stmts.remove(i);
            shrunk.push(FunctionBody {
                statements: new_stmts,
                trailing_expr: self.trailing_expr.clone(),
            });
        }

        // Try simpler trailing expression
        if self.trailing_expr.is_some() {
            shrunk.push(FunctionBody {
                statements: self.statements.clone(),
                trailing_expr: Some("()".to_string()),
            });
            shrunk.push(FunctionBody {
                statements: self.statements.clone(),
                trailing_expr: Some("0".to_string()),
            });
        }

        shrunk
    }
}

/// Program generator
pub struct ProgramGenerator {
    config: GeneratorConfig,
    expr_gen: ExprGenerator,
    type_gen: TypeGenerator,
}

impl ProgramGenerator {
    /// Create a new program generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        let expr_gen = ExprGenerator::new(config.clone());
        let type_gen = TypeGenerator::new(config.clone());
        Self {
            config,
            expr_gen,
            type_gen,
        }
    }

    /// Generate a random program
    pub fn generate<R: Rng>(&self, rng: &mut R) -> ArbitraryProgram {
        let mut ctx = GenerationContext::new();
        let mut source = String::new();
        let mut structure = ProgramStructure {
            imports: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
        };

        // Generate header comment
        source.push_str("// Auto-generated Verum program for fuzz testing\n\n");

        // Generate imports
        let imports = self.generate_imports(rng, &mut ctx);
        for import in &imports {
            source.push_str(&import.source);
            source.push('\n');
        }
        source.push('\n');
        structure.imports = imports;

        // Generate type definitions
        let num_types = rng.random_range(0..=self.config.complexity.max_types);
        for i in 0..num_types {
            let type_def = self.generate_type_def(rng, &mut ctx, i);
            source.push_str(&type_def.source);
            source.push_str("\n\n");
            ctx.types.push(type_def.name.clone());
            structure.types.push(type_def);
        }

        // Generate helper functions
        let num_funcs = if self.config.complexity.max_functions > 1 {
            rng.random_range(0..self.config.complexity.max_functions.saturating_sub(1))
        } else {
            0
        };
        for i in 0..num_funcs {
            let func = self.generate_function(rng, &mut ctx, &format!("func_{}", i), false);
            source.push_str(&func.source);
            source.push_str("\n\n");
            ctx.functions.push(func.name.clone());
            structure.functions.push(func);
        }

        // Generate main function
        let main = self.generate_function(rng, &mut ctx, "main", true);
        source.push_str(&main.source);
        source.push('\n');
        structure.functions.push(main);

        ArbitraryProgram::new(source, structure)
    }

    /// Generate import statements
    fn generate_imports<R: Rng>(
        &self,
        rng: &mut R,
        _ctx: &mut GenerationContext,
    ) -> Vec<ImportDef> {
        let mut imports = Vec::new();

        // Always import core types
        imports.push(ImportDef {
            source: "use verum_std::core::{List, Text, Map, Maybe, Set}".to_string(),
            path: "verum_std::core".to_string(),
        });

        // Optionally import IO
        if rng.random_bool(0.3) {
            imports.push(ImportDef {
                source: "use verum_std::io::{print, println}".to_string(),
                path: "verum_std::io".to_string(),
            });
        }

        // Optionally import async
        if self.config.features.async_await && rng.random_bool(0.3) {
            imports.push(ImportDef {
                source: "use verum_std::async::{spawn, sleep}".to_string(),
                path: "verum_std::async".to_string(),
            });
        }

        imports
    }

    /// Generate a type definition
    fn generate_type_def<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        idx: usize,
    ) -> TypeDef {
        if rng.random_bool(0.6) {
            self.generate_struct_def(rng, ctx, idx)
        } else if rng.random_bool(0.7) {
            self.generate_enum_def(rng, ctx, idx)
        } else {
            self.generate_type_alias(rng, ctx, idx)
        }
    }

    /// Generate a struct definition
    fn generate_struct_def<R: Rng>(
        &self,
        rng: &mut R,
        _ctx: &mut GenerationContext,
        idx: usize,
    ) -> TypeDef {
        let name = format!("Struct_{}", idx);
        let num_fields = rng.random_range(1..=self.config.complexity.max_struct_fields.min(6));

        let mut fields = Vec::new();
        let mut source = format!("struct {} {{\n", name);

        for i in 0..num_fields {
            let field_name = format!("field_{}", i);
            let field_type = self.type_gen.generate_primitive(rng);
            source.push_str(&format!("    {}: {},\n", field_name, field_type.source));
            fields.push((field_name, field_type));
        }

        source.push('}');

        TypeDef {
            source,
            name,
            kind: TypeDefKind::Struct { fields },
        }
    }

    /// Generate an enum definition
    fn generate_enum_def<R: Rng>(
        &self,
        rng: &mut R,
        _ctx: &mut GenerationContext,
        idx: usize,
    ) -> TypeDef {
        let name = format!("Enum_{}", idx);
        let num_variants = rng.random_range(2..=self.config.complexity.max_enum_variants.min(5));

        let mut variants = Vec::new();
        let mut source = format!("enum {} {{\n", name);

        for i in 0..num_variants {
            let variant_name = format!("Variant_{}", i);
            let has_data = rng.random_bool(0.5);

            if has_data {
                let data_type = self.type_gen.generate_primitive(rng);
                source.push_str(&format!("    {}({}),\n", variant_name, data_type.source));
                variants.push((variant_name, Some(data_type)));
            } else {
                source.push_str(&format!("    {},\n", variant_name));
                variants.push((variant_name, None));
            }
        }

        source.push('}');

        TypeDef {
            source,
            name,
            kind: TypeDefKind::Enum { variants },
        }
    }

    /// Generate a type alias
    fn generate_type_alias<R: Rng>(
        &self,
        rng: &mut R,
        _ctx: &mut GenerationContext,
        idx: usize,
    ) -> TypeDef {
        let name = format!("Alias_{}", idx);
        let target = self.type_gen.generate(rng);
        let source = format!("type {} = {}", name, target.source);

        TypeDef {
            source,
            name,
            kind: TypeDefKind::Alias { target },
        }
    }

    /// Generate a function
    fn generate_function<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        name: &str,
        is_main: bool,
    ) -> FunctionDef {
        let is_async = !is_main && self.config.features.async_await && rng.random_bool(0.2);
        let is_pub = !is_main && rng.random_bool(0.3);

        // Generate parameters
        let num_params = if is_main {
            0
        } else {
            rng.random_range(0..=self.config.complexity.max_params.min(4))
        };

        let mut params = Vec::new();
        let mut param_names = Vec::new();

        for i in 0..num_params {
            let param_name = format!("arg_{}", i);
            let param_type = self.type_gen.generate_primitive(rng);
            params.push((param_name.clone(), param_type));
            param_names.push(param_name);
        }

        // Generate return type
        let return_type = if is_main {
            None
        } else if rng.random_bool(0.7) {
            Some(self.type_gen.generate_primitive(rng))
        } else {
            None
        };

        // Generate context requirements
        let contexts = if !is_main && self.config.features.contexts && rng.random_bool(0.2) {
            vec!["Logger".to_string()]
        } else {
            vec![]
        };

        // Build function signature
        let mut source = String::new();

        if is_pub {
            source.push_str("pub ");
        }
        if is_async {
            source.push_str("async ");
        }

        source.push_str(&format!("fn {}(", name));
        source.push_str(
            &params
                .iter()
                .map(|(n, t)| format!("{}: {}", n, t.source))
                .collect::<Vec<_>>()
                .join(", "),
        );
        source.push(')');

        if let Some(ref ret) = return_type {
            source.push_str(&format!(" -> {}", ret.source));
        }

        if !contexts.is_empty() {
            source.push_str(&format!(" using [{}]", contexts.join(", ")));
        }

        source.push_str(" {\n");

        // Add parameters to context
        let old_vars = ctx.variables.clone();
        ctx.variables.extend(param_names);

        // Generate function body
        let body = self.generate_function_body(rng, ctx, return_type.as_ref());

        for stmt in &body.statements {
            source.push_str(&format!("    {}\n", stmt));
        }

        if let Some(ref trailing) = body.trailing_expr {
            source.push_str(&format!("    {}\n", trailing));
        }

        source.push('}');

        // Restore context
        ctx.variables = old_vars;

        FunctionDef {
            source,
            name: name.to_string(),
            params,
            return_type,
            body,
            is_async,
            is_pub,
            contexts,
        }
    }

    /// Generate function body
    fn generate_function_body<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        return_type: Option<&ArbitraryType>,
    ) -> FunctionBody {
        let num_stmts = rng.random_range(1..=self.config.complexity.max_statements.min(10));
        let mut statements = Vec::new();

        for i in 0..num_stmts - 1 {
            let stmt = self.generate_statement(rng, ctx, i);
            statements.push(stmt);
        }

        let trailing_expr = if let Some(ret_ty) = return_type {
            Some(self.generate_typed_expr_string(rng, ctx, ret_ty))
        } else {
            None
        };

        FunctionBody {
            statements,
            trailing_expr,
        }
    }

    /// Generate a statement
    fn generate_statement<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        idx: usize,
    ) -> String {
        match rng.random_range(0..6) {
            0..=2 => self.generate_let_statement(rng, ctx),
            3 => self.generate_if_statement(rng, ctx),
            4 => self.generate_for_statement(rng, ctx),
            _ => {
                let expr = self.expr_gen.generate(rng);
                format!("{};", expr.source)
            }
        }
    }

    /// Generate a let statement
    fn generate_let_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let var_name = ctx.fresh_variable();
        let var_type = self.type_gen.generate_primitive(rng);
        let expr = self.generate_typed_expr_string(rng, ctx, &var_type);

        let is_mut = rng.random_bool(0.3);
        let has_annotation = rng.random_bool(0.5);

        let mut stmt = String::from("let ");
        if is_mut {
            stmt.push_str("mut ");
        }
        stmt.push_str(&var_name);

        if has_annotation {
            stmt.push_str(&format!(": {}", var_type.source));
        }

        stmt.push_str(&format!(" = {};", expr));

        ctx.variables.push(var_name);
        stmt
    }

    /// Generate an if statement
    fn generate_if_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let cond = self.expr_gen.generate_typed(rng, "Bool");
        let inner_stmt = self.generate_let_statement(rng, ctx);

        let mut result = format!("if {} {{\n        {}\n    }}", cond.source, inner_stmt);

        if rng.random_bool(0.4) {
            let else_stmt = self.generate_let_statement(rng, ctx);
            result.push_str(&format!(" else {{\n        {}\n    }}", else_stmt));
        }

        result
    }

    /// Generate a for statement
    fn generate_for_statement<R: Rng>(&self, rng: &mut R, ctx: &mut GenerationContext) -> String {
        let iter_var = ctx.fresh_variable();
        let start = rng.random_range(0..10);
        let end = rng.random_range(start + 1..start + 10);

        let inner_ctx_var = ctx.variables.clone();
        ctx.variables.push(iter_var.clone());

        let inner_stmt = self.generate_let_statement(rng, ctx);

        ctx.variables = inner_ctx_var;

        format!(
            "for {} in {}..{} {{\n        {}\n    }}",
            iter_var, start, end, inner_stmt
        )
    }

    /// Generate a typed expression as a string
    fn generate_typed_expr_string<R: Rng>(
        &self,
        rng: &mut R,
        ctx: &mut GenerationContext,
        target_type: &ArbitraryType,
    ) -> String {
        match &target_type.kind {
            TypeKind::Int => self.expr_gen.generate_typed(rng, "Int").source,
            TypeKind::Float => self.expr_gen.generate_typed(rng, "Float").source,
            TypeKind::Bool => self.expr_gen.generate_typed(rng, "Bool").source,
            TypeKind::Text => self.expr_gen.generate_typed(rng, "Text").source,
            TypeKind::Unit => "()".to_string(),
            TypeKind::List(_) => {
                let len = rng.random_range(0..5);
                let elements: Vec<String> = (0..len)
                    .map(|_| rng.random_range(0..100).to_string())
                    .collect();
                format!("[{}]", elements.join(", "))
            }
            TypeKind::Maybe(_) => {
                if rng.random_bool(0.3) {
                    "None".to_string()
                } else {
                    format!("Some({})", rng.random_range(0..100))
                }
            }
            TypeKind::Tuple(types) => {
                let elements: Vec<String> = types
                    .iter()
                    .map(|t| self.generate_typed_expr_string(rng, ctx, t))
                    .collect();
                format!("({})", elements.join(", "))
            }
            _ => self.expr_gen.generate(rng).source,
        }
    }
}

/// Context for tracking generation state
#[derive(Debug, Clone)]
struct GenerationContext {
    variables: Vec<String>,
    functions: Vec<String>,
    types: Vec<String>,
    var_counter: usize,
}

impl GenerationContext {
    fn new() -> Self {
        Self {
            variables: Vec::new(),
            functions: vec!["print".to_string(), "len".to_string()],
            types: vec![
                "Int".to_string(),
                "Float".to_string(),
                "Bool".to_string(),
                "Text".to_string(),
            ],
            var_counter: 0,
        }
    }

    fn fresh_variable(&mut self) -> String {
        self.var_counter += 1;
        format!("v_{}", self.var_counter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generate_program() {
        let config = GeneratorConfig::default();
        let generator = ProgramGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.source.is_empty());
        assert!(program.source.contains("fn main()"));
        assert!(program.source.contains("use verum_std"));
    }

    #[test]
    fn test_program_has_structure() {
        let config = GeneratorConfig::default();
        let generator = ProgramGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);

        // Should have at least one function (main)
        assert!(!program.structure.functions.is_empty());

        // Main function should exist
        assert!(program.structure.functions.iter().any(|f| f.name == "main"));
    }

    #[test]
    fn test_shrinking() {
        let config = GeneratorConfig::builder()
            .max_functions(3)
            .max_types(2)
            .max_statements(5)
            .build();
        let generator = ProgramGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        let shrunk = program.shrink();

        // Shrunk programs should be simpler
        for s in shrunk {
            assert!(s.complexity <= program.complexity);
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GeneratorConfig::default();
        let generator = ProgramGenerator::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let prog1 = generator.generate(&mut rng1);
        let prog2 = generator.generate(&mut rng2);

        assert_eq!(prog1.source, prog2.source);
    }

    #[test]
    fn test_minimal_config() {
        let config = GeneratorConfig::minimal();
        let generator = ProgramGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.source.is_empty());
        assert!(program.source.contains("fn main()"));

        // Minimal config should produce smaller programs
        assert!(program.structure.types.is_empty());
    }

    #[test]
    fn test_function_body_shrinking() {
        let body = FunctionBody {
            statements: vec![
                "let x = 1;".to_string(),
                "let y = 2;".to_string(),
                "let z = 3;".to_string(),
            ],
            trailing_expr: Some("x + y + z".to_string()),
        };

        let shrunk = body.shrink();

        // Should have shrunk versions with fewer statements
        assert!(!shrunk.is_empty());
        assert!(shrunk.iter().any(|s| s.statements.len() < 3));
    }
}
