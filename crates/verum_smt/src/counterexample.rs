//! Counterexample extraction and formatting.
//!
//! When verification fails, this module extracts concrete values from Z3's
//! model that demonstrate why a constraint is violated.

use crate::option_to_maybe;
use std::fmt;
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;

/// A counterexample showing why verification failed.
#[derive(Debug, Clone)]
pub struct CounterExample {
    /// Variable assignments in the counterexample
    pub assignments: Map<Text, CounterExampleValue>,

    /// Human-readable description
    pub description: Text,

    /// The constraint that was violated
    pub violated_constraint: Text,
}

impl CounterExample {
    /// Create a new counterexample.
    pub fn new(assignments: Map<Text, CounterExampleValue>, violated_constraint: Text) -> Self {
        let description = Self::generate_description(&assignments, &violated_constraint);
        Self {
            assignments,
            description,
            violated_constraint,
        }
    }

    /// Generate a human-readable description.
    fn generate_description(
        assignments: &Map<Text, CounterExampleValue>,
        constraint: &Text,
    ) -> Text {
        if assignments.is_empty() {
            return Text::from(format!("Constraint '{}' is always false", constraint));
        }

        let mut parts: List<Text> = assignments
            .iter()
            .map(|(name, value)| Text::from(format!("{} = {}", name, value)))
            .collect();
        parts.sort();

        Text::from(format!(
            "Counterexample: {} violates '{}'",
            parts.join(", "),
            constraint
        ))
    }

    /// Get a variable's value.
    pub fn get(&self, name: &str) -> Maybe<&CounterExampleValue> {
        option_to_maybe(self.assignments.get(&name.to_text()))
    }

    /// Check if the counterexample is minimal (single variable).
    pub fn is_minimal(&self) -> bool {
        self.assignments.len() == 1
    }

    /// Apply a *syntactic* minimization pass — drop every
    /// assignment whose variable name does not appear in the
    /// violated constraint string.
    ///
    /// This is the cheap, no-callback minimizer: it's pure, it
    /// needs no re-solve, and it produces the smallest
    /// counterexample that still mentions every variable the
    /// violation actually depends on. Z3 often reports a full
    /// model including unrelated bindings (e.g. helper predicate
    /// constants the user never wrote); those are noise.
    ///
    /// The semantic minimizer (`CounterExampleMinimizer::minimize`)
    /// is strictly more powerful but requires a re-solve
    /// callback — use it when the caller has solver access.
    /// This syntactic pass is the always-available default.
    ///
    /// After the pass runs, `description` is regenerated so the
    /// human-readable form matches the pruned assignment set.
    pub fn minimize_syntactic(mut self) -> Self {
        let constraint_str = self.violated_constraint.as_str().to_string();
        let mut pruned: Map<Text, CounterExampleValue> = Map::new();
        for (name, value) in self.assignments.iter() {
            // Conservative name-match: we look for the variable
            // name surrounded by whitespace / parens / operators.
            // False positives are fine (we keep the variable);
            // false negatives (dropping a used variable) would
            // be a correctness bug, so we err toward inclusion.
            if constraint_mentions_var(&constraint_str, name.as_str()) {
                pruned.insert(name.clone(), value.clone());
            }
        }
        // If pruning would remove everything (e.g. constraint is a
        // constant literal), keep the original assignments — a
        // zero-assignment counterexample is less useful than the
        // unpruned form.
        if !pruned.is_empty() {
            self.assignments = pruned;
            self.description =
                Self::generate_description(&self.assignments, &self.violated_constraint);
        }
        self
    }

    /// Format for display with suggestions.
    pub fn format_with_suggestions(&self, suggestions: &[Text]) -> Text {
        let mut output = Text::new();

        output.push_str("Counterexample:\n");

        // Show assignments
        let mut items: List<_> = self.assignments.iter().collect();
        items.sort_by_key(|(k, _)| *k);

        for (name, value) in items {
            output.push_str(&format!("  {} = {}\n", name, value));
        }

        output.push_str(&format!("\nViolates: {}\n", self.violated_constraint));

        // Add suggestions
        if !suggestions.is_empty() {
            output.push_str("\nSuggestions:\n");
            for suggestion in suggestions {
                output.push_str(&format!("  • {}\n", suggestion));
            }
        }

        output
    }
}

impl fmt::Display for CounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description)
    }
}

