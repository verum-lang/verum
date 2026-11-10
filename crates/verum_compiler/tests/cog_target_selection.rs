//! A published cog is compiled for the target its producer was asked to use.
use std::{fs, sync::Arc};
use verum_compiler::precompile::{PrecompileCogConfig, precompile_cog};
use verum_vbc::{
    archive::read_archive_from_file, interpreter::Interpreter, validate::validate_module,
};

#[test]
fn requested_cog_target_controls_declarations_and_function_bodies() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(check_targets)
        .unwrap()
        .join()
        .unwrap();
}

fn check_targets() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("Verum.toml"),
        "[cog]\nname='platform'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("mod.vr"),
        r#"
        module platform;
        @cfg(target_os = "linux") fn declaration() -> Int { 20 }
        @cfg(target_os = "macos") fn declaration() -> Int { 30 }
        public fn probe() -> Int {
            @cfg(target_os = "linux") { return declaration() + 1; }
            @cfg(target_os = "macos") { return declaration() + 2; }
        }
    "#,
    )
    .unwrap();
    for (target, expected) in [
        ("aarch64-apple-darwin", 32),
        ("aarch64-unknown-linux-gnu", 21),
    ] {
        let mut config = PrecompileCogConfig::for_cog(directory.path()).unwrap();
        config.target_triple = Some(target.into());
        config.output_path = directory.path().join(format!("{target}.vbca"));
        precompile_cog(&config).unwrap();
        let archive = read_archive_from_file(&config.output_path).unwrap();
        let mut linker = verum_vbc::linker::VbcLinker::new(target);
        linker.add_archive(&archive).unwrap();
        let module = linker.finalize();
        validate_module(&module).unwrap();
        let probe = module.find_function_by_name("platform.probe").unwrap();
        let observed = Interpreter::new(Arc::new(module))
            .execute_function(probe)
            .unwrap();
        assert_eq!(observed.try_as_i64(), Some(expected), "{target}");
    }
}
