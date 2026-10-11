from pathlib import Path
import argparse, hashlib, json, os, re, signal, subprocess, time

p=argparse.ArgumentParser()
p.add_argument('--origin', required=True)
p.add_argument('--output', required=True)
a=p.parse_args()
origin_path=Path(a.origin)
origin=json.loads(origin_path.read_text())
root=Path('/Users/taaliman/.codex/worktrees/configured-registry-downloads/verum')
out=Path(a.output)
out.mkdir(exist_ok=False)
def sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''): h.update(block)
    return h.hexdigest()
def git(*args):
    return subprocess.run(['git','-C',str(root),*args],capture_output=True,text=True,check=True).stdout.strip()
def sources(): return {n:sha(root/n) for n in origin['source_paths']}
artifact_root=Path('/private/tmp/verum-registry-stage23-canonical-owners/artifacts')
def artifacts():
    return {n:{'bytes':(artifact_root/n).stat().st_size,'sha256':sha(artifact_root/n)} for n in origin['artifacts_before']}
assert origin['exit_code']==0 and origin['selected_tests_complete']
assert origin['source_unchanged'] and origin['source_paths_unchanged'] and origin['artifacts_unchanged']
assert git('rev-parse','HEAD')==origin['source_commit'] and not git('status','--porcelain')
assert sources()==origin['source_paths'] and artifacts()==origin['artifacts_after']
exe=origin['executables']['verum_vbc']['retained']
exe_sha=origin['executables']['verum_vbc']['sha256']
assert sha(exe)==exe_sha
cases=[('shared_carrier_authority_tests::',False,10),
       ('interpreter::heap::tests::drop_layout_gate_record_sizes',True,1),
       ('interpreter::heap::tests::drop_layout_gate_variant_sizes',True,1)]
record={k:origin[k] for k in ['source_commit','source_tree','source_paths','artifacts_before','artifact_schema','artifact_origin_receipt','artifact_origin_receipt_sha256','artifact_origin_source']}
record.update(task='T0379',origin_receipt=str(origin_path),origin_receipt_sha256=sha(origin_path),
              runner_sha256=sha(__file__),executable=exe,executable_sha256=exe_sha,
              scope='Same freshly compiled retained library executable: ten T1752 authority controls and two existing descriptor-layout controls; no Cargo rebuild, no ordinary CLI, bake, AOT or full sweep.',
              deadline_seconds=300,maximum_log_bytes=16*1024**2,cases=[])
(out/'started.json').write_text(json.dumps(record,indent=2)+'\n')
start=time.monotonic(); interruption=None
for i,(name,exact,count) in enumerate(cases):
    flags=[name]+(['--exact'] if exact else [])
    listing=subprocess.run([exe,*flags,'--list'],capture_output=True,text=True,timeout=30,check=True)
    selected=[s[:-6] for s in listing.stdout.splitlines() if s.endswith(': test')]
    assert len(selected)==count,(name,selected)
    command=[exe,*flags,'--test-threads=1','--nocapture']
    path=out/f'{i+1}.log'; case_start=time.monotonic()
    with path.open('w') as log:
        process=subprocess.Popen(command,cwd=root,stdout=log,stderr=subprocess.STDOUT,start_new_session=True,
                                 env={**os.environ,'RUST_MIN_STACK':'16777216'})
        while process.poll() is None:
            time.sleep(.1)
            if time.monotonic()-start>300: interruption='300-second bound'
            if path.stat().st_size>16*1024**2: interruption='16MiB log bound'
            if interruption:
                os.killpg(process.pid,signal.SIGTERM)
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid,signal.SIGKILL); process.wait(timeout=5)
                break
    content=path.read_text(errors='replace')
    result=re.findall(r'^test result: [^\n]+',content,re.M)
    passed=(not interruption and process.returncode==0 and len(result)==1
            and f'{count} passed; 0 failed; 0 ignored;' in result[0])
    record['cases'].append({'command':command,'selected':selected,'count':count,
        'exit_code':process.returncode,'wall_seconds':round(time.monotonic()-case_start,3),
        'log':path.name,'log_sha256':sha(path),'test_results':result,'passed':passed})
    if not passed: break
record.update(wall_seconds=round(time.monotonic()-start,3),interrupted=interruption,
              source_unchanged=git('rev-parse','HEAD')==origin['source_commit'] and not git('status','--porcelain'),
              source_paths_unchanged=sources()==origin['source_paths'],artifacts_after=artifacts(),
              executable_unchanged=sha(exe)==exe_sha)
record['artifacts_unchanged']=record['artifacts_before']==record['artifacts_after']
record['passed']=(len(record['cases'])==len(cases) and all(c['passed'] for c in record['cases'])
                  and all(record[k] for k in ['source_unchanged','source_paths_unchanged','artifacts_unchanged','executable_unchanged']))
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:record[k] for k in ['passed','cases','wall_seconds','interrupted','source_unchanged','source_paths_unchanged','artifacts_unchanged','executable_unchanged']}))
raise SystemExit(0 if record['passed'] else 1)
