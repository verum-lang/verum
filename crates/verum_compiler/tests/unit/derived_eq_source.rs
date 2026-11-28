use super::*;
use crate::phases::semantic_analysis::SemanticAnalysisPhase;
use verum_vbc::{Value, interpreter::Interpreter};

const PROTOCOLS: &str = r#"@no_implicit_prelude module core.base.protocols;
public type PartialEq is protocol { fn eq(&self, other:&Self) -> Bool; };
public type Eq is protocol extends PartialEq {};
"#;

#[test]
fn structural_eq_checks_and_executes_the_declared_components() {
    let source = format!(
        r#"{PROTOCOLS}
        public type Other is protocol {{ fn eq(&self, other:&Self) -> Bool; }};
        public type Token is {{ key:Int, label:Int }};
        implement Other for Token {{ fn eq(&self, other:&Self) -> Bool {{ false }} }}
        implement PartialEq for Token {{ fn eq(&self, other:&Self) -> Bool {{
            if self.key == 99 {{ panic("comparison after an unequal field"); }}
            self.key == other.key
        }} }}
        implement Eq for Token {{}}
        @derive(Eq) type Key<T> is (T);
        @derive(PartialEq, Eq) type Pair is {{ first: Key<Token>, second: Token }};
        @derive(Eq) type Choice is Empty | Single(Token) | Named {{ value: Token }};
        @derive(Eq) type Signal<T> is ();
        @derive(Eq) type WrappedSignal<T> is (Signal<T>);
        type NoEquality is {{ value: Int }};
        fn equal<T:PartialEq>(a:&T,b:&T)->Bool {{ a.eq(b) }}
        fn total<T:Eq>(a:&T) -> Bool {{ true }}
        fn compare_signals(a:&WrappedSignal<NoEquality>, b:&WrappedSignal<NoEquality>)->Bool {{
            total(a) && equal(a, b)
        }}
        fn same_pair(a:&Pair, b:&Pair)->Bool {{ total(a) && equal(a, b) }}
        fn same_choice(a:&Choice, b:&Choice)->Bool {{ total(a) && equal(a, b) }}
        fn token(k:Int, l:Int)->Token {{ Token {{key:k, label:l}} }}
        fn pair(first:Int, second:Int, label:Int)->Pair {{
            Pair {{first:Key(token(first,label)), second:token(second,label)}}
        }}
        fn single(key:Int, label:Int)->Choice {{ Choice.Single(token(key,label)) }}
        fn named(key:Int, label:Int)->Choice {{ Choice.Named {{value:token(key,label)}} }}
        fn probe(which:Int) -> Bool {{
            match which {{
                0 => same_pair(&pair(7,7,4), &pair(7,7,9)),
                1 => same_pair(&pair(7,99,4), &pair(3,7,9)),
                2 => same_pair(&pair(7,7,4), &pair(7,3,9)),
                3 => same_choice(&Choice.Empty, &Choice.Empty),
                4 => same_choice(&single(7,4), &single(7,9)),
                5 => same_choice(&named(7,4), &named(3,9)),
                6 => same_choice(&Choice.Empty, &single(7,4)),
                7 => same_choice(&named(7,4), &single(7,4)),
                _ => same_choice(&named(7,4), &named(7,9))
            }}
        }}"#
    );
    let mut session = crate::Session::new(crate::CompilerOptions::default());
    let file = session
        .load_source_string(&source, std::path::PathBuf::from("protocols.vr"))
        .unwrap();
    let mut pipeline = crate::CompilationPipeline::new_interpreter(&mut session);
    let ast = pipeline.phase_parse(file).unwrap();
    let module = pipeline
        .compile_test_unit_to_vbc(ast)
        .expect("checked project lowering");
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    for module in [module, restored.into()] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let entry = module
            .find_function_by_name("core.base.protocols.probe")
            .unwrap();
        let mut vm = Interpreter::new(module);
        for (which, expected) in [true, false, false, true, true, false, false, false, true]
            .into_iter()
            .enumerate()
        {
            let value = vm
                .execute_function_with_args(entry, &[Value::from_i64(which as i64)])
                .unwrap_or_else(|error| panic!("case {which}: {error:?}"));
            assert_eq!(value.try_as_bool(), Some(expected), "case {which}");
        }
    }
}

