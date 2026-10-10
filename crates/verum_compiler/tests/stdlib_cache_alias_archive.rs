//! Parser-owned alias meaning must reach a real bootstrap archive and metadata.
//! This uses compile_core discovery/parsing/registration/writing on a tiny core
//! tree; no manufactured descriptor or inherited embedded archive supplies it.

use std::fs;
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_types::core_metadata::TypeDescriptorKind;
use verum_vbc::{
    archive::read_archive_from_file,
    types::{TypeParamId, TypeRef},
};

#[test]
fn generic_parameter_aliases_survive_the_actual_bootstrap_archive() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    let input = core.join("mod.vr");
    fs::write(
        &input,
        "module core; public type Identity<T> is T; public type Pick<L, R> is R; public type Marker is T;",
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let config = CoreConfig::new(&core).with_output(&output);
    let mut session = Session::new(CompilerOptions {
        input,
        ..Default::default()
    });
    let result = CompilationPipeline::new_core(&mut session, config)
        .compile_core()
        .expect("actual bootstrap producer compiles the parsed alias source");
    assert_eq!(result.modules_compiled, 1);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.output_size > 0);
    assert_eq!(result.output_path, output);

    let archive = read_archive_from_file(&output).expect("decode the produced archive from disk");
    let module = archive.load_module("core").expect("actual core module");
    let metadata = verum_compiler::archive_metadata::archive_to_core_metadata(&archive);
    let metadata: verum_types::core_metadata::CoreMetadata =
        bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap();
    for (name, parameter, slot) in [("Identity", "T", 0), ("Pick", "R", 1)] {
        let descriptor = module
            .types
            .iter()
            .find(|descriptor| module.get_string(descriptor.name) == Some(name))
            .unwrap_or_else(|| panic!("archive must contain {name}"));
        assert_eq!(
            descriptor.alias_target,
            Some(TypeRef::Generic(TypeParamId(slot)))
        );
        assert_eq!(
            descriptor
                .alias_target_name
                .and_then(|id| module.get_string(id)),
            Some(parameter)
        );
        assert!(
            descriptor.variants.is_empty(),
            "parameter alias became a marker variant"
        );
        let TypeDescriptorKind::Alias { target } = &metadata
            .types
            .get(&Text::from(format!("core.{name}")))
            .expect("exact core alias metadata")
            .kind
        else {
            panic!("{name} metadata must retain alias identity");
        };
        assert_eq!(target.as_str(), parameter);
    }
    let marker = module
        .types
        .iter()
        .find(|descriptor| module.get_string(descriptor.name) == Some("Marker"))
        .expect("unrelated marker declaration");
    assert!(marker.alias_target.is_none());
    assert_eq!(marker.variants.len(), 1);
    assert_eq!(module.get_string(marker.variants[0].name), Some("T"));
}
