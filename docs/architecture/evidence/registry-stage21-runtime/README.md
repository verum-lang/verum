# Fresh array-storage build and registry authentication

An ordinary offline CLI build from `49559a042` automatically regenerated the
stdlib archive, metadata and graph with schema `v53-declared-function-bodies`.
The immutable executable and all three artifacts were captured before target
reuse. The [comparison](comparison.json) records their identities and retains
the exact runners, receipts, inputs and losslessly compressed output.

The unchanged 69-byte SHA-256 known-answer fixture now passes. The existing
registry authentication fixture also passes through the actual configuration,
SHA-256 digest, authority and scope implementations. Its project inputs match
the earlier failing run; only the fixture module header is changed by the
existing project assembler. The four retained eight-byte endian controls pass,
including the previously failing inferred big-endian binding.

These results establish ordinary interpreter execution for those inputs. They
do not establish general cryptographic correctness, timing resistance, native
AOT, bounded HTTP request handling, durable publication or deployed-service
readiness. Complete-project and further integration results remain separate.

The unfiltered library run on the same source retains 2,090 passes, ten network
failures and one existing ignore. Eight failures occurred during connection
setup, one reactor result lacked confirmed setup, and the UDP failure discarded
its underlying error. The five source gates passed. Exact failed output and
source identities remain in the library and source-gate receipts.

The complete registry candidate `c02238261` check finishes in 349.31 seconds
with two errors in `src/node/slice.vr`: the spawned result has `JoinHandle`
where the collection expects `Future`, and the awaited pattern still sees a
`Future`. It includes 29 source modules, with all earlier source inputs
unchanged. The previous response-field and I/O-pattern errors are absent.
T1681 remains open; no complete-project pass is claimed.
