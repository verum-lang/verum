//! Root inline modules exist in the ordinary single-file pipeline before mount processing.
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, OutputFormat, Session, VerifyMode};

fn check(source: &str) -> (bool, Text) {
    let directory = tempfile::tempdir().expect("source directory");
    let input = directory.path().join("control.vr");
    std::fs::write(&input, source).expect("source file");
    let options = CompilerOptions {
        input,
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        check_only: true,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let result = CompilationPipeline::new(&mut session).run_check_only();
    let mut diagnostics: Text = session.format_diagnostics().into();
    for diagnostic in session.diagnostics() {
        diagnostics.push_str(&format!("{diagnostic:?}\n"));
    }
    (result.is_ok(), diagnostics)
}

#[test]
fn ordinary_file_accepts_the_inline_alias_property_program() {
    let source = r#"module wide {
        public type Item is {left:Int,right:Int};
        public const CAP:Int=Item.size;
    }
    module narrow {public type Item is {value:Int};}
    mount wide as short;
    type Cell is {size:Bool};
    fn field(Cell:Cell)->Bool {Cell.size}
    fn width<T>()->Int {T.size}
    fn main() {
        print("property_begin");
        print(field(Cell{size:true}));
        print(short.Item.size);
        print((&unsafe Int).size);
        print(width<&unsafe Byte>());
        let bytes:[Byte;wide.CAP]=[0;wide.CAP];
        print(bytes.len());
        print("property_end");
    }"#;
    let (ok, diagnostics) = check(source);
    assert!(ok, "{diagnostics}");
}

#[test]
fn ordinary_file_keeps_inline_aliases_and_lexical_values_distinct() {
    let source = r#"mount wide as short;
    module wide {public type Item is {left:Int,right:Int};}
    module narrow {public type Item is {left:Bool};}
    mount narrow as small;
    type Cell is {size:Bool};
    type Root is {Item:Cell};
    fn values(a:short.Item,b:small.Item)->Bool {a.left==37 && b.left}
    fn shadow(short:Root)->Bool {short.Item.size}
    fn main() {}"#;
    let (ok, diagnostics) = check(source);
    assert!(ok, "{diagnostics}");
}

#[test]
fn ordinary_file_does_not_publish_a_nested_module_as_a_root_leaf() {
    let source = r#"module outer {public module wide {public type Item is {left:Int};}}
    mount wide as short;
    fn main() {print(short.Item.size);}"#;
    let (ok, diagnostics) = check(source);
    assert!(!ok, "nested leaf is not a root declaration");
    assert!(diagnostics.contains("E402"), "{diagnostics}");
}
