//! Schema Validation Intrinsics (Tier 1 - Requires MetaTypes)
//!
//! Provides compile-time code schema validation builtins for meta-programming.
//! Schemas define structural constraints on code (functions, types, expressions)
//! and can validate token streams against those constraints.
//!
//! ## Schema Builder Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `schema_function()` | `() -> Schema` | Start building a function schema |
//! | `schema_type()` | `() -> Schema` | Start building a type schema |
//!
//! ## Validation Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `schema_validate(code, schema)` | `(Text, Map) -> List<Map>` | Validate code against schema |
//! | `schema_is_function(code)` | `(Text) -> Bool` | Quick check: is code a function? |
//! | `schema_is_type(code)` | `(Text) -> Bool` | Quick check: is code a type? |
//! | `schema_is_expression(code)` | `(Text) -> Bool` | Quick check: is code an expression? |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaTypes]` context.

use verum_common::{List, OrderedMap, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register schema validation builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Schema Builder Functions (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("schema_function"),
        BuiltinInfo::meta_types(
            meta_schema_function,
            "Create a function schema builder",
            "() -> Schema",
        ),
    );
    map.insert(
        Text::from("schema_type"),
        BuiltinInfo::meta_types(
            meta_schema_type,
            "Create a type schema builder",
            "() -> Schema",
        ),
    );

    // ========================================================================
    // Validation Functions (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("schema_validate"),
        BuiltinInfo::meta_types(
            meta_schema_validate,
            "Validate code against a schema, returning list of errors",
            "(Text, Schema) -> List<Map>",
        ),
    );
    map.insert(
        Text::from("schema_is_function"),
        BuiltinInfo::meta_types(
            meta_schema_is_function,
            "Quick check if code represents a function definition",
            "(Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("schema_is_type"),
        BuiltinInfo::meta_types(
            meta_schema_is_type,
            "Quick check if code represents a type definition",
            "(Text) -> Bool",
        ),
    );
    map.insert(
        Text::from("schema_is_expression"),
        BuiltinInfo::meta_types(
            meta_schema_is_expression,
            "Quick check if code represents an expression",
            "(Text) -> Bool",
        ),
    );
}

// ============================================================================
// Schema Constants
// ============================================================================

/// Schema kind key in the schema map
const SCHEMA_KIND_KEY: &str = "__schema_kind";
/// Schema kind value for function schemas
const SCHEMA_KIND_FUNCTION: &str = "function";
/// Schema kind value for type schemas
const SCHEMA_KIND_TYPE: &str = "type";

/// Constraint keys
const CONSTRAINT_MIN_PARAMS: &str = "min_params";
const CONSTRAINT_MAX_PARAMS: &str = "max_params";
const CONSTRAINT_RETURN_TYPE: &str = "return_type";
const CONSTRAINT_IS_ASYNC: &str = "is_async";
const CONSTRAINT_IS_PUBLIC: &str = "is_public";
const CONSTRAINT_HAS_ATTR: &str = "has_attribute";
const CONSTRAINT_NAME_PATTERN: &str = "name_pattern";
const CONSTRAINT_REQUIRED_FIELDS: &str = "required_fields";
const CONSTRAINT_IS_RECORD: &str = "is_record";
const CONSTRAINT_IS_SUM: &str = "is_sum";
const CONSTRAINT_IS_PROTOCOL: &str = "is_protocol";

// ============================================================================
// Schema Builder Functions
// ============================================================================

/// Create a function schema builder
///
/// Returns a Map representing a schema with kind = "function".
/// Additional constraints can be added by inserting keys into the map.
fn meta_schema_function(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let mut schema = OrderedMap::new();
    schema.insert(
        Text::from(SCHEMA_KIND_KEY),
        ConstValue::Text(Text::from(SCHEMA_KIND_FUNCTION)),
    );
    Ok(ConstValue::Map(schema))
}

