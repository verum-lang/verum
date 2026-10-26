/-! Information loss obstructs exact decoding, independently of implementation.
The history/projection/query may be arbitrary; application correspondence is a
separate obligation. No finiteness, probability or statistical model is assumed.
-/
namespace DiagnosticFactorization

set_option warningAsError true
universe u v w x

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

/-- Retain every reachable history consistent with the observation. -/
def Compatible {H : Type u} {D : Type v} {A : Type w}
    (reachable : H → Prop) (projection : H → D) (query : H → A)
    (observed : D) (answer : A) : Prop :=
  ∃ h, reachable h ∧ projection h = observed ∧ query h = answer

def FibreConstant {H : Type u} {D : Type v} {A : Type w}
    (reachable : H → Prop) (projection : H → D) (query : H → A) : Prop :=
  ∀ a b, reachable a → reachable b → projection a = projection b → query a = query b

/-- Exact initialization is a unique value on each occupied reachable fibre.
No representative, default state, or value on an unreachable diagnostic is chosen.
This constructive characterization does not synthesize a decoder algorithm. -/
theorem reachable_initialization_iff {H : Type u} {D : Type v} {A : Type w}
    (reachable : H → Prop) (projection : H → D) (query : H → A) :
    FibreConstant reachable projection query ↔
      ∀ d, (∃ h, reachable h ∧ projection h = d) →
        ∃ a, Compatible reachable projection query d a ∧
          ∀ b, Compatible reachable projection query d b → b = a := by
  constructor
  · intro constant d witness
    obtain ⟨h, hr, hd⟩ := witness
    refine ⟨query h, ⟨h, hr, hd, rfl⟩, ?_⟩
    intro answer compatible
    obtain ⟨k, kr, kd, ka⟩ := compatible
    exact ka.symm.trans (constant k h kr hr (kd.trans hd.symm))
  · intro unique a b ar br collision
    obtain ⟨answer, _, only⟩ := unique (projection a) ⟨a, ar, rfl⟩
    exact (only (query a) ⟨a, ar, rfl, rfl⟩).trans
      (only (query b) ⟨b, br, collision.symm, rfl⟩).symm

/-- Transport the entire compatible family using the same history witness. -/
theorem compatible_family_image {H : Type u} {D : Type v} {A : Type w} {B : Type x}
    (reachable : H → Prop) (projection : H → D) (coarse : H → A) (fine : H → B)
    (forget : B → A) (commutes : ∀ h, forget (fine h) = coarse h) (d : D) (a : A) :
    (∃ b, Compatible reachable projection fine d b ∧ forget b = a) ↔
      Compatible reachable projection coarse d a := by
  constructor
  · rintro ⟨b, ⟨h, hr, hd, hb⟩, ba⟩
    exact ⟨h, hr, hd, (commutes h).symm.trans ((congrArg forget hb).trans ba)⟩
  · rintro ⟨h, hr, hd, ha⟩
    exact ⟨fine h, ⟨h, hr, hd, rfl⟩, (commutes h).trans ha⟩

/-- info: 'DiagnosticFactorization.reachable_initialization_iff' does not depend on any axioms -/
#guard_msgs in
#print axioms reachable_initialization_iff
/-- info: 'DiagnosticFactorization.compatible_family_image' does not depend on any axioms -/
#guard_msgs in
#print axioms compatible_family_image

end DiagnosticFactorization
