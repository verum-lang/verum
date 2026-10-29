import VerumExternalReplay.CompositionalFamily

/-! Coverage of finite lexical program trees by the layered declaration schema.
Primitive and literal indices refer to one fixed, validated source alphabet.
Each control occurrence owns its children in instruction/side order; sharing a
child catalogue does not identify the two selected branch bodies. This is a
declaration-membership theorem, not a codec, VM, native cursor, resource or
scheduler theorem. Native index-capacity checks remain additional premises. -/
namespace DeclarationCoverage
set_option warningAsError true

inductive Form where
  | primitive : Nat → Form
  | branch : Nat → Form
  | fold : Nat → Form
  deriving DecidableEq

structure Instruction where
  form : Form
  operands : List Nat
  deriving DecidableEq

inductive Tree where
  | node : Nat → List Instruction → Nat → List Tree → Tree

def Tree.inputs : Tree → Nat
  | .node inputs _ _ _ => inputs

def childWidths : Form → List Nat
  | .primitive _ => []
  | .branch width => [width,width]
  | .fold width => [width]

def arity (primitives : List Nat) : Form → Option Nat
  | .primitive index => primitives[index]?
  | .branch width => some (width+1)
  | .fold width => if 2 ≤ width then some width else none

/-- Independent lexical well-formedness: eager operand scopes, actual selected
output and each control's exact child widths. No search bounds appear here. -/
def frameValid (primitives : List Nat) (literals inputs : Nat)
    (instructions : List Instruction) (output : Nat) (children : List Tree) : Bool :=
  decide (0 < instructions.length) && decide (output < instructions.length) &&
  instructions.zipIdx.all (fun (instruction,position) =>
    arity primitives instruction.form == some instruction.operands.length &&
    instruction.operands.all (fun operand => decide (operand < inputs+position+literals))) &&
  decide (children.map Tree.inputs = instructions.flatMap (fun i => childWidths i.form))

inductive Valid (primitives : List Nat) (literals : Nat) : Tree → Prop where
  | node (inputs instructions output children)
      (frame : frameValid primitives literals inputs instructions output children = true)
      (subtrees : ∀ child ∈ children, Valid primitives literals child) :
      Valid primitives literals (.node inputs instructions output children)

/-- Exactly the declaration cursor's order: branch at every width, then fold
when that same width has room for accumulator and element. -/
def bodyForms (width : Nat) : List Form :=
  if 2 ≤ width then [.branch width,.fold width] else [.branch width]

def catalogue (primitives depth bodyWidth : Nat) : List Form :=
  (List.range primitives).map Form.primitive ++
    match depth with
    | 0 => []
    | _+1 => (List.range (bodyWidth+1)).flatMap bodyForms

/-- Membership in L[depth,inputs]. The independent bounds are shared by all
layers; each child is drawn from the previous layer at its declared width. -/
def covers (primitives : List Nat) (literals steps bodyWidth : Nat) : Nat → Tree → Bool
  | depth, .node inputs instructions output children =>
    frameValid primitives literals inputs instructions output children &&
    decide (instructions.length ≤ steps) &&
    instructions.all (fun i => decide (i.form ∈ catalogue primitives.length depth bodyWidth)) &&
    match depth with
    | 0 => children.isEmpty
    | prior+1 => children.all (covers primitives literals steps bodyWidth prior)

theorem covers_node (primitives literals steps width depth inputs instructions output children) :
    covers primitives literals steps width depth (.node inputs instructions output children) =
      (frameValid primitives literals inputs instructions output children &&
       decide (instructions.length ≤ steps) &&
       instructions.all (fun i => decide (i.form ∈ catalogue primitives.length depth width)) &&
       match depth with
       | 0 => children.isEmpty
       | prior+1 => children.all (covers primitives literals steps width prior)) := by
  cases depth <;> rfl

structure Needs where
  steps : Nat
  depth : Nat
  bodyWidth : Nat
  deriving DecidableEq

def Needs.join (a b : Needs) : Needs :=
  ⟨max a.steps b.steps,max a.depth b.depth,max a.bodyWidth b.bodyWidth⟩

def Needs.Within (a b : Needs) : Prop :=
  a.steps ≤ b.steps ∧ a.depth ≤ b.depth ∧ a.bodyWidth ≤ b.bodyWidth

theorem within_trans {a b c : Needs} (ab : a.Within b) (bc : b.Within c) : a.Within c :=
  ⟨Nat.le_trans ab.1 bc.1,Nat.le_trans ab.2.1 bc.2.1,Nat.le_trans ab.2.2 bc.2.2⟩

theorem join_left (a b : Needs) : a.Within (a.join b) :=
  ⟨Nat.le_max_left _ _,Nat.le_max_left _ _,Nat.le_max_left _ _⟩

theorem join_right (a b : Needs) : b.Within (a.join b) :=
  ⟨Nat.le_max_right _ _,Nat.le_max_right _ _,Nat.le_max_right _ _⟩

