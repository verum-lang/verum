//! The complete embedded source archive is classified on ordinary workers.
use verum_compiler::{
    embedded_stdlib,
    stdlib_classifier::{ClassificationError, classify_archive},
    stdlib_index,
};

#[test]
fn source_classification_does_not_require_enlarged_worker_stacks() {
    let archive = embedded_stdlib::get_embedded_stdlib().expect("bootstrap source archive");
    let index = stdlib_index::get_module_index().expect("bootstrap source index");
    let expected = index.all_modules().len();
    let report = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || classify_archive(archive, index))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(report.modules.len(), expected);
    assert_eq!(report.stats.total_modules, expected);
    for owner in [
        "core.base.primitives",
        "core.text.text",
        "core.math.simple",
        "core.math.tensor",
    ] {
        let module = report
            .modules
            .iter()
            .find(|module| module.module_path == owner)
            .unwrap_or_else(|| panic!("missing actual source {owner}"));
        assert!(
            !matches!(module.layer, Err(ClassificationError::ParseError(_))),
            "{owner}: {:?}",
            module.layer
        );
        assert!(
            module.breakdown.runtime > 0,
            "source declarations were not visited: {owner}"
        );
    }
}
