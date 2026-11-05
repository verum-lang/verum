use verum_ast::{FileId, ItemKind};
use verum_common::{ConstValue, List, Text};
use verum_fast_parser::FastParser;
use verum_types::{TypeChecker, const_eval::ConstEvaluator};

fn module(source: &str) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("actual source grammar")
}
fn expression(source: &str) -> verum_ast::Expr {
    FastParser::new()
        .parse_expr_str(source, FileId::new(0))
        .expect("actual expression grammar")
}
fn checker() -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_cog("fixture");
    checker.set_current_module_path("fixture.main");
    checker
}
fn check(checker: &mut TypeChecker, source: &str) -> List<Text> {
    let start = checker.diagnostics().len();
    let ast = module(source);
    let mut errors: List<Text> = checker
        .prepare_module_declarations(&ast)
        .into_iter()
        .map(|error| error.to_string().into())
        .collect();
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error.to_string().into());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| error.to_string().into()),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .skip(start)
            .filter(|error| error.is_error())
            .map(|error| format!("{error:?}").into()),
    );
    errors
}
fn accept(checker: &mut TypeChecker, source: &str) {
    let errors = check(checker, source);
    assert!(errors.is_empty(), "{source}\n{errors:?}");
}
#[test]
fn concrete_counts_keep_exact_values() {
    let mut c = checker();
    accept(&mut c, "const Limit:Int=2; fn same(value:[Int;2])->[Int;Limit]{value} fn arithmetic(value:[Int;3])->[Int;1+2]{value}");
    assert!(!check(&mut c, "fn wrong(value:[Int;2])->[Int;1+2]{value}").is_empty());
}
#[test]
fn exact_declared_integer_parameters_remain_supported() {
    for source in [
        "fn same<N:meta Int>(value:[Int;N])->[Int;N]{value}",
        "fn same<N:meta Int>(value:[Int;(N)])->[Int;(N)]{value}",
        "fn same<N:meta USize>(value:[Int;N])->[Int;N]{value}",
        "type Shape<N:meta Int> is {values:[Int;N]};",
    ] { accept(&mut checker(), source); }
}
#[test]
fn non_integer_and_type_parameters_cannot_be_lengths() {
    for source in [
        "fn wrong<N:meta Bool>(value:[Int;N])->Int{42}",
        "fn wrong<T>(value:[Int;T])->Int{42}",
        "const N:Int=2; fn wrong<N:meta Bool>(value:[Int;N])->Int{42}",
        "type Count is {value:Int}; fn wrong<N:meta Count>(value:[Int;N])->Int{42}",
    ] { assert!(!check(&mut checker(), source).is_empty(), "{source}"); }
}
#[test]
fn concrete_evaluation_errors_refuse_the_annotation() {
    for count in ["1/0", "1%0", "-1", "9223372036854775807+1", "9223372036854775808", "true", "missing", "missing.owner.Limit"] {
        let source=format!("fn wrong(value:[Int;2])->[Int;{count}]{{value}}");
        let errors=check(&mut checker(), &source);
        assert!(!errors.is_empty(), "{source}");
        assert!(errors.iter().any(|error| error.contains("array length")), "{errors:?}");
    }
}
#[test]
fn unresolved_compound_cannot_hide_known_bad_or_unknown_subtrees() {
    for count in ["N+missing", "N/0", "N+(1/0)", "N+true", "N+9223372036854775808"] {
        let source=format!("fn wrong<N:meta Int>(value:[Int;N])->[Int;{count}]{{value}}");
        let errors=check(&mut checker(), &source);
        assert!(!errors.is_empty(), "{source}");
    }
}
#[test]
fn invalid_local_shadow_cannot_fall_back_to_declared_parameter() {
    let source="fn wrong<N:meta Int>(value:[Int;N])->[Int;N]{const N:Int=1/0;let local:[Int;N]=value;value}";
    assert!(!check(&mut checker(), source).is_empty(), "{source}");
}
#[test]
fn concrete_local_shadow_of_parameter_is_not_a_wildcard() {
    let source="fn wrong<N:meta Int>(two:[Int;2])->[Int;2]{const N:Int=3;let local:[Int;N]=two;two}";
    assert!(!check(&mut checker(), source).is_empty(), "{source}");
}
#[test]
fn generic_length_does_not_erase_element_type_checks() {
    let source="fn wrong<N:meta Int>(value:[Bool;N])->[Int;N]{value}";
    assert!(!check(&mut checker(), source).is_empty(), "{source}");
}
#[test]
fn forward_count_sources_preserve_order_independent_exact_values() {
    for source in [
        "fn same(value:[Int;3])->[Int;Limit]{value} const Limit:Int=Base+1;const Base:Int=2;",
        "type Buffer is {values:[Int;Limit]}; const Limit:Int=2;",
        "module inner {fn same(value:[Int;2])->[Int;Limit]{value} const Limit:Int=2;}",
    ] { accept(&mut checker(), source); }
}
#[test]
fn forward_count_sources_do_not_capture_local_shadows() {
    let source="const Base:Int=2; const Limit:Int=Base; fn same(two:[Int;2])->[Int;2]{const Base:Int=3;let value:[Int;Limit]=two;value}";
    accept(&mut checker(), source);
}
#[test]
fn source_count_cycles_and_invalid_dependencies_refuse() {
    for source in [
        "fn wrong(value:[Int;2])->[Int;Limit]{value} const Limit:Int=Limit;",
        "fn wrong(value:[Int;2])->[Int;A]{value} const A:Int=B; const B:Int=A;",
        "fn wrong(value:[Int;2])->[Int;Limit]{value} const Limit:Int=Bad+1;const Bad:Int=1/0;",
        "fn wrong(value:[Int;2])->[Int;Limit]{value} const Limit:Int=Missing;",
    ] { assert!(!check(&mut checker(), source).is_empty(), "{source}"); }
}
#[test]
fn count_source_catalog_uses_the_selected_target() {
    let mut c = checker();
    c.set_cfg_target(verum_ast::cfg::TargetConfig::for_target("aarch64-unknown-linux-gnu"));
    accept(&mut c, "@cfg(windows) const Limit:Int=1/0; @cfg(unix) const Limit:Int=2; fn same(two:[Int;2])->[Int;Limit]{two}");
    assert!(!check(&mut c, "@cfg(windows) module foreign {const Limit:Int=2;} fn wrong(two:[Int;2])->[Int;foreign.Limit]{two}").is_empty());
}
#[test]
fn generic_constant_is_not_an_uninstantiated_array_count() {
    assert!(!check(&mut checker(), "const Limit<T>:Int=2;fn wrong(two:[Int;2])->[Int;Limit]{two}").is_empty());
}
#[test]
fn runtime_bindings_never_supply_constant_values_through_names() {
    for source in [
        "const Limit:Int=2;fn wrong(two:[Int;2],Limit:Int)->[Int;2]{let local:[Int;Limit]=two;two}",
        "const Limit:Int=2;fn wrong(two:[Int;2])->[Int;2]{let Limit=3;let local:[Int;Limit]=two;two}",
        "fn wrong(two:[Int;2])->[Int;2]{const Limit:Int=2;let Limit=3;let local:[Int;Limit]=two;two}",
        "const Input:Int=2;fn wrong(two:[Int;2],Input:Int)->[Int;2]{const Limit:Int=Input;let local:[Int;Limit]=two;two}",
        "const Input:Int=2;fn wrong(two:[Int;2],Input:Int)->[Int;2]{let local:[Int;Input+0]=two;two}",
        "fn wrong<N:meta Int>(two:[Int;2],N:Int)->[Int;2]{let local:[Int;N]=two;two}",
    ] { assert!(!check(&mut checker(), source).is_empty(), "{source}"); }
}
#[test]
fn lexical_const_roles_restore_and_can_shadow_runtime_values() {
    for source in [
        "const Limit:Int=2;fn same(two:[Int;2])->[Int;2]{{let Limit=3;}let local:[Int;Limit]=two;local}",
        "fn same(two:[Int;2],Limit:Int)->[Int;2]{const Limit:Int=2;let local:[Int;Limit]=two;local}",
        "fn same(two:[Int;2],Limit:Int)->[Int;2]{let local:[Int;{let Limit=2;Limit}]=two;local}",
        "fn outer(N:Int)->Int{fn inner<N:meta Int>(value:[Int;N])->[Int;N]{value}42}",
    ] { accept(&mut checker(), source); }
}
#[test]
fn implementation_value_parameters_obey_their_actual_lexical_shadow() {
    let prefix="type Shape<N:meta Int> is {unit:()};";
    accept(&mut checker(), &format!("{prefix} implement<N:meta Int> Shape<N>{{fn same(value:[Int;N])->[Int;N]{{value}}}}"));
    let source=format!("{prefix} implement<N:meta Int> Shape<N>{{fn wrong(value:[Int;2],N:Int)->[Int;2]{{let local:[Int;N]=value;value}}}}");
    assert!(!check(&mut checker(), &source).is_empty(), "{source}");
}
#[test]
fn same_spelled_user_nominals_do_not_become_integer_parameter_carriers() {
    for name in ["UInt8", "Int64"] {
        let source=format!("type {name} is {{value:Int}};fn wrong<N:meta {name}>(value:[Int;N])->[Int;N]{{value}}");
        let errors=check(&mut checker(), &source);
        assert!(!errors.is_empty(), "user nominal became primitive: {source}");
        assert!(errors.iter().any(|error| error.contains("declaration-owned integer value parameter")), "{errors:?}");
    }
}
