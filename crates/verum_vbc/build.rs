use std::{env, path::PathBuf};

fn main() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("native/linux_signal_watch.c");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:linux_signal_source={}", source.display());
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && matches!(arch.as_str(), "x86_64" | "aarch64")
    {
        let mut build = cc::Build::new();
        build
            .file(source)
            .flag("-ffreestanding")
            .flag("-fno-stack-protector");
        if arch == "aarch64" {
            build.flag("-mno-outline-atomics");
        }
        build.compile("verum_linux_signal");
    }
}
