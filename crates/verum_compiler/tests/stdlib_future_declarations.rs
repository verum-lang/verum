//! Actual archive bootstrap preserves Future ownership across file modules.
use std::fs;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_vbc::{Instruction, archive::read_archive_from_file, bytecode::decode_instructions};

#[test]
fn actual_platform_queries_survive_archive_bootstrap_and_target_rebinding() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(core.join("intrinsics")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module intrinsics { public module platform; }").unwrap();
    // A focused bootstrap of the two actual definitions, not a full-stdlib claim.
    let mut source = "module core.intrinsics.platform;\n".to_owned();
    let mut count = 0;
    for line in include_str!("../../../core/intrinsics/platform.vr").lines() {
        if line.starts_with("public meta fn target_has_atomic") || line.starts_with("public meta fn target_has_feature") {
            source.push_str(line);
            source.push('\n');
            count += 1;
        }
    }
    assert_eq!(count, 2);
    fs::write(core.join("intrinsics/platform.vr"), source).unwrap();
    let archive = bootstrap(&core);
    for (bits, expected) in [(0, 19), (64, 73)] {
        let facts = verum_vbc::target::TargetCapabilities { max_atomic_bits: bits, pointer_bits: 64, features: Default::default() };
        assert_eq!(execute_with_target(&archive,
            "module app; mount core.intrinsics.platform.{target_has_atomic, target_has_feature};
             fn main() -> Int { if target_has_atomic<UInt64>() && !target_has_feature(\"unknown-cpu-feature\") { 73 } else { 19 } }",
            Some(std::sync::Arc::new(facts))), expected);
    }
}

#[test]
fn actual_layout_queries_survive_archive_bootstrap_and_generic_calls() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(core.join("intrinsics")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module intrinsics { public module type_info; }").unwrap();
    let mut source = "module core.intrinsics.type_info;\n".to_owned();
    let mut count = 0;
    for line in include_str!("../../../core/intrinsics/type_info.vr").lines() {
        if ["size_of", "align_of", "stride_of", "bits_of"].iter()
            .any(|name| line.starts_with(&format!("public meta fn {name}<"))) {
            source.push_str(line);
            source.push('\n');
            count += 1;
        }
    }
    assert_eq!(count, 4);
    fs::write(core.join("intrinsics/type_info.vr"), source).unwrap();
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive,
        "module app; mount core.intrinsics.type_info.{size_of, align_of, stride_of, bits_of};
         type Word is UInt16;
         fn inspect<T>() -> USize { size_of<T>() + align_of<T>() + stride_of<T>() + bits_of<T>() }
         fn main() -> Int { inspect<Word>() as Int }"), 22);
}

#[test]
fn actual_identity_query_survives_archive_bootstrap_and_generic_calls() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(core.join("intrinsics")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module intrinsics { public module type_info; }").unwrap();
    let declaration = include_str!("../../../core/intrinsics/type_info.vr").lines()
        .find(|line| line.starts_with("public meta fn type_id<")).unwrap();
    fs::write(core.join("intrinsics/type_info.vr"), format!("module core.intrinsics.type_info;\n{declaration}\n")).unwrap();
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive,
        "module app; mount core.intrinsics.type_info.type_id;
         type Tag<T> is {value:T}; type Alias<T> is Tag<T>;
         fn property<T>() -> UInt64 { T.id }
         fn main() -> Int {
            if type_id<Tag<Int>>() == property<Alias<Int>>() && type_id<Tag<Int>>() != type_id<Tag<Bool>>() {73} else {19}
         }"), 73);
}

#[test]
fn actual_identity_documentation_example_checks_and_executes() {
    let source = include_str!("fixtures/type_identity.vr");
    let module = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut checker = verum_types::TypeChecker::new();
    checker.register_builtins();
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for item in &module.items { checker.check_item(item).unwrap(); }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(!checker.diagnostics().iter().any(|diagnostic| diagnostic.is_error()));
    let module = verum_vbc::codegen::VbcCodegen::new().compile_module(&module).unwrap();
    let main = module.find_function_by_name("main").unwrap();
    verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module)).execute_function(main).unwrap();
}

#[test]
fn imported_future_outputs_and_contextual_owner_arguments_survive_bootstrap() {
    check_bootstrap(false);
}

#[test]
fn glob_imported_future_output_keeps_declaration_authority() {
    check_bootstrap(true);
}

