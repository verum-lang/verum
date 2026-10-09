from pathlib import Path
import hashlib, json, os, re, signal, subprocess, sys, time
root=Path('/Users/taaliman/.codex/worktrees/registry-record-map-keys/verum')
out=Path(sys.argv[1]); mode=sys.argv[2]; expected=sys.argv[3]
out.mkdir(exist_ok=False)
settings={'CARGO_TARGET_DIR':'/Users/taaliman/.tmp/verum-codex-01a10248/native-target','CARGO_BUILD_JOBS':'2','CARGO_INCREMENTAL':'0','VERUM_NO_AUTO_PRECOMPILE':'1','VERUM_LLVM_DIR':'/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install','RUST_MIN_STACK':'16777216','VERUM_T1698_IR_DIR':str(out/'ir')}
env=os.environ.copy(); env.update(settings)
base=['cargo','test','--offline','--locked','--no-fail-fast']
commands={
 'vbc-source': ['-p','verum_vbc','--test','array_return_storage','--no-default-features','--features','compression,table_dispatch,codegen,ffi'],
 'vbc-unit': ['-p','verum_vbc','--lib','array_storage::tests::','--no-default-features','--features','compression,table_dispatch,codegen,ffi'],
 'native-unit': ['-p','verum_codegen','--lib','array_storage::tests::'],
 'native-source': ['-p','verum_codegen','--test','endian_fixed_bytes'],
}
cmd=base+commands[mode]+['--message-format=json','--','--nocapture','--test-threads=1']
def git(*args): return subprocess.check_output(['git',*args],cwd=root,text=True,stderr=subprocess.PIPE).strip()
def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
assert git('rev-parse','HEAD') == expected, 'wrong frozen source'
assert not git('status','--porcelain'), 'source must be committed'
paths=git('ls-files','Cargo.toml','Cargo.lock','crates/verum_vbc','crates/verum_codegen').splitlines()
source_hashes={name:sha(root/name) for name in paths}
artifact_root=Path(settings['CARGO_TARGET_DIR'])/'precompiled-stdlib'
artifact_names=['runtime.vbca','runtime.core_metadata','runtime.symbol_graph','runtime.vbca.checksum','runtime.vbca.schema']
artifacts={name:sha(artifact_root/name) for name in artifact_names}
receipt={'source':expected,'tree':git('rev-parse','HEAD^{tree}'),'cwd':str(root),'command':cmd,'environment_overrides':settings,'deadline_seconds':600,'source_paths':source_hashes,'artifacts_before':artifacts,'artifact_directory':str(artifact_root),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'cargo':subprocess.check_output(['cargo','--version'],text=True).strip(),'scope':'Focused T1704 parsed source/serialized VBC/interpreter or production LLVM lowering/JIT controls as selected. No ordinary CLI, automatic stdlib baking, AOT executable, no-libc, crypto/registry acceptance.','runner_sha256':sha(__file__)}
(out/'started.json').write_text(json.dumps(receipt,indent=2)+'\n')
start=time.monotonic(); failure=None
with (out/'cargo.jsonl').open('w') as log, (out/'stderr.log').open('w') as err:
 p=subprocess.Popen(cmd,cwd=root,env=env,stdout=log,stderr=err,start_new_session=True)
 (out/'process.json').write_text(json.dumps({'pid':p.pid,'runner_pid':os.getpid()})+'\n')
 while p.poll() is None:
  time.sleep(1)
  content=(out/'stderr.log').read_text(errors='replace')
  if re.search(r'Compiling (z3-sys|cvc5-sys|verum_stdlib_precompiler) ',content): failure='refused unexpected solver/stdlib bootstrap'
  elif re.search(r'(Performing build step for|Building LLVM|Configuring LLVM|-- Building:)',content): failure='refused unexpected native toolchain bootstrap'
  elif time.monotonic()-start>600: failure='deadline'
  if failure:
   os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=10)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
   break
records=[]
for line in (out/'cargo.jsonl').read_text(errors='replace').splitlines():
 try: item=json.loads(line)
 except json.JSONDecodeError: continue
 if item.get('reason')=='compiler-artifact' and item.get('executable'):
  exe=Path(item['executable']); records.append({'path':str(exe),'sha256':sha(exe),'size_bytes':exe.stat().st_size,'target':item['target']['name'],'fresh':item.get('fresh')})
text=(out/'cargo.jsonl').read_text(errors='replace')
receipt.update(returncode=p.returncode,failure=failure,elapsed_seconds=round(time.monotonic()-start,3),source_after=git('rev-parse','HEAD'),source_paths_unchanged=source_hashes=={name:sha(root/name) for name in paths},clean_after=not git('status','--porcelain'),artifacts_after={name:sha(artifact_root/name) for name in artifact_names},executables=records,logs_sha256={name:sha(out/name) for name in ['cargo.jsonl','stderr.log']},test_results=re.findall(r'test result: [^\n]+',text),ir_sha256={str(path.relative_to(out)):sha(path) for path in (out/'ir').glob('*')})
(out/'result.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({key:receipt[key] for key in ['source','returncode','failure','elapsed_seconds','source_paths_unchanged','test_results','executables']},indent=2))
print((out/'stderr.log').read_text(errors='replace')[-5000:])
for line in text.splitlines():
 if not line.startswith('{'): print(line)
