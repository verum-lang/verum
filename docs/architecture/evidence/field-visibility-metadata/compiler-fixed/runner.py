from pathlib import Path
import hashlib,json,os,re,shutil,signal,subprocess,sys,time
root=Path('/Users/taaliman/.codex/worktrees/registry-metadata-lookup/verum'); out=Path(sys.argv[1]); mode=sys.argv[2]; expected=sys.argv[3]
commands={
 'fields':['-p','verum_compiler','--lib','pipeline::stdlib_bootstrap::field_visibility_tests::'],
}
assert mode in commands and mode == 'fields'
out.mkdir(exist_ok=False)
settings={'CARGO_TARGET_DIR':'/Users/taaliman/.tmp/verum-codex-01a10248/native-target','CARGO_BUILD_JOBS':'2','CARGO_INCREMENTAL':'0','VERUM_NO_AUTO_PRECOMPILE':'1','VERUM_LLVM_DIR':'/Users/taaliman/projects/oldman/verum-lang/verum/llvm/install','RUST_MIN_STACK':'16777216','VERUM_T1714_EVIDENCE_DIR':str(out)}
env=os.environ.copy();env.update(settings)
cmd=['cargo','test','--offline','--locked','--no-fail-fast',*commands[mode],'--message-format=json','--','--nocapture','--test-threads=1']
def git(*args):return subprocess.check_output(['git',*args],cwd=root,text=True,stderr=subprocess.PIPE).strip()
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
assert git('rev-parse','HEAD')==expected and not git('status','--porcelain')
paths=git('ls-files','Cargo.toml','Cargo.lock','crates/verum_vbc','crates/verum_codegen','crates/verum_compiler','crates/verum_ast','crates/verum_common','crates/verum_types','grammar/verum.ebnf').splitlines()
sources={path:sha(root/path) for path in paths}
artdir=Path(settings['CARGO_TARGET_DIR'])/'precompiled-stdlib'; names=['runtime.vbca','runtime.core_metadata','runtime.symbol_graph','runtime.vbca.checksum','runtime.vbca.schema']; arts={name:sha(artdir/name) for name in names}
receipt={'source':expected,'tree':git('rev-parse','HEAD^{tree}'),'command':cmd,'environment_overrides':settings,'deadline_seconds':600,'source_paths':sources,'artifacts_before':arts,'artifact_directory':str(artdir),'runner_sha256':sha(__file__),'scope':'T1714 parsed source through actual bootstrap producer, archive wire and bincode CoreMetadata. Selection is the exact recorded command. No ordinary CLI/bake/AOT or complete registry acceptance.'}
(out/'started.json').write_text(json.dumps(receipt,indent=2)+'\n');start=time.monotonic();failure=None
with (out/'cargo.jsonl').open('w') as log,(out/'stderr.log').open('w') as err:
 p=subprocess.Popen(cmd,cwd=root,env=env,stdout=log,stderr=err,start_new_session=True)
 (out/'process.json').write_text(json.dumps({'pid':p.pid,'runner_pid':os.getpid()})+'\n')
 while p.poll() is None:
  time.sleep(1);text=(out/'stderr.log').read_text(errors='replace')
  if re.search(r'Compiling (z3-sys|cvc5-sys|verum_stdlib_precompiler) ',text): failure='refused solver/stdlib bootstrap'
  elif re.search(r'(Performing build step for|Building LLVM|Configuring LLVM|-- Building:)',text):failure='refused native toolchain bootstrap'
  elif time.monotonic()-start>600:failure='deadline'
  if failure:
   os.killpg(p.pid,signal.SIGTERM)
   try:p.wait(timeout=10)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
   break
text=(out/'cargo.jsonl').read_text(errors='replace'); exes=[]
for line in text.splitlines():
 try:item=json.loads(line)
 except json.JSONDecodeError:continue
 if item.get('reason')=='compiler-artifact' and item.get('executable'):
  exe=Path(item['executable']); retained=out/exe.name;subprocess.run(['cp','-c',str(exe),str(retained)],check=True);assert sha(exe)==sha(retained)
  exes.append({'path':str(exe),'sha256':sha(exe),'retained':str(retained),'retained_sha256':sha(retained),'target':item['target']['name']})
receipt.update(returncode=p.returncode,failure=failure,elapsed_seconds=round(time.monotonic()-start,3),source_after=git('rev-parse','HEAD'),clean_after=not git('status','--porcelain'),source_paths_unchanged=sources=={path:sha(root/path) for path in paths},artifacts_after={name:sha(artdir/name) for name in names},executables=exes,logs_sha256={name:sha(out/name) for name in ['cargo.jsonl','stderr.log']},test_results=re.findall(r'test result: [^\n]+',text),ir_sha256={str(path.relative_to(out)):sha(path) for path in (out/'ir').glob('*')})
(out/'result.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps({key:receipt[key] for key in ['source','returncode','failure','elapsed_seconds','test_results']}))
print((out/'stderr.log').read_text(errors='replace')[-7000:])
for line in text.splitlines():
 if not line.startswith('{'):print(line)
assert receipt['source_after']==expected and receipt['clean_after'] and receipt['source_paths_unchanged']
assert arts==receipt['artifacts_after']
if p.returncode or failure or not receipt['test_results']:sys.exit(p.returncode if p.returncode and p.returncode>0 else 1)
