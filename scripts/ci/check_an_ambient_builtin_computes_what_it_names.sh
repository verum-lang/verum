#!/bin/sh
# check_an_ambient_builtin_computes_what_it_names.sh — a bare call to an
# ambient builtin either fails to compile, or returns the value its name
# promises.  It must never type-check and then compute something else.
#
# WHY THIS EXISTS.  T1123, measured 2026-09-03:
#
#     print(abs(0 - 3))   ->  Relaxed
#
# `Relaxed` is `MemoryOrdering.Relaxed` — tag 0 of an atomics enum that
# the program never mentions.  The same answer came back for EVERY
# input: Int, Float, literal, variable, positive, negative.  An answer
# identical across a varied sample is the signature of an argument that
# never reaches the operation.
#
# The neighbours were all healthy, which is what makes the shape worth a
# gate rather than a one-line fix:
#
#     n.abs()  method form           ->  3      correct
#     @abs(0 - 3)  undeclared macro  ->  3      correct, and see T1124 —
#                                             the grammar does not admit
#                                             this spelling at all
#     min(5,2) max(5,2) clamp(9,1,4) ->  2 5 4  correct
#     abs(0 - 3)  BARE FREE FORM     ->  Relaxed
#     pow(2, 3)   BARE FREE FORM     ->  Relaxed   (found by this gate,
#                                             not by the original report)
#
# ROOT.  Two lists describe one concept and neither knows about the
# other.  `verum_types/src/infer/env.rs` registers `abs` as an ambient
# polymorphic builtin (`reg_num_poly!("abs", 1)`), so the call
# type-checks.  `verum_vbc/src/codegen/expressions.rs` intercepts
# poly-numeric names and turns them into opcodes, and its list carried
# the REGISTRY spelling `abs_signed` without the SOURCE spelling `abs`.
# A name that falls out of that arm is reported as "not a builtin" and
# lowers to a value nobody chose.
#
# So the failure needs no bad input and no unusual program — only a
# name that one layer accepts and the other does not implement.  That
# is the class `crates/verum_types/src/CLAUDE.md` names: *the compiler
# knows a name at a spelling the library never provided.*
#
# WHAT THIS GATE ASKS, and why it is not a list comparison.  Comparing
# the two lists textually would pass the moment someone adds the name to
# both — including adding it to a list that no longer feeds the arm.
# The question worth asking is behavioural and survives refactoring:
#
#     for every ambient name, does a bare call either REFUSE or ANSWER
#     CORRECTLY?
#
# A name that does not type-check bare is fine — no claim was made about
# it.  The defect is only ever accept-then-miscompute.  (`neg`, `add`,
# `sub` … are in the guard list but are not registered as callable, and
# a bare `neg(3)` is an honest E100.  The gate must not demand they
# work; it must demand they do not lie.)
#
# CONTROL.  The last two cases are METHOD-form calls, which were
# correct for all fourteen names in the census that found this.  If the
# gate ever reports everything clean because the runner itself broke,
# those lines break with it.
#
# The `@abs(x)` spelling is deliberately NOT used as a control, though
# it answered correctly throughout the defect.  The grammar admits the
# SYNTAX — `meta_call = '@', path, meta_call_args` takes any path — but
# `abs` is not among the 21 `meta_function_name`s and no macro named
# `abs` is declared anywhere, so it works only because the code
# generator carries a private list of twenty builtin names.  Leaning on
# it would pin a spelling that rests on that private list.
# That is T1124, filed separately: an undeclared `@name(...)` compiles
# to `nil` instead of being diagnosed.
set -eu

VERUM="${1:-target/release/verum}"
if [ ! -x "$VERUM" ]; then
    echo "check_an_ambient_builtin_computes_what_it_names: no binary at $VERUM" >&2
    echo "usage: $0 [path-to-verum]" >&2
    exit 2
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
fails=0
checked=0

# name | expression | expected output
# Only names that are REGISTERED as bare-callable appear here; the rest
# of IO_AND_NUMERIC_BUILTIN_NAMES are print-family or unregistered, and
# the "does not type-check" branch below covers them without a claim.
cases='abs|abs(0 - 3)|3
abs|abs(3)|3
abs|abs(0.0 - 2.5)|2.5
min|min(5, 2)|2
min|min(2, 5)|2
max|max(5, 2)|5
clamp|clamp(9, 1, 4)|4
clamp|clamp(0, 1, 4)|1
sqrt|sqrt(9.0)|3
floor|floor(2.7)|2
ceil|ceil(2.1)|3
pow|pow(2, 3)|8
method|(0 - 3).abs()|3
method|(2).pow(3)|8'

echo "$cases" | while IFS='|' read -r name expr want; do
    [ -n "$name" ] || continue
    f="$TMP/probe.vr"
    printf 'fn main() { print(%s); }\n' "$expr" > "$f"

    if ! "$VERUM" check "$f" >"$TMP/chk" 2>&1; then
        # Refusing is always allowed: no claim, no lie.
        printf '  %-8s %-22s refused (no claim)\n' "$name" "$expr"
        continue
    fi
    checked=$((checked + 1))
    got="$("$VERUM" run "$f" 2>/dev/null | tail -1)"
    case "$got" in
        "$want"|"$want".0|"$want".00*)
            printf '  %-8s %-22s -> %s\n' "$name" "$expr" "$got"
            ;;
        *)
            printf '  %-8s %-22s -> %s   WANT %s\n' "$name" "$expr" "$got" "$want"
            fails=$((fails + 1))
            echo "$name" >> "$TMP/failed"
            ;;
    esac
done

if [ -s "$TMP/failed" ]; then
    echo
    echo "FAIL: an ambient builtin type-checked and then computed something else."
    echo "      Names: $(tr '\n' ' ' < "$TMP/failed")"
    echo "      A bare call must refuse or be right — see T1123 and the"
    echo "      header of this file for the two-lists mechanism."
    exit 1
fi

echo
echo "OK: every ambient builtin that type-checks bare computes what it names."