theorem fold_base (xs : List Needs) (base : Needs) :
    base.Within (xs.foldr Needs.join base) := by
  induction xs with
  | nil => exact ⟨Nat.le_refl _,Nat.le_refl _,Nat.le_refl _⟩
  | cons x xs ih => exact within_trans ih (join_right x _)

theorem fold_member (xs : List Needs) (base value : Needs) (mem : value ∈ xs) :
    value.Within (xs.foldr Needs.join base) := by
  induction xs with
  | nil => cases mem
  | cons x xs ih =>
    cases List.mem_cons.mp mem with
    | inl same => subst value; exact join_left x _
    | inr rest => exact within_trans (ih rest) (join_right x _)

/-- A single traversal computes sufficient bounds separately. Root input width
is fixed by the requested source; only lexical child widths enter bodyWidth. -/
def Tree.needs (tree : Tree) : Needs :=
  match tree with
  | .node _ instructions _ children =>
    (children.attach.map (fun child =>
      let needs := child.val.needs
      (⟨needs.steps,needs.depth+1,max child.val.inputs needs.bodyWidth⟩ : Needs))).foldr
        Needs.join ⟨instructions.length,0,0⟩
termination_by sizeOf tree
decreasing_by
  have h := List.sizeOf_lt_of_mem child.property
  simp only [Tree.node.sizeOf_spec]
  omega

theorem needs_frame (inputs instructions output children) :
    instructions.length ≤ (Tree.node inputs instructions output children).needs.steps := by
  rw [Tree.needs]
  exact (fold_base _ (⟨instructions.length,0,0⟩ : Needs)).1

theorem needs_child (inputs instructions output children) (child : Tree) (mem : child ∈ children) :
    child.needs.steps ≤ (Tree.node inputs instructions output children).needs.steps ∧
    child.needs.depth+1 ≤ (Tree.node inputs instructions output children).needs.depth ∧
    child.inputs ≤ (Tree.node inputs instructions output children).needs.bodyWidth ∧
    child.needs.bodyWidth ≤ (Tree.node inputs instructions output children).needs.bodyWidth := by
  have found : (⟨child.needs.steps,child.needs.depth+1,
      max child.inputs child.needs.bodyWidth⟩ : Needs) ∈
      children.attach.map (fun c => (⟨c.val.needs.steps,c.val.needs.depth+1,
        max c.val.inputs c.val.needs.bodyWidth⟩ : Needs)) := by
    exact List.mem_map.mpr ⟨⟨child,mem⟩,by simp,rfl⟩
  have bounded := fold_member _ (⟨instructions.length,0,0⟩ : Needs) _ found
  rw [Tree.needs.eq_1]
  exact ⟨bounded.1,bounded.2.1,
    Nat.le_trans (Nat.le_max_left _ _) bounded.2.2,
    Nat.le_trans (Nat.le_max_right _ _) bounded.2.2⟩

theorem primitive_in_catalogue (primitives depth width index : Nat) (bound : index < primitives) :
    Form.primitive index ∈ catalogue primitives depth width := by
  apply List.mem_append_left
  exact List.mem_map.mpr ⟨index,List.mem_range.mpr bound,rfl⟩

theorem branch_in_catalogue (primitives depth width childWidth : Nat) (bound : childWidth ≤ width) :
    Form.branch childWidth ∈ catalogue primitives (depth+1) width := by
  apply List.mem_append_right
  apply List.mem_flatMap.mpr
  refine ⟨childWidth,List.mem_range.mpr (by omega),?_⟩
  simp only [bodyForms]
  split <;> simp

theorem fold_in_catalogue (primitives depth width childWidth : Nat)
    (bound : childWidth ≤ width) (shape : 2 ≤ childWidth) :
    Form.fold childWidth ∈ catalogue primitives (depth+1) width := by
  apply List.mem_append_right
  apply List.mem_flatMap.mpr
  exact ⟨childWidth,List.mem_range.mpr (by omega),by simp [bodyForms,shape]⟩

theorem frame_arity (primitives literals inputs instructions output children)
    (frame : frameValid primitives literals inputs instructions output children = true)
    (instruction : Instruction) (mem : instruction ∈ instructions) :
    arity primitives instruction.form = some instruction.operands.length := by
  have rows := (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp frame).1).2
  obtain ⟨position,located⟩ := List.getElem?_of_mem mem
  have row := List.all_eq_true.mp rows (instruction,position)
    (List.mk_mem_zipIdx_iff_getElem?.mpr located)
  exact eq_of_beq (Bool.and_eq_true_iff.mp row).1

theorem frame_child (primitives literals inputs instructions output children)
    (frame : frameValid primitives literals inputs instructions output children = true)
    (instruction : Instruction) (mem : instruction ∈ instructions) (width : Nat)
    (port : width ∈ childWidths instruction.form) :
    ∃ child ∈ children, child.inputs = width := by
  have same := of_decide_eq_true (Bool.and_eq_true_iff.mp frame).2
  apply List.mem_map.mp
  rw [same]
  exact List.mem_flatMap.mpr ⟨instruction,mem,port⟩