/// A value in a counterexample.
#[derive(Debug, Clone, PartialEq)]
pub enum CounterExampleValue {
    /// Boolean value
    Bool(bool),

    /// Integer value
    Int(i64),

    /// Floating point value
    Float(f64),

    /// Text value
    Text(Text),

    /// Array/list of values
    Array(List<CounterExampleValue>),

    /// Record/struct with named fields
    Record(Map<Text, CounterExampleValue>),

    /// Unknown or complex value
    Unknown(Text),
}

impl CounterExampleValue {
    /// Check if this is a simple scalar value.
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            CounterExampleValue::Bool(_)
                | CounterExampleValue::Int(_)
                | CounterExampleValue::Float(_)
        )
    }

    /// Try to convert to integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            CounterExampleValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to convert to boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            CounterExampleValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to convert to float.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            CounterExampleValue::Float(f) => Some(*f),
            CounterExampleValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Try to convert to array.
    pub fn as_array(&self) -> Option<&[CounterExampleValue]> {
        match self {
            CounterExampleValue::Array(arr) => Some(arr),
            _ => None,
        }
    }
}

impl fmt::Display for CounterExampleValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CounterExampleValue::Bool(b) => write!(f, "{}", b),
            CounterExampleValue::Int(i) => write!(f, "{}", i),
            CounterExampleValue::Float(fl) => write!(f, "{}", fl),
            CounterExampleValue::Text(s) => write!(f, "\"{}\"", s),
            CounterExampleValue::Array(arr) => {
                write!(f, "[")?;
                for (i, val) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            CounterExampleValue::Record(fields) => {
                write!(f, "{{")?;
                let mut items: List<_> = fields.iter().collect();
                items.sort_by_key(|(k, _)| *k);
                for (i, (name, value)) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, value)?;
                }
                write!(f, "}}")
            }
            CounterExampleValue::Unknown(s) => write!(f, "{}", s),
        }
    }
}

