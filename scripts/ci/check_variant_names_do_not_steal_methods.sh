#!/bin/sh
# check_variant_names_do_not_steal_methods.sh — a sum type's methods must
# not disappear because ANOTHER type happens to use the same variant
# names.
#
# WHY THIS EXISTS.  Structural variant receivers are resolved to a NAME
# through `variant_type_names`, a flat first-registered-wins map.  Two
# kinds of key are written into it: the exact signature
# (`Variant(...)`) and a deliberately coarsened one that keeps only
# variant NAMES.  Before 2026-09-02 both used the same `Variant(`
# prefix, so they shared a namespace — and a map's discriminating power
# is that of the COARSEST key written into it.  Measured (T1069):
#
#     IpAddr is V4(Ipv4Addr) | V6(Ipv6Addr)
#       exact   Variant(V4(Ipv4Addr)|V6(Ipv6Addr))
#       relaxed Variant(V4|V6)
#     Cidr   is V4 { … } | V6 { … }
#       EXACT   Variant(V4|V6)        <- byte-identical to the above
#
#     [varcollide] sig=Variant(V4|V6) kept=IpAddr dropped=Cidr
#     core/net/cidr.vr: no method named `last_address` found for type
#                       `IpAddr` — on a type that declares it
#
# Three types share `V4|V6` in this tree (SocketAddr, IpAddr, Cidr), so
# WHICH one wins depends on what a given compilation loaded.  That is
# why the same file reported 2 or 5 errors depending on the run until
# the load queue was sorted (T1065).
#
# WHAT IT ASSERTS.  Two files identical except for their variant NAMES.
# Sharing names with a neighbour is not a fact about a type's own
# methods, so both must compile.
#
# SCOPE, STATED HONESTLY.  This gate covers the exact-vs-relaxed
# collision.  It does NOT cover exact-vs-exact: `Va(Int)|Vb(Int)` and
# `Va{x:Int}|Vb{x:Int}` produce the SAME exact key, because
# `variant_type_signature_static`'s `_ => String::new()` arm — justified
# in its comment for Unit, primitives and TypeVars — also swallows
# record payloads, which ARE distinctive.  A probe for that half is in
# the T1069 notes and is still red; it is a separate defect and this
# gate does not claim it.
#
# Usage:
#   check_variant_names_do_not_steal_methods.sh [verum]
#   check_variant_names_do_not_steal_methods.sh --selftest [verum]
set -u

SELFTEST=0
if [ "${1:-}" = "--selftest" ]; then
  SELFTEST=1
  shift
fi
VERUM="${1:-target/release/verum}"
[ -x "$VERUM" ] || { printf 'check_variant_names: no verum binary at %s\n' "$VERUM" >&2; exit 2; }
VERUM=$(cd "$(dirname "$VERUM")" && pwd -P)/$(basename "$VERUM")

REPO=$(cd "$(dirname "$0")/../.." && pwd -P)
TMP=$(mktemp -d) || exit 2
trap 'rm -rf "$TMP"' EXIT

# `Shared` reuses the neighbour's variant names, `Distinct` does not.
# The neighbour's payloads are NAMED types, so its exact key differs
# from the subject's and only the relaxed key can collide — which is
# the half this gate covers.
# The NEIGHBOUR is fixed; only the SUBJECT's variant names vary.  Vary
# both and the pair collides in either file — the first version of this
# fixture did exactly that and reported both subjects broken, which the
# "two broken subjects agree" arm caught but which measured nothing.
write_subject() { # $1 = subject's variant prefix, $2 = out file
  cat > "$2" <<VR
module probe.variant_names_$1;

public type Payload is { n: Int };

public type Neighbour is Va(Payload) | Vb(Payload);

public type Subject is ${1}a { x: Int } | ${1}b { x: Int };

implement Subject {
    public fn own_method(&self) -> Int { 1 }
    public fn probe(&self) -> Int { self.own_method() }
}
VR
}
write_subject V "$TMP/shared.vr"     # subject shares Va|Vb with Neighbour
write_subject W "$TMP/distinct.vr"   # subject uses Wa|Wb; nothing collides

# The two files must differ, or the comparison is vacuous — the same
# failure `check_path_does_not_change_verdict.sh` shipped with.
if cmp -s "$TMP/shared.vr" "$TMP/distinct.vr"; then
  printf 'check_variant_names: the two subjects are IDENTICAL — the fixture is broken\n' >&2
  exit 2
fi

verdict() { # $1 file -> error count, or MUTE
  out=$(timeout 180 "$VERUM" check "$1" 2>&1)
  printf '%s' "$out" | grep -q 'Checking\|Finished\|error' || { echo MUTE; return; }
  printf '%s' "$out" | grep -c '^error' || true
}

a=$(cd "$REPO" && verdict "$TMP/shared.vr")
b=$(cd "$REPO" && verdict "$TMP/distinct.vr")

if [ "$SELFTEST" -eq 1 ]; then
  printf 'module probe.variant_names_broken;\n\npublic fn go() -> Int { no_such_name_xyz() }\n' \
    > "$TMP/broken.vr"
  c=$(cd "$REPO" && verdict "$TMP/broken.vr")
  if [ "$c" = MUTE ] || [ "$c" = 0 ]; then
    printf 'selftest: FAILED — a knowingly broken file scored %s\n' "$c"
    exit 1
  fi
  printf 'selftest: ok — shared=%s distinct=%s broken=%s\n' "$a" "$b" "$c"
fi

if [ "$a" = MUTE ] || [ "$b" = MUTE ]; then
  printf 'check_variant_names: FAILED — a subject produced no output (%s / %s).\n' "$a" "$b"
  exit 1
fi

if [ "$a" != "$b" ]; then
  printf 'check_variant_names: FAILED — sharing variant names cost a type its methods.\n'
  printf '  variants named like the neighbour : %s error(s)\n' "$a"
  printf '  variants named differently        : %s error(s)\n' "$b"
  printf '  Both files declare the same methods on the same shape. How a\n'
  printf '  NEIGHBOURING type spells its variants is not a fact about this one.\n'
  printf '  Run with VERUM_TRACE_VARCOLLIDE=1 — it names the pair in one line:\n'
  printf '    [varcollide] sig=... kept=<winner> dropped=<subject>\n'
  printf '  The exact and relaxed signature keys must stay in SEPARATE\n'
  printf '  namespaces (Variant( vs VariantRelaxed( ); all three copies of\n'
  printf '  the builder pair — unify.rs, protocol.rs, infer/modules.rs —\n'
  printf '  must agree on the spelling, since registration writes through\n'
  printf '  one and lookup reads through another.\n'
  exit 1
fi

if [ "$a" != 0 ]; then
  printf 'check_variant_names: FAILED — both subjects report %s error(s).\n' "$a"
  printf '  They agree, but agreement between two broken subjects is not the\n'
  printf '  property this gate exists for.\n'
  exit 1
fi

printf 'check_variant_names: ok — shared and distinct variant names both clean (%s errors)\n' "$a"