#[test]
fn structural_eq_rejects_a_partial_only_field_in_the_owner_and_an_importer() {
    check_unproved_component(
        "Eq",
        "module app; mount core.base.protocols.PartialEq;
        public type PartialOnly is {value:Int};
        implement PartialEq for PartialOnly {fn eq(&self, other:&Self)->Bool {false}}
        implement spoof.Eq for PartialOnly {}
        @derive(Eq) public type Wrap is (PartialOnly);",
    );
}

#[test]
fn structural_partial_eq_rejects_an_inherent_method_in_the_owner_and_an_importer() {
    check_unproved_component(
        "PartialEq",
        "module app;
        public type Pretend is {value:Int};
        implement Pretend {fn eq(&self, other:&Self)->Bool {true}}
        @derive(PartialEq) public type Wrap is (Pretend);",
    );
}

fn check_unproved_component(protocol: &str, owner_source: &str) {
    let parser = verum_fast_parser::FastParser::new();
    let client = format!(
        "module client; mount core.base.protocols.{protocol}; mount app.Wrap;
        fn require<T:{protocol}>(a:&T)->Int {{ 1 }} fn probe(a:&Wrap)->Int {{ require(a) }}"
    );
    let declarations = [
        ("core.base.protocols", PROTOCOLS),
        ("spoof", "module spoof; public type Eq is protocol {};"),
        ("app", owner_source),
        ("client", client.as_str()),
    ];
    let parsed: List<_> = declarations
        .iter()
        .enumerate()
        .map(|(index, (owner, source))| {
            (
                *owner,
                parser
                    .parse_module_str(source, FileId::new(index as u32))
                    .unwrap(),
            )
        })
        .collect();
    let refs: List<_> = parsed
        .iter()
        .map(|(owner, ast)| (*owner, *owner, ast))
        .collect();
    let selected = prepare_source_roster(&refs, &TargetConfig::host(), true).unwrap();
    let graph =
        source_registry(selected.iter().map(|(o, p, m)| (o.as_str(), p.as_str(), m))).unwrap();
    let phase = SemanticAnalysisPhase::new();
    for (owner, _, ast) in selected.iter().skip(2) {
        let errors = check_source_module(&mut phase.configured_checker(), owner, ast, &graph)
            .expect_err("a field needs the selected nominal protocol");
        let expected = format!("core::base::protocols::{protocol}");
        assert!(
            errors
                .iter()
                .any(|e| e.code() == Some("E405") && e.message().contains(expected.as_str())),
            "{owner}: {errors:?}"
        );
    }
}

#[test]
fn structural_eq_rejects_a_competing_handwritten_comparison() {
    let source = format!(
        "{PROTOCOLS}\n@derive(Eq) type Key is ();\n\
        implement PartialEq for Key {{ fn eq(&self, other:&Self)->Bool {{false}} }}"
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let owner = "core.base.protocols";
    let selected = prepare_source_roster(
        &[(owner, "protocols.vr", &ast)],
        &TargetConfig::host(),
        true,
    )
    .unwrap();
    let graph = source_registry([(owner, "protocols.vr", &selected[0].2)]).unwrap();
    let errors = check_source_module(
        &mut SemanticAnalysisPhase::new().configured_checker(),
        owner,
        &selected[0].2,
        &graph,
    )
    .expect_err("two comparisons for the same type cannot provide derived reflexivity");
    assert!(
        errors
            .iter()
            .any(|error| error.message().contains("PartialEq") && error.message().contains("Key")),
        "{errors:?}"
    );
}