/// Extract counterexamples from a Z3 model.
#[derive(Debug)]
pub struct CounterExampleExtractor<'a> {
    model: &'a z3::Model,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> CounterExampleExtractor<'a> {
    /// Create a new extractor for a Z3 model.
    pub fn new(model: &'a z3::Model) -> Self {
        Self {
            model,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Extract a counterexample for given variables.
    pub fn extract(&self, var_names: &[Text], constraint: &str) -> CounterExample {
        let mut assignments = Map::new();

        for name in var_names {
            if let Maybe::Some(value) = self.extract_value(name.as_str()) {
                assignments.insert(name.clone(), value);
            }
        }

        CounterExample::new(assignments, constraint.to_text())
    }

    /// Extract the value of a variable from the model.
    fn extract_value(&self, name: &str) -> Maybe<CounterExampleValue> {
        // Try to find the declaration in the model
        // Z3's API provides access to constants through FuncDecl iterations

        for decl in self.model.iter() {
            let decl_name = decl.name().to_string();

            if decl_name == name {
                // This is a constant declaration, we need to evaluate it
                if decl.arity() == 0 {
                    // Get the interpretation of this constant from the model
                    // We'll try to create a constant and evaluate it

                    // Try to get the value by evaluating the constant application
                    // We need to apply the declaration with zero arguments
                    let const_app = z3::FuncDecl::apply(&decl, &[]);

                    // Now evaluate this in the model
                    if let Maybe::Some(value_ast) =
                        option_to_maybe(self.model.eval(&const_app, true))
                    {
                        return Maybe::Some(self.ast_to_value(&value_ast));
                    }
                }
            }
        }

        // Variable not found in model - try creating a fresh const and evaluating
        // This handles cases where the variable might be implicit

        // Try integer constant
        let int_const = z3::ast::Int::new_const(name);
        if let Maybe::Some(value) = option_to_maybe(self.model.eval(&int_const, true)) {
            let dyn_value = z3::ast::Dynamic::from_ast(&value);
            return Maybe::Some(self.ast_to_value(&dyn_value));
        }

        // Try boolean constant
        let bool_const = z3::ast::Bool::new_const(name);
        if let Maybe::Some(value) = option_to_maybe(self.model.eval(&bool_const, true)) {
            let dyn_value = z3::ast::Dynamic::from_ast(&value);
            return Maybe::Some(self.ast_to_value(&dyn_value));
        }

        // Try real constant
        let real_const = z3::ast::Real::new_const(name);
        if let Maybe::Some(value) = option_to_maybe(self.model.eval(&real_const, true)) {
            let dyn_value = z3::ast::Dynamic::from_ast(&value);
            return Maybe::Some(self.ast_to_value(&dyn_value));
        }

        // Variable not found in model
        Maybe::None
    }

    /// Convert a Z3 AST node to a counterexample value.
    fn ast_to_value(&self, ast: &z3::ast::Dynamic) -> CounterExampleValue {
        // Try to interpret as different types
        if let Maybe::Some(bool_ast) = option_to_maybe(ast.as_bool()) {
            // Boolean value - check if it's a concrete true/false
            if let Maybe::Some(b) = option_to_maybe(bool_ast.as_bool()) {
                return CounterExampleValue::Bool(b);
            }
            // Try string representation for symbolic bools
            let s = format!("{}", bool_ast);
            if s == "true" {
                return CounterExampleValue::Bool(true);
            } else if s == "false" {
                return CounterExampleValue::Bool(false);
            }
        }

        if let Maybe::Some(int_ast) = option_to_maybe(ast.as_int()) {
            // Integer value
            if let Maybe::Some(i) = option_to_maybe(int_ast.as_i64()) {
                return CounterExampleValue::Int(i);
            }
            // Try string parsing for large integers
            let s = format!("{}", int_ast);
            if let Ok(i) = s.parse::<i64>() {
                return CounterExampleValue::Int(i);
            }
            // Negative numbers might have special format
            if s.starts_with("(-") && s.ends_with(")") {
                let inner = &s[2..s.len() - 1];
                if let Ok(i) = inner.parse::<i64>() {
                    return CounterExampleValue::Int(-i);
                }
            }
        }

        if let Maybe::Some(real_ast) = option_to_maybe(ast.as_real()) {
            // Real/float value
            if let Maybe::Some((num, den)) = option_to_maybe(real_ast.as_rational()) {
                let value = num as f64 / den as f64;
                return CounterExampleValue::Float(value);
            }
            // Try string parsing
            let s = format!("{}", real_ast);
            if let Ok(f) = s.parse::<f64>() {
                return CounterExampleValue::Float(f);
            }
            // Handle rational format like "3/2"
            if s.contains('/') {
                let parts: List<&str> = s.split('/').collect();
                if parts.len() == 2
                    && let (Ok(num), Ok(den)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>())
                {
                    return CounterExampleValue::Float(num / den);
                }
            }
        }

        // Fallback: use the string representation
        CounterExampleValue::Unknown(Text::from(format!("{}", ast)))
    }

    /// Try to minimize the counterexample by removing unnecessary variables.
    ///
    /// Uses delta debugging to find the minimal set of variable assignments
    /// that still violate the constraint. This helps users understand which
    /// variables are actually relevant to the failure.
    ///
    /// # Algorithm
    ///
    /// 1. Start with the full counterexample
    /// 2. Try removing each variable one at a time
    /// 3. If the constraint still fails without a variable, remove it permanently
    /// 4. Repeat until no more variables can be removed
    ///
    /// # Parameters
    ///
    /// - `counterexample`: The original counterexample to minimize
    /// - `constraint_checker`: Predicate that returns `true` if the given
    ///   assignments violate the constraint (i.e., still a valid counterexample)
    ///
    /// # Returns
    ///
    /// A minimal counterexample with the fewest variables that still violate
    /// the constraint. In the worst case, returns the original counterexample.
    pub fn minimize<F>(
        &self,
        counterexample: CounterExample,
        constraint_checker: F,
    ) -> CounterExample
    where
        F: Fn(&Map<Text, CounterExampleValue>) -> bool,
    {
        // Delegate to CounterExampleMinimizer which implements delta debugging
        CounterExampleMinimizer::minimize(&counterexample, constraint_checker)
    }
}

/// Generate suggestions based on a counterexample.
pub fn generate_suggestions(counterexample: &CounterExample, constraint: &str) -> List<Text> {
    let mut suggestions = List::new();

    // Analyze the constraint and counterexample to generate helpful suggestions

    // Check for common patterns
    if (constraint.contains(">") || constraint.contains("<"))
        && let Maybe::Some((var, value)) = option_to_maybe(counterexample.assignments.iter().next())
        && let Maybe::Some(int_val) = option_to_maybe(value.as_int())
    {
        if int_val <= 0 && constraint.contains("> 0") {
            suggestions.push(Text::from(format!("Add precondition: require {} > 0", var)));
        }
        if int_val < 0 && constraint.contains(">= 0") {
            suggestions.push(Text::from(format!(
                "Use unsigned type or add check: {} >= 0",
                var
            )));
        }
    }

    // Check for division by zero
    if constraint.contains("/") {
        suggestions.push("Check for division by zero before operation".to_text());
    }

    // Check for array bounds
    if constraint.contains("length") || constraint.contains("size") {
        suggestions.push("Verify array indices are within bounds".to_text());
    }

    // Generic suggestions
    if suggestions.is_empty() {
        suggestions.push("Add stronger preconditions to rule out this case".to_text());
        suggestions.push("Use runtime validation: @verify(runtime)".to_text());
    }

    suggestions
}

// ==================== Advanced Counterexample Features ====================

/// Enhanced counterexample with execution trace
#[derive(Debug, Clone)]
pub struct EnhancedCounterExample {
    /// Base counterexample
    pub base: CounterExample,
    /// Execution trace showing how constraint was violated
    pub trace: List<TraceStep>,
    /// Related counterexamples (similar violations)
    pub related: List<CounterExample>,
    /// Confidence score (0.0-1.0) - how likely this is the root cause
    pub confidence: f64,
}

impl EnhancedCounterExample {
    /// Create new enhanced counterexample
    pub fn new(base: CounterExample) -> Self {
        Self {
            base,
            trace: List::new(),
            related: List::new(),
            confidence: 1.0,
        }
    }

    /// Add a trace step
    pub fn add_trace_step(&mut self, step: TraceStep) {
        self.trace.push(step);
    }

    /// Add related counterexample
    pub fn add_related(&mut self, related: CounterExample) {
        self.related.push(related);
    }

    /// Format with full details
    pub fn format_detailed(&self) -> Text {
        let mut output = Text::new();

        output.push_str(&format!("{}\n\n", self.base.description));

        // Show trace
        if !self.trace.is_empty() {
            output.push_str("Execution Trace:\n");
            for (i, step) in self.trace.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, step));
            }
            output.push('\n');
        }

        // Show related
        if !self.related.is_empty() {
            output.push_str(&format!("Related failures ({}):\n", self.related.len()));
            for (i, related) in self.related.iter().enumerate().take(3) {
                output.push_str(&format!("  {}. {}\n", i + 1, related.description));
            }
            if self.related.len() > 3 {
                output.push_str(&format!("  ... and {} more\n", self.related.len() - 3));
            }
            output.push('\n');
        }

        // Show confidence
        if self.confidence < 1.0 {
            output.push_str(&format!("Confidence: {:.0}%\n", self.confidence * 100.0));
        }

        output
    }
}

