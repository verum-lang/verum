T1747 repairs a checker defect found while reducing the separate T1746
preparation-copy failure. A qualified pattern on a borrowed generic sum lost
its instantiated payload type: `Choice.Value(cell)` on `&Choice<Cell>` produced
an unresolved-field error for `cell.padding`, while the owned and unqualified
borrowed forms checked successfully. This packet establishes that checker
cause and repair. The original runtime allocation-guard panic remains
unresolved.

The guard in `crates/verum_types/src/infer/patterns.rs` now inspects through
reference wrappers when checking whether the expanded scrutinee already
contains the selected tag. It keeps the original expanded type for subsequent
binding. It no longer replaces that instance's concrete payload with a fresh
raw declaration template. Qualified named payloads use the same variant arm;
parser assertions pin that route. Unqualified named payloads use the sibling
record arm without a qualifier override, so no additional production change
was made there.

The [manifest](manifest.json) maps every receipt, raw log, runner, source
binding and dependency artifact record to a losslessly compressed object.
Object entries contain hashes for both compressed and original bytes; identical
payloads are stored once. Seven retained test executables remain outside Git
with their exact identities. Every run used the same bounded standalone
checker profile with one Cargo job, no automatic stdlib bake, and cached solver
artifacts. No native compiler or ordinary Verum execution is included.

| Checkpoint | Source | Result |
| --- | --- | --- |
| Original ten controls | `f7becbe2e` | 3 passed, 7 failed with unresolved payload fields |
| Guard repair | `e8565a5ba` | The same 10 controls passed |
| Extended controls | `ef56bfac9` | 14 passed, 1 failed because the test expected display prose instead of the correct structured `UnknownField` diagnostic |
| Corrected refusal oracle | `6fb57964d` | 15 passed |
| Existing generic variant patterns | `6fb57964d` | 4 passed |
| Existing variant patterns | `6fb57964d` | 17 passed |
| Existing imported variant patterns | `6fb57964d` | 16 passed |

The final 52 distinct controls have no ignored or filtered tests. They cover
concrete payload fields, wrong result types, missing fields, declaration
parameter positions, reversed independent instances, nested patterns and named
payloads. The new fixtures exercise borrowed reads; they do not establish
reference mutability/tier preservation or general imported-owner/alias-order
correctness. The existing imported-pattern controls retain their original
synthetic type setup.

Before the baseline, the selected 14 local Rust library units were explicitly
refreshed after backing up only their fingerprint markers and companions.
The baseline proves each was freshly compiled. External solver artifacts and
source timestamps were untouched. The packet retains the exact refresh plan,
operation, marker backups and prior graph-selection evidence. All 8,771
run/source bindings were also checked against the declared Git commits.
The initial graph preparation's gitlink-as-file error occurred before any
compilation; its inputs are retained without a test verdict.

T1747 still requires the unchanged ordinary Map probe to be replayed with the
fresh compiler. T1746 separately requires investigation of the original
preparation-copy allocation panic. This checker gate provides no Map mutation,
copy-isolation, service, native or AOT acceptance, and changes no runtime guard.

The [main-based checker gate](main/review.json) independently passes all 15
focused controls in 31.227 seconds. All 5,300 source bindings, the retained
executable and unchanged standard-library artifacts are verified. This
checks the isolated main candidate without the pending visibility changes.
