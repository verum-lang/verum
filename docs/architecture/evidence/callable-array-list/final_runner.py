from pathlib import Path
import subprocess, json, os, time, hashlib, signal, sys, re
root=Path('/Users/taaliman/.codex/worktrees/returned-reference-summary/verum')
out=Path(sys.argv[1]); out.mkdir(exist_ok=False)
env=os.environ.copy(); settings={'CARGO_TARGET_DIR':'/Users/taaliman/.tmp/verum-codex-01a10248-registry-client/target','CARGO_INCREMENTAL':'0','CARGO_BUILD_JOBS':'2','VERUM_NO_AUTO_PRECOMPILE':'1','VERUM_LLVM_DIR':'/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install','RUST_MIN_STACK':'16777216'}; env.update(settings)
cmd=['cargo','test','--locked','--offline','--manifest-path',str(root/'Cargo.toml'),'-p','verum_vbc','--no-default-features','--features','compression,table_dispatch,codegen,ffi']+sys.argv[2:]
sha=subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip()
record={'source_commit':sha,'command':cmd,'environment':settings,'timeout_seconds':1200,'scope':'Parsed source to VBC wire to interpreter callable array/List conversion controls; no typechecker, real stdlib archive, ordinary CLI, automatic bake or AOT acceptance'}
if '--lib' in cmd:
 record['scope']='VBC library suite with compression,table_dispatch,codegen,ffi; selection is the exact recorded command; no ordinary CLI, automatic stdlib bake or AOT acceptance. Inherited precompiled artifacts are held unchanged; this does not certify a fresh producer.'
paths=['crates/verum_vbc/src/codegen/array_coercions.rs','crates/verum_vbc/src/codegen/context.rs','crates/verum_vbc/src/codegen/mod.rs','crates/verum_vbc/src/codegen/statements.rs','crates/verum_vbc/src/codegen/expressions.rs','crates/verum_vbc/tests/callable_array_list_conversion.rs','crates/verum_vbc/tests/codegen/array_result_facts.rs']
artifact_root=Path(settings['CARGO_TARGET_DIR'])/'precompiled-stdlib'
artifact_names=['runtime.vbca','runtime.core_metadata','runtime.symbol_graph','runtime.vbca.checksum','runtime.vbca.schema']
record['artifacts_before']={name:hashlib.sha256((artifact_root/name).read_bytes()).hexdigest() for name in artifact_names}
record['source_paths']={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in paths}
(out/'started.json').write_text(json.dumps(record,indent=2)+'\n')
start=time.monotonic(); failure=None
with (out/'gate.log').open('w') as log:
 p=subprocess.Popen(cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
 while p.poll() is None:
  time.sleep(1)
  content=(out/'gate.log').read_text(errors='replace')
  if 'Compiling z3-sys ' in content: failure='refused fresh z3-sys build'
  elif any(x in content for x in ['Compiling verum_codegen ', 'Compiling verum_stdlib_precompiler ', 'Compiling verum_llvm ']): failure='refused native or stdlib producer rebuild'
  elif time.monotonic()-start>1200: failure='timeout'
  if failure:
   os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=10)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
   break
content=(out/'gate.log').read_text(errors='replace')
record.update(exit_code=p.returncode,wall_seconds=round(time.monotonic()-start,3),failure=failure,source_unchanged=sha==subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip(),log_sha256=hashlib.sha256((out/'gate.log').read_bytes()).hexdigest())
record['source_paths_unchanged']=record['source_paths']=={name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in paths}
record['executables']={str(exe):hashlib.sha256(exe.read_bytes()).hexdigest() for match in re.finditer(r'Running [^\n]* \(([^)]+)\)',content) if (exe:=Path(match.group(1))).is_file()}
record['artifacts_after']={name:hashlib.sha256((artifact_root/name).read_bytes()).hexdigest() for name in artifact_names}
record['test_results']=re.findall(r'test result: [^\n]+',content)
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:record[k] for k in ['source_commit','exit_code','wall_seconds','failure','source_paths_unchanged','test_results']}))
print(content[-3500:])