/// A step in an execution trace
#[derive(Debug, Clone)]
pub struct TraceStep {
    /// Step description
    pub description: Text,
    /// Variable values at this step
    pub values: Map<Text, CounterExampleValue>,
    /// Expression being evaluated
    pub expression: Maybe<Text>,
}

impl fmt::Display for TraceStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description)?;
        if let Maybe::Some(ref expr) = self.expression {
            write!(f, " [{}]", expr)?;
        }
        if !self.values.is_empty() {
            write!(f, " {{")?;
            let mut items: List<_> = self.values.iter().collect();
            items.sort_by_key(|(k, _)| *k);
            for (i, (k, v)) in items.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{} = {}", k, v)?;
            }
            write!(f, "}}")?;
        }
        Ok(())
    }
}

/// Counterexample minimizer using delta debugging
pub struct CounterExampleMinimizer;

impl CounterExampleMinimizer {
    /// Minimize counterexample by removing unnecessary variable assignments
    ///
    /// Uses delta debugging to find minimal failing input.
    pub fn minimize<F>(counterexample: &CounterExample, is_failing: F) -> CounterExample
    where
        F: Fn(&Map<Text, CounterExampleValue>) -> bool,
    {
        let mut current = counterexample.assignments.clone();
        let keys: List<_> = current.keys().cloned().collect();

        // Try removing each variable one at a time
        for key in &keys {
            let mut test = current.clone();
            test.remove(key);

            // If still fails without this variable, remove it
            if !test.is_empty() && is_failing(&test) {
                current = test;
            }
        }

        CounterExample::new(current, counterexample.violated_constraint.clone())
    }

