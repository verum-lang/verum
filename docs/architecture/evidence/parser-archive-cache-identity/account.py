from pathlib import Path
import hashlib,json,re,sys
out=Path(sys.argv[1])
receipt_path=out/'result.json'
receipt=json.loads(receipt_path.read_text())
log_path=out/'gate.log'
log=log_path.read_text(errors='replace')
lines=log.splitlines()
starts=[]
for index,line in enumerate(lines):
 if not line.lstrip().startswith('Running ') or '--test-threads=1' not in line: continue
 for name,executable in receipt['executables'].items():
  if executable['path'] in line: starts.append((index,name))
assert len(starts)==2 and len({name for _,name in starts})==2,starts
counts={}
for ordinal,(start,name) in enumerate(starts):
 end=starts[ordinal+1][0] if ordinal+1<len(starts) else len(lines)
 segment='\n'.join(lines[start:end])
 announced=re.findall(r'^running ([0-9]+) tests?$',segment,re.M)
 results=re.findall(r'^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored; ([0-9]+) measured; ([0-9]+) filtered out;[^\n]*',segment,re.M)
 assert announced and results,name
 status,passed,failed,ignored,measured,filtered=results[-1]
 counts[name]={'cargo_invocation_line':start+1,'announced':int(announced[0]),'status':status,'passed':int(passed),'failed':int(failed),'ignored':int(ignored),'measured':int(measured),'filtered':int(filtered)}
 assert int(announced[0])==sum(map(int,[passed,failed,ignored,measured]))
 assert filtered=='0'
record={'raw_receipt_sha256':hashlib.sha256(receipt_path.read_bytes()).hexdigest(),'raw_log_sha256':hashlib.sha256(log_path.read_bytes()).hexdigest(),'source_commit':receipt['source_commit'],'accounting_script_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'explanation':'Per-target results use each actual Cargo test invocation, its first announced count and its final summary. The original flat baseline parser also saw the successful isolated child summary printed as text inside the expected dependency assertion failure; that child is part of one of the seven controls, not an extra selected target. Original receipt and raw log remain unchanged.','targets':counts}
(out/'target-accounting.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(counts))
