import VerumExternalReplay.ExactMass

/-! Causal finite-body prediction and exact canonical-image accounting.
A kernel selects a probability before observing the next bit and then updates
its state. The state can contain learned context and retained counts. This model
supplies neither a learner nor a native adapter. A fixed-length residual mask
may depend on past observations, but must be fixed before the current body.
A lossless typed encoding alone does not imply a normalized distribution on
values: excluded wire words retain their mass in the explicit invalid branch. -/
namespace CausalCoding
open ExactMass
set_option warningAsError true

structure Kernel (State : Type) where
  probability : State → Bool → Fraction
  next : State → Bool → State
  normalized : ∀ s, Equal (add (probability s false) (probability s true)) one

variable {State Value : Type}

def words : Nat → List (List Bool)
  | 0 => [[]]
  | n+1 => (words n).map (false :: ·) ++ (words n).map (true :: ·)

def likelihood (kernel : Kernel State) (state : State) : List Bool → Fraction
  | [] => one
  | bit :: rest => mul (kernel.probability state bit) (likelihood kernel (kernel.next state bit) rest)

def advance (kernel : Kernel State) (state : State) : List Bool → State
  | [] => state
  | bit :: rest => advance kernel (kernel.next state bit) rest

/-- Held-out scoring continues from the trained state without rescoring the prefix. -/
theorem likelihood_append (kernel : Kernel State) (state : State) (past future : List Bool) :
    Equal (likelihood kernel state (past ++ future))
      (mul (likelihood kernel state past) (likelihood kernel (advance kernel state past) future)) := by
  induction past generalizing state with
  | nil => exact eq_symm (eq_trans (mul_comm _ _) (mul_one _))
  | cons bit rest ih =>
    simp only [List.cons_append, likelihood, advance, mul_assoc_exact]
    exact mul_congr_left _ (ih _)

/-- Truncate a long prediction and zero-extend a short one, preserving length. -/
def residual : List Bool → List Bool → List Bool
  | [], body => body
  | _ :: _, [] => []
  | predicted :: prediction, bit :: body => (Bool.xor predicted bit) :: residual prediction body

theorem residual_length (prediction body : List Bool) :
    (residual prediction body).length = body.length := by
  induction prediction generalizing body with
  | nil => rfl
  | cons p ps ih => cases body <;> simp [residual, ih]

theorem residual_append (prediction past future : List Bool) :
    residual prediction (past ++ future) =
      residual prediction past ++ residual (prediction.drop past.length) future := by
  induction past generalizing prediction with
  | nil => cases prediction <;> simp [residual]
  | cons bit rest ih =>
    cases prediction with
    | nil => simp [residual]
    | cons p ps => simpa only [List.cons_append, residual, List.length_cons,
        List.drop_succ_cons] using congrArg (Bool.xor p bit :: ·) (ih ps)

theorem residual_involution (prediction body : List Bool) :
    residual prediction (residual prediction body) = body := by
  induction prediction generalizing body with
  | nil => rfl
  | cons p ps ih =>
    cases body with
    | nil => rfl
    | cons b bs => cases p <;> cases b <;> simp [residual, ih]

theorem residual_injective (prediction : List Bool) (a b : List Bool)
    (same : residual prediction a = residual prediction b) : a = b := by
  have h := congrArg (residual prediction) same
  simpa only [residual_involution] using h

theorem word_length {n : Nat} {body : List Bool} (h : body ∈ words n) : body.length = n := by
  induction n generalizing body with
  | zero => simpa [words] using h
  | succ n ih =>
    simp only [words, List.mem_append, List.mem_map] at h
    rcases h with ⟨tail, ht, rfl⟩ | ⟨tail, ht, rfl⟩ <;> simp [ih ht]

theorem word_complete (body : List Bool) : body ∈ words body.length := by
  induction body with
  | nil => simp [words]
  | cons bit rest ih => cases bit <;> simp [words, ih]

