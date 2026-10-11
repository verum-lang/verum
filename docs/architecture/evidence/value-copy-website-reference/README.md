# Value-copy and shared-ownership documentation validation

Website commits `e3401c248ba608401a6742dee6f02370ff5fc9f8` and
`e7556ca40edea2abc6a3ac0560e76c7b96c0e0f7` are integrated on clean local
website main. The memory model, memory library and derives reference now
describe one-level value copying, reference aliasing, Shared identity and
field-specific cloning without promising recursive isolation. The cookbook
removes a mutation example whose cloned Shared handle cannot satisfy unique
access, and distinguishes strong-owner uniqueness from mutable access.

All five website gates pass on the final committed source: build, typecheck,
links, freshness labels and internal-artifact exclusions. Parent review
verifies 2,015 recorded source bindings against 403 committed Git blobs,
five language implementation source bindings, raw gate logs and 26 rendered
content checks. The previous accepted Map pages and executed cookbook fixture
remain byte-identical.

The [manifest](manifest.json) binds the four source pages, rendered HTML,
gate logs, receipts, review scripts and preserved Map example. Files use
deterministic gzip compression; both original and stored hashes are recorded.
The task receipt predates parent review and main integration, so its original
status and scope notes remain unchanged in the retained copy.

These are documentation corrections. They do not establish an explicit
dereference copy fix, general Clone dispatch, a Shared runtime repair,
independent destruction ownership, registry snapshot isolation or native
AOT parity. No new runnable recipe or deployment is recorded.
