-- ReplayChecker — differential-replay Lean executable.
-- See `docs/architecture/verum-kernel-audit-2026.md` (FV-3) for the
-- cross-language cert-by-cert agreement protocol this implements.

import VerumExternalReplay.ReferenceChecker
import Lean.Data.Json

-- Reads a battery of certificates from `argv[0]` (a JSON file),
-- runs `VerumKernel.verifyCertificate` on each, prints per-cert
-- verdicts to stdout as JSON.
--
-- This is the Lean side of the cert-by-cert agreement check.  The
-- Rust harness (`verum audit --differential-lean-checker`) writes
-- the battery + Rust-side verdicts, invokes this binary, then
-- compares verdicts.  Any disagreement is a real bug in one of the
-- two implementations and fails the audit gate.
--
-- Battery JSON shape (matches `proof_checker::Certificate` serde
-- derive plus an `id` for cross-reference):
--
--   { "schema_version": 1,
--     "certificates": [
--       { "id": "<stable-id>",
--         "term": <Term JSON>,
--         "claimed_type": <Term JSON> }, … ] }
--
-- Term JSON shape (matches Rust Term enum's serde-default):
--   { "Var": <usize> } | { "Universe": <u32> }
--   | { "Pi": [<Term>, <Term>] } | { "Lam": [<Term>, <Term>] }
--   | { "App": [<Term>, <Term>] }
--
-- Verdict JSON shape:
--   { "id": "<id>", "ok": true }
--   | { "id": "<id>", "ok": false, "error": "<short-tag>" }

open Lean (Json)
open VerumKernel

-- Helper: extract a length-2 array from a Json value.
def jsonPair? (j : Json) : Except String (Json × Json) :=
  match j with
  | Json.arr arr =>
    if arr.size = 2 then Except.ok (arr[0]!, arr[1]!)
    else Except.error s!"expected 2-element array, got size {arr.size}"
  | _ => Except.error "expected JSON array"

-- Walk the Lean `Json` AST and reconstruct a `Term`.  Tries each
-- variant key in order; the first present key determines the
-- variant.  Pure function, returns Except.
partial def termFromJson (j : Json) : Except String Term :=
  if let .ok v := j.getObjVal? "Var" then do
    let n ← Json.getNat? v; Except.ok (Term.var n)
  else if let .ok v := j.getObjVal? "Universe" then do
    let n ← Json.getNat? v; Except.ok (Term.universe n)
  else if let .ok v := j.getObjVal? "Pi" then do
    let (a, b) ← jsonPair? v
    let a' ← termFromJson a
    let b' ← termFromJson b
    Except.ok (Term.pi a' b')
  else if let .ok v := j.getObjVal? "Lam" then do
    let (a, b) ← jsonPair? v
    let a' ← termFromJson a
    let b' ← termFromJson b
    Except.ok (Term.lam a' b')
  else if let .ok v := j.getObjVal? "App" then do
    let (a, b) ← jsonPair? v
    let a' ← termFromJson a
    let b' ← termFromJson b
    Except.ok (Term.app a' b')
  else
    Except.error s!"unrecognized Term variant in: {j.compress}"

-- A single battery row.
structure BatteryCert where
  id : String
  term : Term
  claimedType : Term
  deriving Repr

partial def batteryCertFromJson (j : Json) : Except String BatteryCert := do
  let idJ ← j.getObjValAs? Json "id" |>.mapError (fun _ => "missing 'id'")
  let id ← match idJ with
    | Json.str s => Except.ok s
    | _ => Except.error "'id' must be a string"
  let termJ ← j.getObjValAs? Json "term" |>.mapError (fun _ => "missing 'term'")
  let claimJ ← j.getObjValAs? Json "claimed_type" |>.mapError (fun _ => "missing 'claimed_type'")
  let term ← termFromJson termJ
  let claimedType ← termFromJson claimJ
  Except.ok { id, term, claimedType }

partial def batteryFromJson (j : Json) : Except String (List BatteryCert) := do
  let arr ← j.getObjValAs? Json "certificates"
              |>.mapError (fun _ => "missing 'certificates'")
  match arr with
  | Json.arr items =>
    items.toList.mapM batteryCertFromJson
  | _ => Except.error "'certificates' must be a JSON array"

-- Run `verifyCertificate` on a single battery row, package the
-- verdict as a JSON object.
def runOne (cert : BatteryCert) : Json :=
  match verifyCertificate cert.term cert.claimedType with
  | Except.ok () =>
    Json.mkObj [("id", Json.str cert.id), ("ok", Json.bool true)]
  | Except.error e =>
    let tag := match e with
      | .unbound_variable _        => "UnboundVariable"
      | .not_a_type _              => "NotAType"
      | .not_a_function _          => "NotAFunction"
      | .domain_mismatch _ _       => "DomainMismatch"
      | .type_mismatch _ _         => "TypeMismatch"
      | .universe_overflow _       => "UniverseOverflow"
      | .claimed_type_not_a_type _ _ => "ClaimedTypeNotAType"
      | .fuel_exhausted            => "FuelExhausted"
    Json.mkObj
      [ ("id", Json.str cert.id)
      , ("ok", Json.bool false)
      , ("error", Json.str tag)
      ]

-- Top-level entry point.  argv layout:
--   verum_replay_checker <battery.json>
def main (args : List String) : IO UInt32 := do
  match args with
  | [] => do
    IO.eprintln "usage: verum_replay_checker <battery.json>"
    return 2
  | path :: _ =>
    let raw ← IO.FS.readFile path
    match Json.parse raw with
    | Except.error e => do
      IO.eprintln s!"JSON parse error: {e}"
      return 3
    | Except.ok j =>
      match batteryFromJson j with
      | Except.error e => do
        IO.eprintln s!"battery decode error: {e}"
        return 4
      | Except.ok certs => do
        let verdicts := certs.map runOne
        let out := Json.mkObj
          [ ("schema_version", Json.num 1)
          , ("verdicts", Json.arr verdicts.toArray)
          ]
        IO.println (out.compress)
        return 0
