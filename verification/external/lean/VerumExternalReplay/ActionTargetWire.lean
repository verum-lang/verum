import VerumExternalReplay.ActionCapture

/-! Length-delimited action targets over the full byte alphabet. The decoder
proves injectivity on representable lengths; it does not normalize a predictive
law over these typed words or certify native allocation success. -/
namespace ActionTargetWire
open AcquiredObserver ActionCapture
set_option warningAsError true

def littleEndian : Nat → Nat → List UInt8
  | 0, _ => []
  | width+1, value => UInt8.ofNat (value % 256) :: littleEndian width (value / 256)

def readLittleEndian : List UInt8 → Nat
  | [] => 0
  | byte :: rest => byte.toNat + 256 * readLittleEndian rest

theorem little_endian_length (width value : Nat) : (littleEndian width value).length = width := by
  induction width generalizing value with
  | zero => rfl
  | succ width ih => simp [littleEndian, ih]

theorem little_endian_roundtrip (width value : Nat) (fits : value < 256^width) :
    readLittleEndian (littleEndian width value) = value := by
  induction width generalizing value with
  | zero => have : value = 0 := by simpa using fits
            simp [this, littleEndian, readLittleEndian]
  | succ width ih =>
    have smaller : value / 256 < 256^width :=
      (Nat.div_lt_iff_lt_mul (by decide)).mpr (by simpa [Nat.pow_succ] using fits)
    simpa [littleEndian, readLittleEndian, ih _ smaller, UInt8.toNat_ofNat] using Nat.mod_add_div value 256

def frame (body : List UInt8) : List UInt8 := littleEndian 8 body.length ++ body

def readFrame (bytes : List UInt8) : Option (List UInt8 × List UInt8) :=
  let header := bytes.take 8
  if header.length = 8 then
    let size := readLittleEndian header
    let rest := bytes.drop 8
    if size ≤ rest.length then some (rest.take size, rest.drop size) else none
  else none

theorem frame_roundtrip_with_tail (body tail : List UInt8) (fits : body.length < 256^8) :
    readFrame (frame body ++ tail) = some (body, tail) := by
  simp only [frame, List.append_assoc, readFrame,
    List.take_left' (little_endian_length 8 body.length),
    List.drop_left' (little_endian_length 8 body.length),
    little_endian_length, little_endian_roundtrip 8 body.length fits]
  simp

theorem frame_roundtrip (body : List UInt8) (fits : body.length < 256^8) :
    readFrame (frame body) = some (body, []) := by
  simpa using frame_roundtrip_with_tail body [] fits

def kindByte : TargetKind → UInt8
  | .step => 0
  | .refused => 1
  | .terminal .death => 2
  | .terminal .time => 3
  | .terminal .completed => 4
  | .terminal .ended => 5

def readKind (byte : UInt8) : Option TargetKind :=
  match byte.toNat with
  | 0 => some .step | 1 => some .refused | 2 => some (.terminal .death)
  | 3 => some (.terminal .time) | 4 => some (.terminal .completed)
  | 5 => some (.terminal .ended) | _ => none

theorem kind_roundtrip (kind : TargetKind) : readKind (kindByte kind) = some kind := by
  cases kind with
  | step => rfl
  | refused => rfl
  | terminal kind => cases kind <;> rfl

def encode (target : Target) : List UInt8 :=
  [1, kindByte target.kind] ++ frame target.observation.schema ++ frame target.observation.body

def decode : List UInt8 → Option Target
  | version :: kind :: rest => if version = 1 then do
      let kind ← readKind kind
      let (schema, rest) ← readFrame rest
      let (body, tail) ← readFrame rest
      if tail.isEmpty then some ⟨kind, ⟨schema, body⟩⟩ else none
    else none
  | _ => none

def Fits (target : Target) : Prop :=
  target.observation.schema.length < 256^8 ∧ target.observation.body.length < 256^8

theorem target_roundtrip (target : Target) (fits : Fits target) : decode (encode target) = some target := by
  rcases target with ⟨kind, ⟨schema, body⟩⟩
  rcases fits with ⟨hs, hb⟩
  simp [encode, decode, kind_roundtrip, frame_roundtrip_with_tail schema _ hs,
        frame_roundtrip body hb]

theorem encoding_injective (a b : Target) (ha : Fits a) (hb : Fits b) (same : encode a = encode b) : a = b := by
  have equal := congrArg decode same
  simpa [target_roundtrip a ha, target_roundtrip b hb] using equal

theorem encoded_length (target : Target) :
    (encode target).length = 18 + target.observation.schema.length + target.observation.body.length := by
  simp [encode, frame, little_endian_length, Nat.add_comm, Nat.add_left_comm]

inductive Refusal where
  | unobserved | missing | capacity
  deriving DecidableEq

/-- Deterministic preallocation checks only. A successful native reserve must
be observed separately; an allocator failure cannot be predicted by this model. -/
def materialize [DecidableEq Action] (maxBytes : Nat) (c : Capture Action Detail) : Except Refusal (List UInt8) :=
  match semanticTarget c with
  | .error .unobserved => .error .unobserved
  | .error .missing => .error .missing
  | .ok target =>
    if target.observation.schema.length < 256^8 ∧ target.observation.body.length < 256^8 ∧
       18 + target.observation.schema.length + target.observation.body.length ≤ maxBytes then
      .ok (encode target)
    else .error .capacity

theorem materialize_success_iff [DecidableEq Action]
    (maxBytes : Nat) (c : Capture Action Detail) (bytes : List UInt8) :
    materialize maxBytes c = .ok bytes ↔ ∃ target,
      semanticTarget c = .ok target ∧ Fits target ∧
      18 + target.observation.schema.length + target.observation.body.length ≤ maxBytes ∧
      encode target = bytes := by
  cases hs : semanticTarget c with
  | error reason => cases reason <;> simp [materialize, hs]
  | ok target =>
    constructor
    · intro success
      simp only [materialize, hs] at success
      split at success
      next fits =>
        exact ⟨target, rfl, ⟨fits.1, fits.2.1⟩, fits.2.2, Except.ok.inj success⟩
      next refused => contradiction
    · rintro ⟨value, same, fits, bound, encoded⟩
      have equal : target = value := Except.ok.inj same
      subst value
      simp only [materialize, hs]
      rw [ite_eq_left ⟨fits.1, fits.2, bound⟩, encoded]

theorem successful_materialization_decodes [DecidableEq Action]
    (maxBytes : Nat) (c : Capture Action Detail) (bytes : List UInt8)
    (success : materialize maxBytes c = .ok bytes) :
    ∃ target, semanticTarget c = .ok target ∧ decode bytes = some target := by
  obtain ⟨target, hs, fits, _, same⟩ := (materialize_success_iff maxBytes c bytes).mp success
  exact ⟨target, hs, same ▸ target_roundtrip target fits⟩

theorem materialization_within_capacity [DecidableEq Action]
    (maxBytes : Nat) (c : Capture Action Detail) (bytes : List UInt8)
    (success : materialize maxBytes c = .ok bytes) : bytes.length ≤ maxBytes := by
  obtain ⟨target, _, _, capacity, same⟩ := (materialize_success_iff maxBytes c bytes).mp success
  simpa [← same, encoded_length] using capacity

theorem materialization_ignores_resumption [DecidableEq Action]
    (maxBytes : Nat) (c : Capture Action Detail) (birth : Option Sensory) :
    materialize maxBytes {c with resumed := birth} = materialize maxBytes c := rfl

end ActionTargetWire
