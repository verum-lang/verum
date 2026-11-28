//! Verify actual LLVM scope and ordering, link the result, and execute it.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn native_fences_preserve_scope_and_dynamic_order() {
    let module = source_image::module(
        r#"
        fn synchronize(order:Int) {
            @intrinsic("atomic_fence", order);
            @intrinsic("compiler_fence", order);
        }
        fn main()->Int { synchronize(0); synchronize(1); synchronize(2); synchronize(3); synchronize(4); 73 }
    "#,
    );
    source_image::run_module(module, |module| {
        let ir = module.print_to_string().to_string();
        for order in ["acquire", "release", "acq_rel", "seq_cst"] {
            assert!(ir.contains(&format!("fence {order}\n")), "{order}");
            assert!(
                ir.contains(&format!("fence syncscope(\"singlethread\") {order}\n")),
                "{order}"
            );
        }
    });
}

#[test]
fn invalid_order_aborts_the_linked_program() {
    let module =
        source_image::module(r#"fn main()->Int { @intrinsic("compiler_fence", 260); 73 }"#);
    let output = source_image::execute_module(module, |_| {});
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid memory fence ordering"));
}
