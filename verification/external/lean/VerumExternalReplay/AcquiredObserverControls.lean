import VerumExternalReplay.AcquiredObserver
import VerumExternalReplay.ExactNames

/- Synthetic semantic controls; frozen Rust observations are replayed separately. -/
namespace AcquiredObserverControls
open AcquiredObserver
set_option warningAsError true

def first : Event := ⟨1, 2, some ⟨3, 4⟩, 5, 6, 7, 8, 9, some 255⟩
def last : Event := ⟨1, 2, some ⟨3, 4⟩, 10, 11, 12, 13, 14, none⟩
def query : Query := ⟨1, 2, some ⟨3, 4⟩, 15, 16, 17, 18, some 19, some 20⟩

theorem initializer_primitives :
    apply 0 .unit [] = .ok .unit ∧
    apply 0 .zero [] = .ok (.nat 0) ∧
    apply 0 .emptyTrace [] = .ok (.trace []) ∧
    apply 0 .identity [.pair (.trace [first]) (.nat 300)] =
      .ok (.pair (.trace [first]) (.nat 300)) := ⟨rfl, rfl, rfl, rfl⟩

theorem initializer_arity_is_exact :
    apply 0 .unit [.unit] = .error .arity ∧
    apply 0 .zero [.nat 0] = .error .arity ∧
    apply 0 .emptyTrace [.trace []] = .error .arity ∧
    apply 0 .identity [] = .error .arity := ⟨rfl, rfl, rfl, rfl⟩

theorem pose_is_optional_and_preserved :
    apply 0 .eventPose [.event first] = .ok (.nat 255) ∧
    apply 0 .eventPose [.event last] = .error .missing ∧
    apply 0 .queryPose [.query query] = .ok (.nat 19) ∧
    apply 0 .queryPose [.query { query with pose := none }] = .error .missing ∧
    apply 0 .eventPose [.query query] = .error .sort ∧
    apply 0 .queryPose [.event first] = .error .sort ∧
    apply 0 .equal [.event first, .event { first with pose := none }] = .ok (.bool false) :=
  ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

theorem trace_order_and_capacity :
    apply 2 .append [.trace [first], .event last] = .ok (.trace [first, last]) ∧
    apply 1 .append [.trace [first], .event last] = .error .traceCapacity ∧
    apply 2 .at [.trace [first, last], .nat 0] = .ok (.event first) ∧
    apply 2 .last [.trace [first, last]] = .ok (.event last) ∧
    apply 2 .length [.trace [first, last]] = .ok (.nat 2) := ⟨rfl, rfl, rfl, rfl, rfl⟩

theorem projections_preserve_full_fields :
    apply 2 .eventInput [.event first] = .ok (.nat 6) ∧
    apply 2 .eventRaw [.event first] = .ok (.nat 5) ∧
    apply 2 .eventAction [.event first] = .ok (.action 7) ∧
    apply 2 .eventAge [.event first] = .ok (.nat 9) ∧
    apply 2 .queryInput [.query query] = .ok (.nat 16) ∧
    apply 2 .queryRaw [.query query] = .ok (.nat 15) ∧
    apply 2 .queryAge [.query query] = .ok (.nat 18) ∧
    apply 2 .queryAction [.query query] = .ok (.action 20) := ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

theorem structured_operations :
    apply 2 .pair [.nat 300, .bool true] = .ok (.pair (.nat 300) (.bool true)) ∧
    apply 2 .first [.pair (.nat 300) (.bool true)] = .ok (.nat 300) ∧
    apply 2 .second [.pair (.nat 300) (.bool true)] = .ok (.bool true) ∧
    apply 2 .equal [.nat 300, .nat 44] = .ok (.bool false) ∧
    apply 2 .equal [.event first, .event { first with owner := none }] = .ok (.bool false) ∧
    apply 2 .select [.bool false, .nat 300, .unit] = .ok .unit ∧
    apply 2 .next [.nat 300] = .ok (.outcome (.next 300)) ∧
    apply 2 .refused [.nat 300] = .ok (.outcome (.refused 300)) := ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

theorem checked_arithmetic_and_errors :
    apply 2 .add [.nat 18446744073709551614, .nat 1] = .ok (.nat 18446744073709551615) ∧
    apply 2 .add [.nat 18446744073709551615, .nat 1] = .error .overflow ∧
    apply 2 .rem [.nat 14, .nat 3] = .ok (.nat 2) ∧
    apply 2 .rem [.nat 14, .nat 0] = .error .missing ∧
    apply 2 .last [.trace []] = .error .missing ∧
    apply 2 .at [.trace [first], .nat 18446744073709551615] = .error .missing ∧
    apply 2 .add [.unit] = .error .arity ∧
    apply 2 .add [.unit, .unit] = .error .sort := ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

theorem primitive_resource_exact (error : Error) :
    primitiveClass error = .resource ↔ error = .traceCapacity ∨ error = .allocation := by
  cases error <;> simp [primitiveClass]

theorem refusal_classification :
    executionClass .undefined false (some .resource) = .resource ∧
    executionClass .undefined false (some .undefined) = .undefined ∧
    executionClass .budget false none = .resource ∧
    executionClass .undefined true none = .resource := ⟨rfl, rfl, rfl, rfl⟩

theorem metered_names_preserve_refusals :
    ExactNames.metered 1 [300] 300 1 false = ([300], .ok 0, 0) ∧
    ExactNames.metered 1 [300] 44 2 false = ([300], .error .capacity, 0) ∧
    ExactNames.metered 1 [300] 44 1 false = ([300], .error .work, 0) ∧
    ExactNames.metered 1 [300] 300 1 true = ([300], .error .poisoned, 1) := ⟨rfl, rfl, rfl, rfl⟩

end AcquiredObserverControls