#[test]
fn imported_fixed_width_arithmetic_executes_after_archive_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module values; public module scalars; public module reader;
            public module prelude { public mount super.values.*; public mount super.scalars.*; }"),
        ("values/mod.vr", "module core.values; public type Maybe<T> is None | Some(T);"),
        ("scalars/mod.vr", "module core.scalars; mount core.values.Maybe;
            implement UInt64 {
                public fn checked_add(self, rhs: UInt64) -> Maybe<UInt64> {
                    @intrinsic(\"checked_add_u64\", self, rhs)
                }
            }"),
        ("reader/mod.vr", "module core.reader;
            public fn read() -> Int {
                let maximum: UInt64 = 18446744073709551615;
                let ordinary: UInt64 = 36;
                let sum = match ordinary.checked_add(37) { Maybe.Some(n) => n as Int, Maybe.None => 0 };
                match maximum.checked_add(1) { Maybe.None => sum, Maybe.Some(_) => 0 }
            }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.reader.read; fn main() -> Int { read() }"), 73);
}

#[test]
fn named_constant_patterns_execute_the_declared_values() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(&core).unwrap();
    fs::write(core.join("mod.vr"), "module core;
        const FIRST: Int = 3; const SECOND: Int = 5;
        public fn classify(value: Int) -> Int {
            match value { FIRST => 17, SECOND => 23, _ => 0 }
        }
        public fn either(value: Int) -> Int {
            match value { FIRST | SECOND => 11, _ => 0 }
        }").unwrap();
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.{classify, either};
        fn main() -> Int { classify(3) + classify(5) + classify(7) + either(3) + either(5) + either(7) }"), 62);
}

#[test]
fn relative_record_constructors_execute_with_the_declaring_layout() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module values; public module other; public module reader;"),
        ("values/mod.vr", "module core.values; public type Packet is { public body: Int };"),
        ("other/mod.vr", "module core.other; public type Packet is { body: Bool };"),
        ("reader/mod.vr", "module core.reader; public fn read() -> Int {
            let first = super.values.Packet { body: 36 };
            let second: super.values.Packet = super.values.Packet { body: 37 };
            first.body + second.body
        }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.reader.read; fn main() -> Int { read() }"), 73);
    fs::write(core.join("reader/mod.vr"), "module core.reader; public fn read() -> Bool {
        let value = super.other.Packet { body: true }; value.body
    }").unwrap();
    let error = format!("{:#}", try_bootstrap(&core).err().expect("private record constructed"));
    assert!(error.contains("E202") && error.contains("body") && error.contains("Private"), "{error}");
}

#[test]
fn iteration_loans_end_or_escape_with_their_actual_values() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir_all(core.join("reader")).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module reader;").unwrap();
    let reader = core.join("reader/mod.vr");
    let source = "module core.reader; public fn read() -> Int {
        let mut values = [1, 2];
        for item in &values { let _read = *item; }
        values[0] = 71;
        let spare = 0; let mut held = &spare;
        for item in &values { held = item; }
        values[0] + *held
    }
    public fn update_after_iteration() -> Int {
        let mut values = [1, 2];
        for item in &values { let _read = *item; }
        values[0] = 7;
        values[0]
    }";
    fs::write(&reader, source).unwrap();
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.reader.read; fn main() -> Int { read() }"), 73);
    assert_eq!(execute(&archive, "module app; mount core.reader.update_after_iteration; fn main() -> Int { update_after_iteration() }"), 7);
    fs::write(&reader, source.replace("values[0] + *held", "values[0] = 90; *held")).unwrap();
    let error = format!("{:#}", try_bootstrap(&core).err().expect("escaped loan released"));
    assert!(error.contains("E310") && error.contains("values"), "{error}");
}

