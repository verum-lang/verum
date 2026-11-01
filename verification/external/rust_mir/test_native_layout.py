"""Compiler-configuration drift is a refusal, never a renamed native proof."""
import json
import os
from pathlib import Path
import tempfile
import unittest

from native_layout import validate_layout, witness_source
from record_mir import Layout, RecordTranslation
from runtime import execute
from test_record_mir import metadata, unit_witness, mir


class NativeLayoutTests(unittest.TestCase):
    def test_all_field_identities_types_and_moves_are_checked(self):
        layout = Layout(metadata(), 'State', 'update')
        validate_layout(unit_witness(), layout)
        mutations = [
            ('value: move _2, flag: move _3', 'flag: move _3, value: move _2'),
            ('(_1.0: usize)', '(_1.1: usize)'),
            ('_2: usize', '_2: bool'),
            ('owned: move _4', 'owned: move _3'),
            ('_4 = move (_1.2: std::vec::Vec<usize>);', '_4 = make_empty();'),
            ('return;', 'goto -> bb1;'),
            ('fn verumNativeRecordLayout', 'fn unrelated'),
        ]
        for before, after in mutations:
            with self.subTest(mutation=after), self.assertRaises(ValueError):
                validate_layout(unit_witness().replace(before, after), layout)
        with self.assertRaises(ValueError): validate_layout(unit_witness()*2, layout)

    def test_missing_witness_and_changed_native_method_are_refused(self):
        source = mir('        ((*_1).0: usize) = copy _2;\n        return;')
        layout = Layout(metadata(), 'State', 'update')
        for witness in ['', unit_witness(), source.replace('copy _2;', 'const 0_usize;') + unit_witness()]:
            with self.subTest(witness=witness), self.assertRaises(ValueError):
                RecordTranslation(source, 'update', layout, witness)


@unittest.skipUnless(os.getenv('VERUM_REPLAY_RUSTC'), 'Set the pinned Rust compiler for native cfg controls')
class CompiledNativeLayoutTests(unittest.TestCase):
    def compile(self, source, root, extra=()):
        rustc = Path(os.environ['VERUM_REPLAY_RUSTC']).resolve()
        path = root/'source.rs'; path.write_text(source)
        flags = ['--crate-type=lib', '--crate-name=transaction', '--edition=2021', '-C', 'overflow-checks=yes', *extra]
        code, stdout, stderr = execute([str(rustc), *flags, '--emit=mir', str(path), '-o', str(root/'source.mir')], root, root/'native.log')
        self.assertEqual(code, 0, stdout+stderr)
        code, stdout, stderr = execute([str(rustc.with_name('rustdoc')), '--crate-name=transaction', '--edition=2021',
            *extra, '-Z', 'unstable-options', '--document-private-items', '--output-format=json', str(path),
            '-o', str(root/'metadata')], root, root/'doc.log')
        self.assertEqual(code, 0, stdout+stderr)
        layout = Layout(json.loads((root/'metadata/transaction.json').read_bytes()), 'State', 'update')
        (root/'witness.rs').write_text(source+witness_source(layout))
        code, stdout, stderr = execute([str(rustc), *flags, '--emit=mir', str(root/'witness.rs'),
            '-o', str(root/'witness.mir')], root, root/'witness.log')
        return layout, code, stdout+stderr

    def test_cfg_doc_can_reorder_same_typed_fields_but_cannot_authorize_projection(self):
        source = '''
#[cfg(not(doc))] struct State { balance: usize, limit: usize }
#[cfg(doc)] struct State { limit: usize, balance: usize }
impl State { fn update(&mut self, amount: usize) {
    #[cfg(not(doc))] { self.balance = amount; }
    #[cfg(doc)] { self.limit = amount; }
} }
'''
        with tempfile.TemporaryDirectory(prefix='verum-layout-cfg-') as tmp:
            root = Path(tmp); layout, code, log = self.compile(source, root)
            self.assertEqual(code, 0, log)
            self.assertEqual([f.name for f in layout.fields], ['limit', 'balance'])
            # Both are usize, so checking only MIR field types would accept the wrong names.
            self.assertIn('((*_1).0: usize) = copy _2;', (root/'source.mir').read_text())
            with self.assertRaisesRegex(ValueError, 'identity/order'):
                RecordTranslation((root/'source.mir').read_text(), 'update', layout, (root/'witness.mir').read_text())

    def test_cfg_doc_cannot_narrow_an_untouched_scalar_domain(self):
        source = '''
#[cfg(not(doc))] struct State { value: usize }
#[cfg(doc)] struct State { value: bool }
impl State { fn update(&mut self) {} }
'''
        with tempfile.TemporaryDirectory(prefix='verum-layout-type-') as tmp:
            root = Path(tmp); layout, code, log = self.compile(source, root)
            self.assertEqual(code, 0, log)
            with self.assertRaisesRegex(ValueError, 'field type'):
                RecordTranslation((root/'source.mir').read_text(), 'update', layout, (root/'witness.mir').read_text())

    def test_native_fields_and_enum_variants_cannot_be_hidden_by_doc_cfg(self):
        cases = [
            ('''struct State { value: usize, #[cfg(not(doc))] hidden: usize }
impl State { fn update(&mut self) {} }''', 'E0063'),
            ('''#[derive(Clone,Copy)] enum Phase { Idle, Ready, #[cfg(not(doc))] Hidden }
struct State { phase: Phase }
impl State { fn update(&mut self) {} }''', 'E0004'),
        ]
        for source, diagnostic in cases:
            with self.subTest(source=source), tempfile.TemporaryDirectory(prefix='verum-layout-exhaustive-') as tmp:
                _, code, log = self.compile(source, Path(tmp))
                self.assertEqual(code, 1, log)
                self.assertIn(diagnostic, log)

    def test_same_native_cfg_and_complete_enum_roster_pass(self):
        source = '''#[derive(Clone,Copy)] enum Phase { Idle, Ready }
struct State { value: usize, phase: Phase, #[cfg(extra)] owned: Vec<usize> }
impl State { fn update(&mut self, amount: usize) { self.value = amount; } }
'''
        with tempfile.TemporaryDirectory(prefix='verum-layout-positive-') as tmp:
            root = Path(tmp); layout, code, log = self.compile(source, root, ['--cfg', 'extra'])
            self.assertEqual(code, 0, log)
            text = RecordTranslation((root/'source.mir').read_text(), 'update', layout, (root/'witness.mir').read_text()).lean()
            self.assertIn('field2 : Frame2', text)
            self.assertIn('{ initial with field0 := arg2 }', text)


if __name__ == '__main__':
    unittest.main()