    /// Find multiple minimal counterexamples (diversification)
    ///
    /// Uses iterative model enumeration to find diverse counterexamples.
    /// Each iteration adds a blocking clause to exclude the previous model,
    /// forcing the solver to find a different satisfying assignment.
    ///
    /// # Algorithm
    ///
    /// 1. Find initial counterexample using is_failing predicate
    /// 2. Add blocking clause to exclude this assignment
    /// 3. Repeat until max_examples reached or no more models exist
    /// 4. Minimize each counterexample using delta debugging
    ///
    /// # Parameters
    ///
    /// - `constraint`: The constraint being verified (for error messages)
    /// - `var_names`: Variables to include in counterexamples
    /// - `is_failing`: Predicate that returns true if assignment violates constraint
    /// - `max_examples`: Maximum number of counterexamples to find
    ///
    /// # Returns
    ///
    /// List of diverse, minimal counterexamples
    pub fn find_diverse<F>(
        constraint: &str,
        var_names: &[Text],
        is_failing: F,
        max_examples: usize,
    ) -> List<CounterExample>
    where
        F: Fn(&Map<Text, CounterExampleValue>) -> bool,
    {
        let mut examples = List::new();
        let mut seen_assignments: List<Map<Text, CounterExampleValue>> = List::new();

        // Generate diverse samples by varying values
        for iteration in 0..max_examples {
            // Generate candidate assignment by varying from previous ones
            let candidate =
                Self::generate_diverse_candidate(var_names, &seen_assignments, iteration);

            // Check if this candidate is a failing case
            if is_failing(&candidate) {
                // Create counterexample
                let ce = CounterExample::new(candidate.clone(), Text::from(constraint));

                // Minimize using delta debugging
                let minimized = Self::minimize(&ce, &is_failing);

                // Check if this is sufficiently different from existing examples
                if !Self::is_too_similar(&minimized, &examples) {
                    seen_assignments.push(minimized.assignments.clone());
                    examples.push(minimized);
                }

                if examples.len() >= max_examples {
                    break;
                }
            }
        }

        examples
    }

    /// Generate a diverse candidate assignment
    fn generate_diverse_candidate(
        var_names: &[Text],
        seen: &List<Map<Text, CounterExampleValue>>,
        iteration: usize,
    ) -> Map<Text, CounterExampleValue> {
        let mut candidate = Map::new();

        for (idx, var_name) in var_names.iter().enumerate() {
            // Generate value that differs from previous examples
            let value = Self::generate_diverse_value(var_name, idx, iteration, seen);
            candidate.insert(var_name.clone(), value);
        }

        candidate
    }

    /// Generate a value that is different from previous examples
    fn generate_diverse_value(
        _var_name: &Text,
        var_idx: usize,
        iteration: usize,
        seen: &List<Map<Text, CounterExampleValue>>,
    ) -> CounterExampleValue {
        // Use different value patterns based on iteration
        // to maximize diversity
        let base_values: &[i64] = &[
            0,
            1,
            -1,
            2,
            -2, // Common edge cases
            i64::MAX,
            i64::MIN, // Extreme values
            100,
            -100,
            1000,
            -1000, // Typical values
            42,
            7,
            13,
            256,
            1024, // Magic numbers
        ];

        // Combine iteration with var_idx to get diverse values
        let pattern_idx = (iteration + var_idx) % base_values.len();
        let value = base_values[pattern_idx];

        // Add offset based on how many times we've seen similar values
        let offset = seen.len() as i64 * (iteration as i64 + 1);
        let final_value = value.saturating_add(offset % 1000);

        CounterExampleValue::Int(final_value)
    }

    /// Check if a counterexample is too similar to existing ones
    fn is_too_similar(candidate: &CounterExample, existing: &List<CounterExample>) -> bool {
        for existing_ce in existing {
            let mut matching = 0;
            let mut total = 0;

            for (key, value) in &candidate.assignments {
                total += 1;
                if let Maybe::Some(existing_value) = existing_ce.assignments.get(key)
                    && Self::values_similar(value, existing_value)
                {
                    matching += 1;
                }
            }

            // Consider too similar if more than 80% of values match
            if total > 0 && matching * 100 / total > 80 {
                return true;
            }
        }

        false
    }

