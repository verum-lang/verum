# `term/style/color` audit

Module: `core/term/style/color.vr` (322 LOC) — terminal color types
spanning 4 fidelity tiers (NoColor → Base16 → Ansi256 → TrueColor).
Defines Rgb / Hsl / Color 18-variant / Lab records + conversions.

Tests: `unit_test.vr` (~24 unit tests covering Rgb.new + Rgb.from_hex
happy/error paths + Color variants + Hsl/Lab record construction).

Conversion methods (to_lab/to_hsl/to_rgb/to_ansi256/to_base16/
perceptual_distance/darken/lighten/lerp/gradient/luminance) involve
floating-point arithmetic — covered by property tests deferred for
manual round-trip + reference-vector verification.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.term.style.style` | Style holds Color values for fg/bg. |
| `core.term.style.theme` | Theme palette built from Color variants. |
| `core.term.render` | ANSI escape sequence emission consumes Color. |
| `core.term.style.profile` | terminal capability detection + adaptive downscaling. |
| `core.term.widget` | widget chrome colors. |

## 2. Crate-side hardcodes

None today. Color → ANSI escape mapping is pure Verum. Rust-side
terminal output (when implemented) consumes the rendered byte
stream, not Color directly.

## 3. Language-implementation gaps

### §3.1 Color 18-variant ADT — no `Eq` / `Hash` / `Display`

All conversion methods exist but Eq/Hash/Display impls are missing.
Common consumer pattern: `Map<Color, Style>` lookup — needs Hash.
`f"{color}"` — needs Display rendering to ANSI 8-bit short-form
(e.g. "Red" / "rgb(255,128,0)" / "256:123").

**Effort:** medium (~1h) — recursive across 18 variants for Display.

### §3.2 Rgb has no `eq` / `Eq` impl

Pure record — should @derive Eq trivially. Hash impl follows for
Map<Rgb, _> consumer patterns.

**Effort:** trivial (~5 min).

### §3.3 `from_hex` accepts only 6-char; no #FFF (3-char shorthand)

CSS allows `#F80` as shorthand for `#FF8800`. Today `Rgb.from_hex`
rejects. Add shorthand handling per CSS Color Module Level 4.

**Effort:** small (~30 min).

### §3.4 Hsl construction allows out-of-range values

`Hsl { h: 400.0, s: 1.5, l: -0.5 }` constructs successfully despite
the doc-stated ranges [0,360) / [0,1] / [0,1]. Add refinement
constraint OR validating ctor `Hsl.try_new(h, s, l) -> Result<Hsl, _>`.

**Effort:** small (~30 min).

## Action items landed in this branch

* `core-tests/term/style/color/unit_test.vr` — 24 unit tests over
  Rgb.new + Rgb.from_hex (happy + 3 error paths) + Color variants +
  Hsl/Lab record construction.
* `core-tests/term/style/color/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Eq` for Rgb + Color + Hsl + Lab | `core/term/style/color.vr` + tests | 1h |
| Add `Display` for Color (ANSI 8-bit short-form) | `core/term/style/color.vr` + tests | 1h |
| Add #FFF (3-char shorthand) to from_hex | `core/term/style/color.vr` + 2 tests | 30 min |
| Add Hsl refinement / validating ctor | `core/term/style/color.vr` + tests | 30 min |
| Add property tests (to_lab ∘ from_lab round-trip ≈ id, RGB triangle) | this folder + property_test.vr | 2h |
| Sister tests for `core.term.style.{style,theme,modifier,profile,convert,hyperlink,text_builder}` | sister folders | 1 day total |
EOF
