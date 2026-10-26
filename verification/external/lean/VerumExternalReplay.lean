-- Top-level umbrella module for `VerumExternalReplay`.
--
-- Every `.lean` file under `VerumExternalReplay/` is a piece of the
-- external-prover replay corpus. `lake build` here drives every
-- imported module through Lean 4 elaboration; any `sorry` remains a
-- `sorry` (that's the honest IOU surface), but type-errors,
-- undefined references, or shape drift would fail elaboration and
-- hence `lake build`.
import VerumExternalReplay.KernelSoundness
import VerumExternalReplay.ReferenceChecker
import VerumExternalReplay.TermOperations
import VerumExternalReplay.TermScope
import VerumExternalReplay.ReductionScope
import VerumExternalReplay.ReductionSemantics
import VerumExternalReplay.ConservativeRefinement
import VerumExternalReplay.ReceiptCredit
import VerumExternalReplay.DiagnosticFactorization
import VerumExternalReplay.ExactTransports
import VerumExternalReplay.TypedPrograms
import VerumExternalReplay.TypedProgramControls
