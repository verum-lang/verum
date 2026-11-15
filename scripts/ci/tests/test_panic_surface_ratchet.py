#!/usr/bin/env python3
"""Exercise the production panic gate CLI, including the retained T0424 inputs."""
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
GATE = ROOT / "scripts/ci/check_panic_surface_ratchet.py"


class PanicSurfaceRatchetTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="verum-panic-ratchet-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.source = self.directory / "src"
        self.source.mkdir()
        self.baseline = self.directory / "baseline.txt"

    def write(self, source):
        (self.source / "fixture.rs").write_text(source, encoding="utf-8")

    def run_gate(self, *, actual_tree=False):
        arguments = [sys.executable, "-B", str(GATE)]
        if not actual_tree:
            arguments += ["--root", str(self.source), "--baseline", str(self.baseline)]
        result = subprocess.run(
            arguments, cwd=ROOT, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=15,
        )
        return result.returncode, result.stdout + result.stderr

    def seed(self):
        # Seed only this temporary fixture from the real scanner's missing-
        # baseline diagnostic. The production command never writes a baseline.
        self.baseline.unlink(missing_ok=True)
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        match = re.search(r"current production count = (\d+)", text)
        self.assertIsNotNone(match, text)
        self.baseline.write_text(match.group(1) + "\n", encoding="utf-8")

    def test_added_panic_site_fails_the_gate_and_removing_it_passes(self):
        clean = '''
fn emit_thing(x: Option<u32>) -> Result<u32, ()> {
    x.ok_or(())
}
'''
        self.write(clean)
        self.seed()
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.write(clean + '\nfn emit_other(x: Option<u32>) -> u32 {\n    x.expect("cannot fail")\n}\n')
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("emit_other", text)
        self.assertIn("or_llvm_err", text)
        self.write(clean)
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)

    def test_growth_inside_an_existing_panicking_function_fails(self):
        self.write('fn emit(a: Option<u32>) -> u32 {\n    a.expect("one")\n}\n')
        self.seed()
        self.assertEqual(self.run_gate()[0], 0)
        self.write('fn emit(a: Option<u32>) -> u32 {\n    a.expect("one") + a.expect("two")\n}\n')
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("1 -> 2", text)

    def test_conversion_to_diagnostics_passes_and_asks_for_a_reseed(self):
        self.write('fn emit(a: Option<u32>) -> u32 {\n    a.expect("one") + a.expect("two")\n}\n')
        self.seed()
        self.write('fn emit(a: Option<u32>) -> Result<u32, ()> {\n    Ok(a.ok_or(())? + 1)\n}\n')
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.assertIn("improved", text)
        self.assertIn("ratchet", text)

    def test_panic_macros_are_counted_so_the_ratchet_cannot_be_laundered(self):
        self.write('fn emit(a: Option<u32>) -> u32 {\n    a.expect("one")\n}\n')
        self.seed()
        self.write('fn emit(a: Option<u32>) -> u32 {\n    match a {\n        Some(v) => v,\n        None => panic!("one"),\n    }\n}\n')
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.write('fn emit(a: Option<u32>) -> u32 {\n    match a {\n        Some(v) => v,\n        None => unreachable!("one"),\n    }\n}\nfn emit2() -> u32 {\n    todo!()\n}\n')
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("emit2", text)

    def test_comments_and_string_literals_are_not_counted(self):
        self.write(r'''
/// This replaces `.unwrap()` calls with proper error propagation.
// Instead of .unwrap() which panics, use .or_llvm_err()?
/* block comment with .expect("x") and panic!("y") */
fn emit() -> &'static str {
    let _msg = "call .unwrap() here and panic!(\"boom\")";
    let _raw = r#"raw string .expect("z")"#;
    "ok"
}
''')
        self.seed()
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.assertIn("0 panic sites", text)

    def test_cfg_test_modules_are_excluded(self):
        self.write('''
fn emit(a: Option<u32>) -> Result<u32, ()> {
    a.ok_or(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        let v: Option<u32> = Some(1);
        assert_eq!(v.unwrap(), 1);
        std::panic::catch_unwind(|| panic!("fine in tests")).ok();
    }
}
''')
        self.seed()
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.assertIn("0 panic sites", text)

    def test_panic_ok_marker_does_not_exempt_a_site(self):
        # The prototype's exemption policy was not adopted. Retain its input
        # and require the production gate to count this ordinary panic site.
        self.write('''
fn emit(a: Option<u32>) -> u32 {
    // PANIC-OK: enum is exhaustive over a compiler-internal tag
    a.expect("tag")
}
''')
        self.baseline.write_text("0\n", encoding="utf-8")
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("1 panic sites", text)

    def test_committed_baseline_holds(self):
        code, text = self.run_gate(actual_tree=True)
        self.assertEqual(code, 0, text)

    def test_nested_comments_and_all_literal_forms_are_not_sites(self):
        self.write(r'''
/* outer /* inner */ .unwrap() panic!() */
fn text<'ctx>(value: &'ctx str) -> &'ctx str {
    let _raw = br###"quotes " and // .expect("x") /* panic!() */"###;
    let _byte = b".unwrap() unreachable!()";
    let _c = c".expect( todo!()";
    let _quote = '\'';
    let _brace = '{';
    value
}
''')
        self.baseline.write_text("0\n", encoding="utf-8")
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)
        self.assertIn("0 panic sites", text)

    def test_cfg_test_string_braces_do_not_hide_production(self):
        sources = [
            '#[cfg(test)] mod tests { fn t() { let text = "{{"; a.unwrap(); } }',
            "#[cfg ( test )] mod tests { fn t() { let ch = '{'; a.unwrap(); } }",
            'fn message() { let text = "#[cfg(test)] mod fake {{"; }',
            '#[cfg(test)] use thing;\nfn helper() {}',
        ]
        self.baseline.write_text("0\n", encoding="utf-8")
        for source in sources:
            with self.subTest(source=source):
                self.write(source + '\nfn production() { x.expect("must count"); }')
                code, text = self.run_gate()
                self.assertEqual(code, 1, text)
                self.assertIn("1 panic sites", text)
                self.assertIn("production", text)

    def test_cfg_test_fields_and_variants_cannot_hide_following_items(self):
        declarations = [
            "struct S { #[cfg(test)] test: (), }",
            "struct S { #[cfg(test)] test: () }",
            "struct S(#[cfg(test)] ());",
            "struct S(#[cfg(test)] (),);",
            "struct S(#[cfg(test)] fn() -> (), ());",
            "#[cfg(test)] struct S;",
            "enum E { #[cfg(test)] Unit, }",
            "enum E { #[cfg(test)] Unit }",
            "enum E { #[cfg(test)] Tuple(()), }",
            "enum E { #[cfg(test)] Tuple(()) }",
            "enum E { #[cfg(test)] Named { value: () }, }",
            "enum E { Tuple(#[cfg(test)] (),), }",
            "enum E { Named { #[cfg(test)] value: (), }, }",
        ]
        self.baseline.write_text("0\n", encoding="utf-8")
        for declaration in declarations:
            with self.subTest(declaration=declaration):
                self.write(declaration + ' fn production() { panic!("must count"); }')
                code, text = self.run_gate()
                self.assertEqual(code, 1, text)
                self.assertIn("1 panic sites", text)
                self.assertIn("production", text)

    def test_cfg_test_fields_and_variants_cannot_hide_siblings(self):
        declarations = [
            'struct S { #[cfg(test)] gone: (), live: [u8; { panic!("live") }], }',
            'struct S(#[cfg(test)] (), [u8; { panic!("live") }]);',
            'enum E { #[cfg(test)] Gone, Live = { panic!("live") }, }',
            'enum E { #[cfg(test)] Gone(()), Live = { panic!("live") }, }',
            'enum E { #[cfg(test)] Gone { value: () }, Live = { panic!("live") }, }',
            'struct S { #[cfg(test)] gone: [u8; { panic!("test") }], live: [u8; { panic!("live") }], }',
            'struct S { #[cfg(test)] gone: Pair<(), Array<{ panic!("test") }>>, live: [u8; { panic!("live") }], }',
            'enum E { #[cfg(test)] Gone = if 1 < 2 { 0 } else { 1 }, Live = { panic!("live") }, }',
        ]
        self.baseline.write_text("0\n", encoding="utf-8")
        for declaration in declarations:
            with self.subTest(declaration=declaration):
                self.write(declaration)
                code, text = self.run_gate()
                self.assertEqual(code, 1, text)
                self.assertIn("1 panic sites", text)

    def test_cfg_test_generic_headers_keep_balanced_boundaries(self):
        declarations = [
            '#[cfg(test)] fn helper<T, U>() { panic!("test"); }',
            'struct S { #[cfg(test)] callback: Pair<fn() -> (), Array<{ panic!("test") }>>, }',
            'fn generic<#[cfg(test)] T>() { panic!("live"); }',
        ]
        self.baseline.write_text("0\n", encoding="utf-8")
        for declaration in declarations:
            with self.subTest(declaration=declaration):
                suffix = '' if declaration.startswith('fn generic') else ' fn production() { panic!("live"); }'
                self.write(declaration + suffix)
                code, text = self.run_gate()
                self.assertEqual(code, 1, text)
                self.assertIn("1 panic sites", text)

    def test_whitespace_and_error_variants_still_count(self):
        self.write('fn emit() { a. unwrap (); b.expect ("x"); c.unwrap_err(); d.expect_err("x"); }')
        self.baseline.write_text("3\n", encoding="utf-8")
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("4 panic sites", text)

    def test_aggregate_policy_remains_aggregate(self):
        self.write('fn old() { a.unwrap(); }\nfn added() {}')
        self.seed()
        self.write('fn old() {}\nfn added() { a.unwrap(); }')
        code, text = self.run_gate()
        self.assertEqual(code, 0, text)

    def test_missing_source_root_is_a_harness_error(self):
        self.source.rmdir()
        code, text = self.run_gate()
        self.assertEqual(code, 2, text)
        self.assertIn("source root not found", text)

    def test_non_test_configuration_is_counted(self):
        self.write('#[cfg(not(test))] fn emit() { a.unwrap(); }')
        self.baseline.write_text("0\n", encoding="utf-8")
        code, text = self.run_gate()
        self.assertEqual(code, 1, text)
        self.assertIn("1 panic sites", text)


if __name__ == "__main__":
    unittest.main()
