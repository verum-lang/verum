//! Debug attribute targets parsing

use verum_fast_parser::FastParser;
use verum_ast::FileId;

fn test_parse(name: &str, source: &str) {
    let parser = FastParser::new();
    let file_id = FileId::new(0);

    match parser.parse_module_str(source, file_id) {
        Ok(m) => println!("✓ {}: {} items", name, m.items.len()),
        Err(e) => {
            println!("✗ {}: FAILED", name);
            for err in e.iter() {
                println!("  {:?}", err);
            }
        }
    }
}

#[test]
fn test_attribute_targets_separately() {
    println!("\n=== Testing attribute targets ===\n");

    // Test 1: Function with attributes
    test_parse("fn_attr", r#"
@inline
@hot
pub fn hot_function() {}
"#);

    // Test 2: Type with attributes on fields
    test_parse("type_field_attr", r#"
@derive(Clone, Debug)
type FFIStruct is {
    @align(8)
    field1: i64,
    @used
    field2: i32
};
"#);

    // Test 3: Variant type with attributes on variants
    test_parse("variant_attr", r#"
type Status is
    | @default Active
    | @deprecated Inactive
    | Error(Text);
"#);

    // Test 4: Record type with attributes on fields
    test_parse("record_field_attr", r#"
type Config is {
    @serialize(rename = "host_name")
    host: Text,
    @validate(min = 1, max = 65535)
    port: Int
};
"#);

    // Test 5: Function with attributes on parameters
    test_parse("param_attr", r#"
fn process(
    @unused _context: Context,
    @must_use data: &Data
) -> Result {}
"#);

    // Test 6: Impl block with attributes
    test_parse("impl_attr", r#"
@specialize
implement Display for Point {
    fn display(&self) -> Text {}
}
"#);

    // Test 7: Match arms with attributes
    test_parse("match_arm_attr", r#"
fn handle(msg: Message) {
    match msg {
        @cold Error(e) => log_error(e),
        @hot Data(d) => process(d),
        _ => ignore()
    }
}
"#);

    // Test 8: Field initializers with attributes
    test_parse("field_init_attr", r#"
fn create_config() -> Config {
    Config {
        @cfg(debug) debug_mode: true,
        @cfg(release) debug_mode: false,
        ..Default.default()
    }
}
"#);

    // Test 9: Module with attributes
    test_parse("module_attr", r#"
@cfg(test)
module tests {
    fn test_something() {}
}
"#);

    // Test 10: Extern block with attributes
    test_parse("extern_attr", r#"
extern "C" {
    @link_name("c_function_name")
    fn verum_function();
}
"#);

    println!("\n=== Done ===\n");
}
