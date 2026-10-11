from pathlib import Path
import hashlib, json, os, re, signal, subprocess, time
root=Path('/Users/taaliman/.codex/worktrees/configured-registry-downloads/verum')
target=Path('/Users/taaliman/.tmp/verum-codex-01a10248/native-target')
receipt=Path('/private/tmp/verum-T1744-compilation-owned-fixed/result.json')
out=Path('/private/tmp/verum-T1744-adjacent')
out.mkdir(exist_ok=False)
def sha(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as f:
  for part in iter(lambda:f.read(1024*1024),b''):h.update(part)
 return h.hexdigest()
def git(*args):
 return subprocess.run(['git','-C',str(root),*args],check=True,text=True,capture_output=True).stdout.strip()
r=json.loads(receipt.read_text())
assert r['exit_code']==0 and r['selected_tests_complete'] and r['announced_test_counts']==[9]
assert git('rev-parse','HEAD')==r['source_commit'] and not git('status','--porcelain')
exe=Path(r['executables']['verum_compiler']['retained'])
expected_exe=r['executables']['verum_compiler']['retained_sha256']
assert sha(exe)==expected_exe
source_before={name:sha(root/name) for name in r['source_paths']}
assert source_before==r['source_paths']
def artifacts():
 return {name:{'bytes':(target/'precompiled-stdlib'/name).stat().st_size,
               'sha256':sha(target/'precompiled-stdlib'/name)} for name in r['artifacts_after']}
assert artifacts()==r['artifacts_after']
env=os.environ.copy();env.update(r['environment']);env.pop('RUST_TEST_THREADS',None)
filters=[('declared_function_bodies',1,0),('explicit_generic_param_tests',2,0),
         ('bootstrap_glue_tests',2,0),('symbol_graph_declaring_owner_tests',6,0),
         ('wanted_leaf_index_tests',6,1)]
record={'task':'T1744','source_commit':r['source_commit'],'source_tree':r['source_tree'],
        'source_receipt':str(receipt),'source_receipt_sha256':sha(receipt),
        'source_bindings_canonical_sha256':hashlib.sha256(json.dumps(source_before,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
        'runner_sha256':sha(__file__),'executable':str(exe),'executable_sha256':expected_exe,
        'environment':r['environment'],'deadline_seconds':300,'artifacts_before':artifacts(),
        'scope':'Direct selected archive-contract tests from the exact retained nine-control executable; no Cargo rebuild, bake, ordinary CLI, AOT or traffic campaign.', 'groups':[]}
(out/'started.json').write_text(json.dumps(record,indent=2)+'\n')
start=time.monotonic()
for name,count,ignored in filters:
 command=[str(exe),'archive_ctx_loader::'+name+'::','--test-threads=1','--nocapture']
 log=out/(name+'.log');reason=None;begin=time.monotonic()
 with log.open('w') as stream:
  proc=subprocess.Popen(command,cwd=root,env=env,stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
  try:exit_code=proc.wait(timeout=max(0.01,300-(time.monotonic()-start)))
  except subprocess.TimeoutExpired:
   reason='combined300s deadline';os.killpg(proc.pid,signal.SIGTERM)
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait(timeout=10)
   exit_code=proc.returncode
 text=log.read_text(errors='replace')
 counts=[int(n) for n in re.findall(r'^running (\d+) tests?$',text,re.M)]
 result=re.findall(r'^test result: [^\n]+',text,re.M)
 expected=f'test result: ok. {count-ignored} passed; 0 failed; {ignored} ignored;'
 good=exit_code==0 and counts==[count] and len(result)==1 and result[0].startswith(expected) and reason is None
 row={'filter':name,'command':command,'selected':count,'expected_ignored':ignored,'exit_code':exit_code,
      'wall_seconds':round(time.monotonic()-begin,3),'interrupted':reason,'announced_counts':counts,
      'test_results':result,'log':log.name,'log_sha256':sha(log),'passed':good}
 record['groups'].append(row);print(json.dumps(row),flush=True)
 if not good:break
record.update(wall_seconds=round(time.monotonic()-start,3),
 source_unchanged=git('rev-parse','HEAD')==r['source_commit'] and not git('status','--porcelain'),
 source_paths_unchanged=source_before=={name:sha(root/name) for name in source_before},
 executable_unchanged=sha(exe)==expected_exe,artifacts_after=artifacts())
record['artifacts_unchanged']=record['artifacts_before']==record['artifacts_after']
record['passed']=len(record['groups'])==len(filters) and all(g['passed'] for g in record['groups']) and all(record[k] for k in ['source_unchanged','source_paths_unchanged','executable_unchanged','artifacts_unchanged'])
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:record[k] for k in ['passed','wall_seconds','source_unchanged','source_paths_unchanged','executable_unchanged','artifacts_unchanged']}),flush=True)
