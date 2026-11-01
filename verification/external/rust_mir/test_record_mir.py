"""Mutable MIR must retain every field, branch and preceding write on panic."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

from record_mir import Layout, RecordTranslation
from runtime import execute
from native_layout import witness_source, validate_layout


def metadata():
    """Small rustdoc-shaped unit input; compiler-produced metadata is tested below."""
    generic = {'params': [], 'where_predicates': []}
    index = {
        '0': {'name': 'transaction', 'crate_id': 0, 'inner': {'module': {}}},
        '1': {'id': 1, 'name': 'State', 'crate_id': 0, 'inner': {'struct': {
            'generics': generic, 'kind': {'plain': {'has_stripped_fields': False, 'fields': [4,5,6]}}, 'impls': [2]}}},
        '2': {'name': None, 'crate_id': 0, 'inner': {'impl': {'trait': None, 'items': [3],
            'for': {'resolved_path': {'id': 1}}, 'generics': generic}}},
        '3': {'name': 'update', 'crate_id': 0, 'inner': {'function': {
            'header': {'is_const': False, 'is_unsafe': False, 'is_async': False, 'abi': 'Rust'},
            'generics': generic, 'has_body': True, 'sig': {'is_c_variadic': False, 'output': None,
                'inputs': [['self', {'borrowed_ref': {'lifetime': None, 'is_mutable': True, 'type': {'generic': 'Self'}}}],
                           ['value', {'primitive': 'usize'}], ['flag', {'primitive': 'bool'}]]}}}},
    }
    for i,name,ty in [(4,'value',{'primitive': 'usize'}), (5,'flag',{'primitive': 'bool'}),
                      (6,'owned',{'resolved_path': {'id': 10, 'args': None}})]:
        index[str(i)] = {'name': name, 'crate_id': 0, 'inner': {'struct_field': ty}}
    return {'format_version': 61, 'root': 0, 'index': index, 'paths': {
        '1': {'crate_id': 0, 'path': ['transaction', 'State'], 'kind': 'struct'},
        '10': {'crate_id': 2, 'path': ['alloc', 'vec', 'Vec'], 'kind': 'struct'}}}


def mir(body, locals='', blocks=''):
    return ('fn update(_1: &mut State, _2: usize, _3: bool) -> () {\n'
            '    let mut _0: ();\n' + locals + '    bb0: {\n' + body + '\n    }\n' + blocks + '}\n')


def unit_witness():
    return """fn verumNativeRecordLayout(_1: State) -> State {
    debug value => _1;
    let mut _0: State;
    let mut _2: usize;
    let mut _3: bool;
    let mut _4: std::vec::Vec<usize>;
    bb0: {
        _2 = copy (_1.0: usize);
        _3 = copy (_1.1: bool);
        _4 = move (_1.2: std::vec::Vec<usize>);
        _0 = State { value: move _2, flag: move _3, owned: move _4 };
        return;
    }
}
"""


def translate(source, data=None):
    layout = Layout(data or metadata(), 'State', 'update')
    return RecordTranslation(source, 'update', layout, source + unit_witness()).lean()


class RecordTests(unittest.TestCase):
    def test_metadata_controls_full_frame(self):
        text = translate(mir('        ((*_1).0: usize) = copy _2;\n        return;'))
        self.assertIn('structure Record (Frame2 : Type)', text)
        self.assertIn('field2 : Frame2', text)
        self.assertIn('{ initial with field0 := arg2 }', text)
        self.assertIn('(state1, Exit.returned)', text)

    def test_read_captures_state_before_mutation(self):
        text = translate(mir('''        _4 = copy ((*_1).0: usize);
        ((*_1).0: usize) = copy _2;
        ((*_1).0: usize) = copy _4;
        return;''', '    let mut _4: usize;\n'))
        self.assertIn('{ state1 with field0 := initial.field0 }', text)

    def test_read_after_mutation_uses_new_state(self):
        text = translate(mir('''        ((*_1).0: usize) = copy _2;
        _4 = copy ((*_1).0: usize);
        ((*_1).0: usize) = copy _4;
        return;''', '    let mut _4: usize;\n'))
        self.assertIn('{ state1 with field0 := state1.field0 }', text)

    def test_bool_storage_and_branch_use_same_value(self):
        text = translate(mir('''        ((*_1).1: bool) = copy _3;
        switchInt(copy ((*_1).1: bool)) -> [0: bb1, otherwise: bb2];''', blocks='''    bb1: {
        return;
    }
    bb2: {
        ((*_1).0: usize) = copy _2;
        return;
    }
'''))
        self.assertIn('field1 := (decide (arg3 = true))', text)
        self.assertIn('if state1.field1 = true then', text)

    def test_checked_failure_preserves_preceding_writes(self):
        text = translate(mir('''        ((*_1).0: usize) = copy _2;
        _4 = SubWithOverflow(copy _2, const 1_usize);
        assert(!move (_4.1: bool), "underflow") -> [success: bb1, unwind continue];''',
            '    let mut _4: (usize, bool);\n', '''    bb1: {
        ((*_1).0: usize) = move (_4.0: usize);
        return;
    }
'''))
        self.assertIn('if 1 ≤ arg2 then', text)
        self.assertIn('else (state1, Exit.panicked)', text)

    def test_remainder_divisor_is_captured_before_assignment(self):
        text = translate(mir('''        ((*_1).0: usize) = const 7_usize;
        _2 = Rem(const 0_usize, copy _2);
        ((*_1).0: usize) = copy _2;
        return;'''))
        self.assertIn('if arg2 = 0 then (state1, Exit.panicked) else', text)
        self.assertNotIn('if (0 % arg2) = 0', text)

    def test_missing_guard_and_unknown_effects_fail_closed(self):
        for body in [
            'drop(((*_1).2: Vec<usize>)) -> [return: bb1, unwind continue];',
            '((*_1).0: usize) = external(copy _2);\n        return;',
            '((*_1).2: Vec<usize>) = const ();\n        return;',
            '_2 = copy ((*_1).2: usize);\n        return;',
            '((*_1).0: bool) = copy _3;\n        return;',
            '((*_1).9: usize) = copy _2;\n        return;',
            '((*_2).0: usize) = const 1_usize;\n        return;',
            'goto -> bb0;',
            '((*_1).0: usize) = Add(copy _2, const 1_usize);\n        return;',
        ]:
            with self.subTest(body=body), self.assertRaises(ValueError):
                translate(mir('        '+body))
        with self.assertRaises(ValueError):
            translate(mir('''        _4 = AddWithOverflow(copy _2, const 1_usize);
        return;''', '    let mut _4: (usize, bool);\n'))

    def test_incomplete_or_wrong_method_metadata_refused(self):
        for mutate in [
            lambda m: m.update(format_version=60),
            lambda m: m['index']['1']['inner']['struct']['kind']['plain'].update(has_stripped_fields=True),
            lambda m: m['index']['1']['inner']['struct']['kind']['plain'].update(fields=[4,4,6]),
            lambda m: m['index']['2']['inner']['impl']['for']['resolved_path'].update(id=8),
            lambda m: m['index']['3']['inner']['function']['header'].update(is_unsafe=True),
            lambda m: m['index']['3']['inner']['function']['sig']['inputs'][0][1]['borrowed_ref'].update(is_mutable=False),
        ]:
            data = copy.deepcopy(metadata()); mutate(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                translate(mir('        return;'), data)

    def test_parameter_mismatch_and_ambiguous_blocks_refused(self):
        for source in [mir('        return;').replace('_2: usize', '_2: u64'),
                       mir('        return;') * 2,
                       mir('        return;', blocks='    bb9: {\n        return;\n    }\n')]:
            with self.assertRaises(ValueError): translate(source)


@unittest.skipUnless(os.getenv('VERUM_REPLAY_RUSTC') and os.getenv('VERUM_REPLAY_LEAN'),
                     'Set both pinned compiler paths to run actual Rust/Lean semantics checks')
class CompiledRecordTests(unittest.TestCase):
    def test_arbitrary_state_preserves_frame_on_both_return_and_panic(self):
        source = '''struct State { value: usize, flag: bool, owned: Vec<usize> }
impl State {
    fn update(&mut self, x: usize, flag: bool) {
        let previous = self.value;
        self.value = x;
        self.flag = flag;
        if flag { self.value = previous - x; }
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='verum-record-semantics-') as tmp:
            root = Path(tmp)
            rustc = Path(os.environ['VERUM_REPLAY_RUSTC']).resolve()
            lean = Path(os.environ['VERUM_REPLAY_LEAN']).resolve()
            (root/'state.rs').write_text(source)
            for name, command in [('mir', [str(rustc), '--crate-type=lib', '--emit=mir', '-C',
                                          'overflow-checks=yes', 'state.rs', '-o', 'state.mir']),
                                  ('doc', [str(rustc.with_name('rustdoc')), '-Z', 'unstable-options',
                                           '--document-private-items', '--output-format=json', 'state.rs', '-o', 'metadata'])]:
                code, stdout, stderr = execute(command, root, root/f'{name}.log')
                self.assertEqual(code, 0, stdout+stderr)
            layout = Layout(json.loads((root/'metadata/state.json').read_bytes()), 'State', 'update')
            (root/'witness.rs').write_text(source + witness_source(layout))
            code, stdout, stderr = execute([str(rustc), '--crate-type=lib', '--emit=mir', '-C',
                'overflow-checks=yes', 'witness.rs', '-o', 'witness.mir'], root, root/'witness.log')
            self.assertEqual(code, 0, stdout+stderr)
            text = RecordTranslation((root/'state.mir').read_text(), 'update', layout,
                                     (root/'witness.mir').read_text()).lean()
            text += '''
theorem exact_state {Frame2 : Type} (s : Record Frame2) (x : Nat) (flag : Bool) :
    extracted s x flag =
      if flag = true then
        if x ≤ s.field0 then ({ s with field0 := s.field0-x, field1 := flag }, Exit.returned)
        else ({ s with field0 := x, field1 := flag }, Exit.panicked)
      else ({ s with field0 := x, field1 := flag }, Exit.returned) := by
  cases flag <;> rfl
#print axioms exact_state
'''
            (root/'Check.lean').write_text('set_option warningAsError true\n'+text)
            code, stdout, stderr = execute([str(lean), '-M', '1024', '--json', 'Check.lean'], root, root/'lean.log')
            self.assertEqual(code, 0, stdout+stderr)
            diagnostics = [json.loads(line) for line in stdout.splitlines()]
            self.assertEqual([d['data'] for d in diagnostics], ["'exact_state' does not depend on any axioms"])


if __name__ == '__main__':
    unittest.main()