theorem words_nodup (n : Nat) : (words n).Nodup := by
  induction n with
  | zero => simp [words]
  | succ n ih =>
    have branch (bit : Bool) : ((words n).map (bit :: ·)).Nodup := by
      simpa [List.Nodup, List.pairwise_map] using ih
    apply List.nodup_append.mpr ⟨branch false, branch true, ?_⟩
    intro a ha b hb
    obtain ⟨x, _, rfl⟩ := List.mem_map.mp ha
    obtain ⟨y, _, rfl⟩ := List.mem_map.mp hb
    simp

private theorem sum_append (xs ys : List Fraction) :
    Equal (sum (xs ++ ys)) (add (sum xs) (sum ys)) := by
  induction xs with
  | nil => exact eq_symm (eq_trans (add_comm _ _) (add_zero _))
  | cons x xs ih => exact eq_trans (add_congr_left x ih) (eq_symm (add_assoc _ _ _))

private theorem branches_normalized (kernel : Kernel State) (state : State)
    (left right : Fraction) (hl : Equal left one) (hr : Equal right one) :
    Equal (add (mul (kernel.probability state false) left)
               (mul (kernel.probability state true) right)) one := by
  apply eq_trans (add_congr_right _ (eq_trans (mul_congr_left _ hl) (mul_one _)))
  apply eq_trans (add_congr_left _ (eq_trans (mul_congr_left _ hr) (mul_one _)))
  exact kernel.normalized state

/-- Every finite causal binary tree is normalized from every retained state. -/
theorem body_normalized (kernel : Kernel State) (state : State) (n : Nat) :
    Equal (sum ((words n).map (likelihood kernel state))) one := by
  induction n generalizing state with
  | zero => exact add_zero _
  | succ n ih =>
    simp only [words, List.map_append, List.map_map, Function.comp_def, likelihood]
    apply eq_trans (sum_append _ _)
    apply eq_trans (add_congr_right _ (sum_scale _ _ _))
    apply eq_trans (add_congr_left _ (sum_scale _ _ _))
    exact branches_normalized kernel state _ _ (ih _) (ih _)

/-- A prediction fixed before the body permutes the complete word space.
It need not be a good prediction, and its length need not equal the body length. -/
theorem residual_normalized (kernel : Kernel State) (state : State) (prediction : List Bool) (n : Nat) :
    Equal (sum ((words n).map (fun body => likelihood kernel state (residual prediction body)))) one := by
  induction n generalizing state prediction with
  | zero => cases prediction <;> exact add_zero _
  | succ n ih =>
    cases prediction with
    | nil => exact body_normalized kernel state (n+1)
    | cons p ps =>
      cases p <;>
        simp only [words, List.map_append, List.map_map, Function.comp_def, residual,
                   Bool.xor_false, Bool.false_xor, Bool.true_xor, likelihood]
      · apply eq_trans (sum_append _ _)
        apply eq_trans (add_congr_right _ (sum_scale _ _ _))
        apply eq_trans (add_congr_left _ (sum_scale _ _ _))
        exact branches_normalized kernel state _ _ (ih _ ps) (ih _ ps)

      · apply eq_trans (sum_append _ _)
        apply eq_trans (add_congr_right _ (sum_scale _ _ _))
        apply eq_trans (add_congr_left _ (sum_scale _ _ _))
        apply eq_trans (add_comm _ _)
        exact branches_normalized kernel state _ _ (ih _ ps) (ih _ ps)

/-- A common causal kernel and fixed mask give the same prefix marginal for
every suffix length. This is stronger than normalization separately by length. -/
theorem prefix_marginal (kernel : Kernel State) (state : State)
    (prediction past : List Bool) (n : Nat) :
    Equal (sum ((words n).map (fun future =>
      likelihood kernel state (residual prediction (past ++ future)))))
      (likelihood kernel state (residual prediction past)) := by
  have paths (future : List Bool) :
      Equal (likelihood kernel state (residual prediction (past ++ future)))
        (mul (likelihood kernel state (residual prediction past))
          (likelihood kernel (advance kernel state (residual prediction past))
            (residual (prediction.drop past.length) future))) := by
    rw [residual_append]
    exact likelihood_append kernel state _ _
  apply eq_trans (sum_congr (words n) _ _ (fun future _ => paths future))
  apply eq_trans (sum_scale _ _ _)
  apply eq_trans (mul_congr_left _ (residual_normalized kernel _ _ n))
  exact mul_one _