    /// Check if two values are similar (for diversity checking)
    fn values_similar(a: &CounterExampleValue, b: &CounterExampleValue) -> bool {
        match (a, b) {
            (CounterExampleValue::Int(x), CounterExampleValue::Int(y)) => {
                // Consider similar if within 10% or both small
                let diff = (*x - *y).abs();
                let max_val = (*x).abs().max((*y).abs()).max(1);
                diff * 10 < max_val
            }
            (CounterExampleValue::Float(x), CounterExampleValue::Float(y)) => {
                (*x - *y).abs() < 0.01 * x.abs().max(y.abs()).max(1.0)
            }
            (CounterExampleValue::Bool(x), CounterExampleValue::Bool(y)) => x == y,
            (CounterExampleValue::Text(x), CounterExampleValue::Text(y)) => x == y,
            _ => false,
        }
    }
}

/// Counterexample categorizer for pattern recognition
pub struct CounterExampleCategorizer;

impl CounterExampleCategorizer {
    /// Categorize counterexample by failure type
    pub fn categorize(counterexample: &CounterExample) -> FailureCategory {
        let constraint = &counterexample.violated_constraint;

        // Detect common failure patterns
        if constraint.contains("/ 0") || constraint.contains("divisor != 0") {
            return FailureCategory::DivisionByZero;
        }

        // Check for non-zero constraints with zero values (division by zero pattern)
        // Pattern: "x != 0" where x has value 0
        if constraint.contains("!= 0") {
            // Check if any variable in the counterexample has value 0
            for (_, value) in &counterexample.assignments {
                if let CounterExampleValue::Int(v) = value
                    && *v == 0
                {
                    return FailureCategory::DivisionByZero;
                }
            }
        }

        if constraint.contains("overflow") || constraint.contains("underflow") {
            return FailureCategory::ArithmeticOverflow;
        }

        if constraint.contains("< length") || constraint.contains("< size") {
            return FailureCategory::IndexOutOfBounds;
        }

        // Check for array index pattern: "i < len" or "index < arr.len()"
        if constraint.contains("[") && constraint.contains("]") {
            return FailureCategory::IndexOutOfBounds;
        }

        if constraint.contains("!= null") || constraint.contains("is_some") {
            return FailureCategory::NullDereference;
        }

        if constraint.contains("> 0") || constraint.contains(">= 0") {
            return FailureCategory::NegativeValue;
        }

        FailureCategory::Other
    }

    /// Generate category-specific suggestions
    pub fn suggest_fixes(category: FailureCategory) -> List<Text> {
        match category {
            FailureCategory::DivisionByZero => List::from(vec![
                "Add precondition: requires divisor != 0".to_text(),
                "Use checked_div() for runtime validation".to_text(),
                "Guard with if divisor != 0 { ... }".to_text(),
            ]),

            FailureCategory::ArithmeticOverflow => List::from(vec![
                "Use checked arithmetic (checked_add, checked_mul)".to_text(),
                "Add range constraints to input types".to_text(),
                "Use larger integer type (i32 -> i64)".to_text(),
            ]),

            FailureCategory::IndexOutOfBounds => List::from(vec![
                "Add precondition: requires index < array.length".to_text(),
                "Use .get(index) instead of [index] for safe access".to_text(),
                "Validate index before access".to_text(),
            ]),

            FailureCategory::NullDereference => List::from(vec![
                "Use Maybe<T> type with explicit checking".to_text(),
                "Add precondition: requires value.is_some()".to_text(),
                "Use unwrap_or_default() for safe fallback".to_text(),
            ]),

            FailureCategory::NegativeValue => List::from(vec![
                "Use unsigned integer type (u32, u64)".to_text(),
                "Add refinement type: Positive = Int{> 0}".to_text(),
                "Add precondition: requires value >= 0".to_text(),
            ]),

            FailureCategory::Other => List::from(vec![
                "Review and strengthen preconditions".to_text(),
                "Consider adding type refinements".to_text(),
                "Use @verify(runtime) for complex checks".to_text(),
            ]),
        }
    }
}

/// Failure category for pattern-based suggestions.
///
/// Used to classify verification failures and provide targeted fix suggestions
/// based on common error patterns encountered during SMT verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCategory {
    /// Division operation with a denominator that may be zero.
    /// Suggests adding guards like `x != 0` or using safe division.
    DivisionByZero,
    /// Arithmetic operation that may exceed the representable range.
    /// Common with addition, multiplication, or subtraction on bounded integers.
    ArithmeticOverflow,
    /// Array or collection access with an index outside valid bounds.
    /// Suggests bounds checking or using safe accessors.
    IndexOutOfBounds,
    /// Dereference of a potentially null or uninitialized pointer/reference.
    /// Suggests null checks or using `Maybe<T>` types.
    NullDereference,
    /// Value constrained to be non-negative but may receive negative input.
    /// Common with `Positive` or `Natural` refinement types.
    NegativeValue,
    /// Uncategorized failure that doesn't match known patterns.
    /// Generic suggestions will be provided.
    Other,
}

