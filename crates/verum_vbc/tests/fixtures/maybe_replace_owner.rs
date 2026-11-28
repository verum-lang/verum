#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::{FileId, ItemKind};
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

    let selection = [
        ("base/protocols.vr", "Drop"),
        ("base/memory.vr", "replace"),
        ("base/maybe.vr", "Maybe Maybe<T>.take Maybe<T>.replace"),
    ];
    let mut units: verum_common::List<_> = selection
        .iter()
        .enumerate()
        .map(|(i, (file, symbols))| selected_stdlib::source_unit(file, symbols, i))
        .collect();
    let probe = FastParser::new()
        .parse_module_str(
            r#"
        mount core.base.protocols.Drop;
        type affine Token is { digit: Int };
        static mut DROPS: Int = 0;
        implement Drop for Token {
            fn drop(&mut self) { DROPS = DROPS * 10 + self.digit; }
        }
        fn main() -> Int {
            let mut cell: Maybe<Token> = Maybe.Some(Token { digit: 7 });
            {
                let first = cell.replace(Token { digit: 3 });
                if DROPS != 0 { return 1; }
                match &first {
                    Maybe.Some(token) => if token.digit != 7 { return 2; },
                    Maybe.None => return 3,
                }
            }
            if DROPS != 7 { return 4; }
            {
                let last = cell.take();
                if DROPS != 7 || !(cell is Maybe.None) { return 5; }
                match &last {
                    Maybe.Some(token) => if token.digit != 3 { return 6; },
                    Maybe.None => return 7,
                }
            }
            DROPS
        }
    "#,
            FileId::new(100),
        )
        .unwrap();
    units.last_mut().unwrap().items.extend(
        probe
            .items
            .into_iter()
            .filter(|item| !matches!(item.kind, ItemKind::Module(_))),
    );
    let units: verum_common::List<_> = units.iter().collect();
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&units).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}