/// Create a type schema builder
///
/// Returns a Map representing a schema with kind = "type".
fn meta_schema_type(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let mut schema = OrderedMap::new();
    schema.insert(
        Text::from(SCHEMA_KIND_KEY),
        ConstValue::Text(Text::from(SCHEMA_KIND_TYPE)),
    );
    Ok(ConstValue::Map(schema))
}

// ============================================================================
// Validation Functions
// ============================================================================

/// Validate code against a schema
///
/// Parses the code text and checks it against the constraints in the schema map.
/// Returns a list of SchemaError maps, each with "message" and "kind" fields.
fn meta_schema_validate(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch {
            expected: 2,
            got: args.len(),
        });
    }

    let code = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            })
        }
    };

    let schema = match &args[1] {
        ConstValue::Map(m) => m.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Map"),
                found: args[1].type_name(),
            })
        }
    };

    let errors = validate_code_against_schema(ctx, &code, &schema);

    let error_values: List<ConstValue> = errors
        .into_iter()
        .map(|err| {
            let mut error_map = OrderedMap::new();
            error_map.insert(
                Text::from("message"),
                ConstValue::Text(err.message),
            );
            error_map.insert(
                Text::from("kind"),
                ConstValue::Text(err.kind),
            );
            ConstValue::Map(error_map)
        })
        .collect();

    Ok(ConstValue::Array(error_values))
}

/// Quick check: is the code a function definition?
fn meta_schema_is_function(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let code = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            })
        }
    };

    let trimmed = code.as_str().trim();
    // A function definition starts with optional visibility + "fn" keyword
    // or "async fn", or "meta fn"
    let is_fn = is_function_code(trimmed);
    Ok(ConstValue::Bool(is_fn))
}

/// Quick check: is the code a type definition?
fn meta_schema_is_type(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let code = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            })
        }
    };

    let trimmed = code.as_str().trim();
    // A type definition starts with "type" keyword
    let is_type = is_type_code(trimmed);
    Ok(ConstValue::Bool(is_type))
}