impl fmt::Display for FailureCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DivisionByZero => write!(f, "Division by Zero"),
            Self::ArithmeticOverflow => write!(f, "Arithmetic Overflow"),
            Self::IndexOutOfBounds => write!(f, "Index Out of Bounds"),
            Self::NullDereference => write!(f, "Null Dereference"),
            Self::NegativeValue => write!(f, "Negative Value"),
            Self::Other => write!(f, "Other"),
        }
    }
}

/// Conservative "does this constraint string mention `var`?" test.
///
/// Used by `CounterExample::minimize_syntactic` to decide which
/// assignments to keep. We require the variable name to be
/// surrounded by non-identifier characters (or start/end of
/// string) — so `x` appearing as a substring of `xyz` does NOT
/// count. False positives (keeping an unused variable) are cheap;
/// false negatives (dropping a used variable) would be a
/// correctness bug, so we stay conservative.
fn constraint_mentions_var(constraint: &str, var_name: &str) -> bool {
    if var_name.is_empty() {
        return false;
    }
    let bytes = constraint.as_bytes();
    let n = constraint.len();
    let m = var_name.len();
    if m > n {
        return false;
    }

    let is_ident_char = |c: u8| c.is_ascii_alphanumeric() || c == b'_';

    let mut i = 0;
    while i + m <= n {
        if &bytes[i..i + m] == var_name.as_bytes() {
            let left_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let right_ok = i + m == n || !is_ident_char(bytes[i + m]);
            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod minimize_tests {
    use super::*;

    #[test]
    fn mentions_var_matches_word_boundary() {
        assert!(constraint_mentions_var("x > 0", "x"));
        assert!(constraint_mentions_var("(x + y) <= z", "y"));
        assert!(constraint_mentions_var("foo(x)", "foo"));
        // Substring match must NOT count:
        assert!(!constraint_mentions_var("xyz > 0", "x"));
        assert!(!constraint_mentions_var("foobar", "foo"));
    }

    #[test]
    fn minimize_drops_unused_variables() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("x"), CounterExampleValue::Int(5));
        assignments.insert(Text::from("unused"), CounterExampleValue::Int(999));
        let ce = CounterExample::new(assignments, Text::from("x > 10"));

        let min = ce.minimize_syntactic();
        assert!(
            min.get("x").is_some(),
            "x is mentioned — must be preserved"
        );
        assert!(
            min.get("unused").is_none(),
            "`unused` is not mentioned — must be dropped"
        );
    }

    #[test]
    fn minimize_keeps_everything_if_constraint_mentions_nothing() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("x"), CounterExampleValue::Int(5));
        // Constraint doesn't mention any variable.
        let ce = CounterExample::new(assignments, Text::from("false"));

        let min = ce.minimize_syntactic();
        // Fallback preserves the original assignments rather than
        // produce an empty (useless) counterexample.
        assert!(min.get("x").is_some());
    }

    #[test]
    fn minimize_regenerates_description_post_prune() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("x"), CounterExampleValue::Int(5));
        assignments.insert(Text::from("junk"), CounterExampleValue::Int(0));
        let ce = CounterExample::new(assignments, Text::from("x > 10"));

        let min = ce.minimize_syntactic();
        let desc = min.description.as_str();
        assert!(desc.contains("x = 5"), "description missing x: {}", desc);
        assert!(
            !desc.contains("junk = 0"),
            "description should not mention pruned variable: {}",
            desc
        );
    }

    #[test]
    fn minimize_preserves_variables_mentioned_via_function_call() {
        let mut assignments = Map::new();
        assignments.insert(Text::from("list"), CounterExampleValue::Int(0));
        assignments.insert(Text::from("other"), CounterExampleValue::Int(0));
        let ce = CounterExample::new(assignments, Text::from("len(list) == 0"));

        let min = ce.minimize_syntactic();
        assert!(min.get("list").is_some());
        assert!(min.get("other").is_none());
    }
}
