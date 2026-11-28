use super::*;
use verum_diagnostics::Span;

#[test]
fn repeated_sites_count_diagnostics_and_owners_independently() {
    let sources: List<_> = ["model", "left", "right"].iter().enumerate().map(|(index, name)| {
        let module = verum_ast::Module::new(List::new(), verum_ast::FileId::new(index as u32), Default::default());
        (Text::from(*name), Text::from(format!("{name}.vr")), module)
    }).collect();
    let span = Span::new("model.vr", 2, 5, 9);
    let diagnostic = |span: Span, code: &str, message: &str, label: &str| {
        DiagnosticBuilder::error().code(code).message(message)
            .span_label(span.clone(), label).span_label(span, label).build()
    };
    let shared = diagnostic(span.clone(), "E405", "unproved bound", "required here");
    let other = diagnostic(Span::new("model.vr", 2, 5, 12), "E405", "unproved bound", "required here");
    let foreign = diagnostic(Span::new("elsewhere.vr", 2, 5, 9), "E405", "unproved bound", "required here");
    let checker = |diagnostics| SourceCheckOutcome { diagnostics, ..Default::default() };
    let outcomes = [
        checker(List::new()),
        checker(List::from_iter([
            shared.clone(), shared.clone(), other, foreign.clone(), foreign,
            diagnostic(span.clone(), "E405", "different error", "required here"),
            diagnostic(span.clone(), "E400", "unproved bound", "required here"),
            diagnostic(span.clone(), "E405", "unproved bound", "another label"),
        ])),
        checker(List::from_iter([shared])),
    ];
    assert_eq!(shared_diagnostic_locations(&sources, &outcomes).as_str(),
        "Repeated primary source diagnostics:\n  model.vr:2:5: E405 unproved bound — required here (3 diagnostics in 2 modules)\n");
    let single = shared_diagnostic_locations(&sources.as_slice()[..2], &outcomes[..2]);
    assert!(single.contains("(2 diagnostics in 1 module)"), "{single}");
}
