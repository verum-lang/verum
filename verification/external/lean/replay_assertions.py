"""Shared strict Lean replay assertions; tool failures are never negative controls."""
from pathlib import Path
import shutil
import tempfile
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches, declarations

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'

class ReplayAssertions:
    def prove(self, data, select=None):
        source,counts = self.emitter(data)
        with tempfile.TemporaryDirectory(prefix='verum-expressiveness-') as directory:
            directory = Path(directory)
            for name in ('lakefile.toml','lean-toolchain'):
                shutil.copyfile(ROOT/name,directory/name)
            library = directory/'.lake/build/lib/lean'; library.mkdir(parents=True)
            (library/'VerumExternalReplay').symlink_to(ROOT/'.lake/build/lib/lean/VerumExternalReplay',
                                                       target_is_directory=True)
            shared = directory/'WitnessDeclarations.lean'
            common = declarations(source)
            if select is not None:
                # Recompute the selected false predicate without trace rewrite
                # hints. Only positive replay may accept an export, and it still
                # checks and audits every shared theorem and emitted obligation.
                common = '\n\n'.join(b for b in common.split('\n\n')
                                      if not b.startswith('@[cbv_eval]\ntheorem '))
            shared.write_text(common)
            require_success(run('lake','env','lean','-o',str(library/'WitnessDeclarations.olean'),
                                str(shared),cwd=directory))
            if select is not None:
                claims = [b for b in source.split('\n\n') if b.startswith('theorem ') and select in b]
                self.assertEqual(len(claims),1,claims)
                header = [b for b in source.split('\n\n')
                          if b.startswith(('import ','open ','set_option ','namespace '))]
                source = '\n\n'.join(header+[claims[0]])
            audited = 0
            for i,(part,names) in enumerate(batches(source,4,'WitnessDeclarations')):
                path = directory/f'check{i}.lean';path.write_text(part)
                result = check(path, FALSE if select is not None else None, cwd=directory)
                if select is not None:
                    errors = [d for d in result['diagnostics'] if d['severity']=='error']
                    self.assertEqual(len(errors),1,errors)
                    self.assertTrue(errors[0]['data'].startswith(FALSE),errors)
                else:
                    roster = [d['data'] for d in result['diagnostics'] if d['severity']=='information']
                    self.assertEqual(len(roster),len(names))
                    for row in roster:
                        self.assertTrue(row.endswith(('does not depend on any axioms',
                            'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')),row)
                    audited += len(names)
            if select is None: self.assertEqual(audited,counts['claims'])
        return counts

