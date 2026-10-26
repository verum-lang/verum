/-! Information loss obstructs exact decoding, independently of implementation.
The history/projection/query may be arbitrary; application correspondence is a
separate obligation. No finiteness, probability or statistical model is assumed.
-/
namespace DiagnosticFactorization

set_option warningAsError true
universe u v w

/-- Any decoder agreeing with the query at both histories forces equal answers
    whenever the histories have identical diagnostics. -/
theorem fibre_constant {History : Sort u} {Diagnostic : Sort v} {Answer : Sort w}
    (projection : History → Diagnostic) (query : History → Answer)
    (decoder : Diagnostic → Answer) (a b : History)
    (collision : projection a = projection b)
    (correctA : decoder (projection a) = query a)
    (correctB : decoder (projection b) = query b) : query a = query b :=
  correctA.symm.trans ((congrArg decoder collision).trans correctB)

/-- A single unequal-answer collision rules out every total decoder that is
    correct on both histories, not just a particular reconstruction algorithm. -/
theorem no_decoder_on_pair {History : Sort u} {Diagnostic : Sort v} {Answer : Sort w}
    (projection : History → Diagnostic) (query : History → Answer) (a b : History)
    (collision : projection a = projection b) (distinct : query a ≠ query b) :
    ¬ ∃ decoder : Diagnostic → Answer,
      decoder (projection a) = query a ∧ decoder (projection b) = query b := by
  rintro ⟨decoder, correctA, correctB⟩
  exact distinct (fibre_constant projection query decoder a b collision correctA correctB)

theorem no_global_factorization {History : Sort u} {Diagnostic : Sort v} {Answer : Sort w}
    (projection : History → Diagnostic) (query : History → Answer) (a b : History)
    (collision : projection a = projection b) (distinct : query a ≠ query b) :
    ¬ ∃ decoder : Diagnostic → Answer, ∀ history, decoder (projection history) = query history := by
  rintro ⟨decoder, correct⟩
  exact no_decoder_on_pair projection query a b collision distinct ⟨decoder, correct a, correct b⟩

/-- info: 'DiagnosticFactorization.fibre_constant' does not depend on any axioms -/
#guard_msgs in
#print axioms fibre_constant
/-- info: 'DiagnosticFactorization.no_decoder_on_pair' does not depend on any axioms -/
#guard_msgs in
#print axioms no_decoder_on_pair
/-- info: 'DiagnosticFactorization.no_global_factorization' does not depend on any axioms -/
#guard_msgs in
#print axioms no_global_factorization

end DiagnosticFactorization