/// Quick check: is the code an expression?
fn meta_schema_is_expression(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let code = match &args[0] {
        ConstValue::Text(t) => t.clone(),
        _ => {
            return Err(MetaError::TypeMismatch {
                expected: Text::from("Text"),
                found: args[0].type_name(),
            })
        }
    };

    let trimmed = code.as_str().trim();
    // An expression is code that is NOT a function or type definition
    // and NOT a statement block (no leading let/mount/implement)
    let is_expr = is_expression_code(trimmed);
    Ok(ConstValue::Bool(is_expr))
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Schema validation error
struct SchemaError {
    message: Text,
    kind: Text,
}

impl SchemaError {
    fn new(kind: &str, message: impl Into<Text>) -> Self {
        Self {
            message: message.into(),
            kind: Text::from(kind),
        }
    }
}

/// Check if code looks like a function definition
fn is_function_code(code: &str) -> bool {
    // Strip leading attributes (@derive, etc.)
    let stripped = skip_attributes(code);
    let stripped = skip_visibility(stripped);

    stripped.starts_with("fn ")
        || stripped.starts_with("fn(")
        || stripped.starts_with("async fn ")
        || stripped.starts_with("meta fn ")
}

/// Check if code looks like a type definition
fn is_type_code(code: &str) -> bool {
    let stripped = skip_attributes(code);
    let stripped = skip_visibility(stripped);

    stripped.starts_with("type ")
}

/// Check if code looks like an expression (not a definition or statement)
fn is_expression_code(code: &str) -> bool {
    let stripped = skip_attributes(code);
    let stripped = skip_visibility(stripped);

    // Not a function, type, let binding, mount, or implement
    !stripped.starts_with("fn ")
        && !stripped.starts_with("async fn ")
        && !stripped.starts_with("meta fn ")
        && !stripped.starts_with("type ")
        && !stripped.starts_with("let ")
        && !stripped.starts_with("mount ")
        && !stripped.starts_with("implement ")
}

/// Skip leading @ attributes in code
fn skip_attributes(code: &str) -> &str {
    let mut rest = code;
    loop {
        rest = rest.trim_start();
        if rest.starts_with('@') {
            // Skip to end of attribute (find matching paren or end of line)
            if let Some(paren_start) = rest.find('(') {
                let mut depth = 1;
                let after_paren = &rest[paren_start + 1..];
                let mut chars = after_paren.char_indices();
                while let Some((i, ch)) = chars.next() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                rest = &after_paren[i + 1..];
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if depth > 0 {
                    // Malformed, skip to end of line
                    if let Some(nl) = rest.find('\n') {
                        rest = &rest[nl + 1..];
                    } else {
                        return rest;
                    }
                }
            } else if let Some(nl) = rest.find('\n') {
                rest = &rest[nl + 1..];
            } else {
                return rest;
            }
        } else {
            return rest;
        }
    }
}

/// Skip leading visibility modifier (pub)
fn skip_visibility(code: &str) -> &str {
    let trimmed = code.trim_start();
    if trimmed.starts_with("pub ") {
        &trimmed[4..]
    } else {
        trimmed
    }
}

/// Validate code against a schema, returning a list of errors
fn validate_code_against_schema(
    ctx: &MetaContext,
    code: &Text,
    schema: &OrderedMap<Text, ConstValue>,
) -> Vec<SchemaError> {
    let mut errors = Vec::new();

    let schema_kind = schema
        .get(&Text::from(SCHEMA_KIND_KEY))
        .and_then(|v| {
            if let ConstValue::Text(t) = v {
                Some(t.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| Text::from("unknown"));

    let trimmed = code.as_str().trim();

    match schema_kind.as_str() {
        SCHEMA_KIND_FUNCTION => {
            validate_function_schema(ctx, trimmed, schema, &mut errors);
        }
        SCHEMA_KIND_TYPE => {
            validate_type_schema(ctx, trimmed, schema, &mut errors);
        }
        _ => {
            errors.push(SchemaError::new(
                "invalid_schema",
                format!("Unknown schema kind: {}", schema_kind),
            ));
        }
    }

    errors
}

/// Validate code against a function schema
fn validate_function_schema(
    _ctx: &MetaContext,
    code: &str,
    schema: &OrderedMap<Text, ConstValue>,
    errors: &mut Vec<SchemaError>,
) {
    // First check that this is actually a function
    if !is_function_code(code) {
        errors.push(SchemaError::new(
            "not_function",
            "Expected function definition but code does not start with 'fn'",
        ));
        return;
    }

    // Extract function signature components via simple text parsing
    let stripped = skip_attributes(code);
    let stripped = skip_visibility(stripped);

    // Check async constraint
    if let Some(ConstValue::Bool(expected_async)) =
        schema.get(&Text::from(CONSTRAINT_IS_ASYNC))
    {
        let is_async = stripped.starts_with("async fn ");
        if *expected_async != is_async {
            if *expected_async {
                errors.push(SchemaError::new(
                    "async_mismatch",
                    "Schema requires async function but function is not async",
                ));
            } else {
                errors.push(SchemaError::new(
                    "async_mismatch",
                    "Schema requires non-async function but function is async",
                ));
            }
        }
    }

    // Check public constraint
    if let Some(ConstValue::Bool(expected_pub)) =
        schema.get(&Text::from(CONSTRAINT_IS_PUBLIC))
    {
        let is_pub = code.trim_start().starts_with("pub ");
        if *expected_pub != is_pub {
            if *expected_pub {
                errors.push(SchemaError::new(
                    "visibility_mismatch",
                    "Schema requires public function but function is not public",
                ));
            } else {
                errors.push(SchemaError::new(
                    "visibility_mismatch",
                    "Schema requires private function but function is public",
                ));
            }
        }
    }

    // Check name pattern constraint
    if let Some(ConstValue::Text(pattern)) =
        schema.get(&Text::from(CONSTRAINT_NAME_PATTERN))
    {
        let fn_name = extract_function_name(stripped);
        if let Some(name) = fn_name {
            if !name.contains(pattern.as_str()) {
                errors.push(SchemaError::new(
                    "name_mismatch",
                    format!(
                        "Function name '{}' does not match pattern '{}'",
                        name, pattern
                    ),
                ));
            }
        }
    }

    // Check has_attribute constraint
    if let Some(ConstValue::Text(attr_name)) =
        schema.get(&Text::from(CONSTRAINT_HAS_ATTR))
    {
        let has_attr = code.contains(&format!("@{}", attr_name));
        if !has_attr {
            errors.push(SchemaError::new(
                "missing_attribute",
                format!("Function missing required attribute @{}", attr_name),
            ));
        }
    }

    // Check parameter count constraints
    let param_count = count_function_params(stripped);

    if let Some(ConstValue::Int(min)) =
        schema.get(&Text::from(CONSTRAINT_MIN_PARAMS))
    {
        if (param_count as i128) < *min {
            errors.push(SchemaError::new(
                "too_few_params",
                format!(
                    "Function has {} parameters but schema requires at least {}",
                    param_count, min
                ),
            ));
        }
    }

    if let Some(ConstValue::Int(max)) =
        schema.get(&Text::from(CONSTRAINT_MAX_PARAMS))
    {
        if (param_count as i128) > *max {
            errors.push(SchemaError::new(
                "too_many_params",
                format!(
                    "Function has {} parameters but schema allows at most {}",
                    param_count, max
                ),
            ));
        }
    }

    // Check return type constraint
    if let Some(ConstValue::Text(expected_ret)) =
        schema.get(&Text::from(CONSTRAINT_RETURN_TYPE))
    {
        let actual_ret = extract_return_type(stripped);
        if let Some(ret) = actual_ret {
            if ret != expected_ret.as_str() {
                errors.push(SchemaError::new(
                    "return_type_mismatch",
                    format!(
                        "Function returns '{}' but schema requires '{}'",
                        ret, expected_ret
                    ),
                ));
            }
        }
    }
}

/// Validate code against a type schema
fn validate_type_schema(
    _ctx: &MetaContext,
    code: &str,
    schema: &OrderedMap<Text, ConstValue>,
    errors: &mut Vec<SchemaError>,
) {
    // Check that this is actually a type definition
    if !is_type_code(code) {
        errors.push(SchemaError::new(
            "not_type",
            "Expected type definition but code does not start with 'type'",
        ));
        return;
    }

    // Check name pattern
    if let Some(ConstValue::Text(pattern)) =
        schema.get(&Text::from(CONSTRAINT_NAME_PATTERN))
    {
        let type_name = extract_type_name(code);
        if let Some(name) = type_name {
            if !name.contains(pattern.as_str()) {
                errors.push(SchemaError::new(
                    "name_mismatch",
                    format!(
                        "Type name '{}' does not match pattern '{}'",
                        name, pattern
                    ),
                ));
            }
        }
    }

    // Check has_attribute
    if let Some(ConstValue::Text(attr_name)) =
        schema.get(&Text::from(CONSTRAINT_HAS_ATTR))
    {
        let has_attr = code.contains(&format!("@{}", attr_name));
        if !has_attr {
            errors.push(SchemaError::new(
                "missing_attribute",
                format!("Type missing required attribute @{}", attr_name),
            ));
        }
    }

    // Check is_record constraint
    if let Some(ConstValue::Bool(true)) =
        schema.get(&Text::from(CONSTRAINT_IS_RECORD))
    {
        // Record types have "is {" in their definition
        if !code.contains("is {") && !code.contains("is{") {
            errors.push(SchemaError::new(
                "kind_mismatch",
                "Schema requires record type but type is not a record",
            ));
        }
    }

    // Check is_sum constraint
    if let Some(ConstValue::Bool(true)) =
        schema.get(&Text::from(CONSTRAINT_IS_SUM))
    {
        // Sum types use | to separate variants
        let after_is = code.split_once(" is ");
        if let Some((_, body)) = after_is {
            if !body.contains('|') {
                errors.push(SchemaError::new(
                    "kind_mismatch",
                    "Schema requires sum type but type has no variants (|)",
                ));
            }
        }
    }

    // Check is_protocol constraint
    if let Some(ConstValue::Bool(true)) =
        schema.get(&Text::from(CONSTRAINT_IS_PROTOCOL))
    {
        if !code.contains("is protocol") {
            errors.push(SchemaError::new(
                "kind_mismatch",
                "Schema requires protocol type but type is not a protocol",
            ));
        }
    }

    // Check required fields constraint
    if let Some(ConstValue::Array(required)) =
        schema.get(&Text::from(CONSTRAINT_REQUIRED_FIELDS))
    {
        for field_val in required.iter() {
            if let ConstValue::Text(field_name) = field_val {
                // Simple check: field name followed by colon in the body
                let field_pattern = format!("{}: ", field_name);
                let field_pattern2 = format!("{}:", field_name);
                if !code.contains(&field_pattern) && !code.contains(&field_pattern2) {
                    errors.push(SchemaError::new(
                        "missing_field",
                        format!("Type missing required field '{}'", field_name),
                    ));
                }
            }
        }
    }

    // Check is_public
    if let Some(ConstValue::Bool(expected_pub)) =
        schema.get(&Text::from(CONSTRAINT_IS_PUBLIC))
    {
        let is_pub = code.trim_start().starts_with("pub ");
        if *expected_pub != is_pub {
            if *expected_pub {
                errors.push(SchemaError::new(
                    "visibility_mismatch",
                    "Schema requires public type but type is not public",
                ));
            } else {
                errors.push(SchemaError::new(
                    "visibility_mismatch",
                    "Schema requires private type but type is public",
                ));
            }
        }
    }
}

/// Extract function name from code starting at "fn ..."
fn extract_function_name(code: &str) -> Option<&str> {
    // Skip "async ", "meta ", etc. to get to "fn name"
    let rest = if code.starts_with("async fn ") {
        &code[9..]
    } else if code.starts_with("meta fn ") {
        &code[8..]
    } else if code.starts_with("fn ") {
        &code[3..]
    } else {
        return None;
    };

    // Name ends at '(' or '<' or whitespace
    let end = rest
        .find(|c: char| c == '(' || c == '<' || c.is_whitespace())
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

/// Count function parameters (simple heuristic: count commas in first paren group + 1)
fn count_function_params(code: &str) -> usize {
    // Find the opening paren of the parameter list
    let paren_start = match code.find('(') {
        Some(i) => i,
        None => return 0,
    };

    let after_paren = &code[paren_start + 1..];
    let mut depth = 1;
    let mut comma_count = 0;
    let mut has_content = false;

    for ch in after_paren.chars() {
        match ch {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | '>' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => comma_count += 1,
            c if !c.is_whitespace() && depth == 1 => has_content = true,
            _ => {}
        }
    }

    if has_content || comma_count > 0 {
        comma_count + 1
    } else {
        0
    }
}

/// Extract return type from function signature (after "->")
fn extract_return_type(code: &str) -> Option<&str> {
    // Find "->" before the body "{"
    let body_start = code.find('{')?;
    let sig = &code[..body_start];
    let arrow = sig.rfind("->")?;
    let ret = sig[arrow + 2..].trim();
    if ret.is_empty() {
        None
    } else {
        Some(ret)
    }
}

/// Extract type name from "type Name is ..."
fn extract_type_name(code: &str) -> Option<&str> {
    let stripped = skip_attributes(code);
    let stripped = skip_visibility(stripped);

    if !stripped.starts_with("type ") {
        return None;
    }

    let rest = &stripped[5..];
    let end = rest
        .find(['<', ' ', ';', '{'])
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_function_creates_map() {
        let mut ctx = MetaContext::new();
        let result = meta_schema_function(&mut ctx, List::new()).unwrap();
        if let ConstValue::Map(m) = result {
            assert_eq!(
                m.get(&Text::from(SCHEMA_KIND_KEY)),
                Some(&ConstValue::Text(Text::from(SCHEMA_KIND_FUNCTION)))
            );
        } else {
            panic!("Expected Map");
        }
    }

    #[test]
    fn test_schema_type_creates_map() {
        let mut ctx = MetaContext::new();
        let result = meta_schema_type(&mut ctx, List::new()).unwrap();
        if let ConstValue::Map(m) = result {
            assert_eq!(
                m.get(&Text::from(SCHEMA_KIND_KEY)),
                Some(&ConstValue::Text(Text::from(SCHEMA_KIND_TYPE)))
            );
        } else {
            panic!("Expected Map");
        }
    }

    #[test]
    fn test_is_function_positive() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("fn foo(x: Int) -> Int { x }"))]);
        let result = meta_schema_is_function(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_is_function_async() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("async fn bar() { }"))]);
        let result = meta_schema_is_function(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_is_function_negative() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("type Foo is { x: Int };"))]);
        let result = meta_schema_is_function(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_is_type_positive() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("type Point is { x: Float, y: Float };"))]);
        let result = meta_schema_is_type(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_is_type_negative() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("fn foo() {}"))]);
        let result = meta_schema_is_type(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_is_expression_positive() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("x + y * 2"))]);
        let result = meta_schema_is_expression(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(true));
    }

    #[test]
    fn test_is_expression_negative_for_fn() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("fn foo() {}"))]);
        let result = meta_schema_is_expression(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Bool(false));
    }

    #[test]
    fn test_validate_function_schema_basic() {
        let mut ctx = MetaContext::new();
        let mut schema = OrderedMap::new();
        schema.insert(
            Text::from(SCHEMA_KIND_KEY),
            ConstValue::Text(Text::from(SCHEMA_KIND_FUNCTION)),
        );
        let args = List::from(vec![
            ConstValue::Text(Text::from("fn add(x: Int, y: Int) -> Int { x + y }")),
            ConstValue::Map(schema),
        ]);
        let result = meta_schema_validate(&mut ctx, args).unwrap();
        if let ConstValue::Array(errors) = result {
            assert!(errors.is_empty(), "Expected no errors: {:?}", errors);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_validate_function_not_a_function() {
        let mut ctx = MetaContext::new();
        let mut schema = OrderedMap::new();
        schema.insert(
            Text::from(SCHEMA_KIND_KEY),
            ConstValue::Text(Text::from(SCHEMA_KIND_FUNCTION)),
        );
        let args = List::from(vec![
            ConstValue::Text(Text::from("type X is Int;")),
            ConstValue::Map(schema),
        ]);
        let result = meta_schema_validate(&mut ctx, args).unwrap();
        if let ConstValue::Array(errors) = result {
            assert_eq!(errors.len(), 1);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_extract_function_name() {
        assert_eq!(extract_function_name("fn foo(x: Int)"), Some("foo"));
        assert_eq!(extract_function_name("async fn bar()"), Some("bar"));
        assert_eq!(extract_function_name("meta fn baz<T>()"), Some("baz"));
    }

    #[test]
    fn test_count_function_params() {
        assert_eq!(count_function_params("fn foo()"), 0);
        assert_eq!(count_function_params("fn foo(x: Int)"), 1);
        assert_eq!(count_function_params("fn foo(x: Int, y: Int)"), 2);
        assert_eq!(count_function_params("fn foo(x: Map<K, V>, y: Int)"), 2);
    }

    #[test]
    fn test_extract_type_name() {
        assert_eq!(extract_type_name("type Point is { x: Float };"), Some("Point"));
        assert_eq!(extract_type_name("type Option<T> is None | Some(T);"), Some("Option"));
    }

    #[test]
    fn test_skip_attributes() {
        assert_eq!(skip_attributes("@derive(Eq)\nfn foo() {}").trim_start(), "fn foo() {}");
        assert_eq!(skip_attributes("fn foo() {}"), "fn foo() {}");
    }
}
