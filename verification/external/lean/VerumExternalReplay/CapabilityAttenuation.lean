import Std

namespace Verum.CapabilityAttenuation

inductive Right where
  | read | write | readWrite | admin | other (id : Nat)
  deriving DecidableEq

def covers (held requested : Right) : Bool :=
  held == requested || held == .admin ||
    (held == .readWrite && (requested == .read || requested == .write))

theorem covers_trans {a b c : Right}
    (ab : covers a b = true) (bc : covers b c = true) : covers a c = true := by
  simp only [covers, Bool.or_eq_true, Bool.and_eq_true, beq_iff_eq] at *
  rcases ab with (rfl | rfl) | ⟨rfl, rfl | rfl⟩
  · exact bc
  · exact Or.inl (Or.inr rfl)
  · simp_all
  · simp_all

inductive View where
  | full | restricted (rights : List Right)

def grants (rights : List Right) (requested : Right) : Bool :=
  rights.any (fun held => covers held requested)

def accepts : View → View → Bool
  | .full, _ => true
  | .restricted _, .full => false
  | .restricted held, .restricted required => required.all (grants held)

def canUse : View → Right → Bool
  | .full, _ => true
  | .restricted rights, r => grants rights r

theorem preserves_permissions {source target : View} (accepted : accepts source target = true)
    (r : Right) (allowed : canUse target r = true) : canUse source r = true := by
  cases source with
  | full => rfl
  | restricted held =>
    cases target with
    | full => contradiction
    | restricted required =>
      obtain ⟨intermediate, member, supplied⟩ := List.any_eq_true.mp allowed
      obtain ⟨original, owned, granted⟩ :=
        List.any_eq_true.mp (List.all_eq_true.mp accepted intermediate member)
      exact List.any_eq_true.mpr ⟨original, owned, covers_trans granted supplied⟩

end Verum.CapabilityAttenuation
