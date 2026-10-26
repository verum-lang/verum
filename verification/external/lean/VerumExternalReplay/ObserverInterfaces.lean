import VerumExternalReplay.ObserverAdmission

/-! Structural interface data used by the frozen adapter. Descriptor equality
compares values, including input epoch, port order and support. It is not object
identity, a physical-history certificate, or semantic equivalence of programs. -/
namespace ObserverInterfaces
set_option warningAsError true

structure Descriptor where
  inputEpoch : Nat
  ageMod : Nat
  eventsMod : Nat
  axes : Bool × Bool × Bool
  support : List Nat
  deriving DecidableEq

inductive Port where
  | carrier | key | lifeAge | underClock | pose
  deriving DecidableEq

structure Interface where
  descriptor : Descriptor
  ports : List Port
  deriving DecidableEq

end ObserverInterfaces
