"""Private, bounded Lean compilation shared by executable proof regressions."""
import json
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import ROOT, require_success, run
from observer_export_replay import module_closure

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class IsolatedLeanTest(unittest.TestCase):
    MODULES = ()
    FIXTURES = ()

    @classmethod
    def setUpClass(cls):
        # No shared build products: every run checks the captured source afresh.
        cls.directory = tempfile.TemporaryDirectory(prefix='verum-isolated-lean-')
        cls.addClassCleanup(cls.directory.cleanup)
        cls.snapshot = Path(cls.directory.name)
        modules = module_closure(cls.MODULES)
        paths = ['lean-toolchain', 'lakefile.toml', *cls.FIXTURES]
        paths += [f'VerumExternalReplay/{name}.lean' for name in modules]
        for name in paths:
            destination = cls.snapshot / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((ROOT / name).read_bytes())
        cls.sources = {name: (cls.snapshot / f'VerumExternalReplay/{name}.lean').read_text()
                       for name in modules}
        for name in modules:
            output = cls.snapshot / f'.lake/build/lib/lean/VerumExternalReplay/{name}.olean'
            output.parent.mkdir(parents=True, exist_ok=True)
            require_success(run('lake', 'env', 'lean', '-M', '1024',
                                '-o', str(output), f'VerumExternalReplay/{name}.lean',
                                timeout=60, cwd=cls.snapshot))

    def prove(self, source, *, failure=False, expected_error=FALSE):
        path = self.snapshot / 'Check.lean'
        path.write_text(source)
        result = run('lake', 'env', 'lean', '-M', '1024', '--json', str(path),
                     timeout=60, cwd=self.snapshot)
        self.assertEqual(result.stderr, '')
        diagnostics = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        errors = [d for d in diagnostics if d['severity'] == 'error']
        if failure:
            self.assertEqual(result.returncode, 1, diagnostics)
            self.assertEqual(len(errors), 1, diagnostics)
            self.assertTrue(errors[0]['data'].startswith(expected_error), errors)
        else:
            self.assertEqual(result.returncode, 0, diagnostics)
            self.assertEqual(errors, [], diagnostics)
        self.assertFalse(any(d['severity'] == 'warning' for d in diagnostics), diagnostics)
        return [d['data'] for d in diagnostics if d['severity'] == 'information']

    def assert_axioms(self, rows, names):
        self.assertEqual(len(rows), len(names), rows)
        for name, row in zip(names, rows):
            self.assertIn(row, [f"'{name}' does not depend on any axioms",
                                f"'{name}' depends on axioms: [propext]",
                                f"'{name}' depends on axioms: [propext, Quot.sound]"])