#[test]
fn imported_associated_sum_survives_nested_async_signature_and_execution() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module values; public module feed; public module consumer;"),
        ("values/mod.vr", "module core.values;
            public type Choice<T> is Empty | Present(T);
            public type Outcome<T, E> is Success(T) | Failure(E);"),
        ("feed/mod.vr", "module core.feed; mount core.values.Choice;
            public type Feed is protocol { type Item; async fn next(&mut self) -> Choice<Self.Item>; };"),
        ("consumer/mod.vr", "module core.consumer;
            mount core.values.{Choice, Outcome}; mount core.feed.Feed;
            public type Source is { value: Int };
            implement Feed for Source {
                type Item = Outcome<Int, Bool>;
                async fn next(&mut self) -> Choice<Outcome<Int, Bool>> {
                    Choice.Present(Outcome.Success(self.value))
                }
            }
            public async fn read() -> Int {
                let mut source = Source { value: 73 };
                match source.next().await {
                    Choice.Present(Outcome.Success(value)) => value,
                    _ => 0,
                }
            }"),
    ] {
        let file = core.join(file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive,
        "module app; mount core.consumer.read; async fn main() -> Int { read().await }"), 73);
    let file = core.join("consumer/mod.vr");
    let source = fs::read_to_string(&file).unwrap();
    fs::write(file, source.replace("type Item = Outcome<Int, Bool>", "type Item = Outcome<Bool, Int>")).unwrap();
    let error = format!("{:#}", try_bootstrap(&core).err().expect("wrong associated carrier accepted"));
    assert!(error.contains("E427") && error.contains("next"), "{error}");
}

#[test]
fn inferred_constructor_carriers_survive_archive_bootstrap_and_execution() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module values; public module reader;"),
        ("values/mod.vr", "module core.values;
            public type Event<T> is Empty | Value(T) | Pair(T, T);
            public type Other<T> is Empty | Value(T) | Pair(T, T);
            public type Same<T> is protocol {};
            implement<T> Same<T> for T {}
            public fn same<A, B: Same<A>>(a: A, b: B) -> B { b }"),
        ("reader/mod.vr", "module core.reader; mount core.values.{Event, Other, same};
            fn score_empty(value: Event<Int>) -> Int { match value { Event.Empty => 2, _ => 0 } }
            type Input is { count: Int };
            implement Input {
                fn map<R>(self, f: fn(Int) -> R) -> R { f(self.count) }
            }
            public fn read() -> Int {
                let expected: Event<Int> = Event.Empty;
                let input = Input { count: 19 };
                let value = input.map(|n| Event.Value(n));
                let pair = Event.Pair(23, 29);
                let empty = Event.Empty;
                let first = match same(expected, value) { Event.Value(n) => n, _ => 0 };
                let second = match same(expected, pair) { Event.Pair(a, b) => a + b, _ => 0 };
                let third = score_empty(empty);
                first + second + third
            }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.reader.read; fn main() -> Int { read() }"), 73);
    let file = core.join("reader/mod.vr");
    let source = fs::read_to_string(&file).unwrap();
    fs::write(&file, source.replace("Event.Value(n)", "Other.Value(n)")).unwrap();
    let error = format!("{:#}", try_bootstrap(&core).err().expect("different nominal owner accepted"));
    assert!(error.contains("Same"), "{error}");
}

#[test]
fn inline_reexport_matches_the_declaring_generic_sum_after_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        (
            "mod.vr",
            "module core; public module values; public module reader;
             public module surface { public mount super.values.*; }",
        ),
        (
            "values/mod.vr",
            "module core.values; public type Choice<T> is Missing | Found(T);",
        ),
        (
            "reader/mod.vr",
            "module core.reader; mount core.{Choice};
             fn read(value: core.values.Choice<Int>) -> Int {
                 match value { Choice.Missing => 3, Choice.Found(n) => n }
             }
             public fn check_both() -> Int { read(Choice.Missing) + read(Choice.Found(8)) }",
        ),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(
        execute(&archive, "module app; mount core.reader.check_both; fn main() -> Int { check_both() }"),
        11
    );
}

#[test]
fn cyclic_mounts_bind_the_exact_declared_sum_constructor_before_its_body() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module model; public module reader;"),
        ("model/mod.vr", "module core.model; public module either; public module decoy;
             mount core.reader.control.anchor; public fn anchor_value() -> Int { anchor() }"),
        ("model/either.vr", "module core.model.either; public type Choice<T> is Missing | Found(T);"),
        ("model/decoy.vr", "module core.model.decoy; public type Choice<T> is Found(T) | Missing;"),
        ("reader/mod.vr", "module core.reader; public module control;"),
        ("reader/control.vr", "module core.reader.control;
             mount core.model.either.{Choice, Missing, Found};
             public fn anchor() -> Int { 41 }
             fn score(value: Choice<Int>) -> Int {
                 match value { Choice.Missing => 7, Choice.Found(n) => n }
             }
             public fn both() -> Int { score(Missing) + score(Found(anchor())) }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive, "module app; mount core.reader.control.both;
        fn main() -> Int { both() }"), 48);

    let source = core.join("reader/control.vr");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(&source, text.replace("Choice, Missing, Found", "Choice, Missing, Nonexistent as Found")).unwrap();
    let error = format!("{:#}", try_bootstrap(&core).unwrap_err());
    assert!(error.contains("core.model.either.Nonexistent"), "{error}");
}

#[test]
fn a_renamed_sum_owns_its_nullary_and_payload_constructors() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module protocol; public module reader; public module decoy;"),
        ("protocol/mod.vr", "module core.protocol; public type ActualError is Corrupt(Int) | Partial;"),
        ("decoy/mod.vr", "module core.decoy; public type SocketIssue is Partial | Corrupt(Int) | Other | Extra(Int);"),
        ("reader/mod.vr", "module core.reader;
            mount core.protocol.{ActualError as ParseError};
            public type Envelope is Held(ParseError);
            fn code(envelope: Envelope) -> Int {
                match envelope { Envelope.Held(value) => match value {
                    ParseError.Partial => 41, ParseError.Corrupt(n) => n } }
            }
            public fn both() -> Int {
                code(Envelope.Held(ParseError.Partial)) + code(Envelope.Held(ParseError.Corrupt(7)))
            }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let path = core.join("reader/mod.vr");
    let source = fs::read_to_string(&path).unwrap();
    for qualifier in ["ParseError", "core.protocol.ActualError"] {
        fs::write(&path, source.replace("ParseError.", &format!("{qualifier}."))).unwrap();
        let archive = bootstrap(&core);
        let module = archive.load_module("core.reader").unwrap();
        let id = module.find_function_by_name("core.reader.both").unwrap();
        let both = module.functions.iter().find(|function| function.id == id).unwrap();
        let start = both.bytecode_offset as usize;
        let code = decode_instructions(&module.bytecode[start..start + both.bytecode_length as usize]).unwrap();
        let constructors: Vec<_> = code.into_iter().filter_map(|op| {
            let Instruction::MakeVariantTyped { type_id, tag, field_count, .. } = op else { return None };
            let ty = module.get_type(verum_vbc::types::TypeId(type_id)).unwrap();
            let owner = ty.origin_module.and_then(|id| module.get_string(id))?;
            (owner == "core.protocol" && module.get_string(ty.name) == Some("ActualError"))
                .then_some((tag, field_count))
        }).collect();
        assert_eq!(constructors, [(1, 0), (0, 1)]);
        let caller = "module app; mount core.reader.both; fn main() -> Int { both() }";
        assert_eq!(execute(&archive, caller), 48);

        for (valid, invalid, name) in [("ParseError.Partial))", "ParseError.Other))", "Other"),
            ("ParseError.Corrupt(7)", "ParseError.Extra(7)", "Extra")] {
            fs::write(&path, source.replace(valid, invalid).replace("ParseError.", &format!("{qualifier}."))).unwrap();
            let rejection = try_bootstrap(&core).err().unwrap_or_else(|| panic!("accepted {invalid}"));
            let error = format!("{rejection:#}");
            assert!(error.contains(name) && error.contains("ActualError"), "{error}");
        }
    }
}

#[test]
fn a_protocol_default_resolves_bare_constructors_in_its_declaration() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module api; public module consumer; public module decoy;"),
        ("api/mod.vr", "module core.api;
             public type Position is End | Start(Int);
             public type Seek is protocol {
                 fn seek(&mut self, p: Position) -> Int;
                 fn rewind(&mut self) -> Int { self.seek(Start(41)) }
             }"),
        ("decoy/mod.vr", "module core.decoy; public type Noise is Start(Int) | End;"),
        ("consumer/mod.vr", "module core.consumer; mount core.api.{Seek, Position};
             public type Cursor is { value: Int };
             implement Seek for Cursor {
                 fn seek(&mut self, p: Position) -> Int {
                     match p { Position.End => 99, Position.Start(n) => n }
                 }
             }
             public fn run_cursor() -> Int { let mut c = Cursor { value: 0 }; c.rewind() }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    assert_eq!(execute(&bootstrap(&core),
        "module app; mount core.consumer.run_cursor; fn main() -> Int { run_cursor() }"), 41);
}

#[test]
fn transitive_callable_facades_cross_units_without_losing_the_declaring_body() {
    for (public_mount, public_function) in [(true, true), (false, true), (true, false)] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("core");
        for (file, source) in [
            ("mod.vr", "module core; public module work;"),
            ("work/mod.vr", "module core.work; public module app; public module decoy;
                 public mount .app.{publish as run};
                 public module prelude { public mount super.run; }"),
            ("work/decoy.vr", "module core.work.decoy;
                 public fn run() -> Int { 17 } public fn publish() -> Int { 19 }"),
            ("work/app/mod.vr", if public_mount {
                 "module core.work.app; public module engine; public mount .engine.{actual as publish};"
             } else {
                 "module core.work.app; public module engine; mount .engine.{actual as publish};"
             }),
            ("work/app/engine.vr", if public_function {
                 "module core.work.app.engine; public fn actual() -> Int { 41 }"
             } else {
                 "module core.work.app.engine; fn actual() -> Int { 41 }"
             }),
        ] {
            let path = core.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        if public_mount && public_function {
            let archive = bootstrap(&core);
            let module = archive.load_module("core.work").unwrap();
            for path in ["core.work.run", "core.work.prelude.run"] {
                let targets: std::collections::BTreeSet<_> = module.mount_aliases.iter()
                    .filter(|(alias, _, _)| module.get_string(*alias) == Some(path))
                    .map(|(_, _, target)| module.get_string(*target).unwrap())
                    .collect();
                assert_eq!(targets, ["core.work.app.engine.actual"].into_iter().collect(), "{path}");
            }
            assert_eq!(
                execute(&archive, "module app; mount core.work.prelude.run; fn main() -> Int { run() }"),
                41
            );
        } else {
            let error = format!("{:#}", try_bootstrap(&core).unwrap_err());
            assert!(error.contains("invalid source mount") && error.contains("core.work.app.publish"), "{error}");
        }
    }
}

#[test]
fn an_owning_unit_completes_its_source_with_the_declared_prelude_owner() {
    for origin in ["model/origin.vr", "model/origin/mod.vr"] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("core");
        for (file, source) in [
            ("mod.vr", "module core; public module model;
                 public module prelude { public mount core.model.origin.Node; }"),
            (origin, "module core.model.origin; public type Node is { value: Int };
                 public fn make() -> Node { Node { value: 47 } }"),
            ("model/mod.vr", "module core.model; public module origin; public module decoy;
                 public module unit; public module consumer;"),
            ("model/decoy.vr", "module core.model.decoy; public type Node is Other(Int);"),
            ("model/unit.vr", "module core.model.unit;
                 public type Carrier is { witness: Type, node: Node };
                 public fn read(node: Node) -> Int { node.value }"),
            ("model/consumer.vr", "module core.model.consumer;
                 mount core.model.unit.{Carrier, read};
                 public fn keep(x: Carrier) -> Carrier { x }
                 public fn probe() -> Int { read(core.model.origin.make()) }"),
        ] {
            let path = core.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }

        let archive = bootstrap(&core);
        let model = archive.load_module("core.model").unwrap();
        let bindings = model.declaration_type_bindings.get(&"core.model.unit".into()).unwrap();
        assert_eq!(bindings.get(&"Node".into()).unwrap().as_deref(), Some("core.model.origin.Node"));
        let carrier = model.types.iter().find(|ty| model.get_string(ty.name) == Some("Carrier")).unwrap();
        let node = model.get_type(carrier.fields[1].type_ref.base_type_id().unwrap()).unwrap();
        let owner = node.origin_module.and_then(|id| model.get_string(id)).unwrap();
        assert_eq!(owner, "core.model.origin");
        assert_eq!(
            verum_vbc::module::qualify_module_name(owner, model.get_string(node.name).unwrap()),
            "core.model.origin.Node"
        );
        assert_eq!(execute(&archive, "module app; mount core.model.consumer.probe; fn main() -> Int { probe() }"), 47);
    }
}

#[test]
fn sibling_private_functions_keep_their_call_and_result_owners() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let codecs = core.join("codecs");
    fs::create_dir_all(&codecs).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module codecs;").unwrap();
    fs::write(codecs.join("mod.vr"), "module core.codecs; public module left; public module right;").unwrap();
    for (file, sum, first, second, value) in [
        ("left", "Amber", "Kept", "Gone", 3),
        ("right", "Blue", "Seen", "Absent", 4),
    ] {
        fs::write(codecs.join(format!("{file}.vr")), format!(
            "module core.codecs.{file};
             public type {sum} is {first}(Int) | {second};
             fn decode_item() -> {sum} {{ {sum}.{first}({value}) }}
             public fn read_{file}() -> Int {{
                 let item = decode_item();
                 match item {{ {sum}.{first}(x) => x, {sum}.{second} => 0 }}
             }}"
        )).unwrap();
    }
    let archive = bootstrap(&core);
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.codecs.left.read_left; mount core.codecs.right.read_right;
             fn main() -> Int { read_left() * 10 + read_right() }",
        ),
        34
    );
}

#[test]
fn a_callback_bound_owns_its_result_in_an_imported_protocol_default() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for module in ["rules", "consumer", "decoy"] {
        fs::create_dir_all(core.join(module)).unwrap();
    }
    fs::write(core.join("mod.vr"), "module core; public module rules; public module consumer; public module decoy;").unwrap();
    fs::write(core.join("rules/mod.vr"), r#"
        module core.rules;
        public type Decision is Smaller | Equal | Larger;
        public type Walk is protocol {
            type Item;
            fn next(&mut self) -> Self.Item;
            fn check<F: fn(&Self.Item, &Self.Item) -> Decision>(mut self, cmp: F) -> Bool {
                let a = self.next();
                let b = self.next();
                match cmp(&a, &b) { Decision.Larger => false, _ => true }
            }
        }
    "#).unwrap();
    fs::write(core.join("consumer/mod.vr"), r#"
        module core.consumer;
        mount core.rules.Walk;
        public type Walker is { item: Int };
        implement Walk for Walker {
            type Item = Int;
            fn next(&mut self) -> Int { self.item }
        }
    "#).unwrap();
    fs::write(core.join("decoy/mod.vr"), r#"
        module core.decoy;
        public fn cmp(a: &Int, b: &Int) -> Int { 42 }
        public fn callback(a: &Int, b: &Int) -> Int { 43 }
    "#).unwrap();
    let app = r#"
        module app;
        mount core.consumer.Walker;
        mount core.rules.Decision;
        fn main() -> Int {
            let low = Walker { item: 1 };
            let high = Walker { item: 2 };
            if low.check(|a, b| Decision.Smaller) && !high.check(|a, b| Decision.Larger) {
                17
            } else { 0 }
        }
    "#;

    let rules_path = core.join("rules/mod.vr");
    let inline = fs::read_to_string(&rules_path).unwrap();
    let predicate = inline
        .replace("check<F: fn(&Self.Item, &Self.Item) -> Decision>", "check<F>")
        .replace("-> Bool {", "-> Bool where type F = fn(&Self.Item, &Self.Item) -> Decision {")
        .replace("match cmp(", "let callback = cmp; match callback(");
    for valid in [inline, predicate] {
        fs::write(&rules_path, &valid).unwrap();
        assert_eq!(execute(&bootstrap(&core), app), 17);
        for wrong_variant in ["Wrong.Larger", "Wrong.No"] {
            let invalid = valid.replace("Decision.Larger =>", &format!("{wrong_variant} =>"));
            fs::write(&rules_path, invalid + "\npublic type Wrong is No | Larger;").unwrap();
            let error = try_bootstrap(&core).err().expect("foreign enum pattern must fail");
            let diagnostic = format!("{error:#}");
            assert!(diagnostic.contains(&format!(
                "no declared tag for variant pattern '{}' on Some(\"core.rules.Decision\")",
                wrong_variant.replace('.', "::")
            )), "{diagnostic}");
        }
    }
}

#[test]
fn an_earlier_consumer_cannot_change_a_forward_method_result() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let db = core.join("db");
    let ops = db.join("ops");
    fs::create_dir_all(&ops).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module db;").unwrap();
    fs::write(db.join("mod.vr"),
        "module core.db; public module ops;
         mount core.db.ops.declarations.{Pool,Entry};
         public fn peek(p: &Pool) -> Entry { p.get(0) }").unwrap();
    fs::write(ops.join("mod.vr"),
        "module core.db.ops; public module declarations; public module actions;").unwrap();
    fs::write(ops.join("declarations.vr"),
        "module core.db.ops.declarations;
         public type Other is KBool(Bool) | KInt(Int);
         public type Entry is KInt(Int) | KBool(Bool);
         public type Pool is { stored: Entry };
         implement Pool { public fn get(&self, idx: Int) -> Entry { self.stored } }
         public type Program is { constants: Pool };").unwrap();
    fs::write(ops.join("actions.vr"),
        "module core.db.ops.actions; mount core.db.ops.declarations.{Program,Pool,Entry};
         public fn step(p: &Program) -> Int {
             match p.constants.get(0) { KInt(i) => i, KBool(_) => 1 }
         }
         public fn probe() -> Int {
             let p = Program { constants: Pool { stored: Entry.KInt(49) } };
             step(&p)
         }").unwrap();
    let archive = bootstrap(&core);
    assert_eq!(execute(&archive,
        "module app; mount core.db.ops.actions.probe; fn main() -> Int { probe() }"), 49);
}

fn check_bootstrap(glob: bool) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for directory in ["values", "scheduler", "network"] {
        fs::create_dir_all(core.join(directory)).unwrap();
    }
    fs::write(core.join("mod.vr"), "module core; public module values; public module scheduler; public module network; public mount .values.Maybe;").unwrap();
    fs::write(
        core.join("values/mod.vr"),
        r#"
module core.values;
public type Result<T, E> is Ok(T) | Err(E);
public type Maybe<T> is None | Some(T);
public type StreamError is { code: Int };
public type IoError is StreamError;
public type IoResult<T> is Result<T, StreamError>;
"#,
    )
    .unwrap();
    fs::write(
        core.join("scheduler/mod.vr"),
        "module core.scheduler; public module future; public module select;",
    )
    .unwrap();
    fs::write(
        core.join("scheduler/future.vr"),
        r#"
module core.scheduler.future;
public type Poll<T> is Ready(T) | Pending;
public type Waker is { marker: Int };
public type Context is { waker: &Waker };
implement Context { public fn from_waker(waker: &Waker) -> Context { Context { waker: waker } } }
public fn noop_waker() -> Waker { Waker { marker: 0 } }
public type Thread is ();
implement Thread { public fn yield_now() {} }
public type Future is protocol {
    type Output;
    fn poll(&mut self, cx: &mut Context) -> Poll<Self.Output>;
};
public type Complete<T> is { value: T };
implement<T> Future for Complete<T> {
    type Output = T;
    fn poll(&mut self, _cx: &mut Context) -> Poll<T> { Poll.Ready(self.value) }
}
public fn complete<T>(value: T) -> Complete<T> { Complete { value: value } }
"#,
    )
    .unwrap();
    fs::write(core.join("scheduler/select.vr"), r#"
module core.scheduler.select;
mount core.scheduler.future.{Future, Poll, Context, Waker, Thread, noop_waker};
mount core.Maybe;
mount core.values.{IoResult, Result};
public type WriteValue<T> is { payload: T, count: Int };
implement<T> Future for WriteValue<T> {
    type Output = IoResult<Int>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<IoResult<Int>> {
        Poll.Ready(Result.Ok(self.count))
    }
}
public type Tagged<P, T> is { value: Maybe<T> };
implement<P, T> Future for Tagged<P, T> {
    type Output = Maybe<P>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Maybe<P>> { Poll.Ready(Maybe.None) }
}
public async fn choose<X, P, T>(x: X, value: T) -> Maybe<P> { Tagged { value: Maybe.Some(value) }.await }
"#).unwrap();
    fs::write(
        core.join("network/mod.vr"),
        "module core.network; public module tcp;",
    )
    .unwrap();
    fs::write(core.join("network/tcp.vr"), r#"
module core.network.tcp;
mount core.scheduler.future.{Future, Poll, Context, Waker, Thread, noop_waker};
mount core.values.{Result, IoError};
mount core.scheduler.select.WriteValue;
public type AcceptFuture<'a> is { value: &'a Int };
implement<'a> Future for AcceptFuture<'a> {
    type Output = Result<(Int, Int), IoError>;
    fn poll(&mut self, _cx: &mut Context) -> Poll<Result<(Int, Int), IoError>> {
        Poll.Ready(Result.Ok((*self.value, 9)))
    }
}
public async fn forward() -> Int {
    let value = core.scheduler.future.complete(37).await;
    let write: WriteValue<Int> = WriteValue { payload: value, count: value };
    select { result = write.await => { match result { Result.Ok(count) => count, _ => 0 } } }
}
public async fn accept(value: &Int) -> Result<(Int, Int), IoError> { AcceptFuture { value: value }.await }
"#).unwrap();
    if glob {
        let path = core.join("network/tcp.vr");
        let source = fs::read_to_string(&path).unwrap().replace(
            "mount core.values.{Result, IoError};",
            "mount core.values.*;",
        );
        fs::write(path, source).unwrap();
    }
    let archive = bootstrap(&core);
    for (module, name) in [
        ("core.scheduler", "choose"),
        ("core.network", "accept"),
        ("core.network", "forward"),
    ] {
        let module = archive.load_module(module).unwrap();
        let function = module
            .functions
            .iter()
            .find(|f| {
                module
                    .strings
                    .get(f.name)
                    .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
            })
            .unwrap();
        let code = decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        let panics: Vec<_> = code
            .iter()
            .filter_map(|op| match op {
                Instruction::Panic { message_id } => {
                    module.strings.get(verum_vbc::StringId(*message_id))
                }
                _ => None,
            })
            .collect();
        assert!(
            function.has_source_body && panics.is_empty(),
            "{name}: {panics:?}"
        );
    }
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.network.tcp.forward; async fn main() -> Int { forward().await }",
        ),
        37
    );
}

fn bootstrap(core: &std::path::Path) -> verum_vbc::archive::VbcArchive {
    try_bootstrap(core).unwrap()
}

fn try_bootstrap(core: &std::path::Path) -> anyhow::Result<verum_vbc::archive::VbcArchive> {
    let output = core.parent().unwrap().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(core).with_output(&output))
            .compile_core()?;
    anyhow::ensure!(result.errors.is_empty(), "{:?}", result.errors);
    Ok(read_archive_from_file(&output)?)
}

fn execute(archive: &verum_vbc::archive::VbcArchive, source: &str) -> i64 {
    execute_with_target(archive, source, None)
}

fn execute_with_target(archive: &verum_vbc::archive::VbcArchive, source: &str,
    target: Option<std::sync::Arc<verum_vbc::target::TargetCapabilities>>) -> i64 {
    let app = verum_fast_parser::Parser::new(source)
        .parse_module()
        .unwrap();
    let mut codegen =
        verum_vbc::codegen::VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("app"));
    verum_compiler::archive_ctx_loader::ArchiveCtxCache::new(&archive).apply_lazy_with_types(&mut codegen,
        &app,
        &[],
    ).expect("valid archive");
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let app = codegen.finalize_module_from_state().unwrap();
    let main = app.find_function_by_name("app.main").unwrap();
    let mut interpreter = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(app));
    interpreter.state.config.target_capabilities = target;
    let value = interpreter.execute_function(main).unwrap();
    assert!(value.is_int());
    value.as_i64()
}

#[test]
fn cyclic_dependency_completes_owned_variant_and_record_fields() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (path, source) in [
        (
            "mod.vr",
            "module core; public module values; public module workers; public module clock; public module coordination;",
        ),
        (
            "values/mod.vr",
            "module core.values; public type Maybe<T> is None | Some(T); public type Slots<T> is { len: Int }; implement<T> Slots<T> { public fn empty() -> Slots<T> { Slots { len: 0 } } }",
        ),
        (
            "coordination/mod.vr",
            "module core.coordination; public type Mutex<T> is { data: T }; implement<T> Mutex<T> { public fn new(data: T) -> Mutex<T> { Mutex { data: data } } }",
        ),
        (
            "clock/mod.vr",
            "module core.clock; public module instant; public mount .instant.{Instant};",
        ),
        (
            "clock/instant.vr",
            r#"
            module core.clock.instant;
            mount core.workers.cancellation.CancelReason;
            public type Instant is { nanos: Int };
            implement Instant { public fn record_reason(&self, _reason: CancelReason) -> Int { self.nanos } }
        "#,
        ),
        (
            "workers/mod.vr",
            "module core.workers; public module cancellation;",
        ),
        (
            "workers/cancellation.vr",
            r#"
            module core.workers.cancellation;
            mount core.clock.Instant;
            mount core.coordination.Mutex;
            mount core.values.{Maybe, Slots};
            public type CancelReason is Cancelled | Timeout { deadline: Instant };
            public type Flag is { reason: Mutex<Maybe<CancelReason>>, observed: Mutex<Slots<Instant>> };
            public fn probe() -> Int {
                let flag = Flag { reason: Mutex.new(Maybe.None), observed: Mutex.new(Slots.empty()) };
                match flag.reason.data { Maybe.None => 79, Maybe.Some(_) => 0 }
            }
        "#,
        ),
    ] {
        let file = core.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    let archive = bootstrap(&core);
    let module = archive.load_module("core.workers").unwrap();
    let ty = |name: &str| {
        module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some(name))
            .unwrap()
    };
    let reason = ty("CancelReason");
    let timeout = reason
        .variants
        .iter()
        .find(|v| module.get_string(v.name) == Some("Timeout"))
        .unwrap();
    let clock = timeout.fields[0]
        .declaration_type
        .as_ref()
        .unwrap()
        .base_type_id()
        .unwrap();
    assert_eq!(
        module.get_string(module.get_type(clock).unwrap().origin_module.unwrap()),
        Some("core.clock.instant")
    );
    let flag = ty("Flag");
    let mutex = flag.fields[0]
        .declaration_type
        .as_ref()
        .unwrap()
        .base_type_id()
        .unwrap();
    assert_eq!(
        module.get_string(module.get_type(mutex).unwrap().origin_module.unwrap()),
        Some("core.coordination")
    );
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("probe"))
        })
        .unwrap();
    let bytecode = &module.bytecode
        [probe.bytecode_offset as usize..(probe.bytecode_offset + probe.bytecode_length) as usize];
    let witnesses = decode_instructions(bytecode)
        .unwrap()
        .into_iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect::<Vec<_>>();
    let instant = verum_vbc::types::TypeRef::Concrete(clock);
    let slots = flag.fields[1].declaration_type.as_ref().unwrap();
    let verum_vbc::types::TypeRef::Instantiated { args, .. } = slots else {
        panic!("expected Mutex<Slots<Instant>>")
    };
    assert!(
        witnesses
            .iter()
            .any(|args| args.as_slice() == [instant.clone()]),
        "{witnesses:?}"
    );
    assert!(
        witnesses
            .iter()
            .any(|actual| actual.as_slice() == args.as_slice()),
        "{witnesses:?}"
    );
    assert_eq!(
        execute(
            &archive,
            "module app; mount core.workers.cancellation.probe; fn main() -> Int { probe() }"
        ),
        79
    );
}

#[test]
fn empty_collection_borrows_only_the_declared_element_type() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    // The canonical collection's runtime constructor and bootstrap dependencies
    // follow a different path than an ordinary user-defined static method.
    for path in [
        "base",
        "mem",
        "sys",
        "sys/linux",
        "sys/windows",
        "sys/darwin",
        "intrinsics",
        "text",
        "collections",
        "consumer",
    ] {
        fs::create_dir_all(core.join(path)).unwrap();
        fs::write(
            core.join(path).join("mod.vr"),
            format!("module core.{};", path.replace('/', ".")),
        )
        .unwrap();
    }
    fs::write(
        core.join("mod.vr"),
        "module core; public module collections; public module consumer;",
    )
    .unwrap();
    fs::write(
        core.join("collections/mod.vr"),
        "module core.collections; public module list;",
    )
    .unwrap();
    fs::write(core.join("collections/list.vr"), "module core.collections.list; public type List<T> is { len: Int }; implement<T> List<T> { public fn new() -> List<T> { List { len: 0 } } }").unwrap();
    fs::write(
        core.join("consumer/mod.vr"),
        r#"
        module core.consumer;
        mount core.collections.list.List;
        public type Slot<T> is { value: T };
        implement<T> Slot<T> { public fn new(x: T) -> Slot<T> { Slot { value: x } } }
        public type Batch is { clocks: Slot<List<Int>>, names: Slot<List<Text>> };
        public fn batches<T>(_unrelated: T) -> Batch {
            Batch { clocks: Slot.new(List.new()), names: Slot.new(List.new()) }
        }
    "#,
    )
    .unwrap();
    let archive = bootstrap(&core);
    let module = archive.load_module("core.consumer").unwrap();
    let batches = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("batches"))
        })
        .unwrap();
    let bytecode = &module.bytecode[batches.bytecode_offset as usize
        ..(batches.bytecode_offset + batches.bytecode_length) as usize];
    let calls: Vec<_> = decode_instructions(bytecode)
        .unwrap()
        .into_iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect();
    let batch = module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some("Batch"))
        .unwrap();
    let expected: Vec<_> = batch
        .fields
        .iter()
        .map(|field| {
            let Some(verum_vbc::types::TypeRef::Instantiated { args, .. }) =
                &field.declaration_type
            else {
                panic!("Slot field missed its declared value type");
            };
            args.clone()
        })
        .collect();
    use verum_vbc::types::{TypeId, TypeRef};
    assert_eq!(
        expected,
        [TypeId::INT, TypeId::TEXT].map(|element| {
            vec![TypeRef::Instantiated {
                base: TypeId::LIST,
                args: vec![TypeRef::Concrete(element)],
            }]
        })
    );
    assert_eq!(calls, expected);
}

#[test]
fn file_namespace_mounts_execute_the_declared_child_under_its_local_name() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    for (file, source) in [
        ("mod.vr", "module core; public module reader; public module model; public module rival;"),
        ("model/mod.vr", "module core.model; public module control;"),
        ("model/control.vr", "module core.model.control; public fn read() -> Int { 41 }"),
        ("rival/mod.vr", "module core.rival; public fn control() -> Int { 17 } public fn read() -> Int { 99 }"),
        ("reader/mod.vr", "module core.reader; mount core.model.control as operations;
            public fn entry() -> Int { operations.read() }"),
    ] {
        let path = core.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    let caller = "module app; mount core.reader.entry; fn main() -> Int { entry() }";
    assert_eq!(execute(&bootstrap(&core), caller), 41);

    fs::write(core.join("model/mod.vr"), "module core.model; module control;").unwrap();
    let error = format!("{:#}", try_bootstrap(&core).unwrap_err());
    assert!(error.contains("core.model.control") && error.contains("inaccessible"), "{error}");
}
