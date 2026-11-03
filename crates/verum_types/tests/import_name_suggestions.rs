use verum_common::{List, Shared, Text};
use verum_types::TypeError;

fn suggestions(target: &str, candidates: &[&str]) -> Text {
    let diagnostic = TypeError::ImportItemNotFound {
        item_name: target.into(),
        module_path: "project.api".into(),
        available_items: Shared::new(candidates.iter().map(|name| Text::from(*name)).collect()),
        declared_in: List::new(),
        span: verum_ast::Span::default(),
    }
    .to_diagnostic();
    assert_eq!(diagnostic.code(), Some("E401"));
    diagnostic
        .helps()
        .iter()
        .find(|help| help.message.starts_with("did you mean"))
        .map(|help| help.message.clone())
        .unwrap_or_default()
}

#[test]
fn import_hints_keep_prefix_candidates_exact_scores_and_stable_ties() {
    assert_eq!(
        suggestions("cat", &["catalogue", "cut", "cart", "care", "unrelated"]).as_str(),
        "did you mean one of these? `cut`, `cart`, `care`"
    );
    assert_eq!(
        suggestions("cat", &["catalogue", "c", "unrelated"]).as_str(),
        "did you mean one of these? `c`, `catalogue`"
    );
    assert_eq!(
        suggestions("Жук", &["Жукология", "жак", "лук", "unrelated"]).as_str(),
        "did you mean one of these? `жак`, `лук`, `Жукология`"
    );
    assert!(suggestions("missing", &["completely_unrelated"]).is_empty());
}
