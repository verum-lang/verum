use verum_ast::cfg::TargetConfig;

#[test]
fn an_unknown_vendor_does_not_replace_the_os() {
    for (triple, os) in [
        ("aarch64-unknown-linux", "linux"),
        ("riscv64-unknown-freebsd", "freebsd"),
        ("wasm32-unknown-unknown", "unknown"),
    ] {
        let target = TargetConfig::for_target(triple);
        assert_eq!(target.target_vendor.as_str(), "unknown");
        assert!(target.matches("target_os", os), "{triple}: {target:?}");
        assert_eq!(target.target_env.as_str(), "");
        if os != "unknown" { assert!(target.is_set("unix")); }
    }
}

#[test]
fn bare_metal_abbreviations_keep_the_abi_environment() {
    let target = TargetConfig::for_target("thumbv7em-none-eabihf");
    assert!(target.matches("target_os", "none"));
    assert!(target.matches("target_env", "eabihf"));
    assert!(target.matches("target_vendor", "unknown"));
}