theorem instruction_covered (primitives literals inputs instructions output children)
    (frame : frameValid primitives literals inputs instructions output children = true)
    (instruction : Instruction) (mem : instruction ∈ instructions) (depth width : Nat)
    (deep : (Tree.node inputs instructions output children).needs.depth ≤ depth)
    (wide : (Tree.node inputs instructions output children).needs.bodyWidth ≤ width) :
    instruction.form ∈ catalogue primitives.length depth width := by
  have ha := frame_arity _ _ _ _ _ _ frame instruction mem
  have childBound : ∀ w ∈ childWidths instruction.form, w ≤ width ∧ 0 < depth := by
    intro w hw
    obtain ⟨child,hc,same⟩ := frame_child _ _ _ _ _ _ frame instruction mem w hw
    have hc := needs_child inputs instructions output children child hc
    constructor
    · rw [← same]; exact Nat.le_trans hc.2.2.1 wide
    · have := Nat.le_trans hc.2.1 deep; omega
  cases form : instruction.form with
  | primitive index =>
    rw [form] at ha
    obtain ⟨bound,_⟩ := List.getElem?_eq_some_iff.mp ha
    exact primitive_in_catalogue _ _ _ _ bound
  | branch childWidth =>
    have bound := childBound childWidth (by simp [form,childWidths])
    cases depth with
    | zero => omega
    | succ prior => exact branch_in_catalogue _ _ _ _ bound.1
  | fold childWidth =>
    have bound := childBound childWidth (by simp [form,childWidths])
    have shape : 2 ≤ childWidth := by
      simp only [form,arity] at ha
      split at ha
      · assumption
      · cases ha
    cases depth with
    | zero => omega
    | succ prior => exact fold_in_catalogue _ _ _ _ bound.1 shape

/-- Every well-formed finite tree is admitted under sufficient independent
bounds. The proof constructs membership of each actual instruction alternative
and recursively admits both branch bodies, not just the observed branch. -/
theorem finite_tree_coverage (primitives literals steps width depth) (tree : Tree)
    (valid : Valid primitives literals tree)
    (bounded : tree.needs.Within ⟨steps,depth,width⟩) :
    covers primitives literals steps width depth tree = true := by
  induction valid generalizing depth with
  | node inputs instructions output children frame subtrees ih =>
    rw [covers_node]
    simp only [Bool.and_eq_true_iff,decide_eq_true_eq]
    refine ⟨⟨⟨frame,Nat.le_trans (needs_frame _ _ _ _) bounded.1⟩,?_⟩,?_⟩
    · apply List.all_eq_true.mpr
      intro instruction mem
      exact decide_eq_true (instruction_covered _ _ _ _ _ _ frame instruction mem
        depth width bounded.2.1 bounded.2.2)
    · cases depth with
      | zero =>
        cases children with
        | nil => rfl
        | cons child rest =>
          have h := needs_child inputs instructions output (child::rest) child (by simp)
          have impossible : child.needs.depth+1 ≤ 0 := Nat.le_trans h.2.1 bounded.2.1
          omega
      | succ prior =>
        apply List.all_eq_true.mpr
        intro child mem
        have h := needs_child inputs instructions output children child mem
        apply ih child mem prior
        exact ⟨Nat.le_trans h.1 bounded.1,
          Nat.le_of_succ_le_succ (Nat.le_trans h.2.1 bounded.2.1),
          Nat.le_trans h.2.2.2 bounded.2.2⟩

theorem sufficient_bounds (primitives literals) (tree : Tree) (valid : Valid primitives literals tree) :
    covers primitives literals tree.needs.steps tree.needs.bodyWidth tree.needs.depth tree = true :=
  finite_tree_coverage _ _ _ _ _ tree valid ⟨Nat.le_refl _,Nat.le_refl _,Nat.le_refl _⟩

theorem coverage_sound (primitives literals steps width depth) (tree : Tree)
    (covered : covers primitives literals steps width depth tree = true) : Valid primitives literals tree := by
  induction depth generalizing tree with
  | zero =>
    cases tree with
    | node inputs instructions output children =>
      have parts := Bool.and_eq_true_iff.mp covered
      have frame := (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp parts.1).1).1
      have empty : children = [] := List.isEmpty_iff.mp parts.2
      apply Valid.node _ _ _ _ frame
      intro child mem
      rw [empty] at mem
      cases mem
  | succ depth ih =>
    cases tree with
    | node inputs instructions output children =>
      have parts := Bool.and_eq_true_iff.mp covered
      have frame := (Bool.and_eq_true_iff.mp (Bool.and_eq_true_iff.mp parts.1).1).1
      apply Valid.node _ _ _ _ frame
      intro child mem
      exact ih child (List.all_eq_true.mp parts.2 child mem)

end DeclarationCoverage
