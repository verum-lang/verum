//! Link the transitive dependencies of a closed Linux process, without a C
//! runtime, loader or libraries. A cross build can export the exact ELF for
//! execution inside a kernel of the selected architecture.
use std::{path::PathBuf, process::Command};
use verum_llvm::{
    OptimizationLevel,
    module::{Linkage, Module},
    passes::PassBuilderOptions,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target},
};

pub fn check(
    module: &Module<'_>,
    arch: &str,
    entry: &str,
    case: &str,
    expected: i32,
    stdout: &[u8],
) {
    let start = match arch {
        "aarch64" => format!("bl {entry}\nb verum_os_exit"),
        "x86_64" => format!("callq {entry}\nmovl %eax, %edi\ncallq verum_os_exit"),
        other => panic!("unsupported test architecture {other}"),
    };
    module.set_inline_assembly(&format!(".text\n.globl _start\n_start:\n{start}\n"));
    module.verify().expect("verified module before linking");
    // A standalone process exports no callable API; the assembly entry is
    // the only root. Keep its two callees visible to LLVM's global DCE.
    for f in module
        .get_functions()
        .filter(|f| f.count_basic_blocks() != 0)
    {
        let name = f.get_name().to_str().unwrap();
        if name != entry && name != "verum_os_exit" {
            f.set_linkage(Linkage::Internal);
        }
    }
    Target::initialize_all(&InitializationConfig::default());
    let triple = module.get_triple();
    let machine = Target::from_triple(&triple)
        .unwrap()
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();
    module
        .run_passes(
            "globaldce,function(mem2reg,simplifycfg),globaldce",
            &machine,
            PassBuilderOptions::create(),
        )
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let object = dir.path().join("process.o");
    let exe = dir.path().join(case);
    machine
        .write_to_file(module, FileType::Object, &object)
        .unwrap();
    let config = verum_codegen::link::NoLibcConfig::linux();
    let lld = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../llvm/install/bin/ld.lld");
    let linked = Command::new(if lld.is_file() { lld } else { "ld.lld".into() })
        .arg("-static")
        .args(config.linker_flags())
        .arg("-e")
        .arg(&config.entry_point)
        .arg("-o")
        .arg(&exe)
        .arg(object)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    if let Some(out) = std::env::var_os("VERUM_TEST_LINUX_ELF_DIR") {
        std::fs::copy(&exe, PathBuf::from(out).join(format!("{case}-{arch}"))).unwrap();
    }
    if cfg!(target_os = "linux") && std::env::consts::ARCH == arch {
        let run = Command::new(&exe).output().unwrap();
        assert_eq!(run.status.code(), Some(expected));
        assert_eq!(run.stdout, stdout);
    }
}