structure Codec (Value : Type) where
  encode : Value → List Bool
  decode : List Bool → Option Value
  roundtrip : ∀ value, decode (encode value) = some value

theorem encode_injective (codec : Codec Value) (a b : Value) (h : codec.encode a = codec.encode b) : a = b := by
  have decoded := congrArg codec.decode h
  simpa only [codec.roundtrip, Option.some.injEq] using decoded

def canonical (codec : Codec Value) (body : List Bool) : Bool :=
  match codec.decode body with
  | none => false
  | some value => codec.encode value == body

theorem encoded_canonical (codec : Codec Value) (value : Value) :
    canonical codec (codec.encode value) = true := by simp [canonical, codec.roundtrip]

theorem canonical_iff (codec : Codec Value) (body : List Bool) :
    canonical codec body = true ↔ ∃ value, codec.encode value = body := by
  constructor
  · unfold canonical
    split
    · simp
    · rename_i value h
      intro equal
      exact ⟨value, by simpa using equal⟩
  · rintro ⟨value, rfl⟩
    exact encoded_canonical codec value

/-- Both sides of the partition remain visible; no renormalization is implicit. -/
def selectedMass (kernel : Kernel State) (state : State) (prediction : List Bool)
    (n : Nat) (keep : List Bool → Bool) : Fraction :=
  sum ((words n).map (fun body => if keep body then likelihood kernel state (residual prediction body) else zero))

theorem selected_bounded (kernel : Kernel State) (state : State) (prediction : List Bool)
    (n : Nat) (keep : List Bool → Bool) : AtMost (selectedMass kernel state prediction n keep) one := by
  apply le_trans (sum_mono (words n) _ _ ?_) (eq_le (residual_normalized kernel state prediction n))
  intro body _
  split
  · exact le_refl _
  · exact zero_le _

theorem selected_partition (kernel : Kernel State) (state : State) (prediction : List Bool)
    (n : Nat) (keep : List Bool → Bool) :
    Equal (add (selectedMass kernel state prediction n keep)
      (selectedMass kernel state prediction n (fun body => !keep body))) one := by
  apply eq_trans (eq_symm (sum_add (words n) _ _))
  apply eq_trans (sum_congr (words n) _ _ ?_) (residual_normalized kernel state prediction n)
  intro body _
  cases h : keep body <;> simp only [h, Bool.not_false, Bool.not_true, Bool.false_eq_true, ↓reduceIte]
  · exact eq_trans (add_comm _ _) (add_zero _)
  · exact add_zero _

private theorem add_same_iff_zero (a b : Fraction) : Equal (add a b) a ↔ b.numerator = 0 := by
  have ha := a.positive
  simp only [Equal, add, Nat.add_mul]
  constructor
  · intro h
    have : b.numerator * a.denominator * a.denominator = 0 := by
      simp only [Nat.mul_comm, Nat.mul_left_comm] at h ⊢
      omega
    simpa only [Nat.mul_eq_zero, Nat.ne_of_gt ha, or_false] using this
  · intro h
    simp [h, Nat.mul_comm, Nat.mul_left_comm]

/-- Restriction is normalized exactly when its omitted mass is zero. -/
theorem selected_normalized_iff (kernel : Kernel State) (state : State) (prediction : List Bool)
    (n : Nat) (keep : List Bool → Bool) :
    Equal (selectedMass kernel state prediction n keep) one ↔
      (selectedMass kernel state prediction n (fun body => !keep body)).numerator = 0 := by
  have partition := selected_partition kernel state prediction n keep
  constructor
  · intro normalized
    exact (add_same_iff_zero _ _).mp (eq_trans partition (eq_symm normalized))
  · intro empty
    exact eq_trans (eq_symm ((add_same_iff_zero _ _).mpr empty)) partition

end CausalCoding
