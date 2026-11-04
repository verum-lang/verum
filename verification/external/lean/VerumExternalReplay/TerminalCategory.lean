/-
Independent mathematical replay of core.math.concrete_accessible's terminal
operations. Unit is both the object and arrow carrier; every arrow is ().
This establishes these operation laws, not universal accessibility, the Rust
checker, or correctness of the Verum-to-VBC compiler. The corresponding actual
Verum modules are separately exercised by terminal_category_source.rs.
-/
namespace Verum.TerminalCategory

def identity (_ : Unit) : Unit := ()
def compose (_ _ : Unit) : Unit := ()
def source (_ : Unit) : Unit := ()
def target (_ : Unit) : Unit := ()

theorem identity_source (a : Unit) : source (identity a) = a := by cases a; rfl
theorem identity_target (a : Unit) : target (identity a) = a := by cases a; rfl
theorem composition_source (f g : Unit) : source (compose f g) = source f := rfl
theorem composition_target (f g : Unit) : target (compose f g) = target g := rfl
theorem left_identity (f : Unit) : compose (identity (source f)) f = f := by cases f; rfl
theorem right_identity (f : Unit) : compose f (identity (target f)) = f := by cases f; rfl
theorem associativity (f g h : Unit) : compose (compose f g) h = compose f (compose g h) := rfl

#print axioms identity_source
#print axioms identity_target
#print axioms composition_source
#print axioms composition_target
#print axioms left_identity
#print axioms right_identity
#print axioms associativity
end Verum.TerminalCategory
