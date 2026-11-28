use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_module_path("sample");
    let mut errors = checker.prepare_module_declarations(&module);
    errors.extend(
        module
            .items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

const TYPES: &str = "type affine Token is {value:Int}; type Choice is Scalar(Int) | Owned(Token); type Holder is {choice:Choice};";

#[test]
fn borrowed_match_place_reads_only_the_selected_payload() {
    let mut failed = List::new();
    for place in [
        "value.choice",
        "(*value).choice",
        "value[0].choice",
        "value.0.choice",
    ] {
        let ty = if place.contains("[0]") {
            "[Holder;1]"
        } else if place.contains(".0") {
            "(Holder,Int)"
        } else {
            "Holder"
        };
        for body in [
            format!("match {place} {{Choice.Scalar(n)=>n,Choice.Owned(ref token)=>token.value}}"),
            format!(
                "let result=match {place} {{Choice.Scalar(n)=>n,Choice.Owned(ref token)=>token.value}}; result"
            ),
            format!(
                "let first=match {place} {{Choice.Scalar(_)=>1,Choice.Owned(_)=>2}}; let second=match {place} {{Choice.Scalar(n)=>n,Choice.Owned(ref token)=>token.value}}; first+second"
            ),
        ] {
            let source = format!("{TYPES} fn inspect(value:&{ty})->Int {{{body}}}");
            let found = errors(&source);
            if !found.is_empty() {
                failed.push(format!("{source}: {found:?}"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn borrowed_match_place_cannot_move_an_affine_payload() {
    for body in [
        "fn invalid(value:&Holder)->Token {match value.choice {Choice.Owned(token)=>token,Choice.Scalar(_)=>Token{value:0}}}",
        "fn invalid(value:&Holder)->Token {let result=match value.choice {Choice.Owned(token)=>token,Choice.Scalar(_)=>Token{value:0}}; result}",
        "fn invalid(value:&Holder)->Choice {match value.choice {owned=>owned}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(
            found.iter().any(|e| e.contains("out of borrowed storage")),
            "{body}: {found:?}"
        );
    }
}

#[test]
fn ref_payload_borrows_keep_their_owner_alive_and_exclude_mutation() {
    for body in [
        "fn invalid()->&Token {let local=Holder{choice:Choice.Owned(Token{value:73})}; match local.choice {Choice.Owned(ref token)=>token,Choice.Scalar(_)=>panic(\"empty\")}}",
        "fn invalid(value:&mut Holder)->Int {match value.choice {Choice.Owned(ref token)=>{value.choice=Choice.Scalar(0);token.value},Choice.Scalar(n)=>n}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(
            found.iter().any(|e| e.contains("borrow")
                || e.contains("does not live long enough")
                || e.contains("outlives")),
            "{body}: {found:?}"
        );
    }
}

#[test]
fn explicitly_borrowed_control_uses_the_same_payload_types() {
    let source = format!(
        "{TYPES} fn inspect(value:&Holder)->Int {{match &value.choice {{Choice.Scalar(n)=>*n,Choice.Owned(token)=>token.value}}}}"
    );
    let found = errors(&source);
    assert!(found.is_empty(), "{source}: {found:?}");
}

#[test]
fn ref_patterns_cannot_escape_computed_or_shadowed_local_storage() {
    for body in [
        "fn make()->Choice {Choice.Owned(Token{value:73})} fn invalid()->&Token {match make() {Choice.Owned(ref token)=>token,Choice.Scalar(_)=>panic(\"empty\")}}",
        "fn invalid()->&Token {let local=Holder{choice:Choice.Owned(Token{value:73})}; match local.choice {Choice.Owned(ref local)=>local,Choice.Scalar(_)=>panic(\"empty\")}}",
        "fn invalid(value:&Choice)->Token {match value {&Choice.Owned(token)=>token,&Choice.Scalar(_)=>Token{value:0}}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(!found.is_empty(), "{body}: {found:?}");
    }
}

#[test]
fn explicit_mutable_patterns_preserve_write_permission() {
    for reference in ["&", "&checked", "&unsafe"] {
        let body = format!(
            "fn invalid(value:{reference} Holder)->Int {{match value.choice {{Choice.Owned(ref mut token)=>{{token.value=0;token.value}},Choice.Scalar(n)=>n}}}}"
        );
        let found = errors(&format!("{TYPES} {body}"));
        assert!(
            found
                .iter()
                .any(|e| e.contains("shared") || e.contains("immutable")),
            "{body}: {found:?}"
        );
    }
}

#[test]
fn pure_discriminant_or_ref_inspection_does_not_consume_owned_subject() {
    for body in [
        "fn inspect(value:Choice)->Int {let first=match value {Choice.Owned(ref token)=>token.value,Choice.Scalar(n)=>n}; let second=match value {Choice.Owned(ref token)=>token.value,Choice.Scalar(n)=>n};first+second}",
        "fn inspect(value:&mut Holder)->Int {match value.choice {Choice.Owned(ref mut token)=>{token.value=73;token.value},Choice.Scalar(n)=>n}}",
        "fn make()->Choice {Choice.Owned(Token{value:73})} fn inspect()->Int {match make() {Choice.Owned(ref token)=>token.value,Choice.Scalar(n)=>n}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(found.is_empty(), "{body}: {found:?}");
    }
}

#[test]
fn an_owned_pattern_still_consumes_its_subject_once() {
    let source = format!(
        "{TYPES} fn invalid(value:Choice)->Int {{let token=match value {{Choice.Owned(token)=>token,Choice.Scalar(n)=>Token{{value:n}}}}; match value {{Choice.Owned(_)=>token.value,Choice.Scalar(n)=>n}}}}"
    );
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|e| e.contains("moved") || e.contains("consum")),
        "{source}: {found:?}"
    );
}

#[test]
fn distinct_mutable_payloads_keep_their_field_identity() {
    let source = "type Pair is Both(Int,Int); fn update(pair:&mut Pair)->Int {match pair {&Pair.Both(ref mut first,ref mut second)=>{*first=36;*second=37;*first+*second}}}";
    let found = errors(source);
    assert!(found.is_empty(), "{source}: {found:?}");
}

#[test]
fn coarsened_subjects_and_array_elements_keep_conflicting_writes_rejected() {
    for body in [
        "fn invalid(value:&mut Holder)->Int {match (*value).choice {Choice.Owned(ref token)=>{value.choice=Choice.Scalar(0);token.value},Choice.Scalar(n)=>n}}",
        "fn invalid(value:&mut [Holder;2],index:Int)->Int {match value[index].choice {Choice.Owned(ref token)=>{value[0].choice=Choice.Scalar(0);token.value},Choice.Scalar(n)=>n}}",
        "fn invalid(value:&mut [Int;1])->Int {match *value {[ref first]=>{value[0]=0;*first}}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(
            found.iter().any(|e| e.contains("borrow")),
            "{body}: {found:?}"
        );
    }
}

#[test]
fn a_ref_pattern_may_return_the_incoming_owners_payload() {
    let source = format!(
        "{TYPES} fn select(value:&Holder)->&Token {{match value.choice {{Choice.Owned(ref token)=>token,Choice.Scalar(_)=>panic(\"empty\")}}}}"
    );
    let found = errors(&source);
    assert!(found.is_empty(), "{source}: {found:?}");
}

#[test]
fn conditional_ref_bindings_borrow_the_subject_in_both_typing_modes() {
    for body in [
        "fn inspect(value:&Choice)->Int {if let Choice.Owned(ref token)=*value {token.value} else {0}}",
        "fn inspect(value:&Choice)->Int {let n=if let Choice.Owned(ref token)=*value {token.value} else {0};n}",
        "fn inspect(value:&Holder, other:&Holder)->Int {if let Choice.Owned(ref token)=value.choice && let Choice.Scalar(n)=other.choice && n>0 {token.value+n} else {0}}",
        "fn inspect(value:&Holder)->Int {if value.choice is Choice.Owned(ref token) && token.value>0 {token.value} else {0}}",
        "fn inspect(value:Choice)->Int {if let Choice.Owned(token)=value {return token.value;} if let Choice.Scalar(n)=value {n} else {0}}",
        "fn select(value:&Holder)->&Token {if let Choice.Owned(ref token)=value.choice {token} else {panic(\"missing\")}}",
        "fn inspect(value:&mut Holder)->Int {if let Choice.Owned(ref mut token)=value.choice {token.value=73;token.value} else {value.choice=Choice.Scalar(0);0}}",
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(found.is_empty(), "{body}: {found:?}");
    }
}

#[test]
fn conditional_payload_access_preserves_ownership_permissions_and_storage_lifetime() {
    for (body, reasons) in [
        ("fn invalid(value:&Choice)->Token {if let Choice.Owned(token)=*value {token} else {Token{value:0}}}", &["out of borrowed storage"][..]),
        ("fn invalid(value:&Holder)->Int {if let Choice.Owned(ref mut token)=value.choice {token.value=0;token.value} else {0}}", &["immutable", "shared"]),
        ("fn invalid(value:&mut Holder)->Int {if let Choice.Owned(ref token)=value.choice {value.choice=Choice.Scalar(0);token.value} else {0}}", &["borrow"]),
        ("fn invalid(value:&mut Holder,fallback:&Token)->Int {let selected=if let Choice.Owned(ref token)=value.choice {token} else {fallback};value.choice=Choice.Scalar(0);selected.value}", &["borrow"]),
        ("fn make()->Choice {Choice.Owned(Token{value:73})} fn invalid()->&Token {if let Choice.Owned(ref token)=make() {token} else {panic(\"missing\")}}", &["does not live long enough", "outlives", "escapes"]),
        ("fn inspect(t:Token)->Int {t.value} fn invalid(value:Choice)->Int {if let Choice.Owned(token)=value {inspect(token);} if let Choice.Owned(next)=value {next.value} else {0}}", &["moved", "consum"]),
    ] {
        let found = errors(&format!("{TYPES} {body}"));
        assert!(found.iter().any(|error| reasons.iter().any(|reason| error.contains(reason))), "{body}: {found:?}");
    }
}

#[test]
fn a_failed_later_guard_preserves_consumption_from_earlier_guards() {
    let source = format!(
        "{TYPES} fn positive(t:Token)->Bool {{t.value>0}} fn invalid(value:Choice, t:Token)->Int {{if let Choice.Scalar(n)=value && positive(t) {{n}} else {{t.value}}}}"
    );
    let found = errors(&source);
    assert!(found.iter().any(|error| error.contains("moved") || error.contains("consum")), "{found:?}");
}

#[test]
fn mutable_payload_patterns_use_the_declared_method_receiver_permission() {
    let inspect = |receiver: &str| errors(&format!("{TYPES} implement Holder {{
        fn inspect({receiver})->Int {{
            if let Choice.Owned(ref mut token)=self.choice {{token.value=73; token.value}} else {{0}}
        }}
    }}"));
    let valid = inspect("&mut self");
    assert!(valid.is_empty(), "{valid:?}");
    let invalid = inspect("&self");
    assert!(invalid.iter().any(|message| message.contains("shared") || message.contains("immutable")),
        "{invalid:?}");
}
