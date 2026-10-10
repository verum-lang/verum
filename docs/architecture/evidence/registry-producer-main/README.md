# Registry-driven compiler producer integration

The source commit `86c7bcc1336ef07f3ee39348cc10f51016df113a` integrates
signed array storage and native sign extension, callable return conversion
before lexical cleanup, and complete declared field policies in archives.
It builds on the existing `main` commit `26c43f6bd`.

The unfiltered type-checker suite passes: **4,016 passed, 0 failed,
3 existing ignored**, with unchanged committed source. The recorded preflight
compares every selected bytecode dependency tree with the previously tested
`bfc2ccc7e4174bd4e2cf3708fb05fe7afdd36358`; all match exactly. That bytecode
library gate passed **2,109 tests, 0 failed, 1 existing ignored**. Internal
references, dead module calls, live task citations and authored whitespace
checks also pass.

A later bytecode gate at `77f1ba4b6`, which additionally contains the generic
alias parser fix and record-field consumer work, completed with **2,105 passed,
4 failed, 1 ignored**. The failures are three TCP fixture connection deadlines
and one UDP fixture receive deadline. Their exact logs are retained here;
no cause or regression attribution has yet been established. A prior green
run does not make this newer run green.

The producer landing does not include the pending record-field enforcement,
structural-conversion authority checks or registry authentication candidate.
It does not establish fresh ordinary CLI/archive, registry runtime, AOT or
complete no-libc acceptance. Those require their own gates.

`manifest.json` binds each original receipt/log and its deterministic gzip
copy. Executables remain at the receipt locations and are represented by
hashes; none is embedded in the repository. Commands are the recorded commands,
not rewritten reconstructions. Raw logs preserve their original whitespace.
