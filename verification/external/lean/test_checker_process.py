"""Deadlines must stop the checker, not merely its launcher."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from check_typed_programs import run


class CheckerProcessTests(unittest.TestCase):
    def test_output_and_failed_status_are_preserved(self):
        result = run(sys.executable, '-c',
                     'import sys; print("out"); print("err",file=sys.stderr); sys.exit(7)')
        self.assertEqual((result.returncode,result.stdout,result.stderr),(7,'out\n','err\n'))

    @unittest.skipUnless(os.name == 'posix','POSIX process-group cleanup')
    def test_timeout_kills_descendant_and_remains_failure(self):
        with tempfile.TemporaryDirectory(prefix='verum-checker-timeout-') as directory:
            root = Path(directory)
            child = root/'child.py'
            child.write_text('from pathlib import Path\nimport time\n'
                'Path("started").write_text("ready")\n'
                'time.sleep(1.5)\nPath("escaped").write_text("still running")\n')
            launcher = ('import subprocess,sys,time\n'
                'subprocess.Popen([sys.executable,"child.py"])\n'
                'print("launched",flush=True)\ntime.sleep(30)\n')
            started = time.monotonic()
            with self.assertRaises(subprocess.TimeoutExpired) as caught:
                run(sys.executable,'-c',launcher,timeout=0.6,cwd=root)
            self.assertLess(time.monotonic()-started,3)
            self.assertIn(b'launched',caught.exception.output)
            self.assertTrue((root/'started').exists(),'child did not start before timeout')
            time.sleep(1.6)
            self.assertFalse((root/'escaped').exists(),'timed-out child kept executing')


if __name__ == '__main__': unittest.main()
