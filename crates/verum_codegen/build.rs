use std::{env, path::PathBuf, process::Command};

fn main() {
    let source = PathBuf::from(env::var_os("DEP_VERUM_VBC_LINUX_SIGNAL_SOURCE").unwrap());
    let llvm = PathBuf::from(env::var_os("DEP_VERUM_LLVM_ROOT").unwrap());
    let clang = llvm
        .join("bin")
        .join(format!("clang{}", env::consts::EXE_SUFFIX));
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", source.display());
    for arch in ["x86_64", "aarch64"] {
        let mut compiler = Command::new(&clang);
        compiler.args([
            "-target",
            &format!("{arch}-unknown-linux-gnu"),
            "-O2",
            "-ffreestanding",
            "-fno-stack-protector",
            "-fno-asynchronous-unwind-tables",
            "-fno-unwind-tables",
            "-fPIC",
            "-nostdinc",
            "-emit-llvm",
            "-c",
        ]);
        if arch == "aarch64" {
            compiler.arg("-mno-outline-atomics");
        }
        let status = compiler
            .arg(&source)
            .arg("-o")
            .arg(output.join(format!("linux_signal_{arch}.bc")))
            .status()
            .unwrap_or_else(|error| panic!("could not run {}: {error}", clang.display()));
        assert!(
            status.success(),
            "failed to compile the {arch} signal runtime"
        );
    }
}
