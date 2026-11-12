//! Native aggregate adaptation must be explicit, never a pointer to the header.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::context::Context;
use verum_vbc::codegen::VbcCodegen;

#[test]
fn native_layouts_requiring_field_adaptation_are_refused() {
    for (fields, values) in [
        ("a: Int32, b: Int64", "a: 1 as Int32, b: 2 as Int64"),
        ("a: [Int64; 2]", "a: [1 as Int64, 2 as Int64]"),
    ] {
        let source = format!(
            r#"
            type Packet is {{ {fields} }};
            extern "C" {{ fn inspect_packet(packet: &unsafe Packet) -> Int32; }}
            fn main() -> Int32 {{
                let packet = Packet {{ {values} }};
                unsafe {{ inspect_packet(&packet) }}
            }}
        "#
        );
        let module = VbcCodegen::new()
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap();
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("aggregate-refusal").with_debug_info(false),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("unsupported layout must refuse AOT");
        assert!(
            error
                .to_string()
                .contains("native FFI aggregate requires field adaptation"),
            "{error}"
        );
    }
}
