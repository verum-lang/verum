import hashlib,json,os,pathlib,signal,subprocess,sys,time
root=pathlib.Path('/Users/taaliman/.codex/worktrees/returned-reference-summary/verum')
out=pathlib.Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=False)
env_overrides={'CARGO_TARGET_DIR':'/Users/taaliman/.tmp/verum-codex-01a10248/native-target','CARGO_BUILD_JOBS':'2','CARGO_INCREMENTAL':'0','VERUM_NO_AUTO_PRECOMPILE':'1','VERUM_LLVM_DIR':'/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install','RUST_MIN_STACK':'16777216'}
env_overrides['VERUM_T1700_IR_DIR']=str(out/'ir')
cmd=['cargo','test','--offline','--locked','--no-fail-fast','-p','verum_codegen']
for target in sys.argv[2:]:cmd+=['--test',target]
cmd+=['--message-format=json','--','--nocapture','--test-threads=1']
sha=lambda p:hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
source=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()
r={'source':source,'command':cmd,'cwd':str(root),'environment_overrides':env_overrides,'deadline_seconds':600}
paths=['crates/verum_vbc/src/codegen/array_coercions.rs','crates/verum_vbc/src/codegen/context.rs','crates/verum_vbc/src/codegen/mod.rs','crates/verum_vbc/src/codegen/expressions.rs','crates/verum_vbc/src/codegen/statements.rs','crates/verum_codegen/tests/callable_array_list_conversion.rs']; r['source_paths']={n:sha(root/n) for n in paths}
r['scope']='Parsed source and decoded serialized VBC through production LLVM lowering and JIT; bounded host allocation substrate. No ordinary CLI/AOT, fresh standard-library archive, allocator lifecycle, no-libc or registry acceptance.'
artifact_root=pathlib.Path(env_overrides['CARGO_TARGET_DIR'])/'precompiled-stdlib'; names=['runtime.vbca','runtime.core_metadata','runtime.symbol_graph','runtime.vbca.checksum','runtime.vbca.schema']; r['artifacts_before']={n:sha(artifact_root/n) for n in names}
assert not subprocess.check_output(['git','-C',str(root),'status','--porcelain'],text=True)
start=time.monotonic();timeout=False
with (out/'cargo.jsonl').open('w') as stdout,(out/'stderr.log').open('w') as stderr:
 p=subprocess.Popen(cmd,cwd=root,env=os.environ|env_overrides,stdout=stdout,stderr=stderr,start_new_session=True)
 try:p.wait(timeout=600)
 except subprocess.TimeoutExpired:timeout=True;os.killpg(p.pid,signal.SIGTERM);p.wait(timeout=15)
r.update(returncode=p.returncode,timed_out=timeout,elapsed_seconds=round(time.monotonic()-start,3),source_after=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip())
r['executables']=[]
for line in (out/'cargo.jsonl').read_text().splitlines():
 try:item=json.loads(line)
 except:continue
 if item.get('reason')=='compiler-artifact' and item.get('executable'):
  exe=item['executable'];r['executables'].append({'path':exe,'sha256':sha(exe)})
r['logs_sha256']={n:sha(out/n) for n in ['cargo.jsonl','stderr.log']}
r['source_paths_unchanged']=r['source_paths']=={n:sha(root/n) for n in paths}
r['artifacts_after']={n:sha(artifact_root/n) for n in names}
r['ir_sha256']={str(p.relative_to(out)):sha(p) for p in (out/'ir').glob('*.ll')}
(out/'result.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({k:r[k] for k in ['source','returncode','timed_out','elapsed_seconds','source_paths_unchanged']})); print((out/'cargo.jsonl').read_text()[-3500:]);print((out/'stderr.log').read_text()[-1500:])
