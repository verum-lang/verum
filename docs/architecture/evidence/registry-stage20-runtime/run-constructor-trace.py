"""Execute immutable standalone fixtures with an identified ordinary interpreter."""
import argparse, hashlib, json, os, signal, subprocess, tempfile, time
from pathlib import Path
p=argparse.ArgumentParser()
p.add_argument('--build-dir',type=Path,required=True)
p.add_argument('--evidence-dir',type=Path,required=True)
p.add_argument('--timeout',type=int,default=180)
p.add_argument('--case',action='append',nargs=2,metavar=('FIXTURE','EXPECTED_STDOUT'),required=True)
a=p.parse_args()
def sha(p):
    h=hashlib.sha256()
    with Path(p).open('rb') as f:
        for b in iter(lambda:f.read(1048576),b''): h.update(b)
    return h.hexdigest()
build_file=a.build_dir/'build-manifest.json'
build=json.loads(build_file.read_text()); cli=Path(build['cli']['path'])
assert sha(cli)==build['cli']['sha256']
a.evidence_dir.mkdir(exist_ok=False)
all_pass=True
for supplied,expected in a.case:
    fixture=Path(supplied).resolve(); out=a.evidence_dir/fixture.stem; out.mkdir()
    frozen=out/fixture.name; frozen.write_bytes(fixture.read_bytes())
    env={'NO_COLOR':'1','VERUM_NO_OBJECT_CACHE':'1','TMPDIR':'/private/tmp','VERUM_TRACE_CALLS':'1','VERUM_TRACE_PC':'__tls_init_','VERUM_TRACE_PC_DECODE':'1','VERUM_TRACE_CTOR_SKIP':'1'}
    command=[str(cli),'run','--tier','interpret',str(frozen)]
    record={'language_commit':build['engine_commit'],'build_manifest_sha256':sha(build_file),
        'cli_sha256':sha(cli),'fixture_path':str(fixture),'fixture_sha256':sha(fixture),
        'executed_fixture':str(frozen),'command':command,'expected_stdout':expected,
        'deadline_seconds':a.timeout,'environment_overrides':env,
        'scope':'Standalone ordinary interpreter fixture; no whole-project, AOT, cryptographic evidence or service acceptance.'}
    started=time.monotonic(); timed_out=False
    with tempfile.TemporaryDirectory(prefix='verum-identified-',dir='/private/tmp') as cwd:
        record['working_directory']=cwd
        with (out/'stdout.log').open('wb') as stdout, (out/'stderr.log').open('wb') as stderr:
            proc=subprocess.Popen(command,cwd=cwd,env={**os.environ,**env},stdout=stdout,stderr=stderr,start_new_session=True)
            (out/'started.json').write_text(json.dumps({**record,'pid':proc.pid},indent=2)+'\n')
            try: code=proc.wait(timeout=a.timeout)
            except subprocess.TimeoutExpired:
                timed_out=True; os.killpg(proc.pid,signal.SIGTERM)
                try: code=proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid,signal.SIGKILL); code=proc.wait()
    record.update(elapsed_seconds=round(time.monotonic()-started,3),returncode=code,timed_out=timed_out,
        source_unchanged=sha(fixture)==record['fixture_sha256'] and sha(frozen)==record['fixture_sha256'],
        cli_unchanged=sha(cli)==record['cli_sha256'],
        exact_stdout=(out/'stdout.log').read_text()==expected,
        logs={name:sha(out/name) for name in ['stdout.log','stderr.log']})
    passed=code==0 and not timed_out and record['source_unchanged'] and record['cli_unchanged'] and record['exact_stdout']
    record['status']='passed' if passed else 'timeout_without_verdict' if timed_out else 'failed'
    all_pass=all_pass and passed
    (out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({key:record[key] for key in ['fixture_path','status','elapsed_seconds','returncode','exact_stdout']}),flush=True)
raise SystemExit(0 if all_pass else 1)
