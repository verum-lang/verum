import Init

/- The mathematical principle selected by verum-native-funext/1.
   This audits the rule's foundation, not the soundness of the entire
   executable checker or the correspondence of a runtime implementation. -/
namespace VerumExtensionalFoundation

universe u v

theorem dependent_function_extensionality
    {A : Type u} {B : A → Type v} {f g : (x : A) → B x}
    (pointwise : ∀ x, f x = g x) : f = g :=
  funext pointwise

#print axioms dependent_function_extensionality

end VerumExtensionalFoundation
