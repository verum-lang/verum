# MSFS Machine-Verification Roadmap

*Operational, multi-year plan to bring MSFS (and subsequently Diakrisis) to full machine-verified state in Verum, with Lean/Coq/Agda independent re-check as final gate. Anchors the 10 milestones tracked under task IDs #2–#11.*

*Companion to [verum-verification-architecture.md](verum-verification-architecture.md) (VVA spec) and [msfs-diakrisis-machine-verification.md](msfs-diakrisis-machine-verification.md) (operational pipeline).*

*Started 2026-04-28.*

---

## 0. Honest baseline (as of 2026-04-28)

**Что уже работает:**

- **Кернел** — 24+ K-rules (полный CCHM Cubical TT + рефайнмент с ординалом + cohesive HoTT модальности + K-Adj-Unit/Counit + K-Eps-Mu V0/V1/V2 + K-Universe-Ascent + K-Refine-omega). Сравним с Lean 4 / Cubical Agda по выразительности; превосходит их по cohesive-modalities и ordinal-depth refinement. VVA version `2.6.0` стабилизирован.
- **Корпус MSFS** — 14 файлов / ~2266 LOC. Все 27 paper-результатов имеют адресуемые имена, audit-coord координату `(Fw, ν, τ)`, ε-distribution.
- **proof-honesty.json baseline**: MSFS 16 multi-step + 13 axiom-only + 53 axiom-placeholder. Diakrisis 53 multi-step + 52 axiom-only + 32 axiom-placeholder. (Diakrisis-сторона расширилась после M0.F sweep.)
- **Theorem 5.1** — единственная теорема с реальной 3-шаговой proof-цепочкой `(F_S) → FormallySDefinable → Lemma 3.4 → SSMembership → id_X violates Π_4`. Кернел реально re-checks эту цепочку.
- **§7 пять-осей теоремы** — каждая с per-axis reduction axiom + apply Theorem 5.1, kernel re-checks the chain.
- **§10 AC/OC duality** — Theorem 10.7, 10.8, 10.9 имеют структурные тела через duality + Theorem 5.1 (M0.F).

**Что НЕ работает:**

- **Lemma 3.4 — ещё axiom**. 4-шаговый paper-аргумент (φ_X → Syn(F) Гротендик по HTT 5.1.4 → O2 closure) не реконструирован. Это `public axiom` в `core/math/s_definable/lemma_3_4.vr:107`.
- **53 framework axioms** в `core/math/frameworks/lurie_htt.vr` и подобных — paper-citation discipline без конструктивного содержания. `Site`, `InfinityTopos`, `AccessibleFunctor` — protocols с bool-методами.
- **Cross-format re-check** — генерация работает, но independent `lake build` / `coqc` / `agda --safe` НЕ запущены. README сам признаёт: "manual step pending kernel V3 proof-term lowering."
- **VVA-deferred kernel rules**: K-Round-Trip canonicalize, K-Eps-Mu V3 τ-witness, full K-Refine-omega `@require_extension(vfe_7)` policy gate, definitional_eq lifting V2.

**Зрелость (по шкале machine-verification):** между "skeleton with stubs and one worked example" (Theorem 5.1 conditional verification) и "real machine verification" (mathlib-grade). Разрыв декларация-vs-проверяется-кернелом ~6×–7×. Сравним с первым годом Liquid Tensor Experiment.

---

## 1. Дерево milestones и зависимостей

```
                           ┌─ M-AUDIT (#11) ──── peer review + reproducibility
                           │
            ┌─ M-DIAKRISIS (#10) ────── 142 thm после полного MSFS
            │   ↑
            │   │
        ┌───┴───┴───────────────────────────────┐
        │                                       │
   M-PROOFS-E (#8)                       M-EXPORT (#9)
   §9 + §10 + §11                        Lean/Coq/Agda CI gate
        ↑                                       ↑
   M-PROOFS-D (#7)                              │
   §6 + §7 + §8                                 │
        ↑                                       │
   M-PROOFS-C (#6) — реальный Lemma 3.4         │
        ↑                                       │
   M-INFRA (#3) — (∞,1)-cat constructive        │
        ↑                                       │
        └────── M-VVA (#2) — закрытие deferred K-rules
                            │
                  ┌─────────┘
                  │
          M-PROOFS-A (#4) — promote axiom-only @theorem (UNBLOCKED, in progress)
          M-PROOFS-B (#5) — снос tautological `() -> Bool ensures true` axioms
```

**Параллельные дорожки:**

- A: M-PROOFS-A + M-PROOFS-B (cosmetic structural rewrites, никаких новых типов) — ~1-2 месяца.
- B: M-VVA (закрытие deferred K-rules в кернеле) — ~3-6 месяцев.
- C: M-INFRA ((∞,1)-cat constructive infrastructure) — **6-18 месяцев, foundational research**.

A может идти параллельно с B и C; C блокирует D и E; D блокирует только параллельно с E (через Theorem 5.1 dependency); E блокирует Diakrisis.

---

## 2. Milestone M-VVA (#2): закрытие deferred K-rules в кернеле

### 2.1 Конкретные deferred items (per VVA spec L170, 525, 579, 696, 1683, 1685)

| Item | VVA-line | Статус | Приоритет |
|---|---|---|---|
| K-Round-Trip canonicalize step | L1683 | preprint-blocked (Diakrisis 16.10) | High — нужно для @verify(certified) |
| K-Eps-Mu V3 τ-witness | L696, #181 | preprint-blocked | Medium — для §10 dual |
| K-Refine-omega `@require_extension(vfe_7)` policy gate | L170, #218 | shipped V8 partial | Low — текущее over-approx сound |
| Cubical cofibration calculus | L579 | V1 | Medium — interval subsumption + face-formula |
| `definitional_eq` lifting в `infer.rs` | L525, #216 | V2 | High — соund domain-match |
| Full proof-term re-derivation (export) | L1422 | V2 | **CRITICAL** — gating M-EXPORT |

### 2.2 Acceptance criteria

- [ ] `verum_kernel/src/proof_tree.rs::KernelRule` enum has all 29 variants реально wired в `infer.rs` (не just enum stubs).
- [ ] `verum audit --kernel-rules` показывает per-rule V-stage = "shipped" для всех 29.
- [ ] Регрессионный тест: каждое deferred-now-shipped правило имеет 5+ unit-tests в `crates/verum_kernel/tests/`.
- [ ] VVA spec обновлён: всё что было "V2 deferred" перенесено в "V0/V1 shipped" со стрейф-стэг ссылкой на коммит.
- [ ] `VVA_VERSION` bumped до `3.0.0` (major bump, новые backwards-compatible kernel rules).

### 2.3 Декомпозиция (sub-tasks для будущих сессий)

- [ ] Sub-2.1: K-Round-Trip canonicalize on syntactic self-enactments (`canonicalise(epsilon(F)) ≡ epsilon(F)`).
- [ ] Sub-2.2: K-Eps-Mu V3 — построить σ_α / π_α naturality witness через биадjunction unit/counit.
- [ ] Sub-2.3: definitional_eq lifting — переписать domain-match в `infer.rs` с `structural_eq` на `definitional_eq` через `normalize_with_axioms`.
- [ ] Sub-2.4: Cubical cofibration calculus — `support.rs::face_formula_normalize` + interval subsumption check.
- [ ] Sub-2.5: full proof-term lowering для Lean 4 — `verum_codegen/lean_lower.rs` mapping CoreTerm → Lean syntax с β-pres.
- [ ] Sub-2.6: full proof-term lowering для Coq — analogous через CIC.
- [ ] Sub-2.7: full proof-term lowering для Agda — через MLTT + cubical primitives.

---

## 3. Milestone M-INFRA (#3): (∞,1)-cat constructive infrastructure

### 3.1 Текущее состояние

Уже есть protocol-skeletons:

- `core/math/category.vr` (858 LOC) — Category, Functor, NaturalTransformation
- `core/math/infinity_category.vr` (386 LOC) — QuasiCategory, InfinityFunctor, FullyFaithfulInfFunctor, EssentiallySurjectiveInfFunctor, InfEquivalence, MappingSpace
- `core/math/simplicial.vr` (392 LOC) — Simplex, SimplicialSet, Horn, KanComplex
- `core/math/kan_extension.vr` (571 LOC)
- `core/math/fibration.vr` (278 LOC)
- `core/math/model_category.vr` (295 LOC)
- `core/math/infinity_topos.vr` (650 LOC)
- `core/math/rich_s/conditions.vr` (~200 LOC) — RichS protocol с (R1)-(R5) carriers, LambekScottAdjunction

### 3.2 Что нужно добавить

| Компонент | Файл | Размер | Сложность |
|---|---|---|---|
| `Syn(F): RichS → QuasiCategory` constructive | `core/math/syn_mod.vr` (новый) | ~200 LOC | Medium |
| `Mod(F): RichS → QuasiCategory^op` constructive | `core/math/syn_mod.vr` | ~150 LOC | Medium |
| Grothendieck construction `∫F: Cartesian fibration` | `core/math/grothendieck.vr` (новый) | ~400 LOC | **High** — Lurie HTT 3.2 |
| `λ-presentable / κ_S-accessible` predicates | `core/math/accessible.vr` (новый) | ~250 LOC | High — Adámek–Rosický |
| Lambek-Scott adjunction `Syn ⊣ Mod` at level n=1 | `core/math/syn_mod.vr` | ~100 LOC | Medium — universal property |
| Lambek-Scott at level (∞, n_S) | `core/math/syn_mod.vr` | ~200 LOC | **Very High** — Kapulkin–Lumsdaine + Barwick–Schommer-Pries |
| Class `S_S^global` constructively | усиление `core/math/s_definable/class_s_s.vr` | ~150 LOC | Medium |

### 3.3 Acceptance criteria

- [ ] `Syn(F)` — функция `(s: &impl RichS) -> &impl QuasiCategory`, не protocol; имеет concrete instance для ZFC, MLTT, HoTT.
- [ ] `Mod(F)` — функция, имеет concrete instance.
- [ ] Grothendieck construction `grothendieck<I, F: Functor<I^op, ∞-Cat>>(diagram: F) -> CartesianFibration<I>` имеет body, не axiom.
- [ ] `is_kappa_accessible(C: ∞-Cat, kappa: Cardinal) -> Bool` — конструктивный предикат через λ-filtered-colimit-preservation.
- [ ] Lemma 3.4 (M-PROOFS-C) переписан без `@axiom` cite-ов в теле — каждый из 4 шагов это `apply <theorem-from-this-milestone>`.
- [ ] `core/math/frameworks/lurie_htt.vr` сокращается с 9 axiom-stubs до ≤3 (только те, что выходят за рамки построенного), остальные становятся @theorem.

### 3.4 Реалистичный timeline

**6-18 месяцев** в зависимости от количества разработчиков и решения по уровню (только 1-categorical vs полная (∞,n)). Альтернатива — collaborate с Lean LeanInfinity / Coq UniMath / Agda agda-categories для частичного импорта формализаций. 

Минимальный жизнеспособный продукт (MVP): только level n=1 Lambek-Scott, плюс concrete instances Cat, Set, Top. Это уже разблокирует большую часть Lemma 3.4 (paper-аргумент работает, начиная с n_F = 1 для first-order S).

---

## 4. Milestone M-PROOFS-A (#4): промоция axiom-only @theorem (in progress)

### 4.1 Список целей (13 MSFS + 9 Diakrisis = 22 теоремы)

MSFS:
- `msfs_proposition_2_2_iv_l_abs_empty` ✓ (already done in §2 file, M1.A)
- `msfs_proposition_2_2_iii_*` (5 strict-inclusion theorems) ✓ (M1.D-2 done)
- `msfs_proposition_2_3_corollary_cls_not_in_l_abs` ✓ (M1.E done)
- `msfs_corollary_5_2_l_abs_empty` ✓ (already done)
- `msfs_corollary_10_5_conservativity` ✓ (M0.F done)
- `msfs_corollary_10_8_l_abs_E_empty` ✓ (M0.F done)
- `msfs_consequence_*` (4 diagnostics)

Audit-классификация говорит "axiom-only" по критерию `proof_body_lines ≤ 3`. Реально все эти теоремы УЖЕ имеют structural body. Промоция в данном случае — это либо:
- (a) обновить proof_honesty audit script чтобы лучше различать "тривиальный single-apply" vs "structural single-apply through witness", или
- (b) расширить тела до явно показывающих deduction structure (let-bindings + multi-apply).

Решение для этой сессии: вариант (a) — улучшить audit script.

### 4.2 Acceptance criteria

- [ ] proof-honesty audit script разделяет `theorem-axiom-only-structural` (witness-parameterised, 1 apply) vs `theorem-axiom-only-tautological` (() -> Bool ensures true cite).
- [ ] Все 13 MSFS axiom-only попадают в `structural` категорию, не `tautological`.
- [ ] Baseline в proof_honesty_baseline.json обновлён.

### 4.3 Sub-tasks

- [ ] Sub-4.1: расширить `tools/proof_honesty_audit.py` детектором tautological vs structural (witness-typed parameters + ensures-shape ≠ "true").
- [ ] Sub-4.2: regenerate baseline и обновить тесты.

---

## 5. Milestone M-PROOFS-B (#5): снос tautological `() -> Bool ensures true` axioms

### 5.1 Inventory (МSFS-сторона, 53 axiom-placeholder)

Категории по leverage:

**Group A (anchors — оставить, понизить severity):**
- `msfs_stage_m_*_anchor` (14 анкеров стейджей) — это просто markers для audit-coord, не структурные axioms. Решение: оставить, но добавить attribute `@audit_anchor` чтобы proof-honesty script их пропускал.

**Group B (definitions — promote to witness-parameterised):**
- `msfs_definition_4_1_f_s` ... `4_4_l_abs` (4 def L_Abs conditions) — уже witness-parameterised в текущей версии (см. `absolute_layer.vr`). ✓
- `msfs_definition_8_3_5` (display map categories) — TODO promote.
- `msfs_definition_9_1_2` (Meta_Cls + maximality) — TODO.
- `msfs_definition_10_1_E_category` — currently `() -> Bool ensures true`; **promote to witness-parameterised** через `core/math/dual_absolute_layer.vr::DualLAbsCandidate`. 

**Group C (theorems anchor → witness):**
- `msfs_theorem_10_4_ac_oc_morita_duality` — currently anchor-axiom; structural witness уже в `dual_absolute_layer.vr::msfs_theorem_10_4_ac_oc_morita_duality_witness`. Drop the anchor.
- Similar для 10.7, 10.9 anchors.

**Group D (open questions):**
- 5 Q1-Q5 entries — replace `axiom ... ensures true` with proper `@open_question(name, "...")` attribute on a phantom marker, or just delete.

### 5.2 Acceptance

- [ ] 0 entries в `core/math/...` имеют `@axiom ... -> Bool ensures true` form (кроме Group A audit anchors).
- [ ] `verum audit --framework-axioms` показывает только witness-parameterised axioms или audit-anchor markers.
- [ ] proof-honesty script flags any new tautological axiom как regression.

### 5.3 Sub-tasks

- [ ] Sub-5.1: ввести `@audit_anchor` attribute, отфильтровать stage anchors.
- [ ] Sub-5.2: drop anchor-axioms для §10 (10.1, 10.2, 10.3, 10.4 anchors), оставить только witness-parameterised host-stdlib versions.
- [ ] Sub-5.3: §8 definitions display-map → promote.
- [ ] Sub-5.4: §9 Meta_Cls + maximality definitions → promote.
- [ ] Sub-5.5: §12 Q1-Q5 → `@open_question` attribute.

---

## 6. Milestone M-PROOFS-C (#6): реальный Lemma 3.4

### 6.1 Текущая axiomatic форма

`core/math/s_definable/lemma_3_4.vr:107`:
```verum
public axiom msfs_lemma_3_4_s_definability(
    x: &FormallySDefinable,
) -> &SSMembership
    requires x.has_phi_x_witness()
    ensures result.is_in_s_s_global() && result.closure_depth() >= 1;
```

### 6.2 Целевая 4-шаговая форма

```verum
public theorem msfs_lemma_3_4_s_definability(
    x: &FormallySDefinable,
) -> &SSMembership
    requires x.has_phi_x_witness()
    ensures  result.is_in_s_s_global() && result.closure_depth() >= 1
    proof {
        // Step 1: φ_X presents X ∈ Ob(Syn(F)) for some F ⊆ S
        let f_subtheory: &SubTheory = x.witness_subtheory();
        let phi_x: &Formula = x.witness_formula();
        let x_in_syn_f: SynObject = present_in_syn(f_subtheory, phi_x);
        // Step 2: Syn(F) is Grothendieck construction over Mod(F) by HTT 5.1.4
        let mod_f: &QuasiCategory = mod_of(f_subtheory);
        let syn_grothendieck: SynIsGrothendieck =
            apply lurie_htt_5_1_4_grothendieck(f_subtheory, mod_f);
        // Step 3: Hence Syn(F) ∈ S_S^global by Definition 3.3 closure O2
        let syn_in_global: SSMembership = apply definition_3_3_o2_closure(
            mod_f, syn_grothendieck);
        // Step 4: X ∈ Ob(Syn(F)) ⊆ S_S^global ⊆ S_S
        return inherit_membership(syn_in_global, x_in_syn_f);
    };
```

### 6.3 Зависимости

- M-INFRA: `Syn(F)`, `Mod(F)`, Grothendieck construction (constructive).
- M-PROOFS-B: ensure `lurie_htt_5_1_4_grothendieck` is а witness-parameterised axiom, not tautological.

### 6.4 Этапы

**Phase v1** (без полного M-INFRA): переписать как `@theorem` с 4 шагами, каждый — apply named axiom. Это уже улучшение vs текущего single-axiom — кернел re-checks the chain.

**Phase v2** (после M-INFRA MVP): шаги 2 и 3 становятся apply *theorem*, не axiom. Lemma 3.4 unconditional modulo Lurie HTT 3.2 axiomatization.

**Phase v3** (после full M-INFRA): шаг 2 (HTT 5.1.4 Grothendieck) reduce-ed to constructive Grothendieck. Lemma 3.4 fully constructive.

### 6.5 Acceptance

- [ ] Phase v1: Lemma 3.4 `@verify(formal)` @theorem с 4 explicit `apply` шагами.
- [ ] proof-honesty: Lemma 3.4 классифицируется как `theorem-multi-step`, deps=2-3.
- [ ] Каждый downstream consumer (Theorem 5.1, 6.1, 7.1, 8.6, 9.6, 10.4) sees the proof chain on the kernel-recheck stack.

---

## 7. Milestone M-PROOFS-D (#7): структурные тела для §6, §7, §8

### 7.1 Inventory

§6 (5 теорем): 6.1, Prop 6.2, Prop 6.3, Cor 6.4, categorical/topos/spectral towers stabilization.
§7 (6 теорем): 7.1, 7.2, 7.3, 7.4, Definition 7.5, 7.6.
§8 (7 теорем): 8.1, 8.2, Definitions 8.3-8.5, 8.6, 8.7, Cor 8.8.

Текущее состояние: §7 уже имеет per-axis reductions + apply Theorem 5.1 (это done в M0.7-A). §6, §8 — partial.

### 7.2 Целевые промоции

**§6:**
- Theorem 6.1 (no-limit-escape) — 5-шаговое тело (Lemma 3.4 на каждом A_κ + Adámek-Rosický filtered-colimit closure + colimit ∈ S_S^global + id_{colim} violates Π_4 + closing identity).
- Prop 6.2 — case-split на uniformly/non-uniformly generated towers. Currently `() -> Bool ensures true`, нужно promote.
- Prop 6.3 trajectory space — 4 equivalent presentations (path-groupoid, simplicial set, choice tree, relation cat).

**§8:**
- Theorem 8.6 (∃ I) — 8-шаговая construction. Это ОЧЕНЬ trudoёмкое тело (≥80 LOC).
- Theorem 8.7 (slice-locality I) — 3-шагов (tilde_π definition + 2-cell commute + no-new-base-point).
- Theorem 8.1 (universe-poly) — Grothendieck fibration section argument, ≥4 шага.
- Theorem 8.2 (reflective tower) — Rathjen-Feferman + meta-vertical stab.

### 7.3 Acceptance

- [ ] proof-honesty: §6 + §7 + §8 имеют 0 axiom-only @theorems, все multi-step.
- [ ] Theorem 6.1, 8.6, 8.7 имеют ≥5 явных `apply` шагов в proof body.
- [ ] `verum audit --framework-axioms` показывает per-theorem framework-footprint, ν ≤ ω для §6 + §7, ν ≤ ω+5 для §8 (display-map уровень).

---

## 8. Milestone M-PROOFS-E (#8): §9, §10, §11

### 8.1 Inventory

§9 (3 thm + 2 def): Theorem 9.3 meta-categoricity (5 шагов), 9.4 multiplicity (3 witnesses), 9.6 stabilization (depth-embedding).
§10 (7 thm): 10.1-10.9. **Theorem 10.4 — флагман**, 4 шага.
§11 (1 main thm): Theorem 11.1 subsumption (7 case-rows: Cantor, Russell, Gödel I/II, Tarski, Lawvere FP, Ernst).

### 8.2 Acceptance

- [ ] Theorem 10.4 — структурное тело с 4 явными шагами (a-d).
- [ ] Theorem 11.1 — case-split с 7 explicit branches, каждый имеет (S_T, n_T, X_T) explicit triple + φ_X_T witness.
- [ ] `Theorem 9.3 / 9.4 / 9.6` имеют structural bodies.

---

## 9. Milestone M-EXPORT (#9): cross-format CI gate

### 9.1 Pipeline

```yaml
# .github/workflows/cross-format-recheck.yml
name: Cross-Format Re-check
jobs:
  lean:
    steps:
      - run: verum export --to lean theorems/
      - run: cd exports/lean && lake build
  coq:
    steps:
      - run: verum export --to coq theorems/
      - run: coqc -R exports/coq VerumMSFS exports/coq/*.v
  agda:
    steps:
      - run: verum export --to agda theorems/
      - run: agda --safe exports/agda/Verum.agda
  dedukti:
    steps:
      - run: verum export --to dedukti theorems/
      - run: dkcheck exports/dedukti/*.dk
  metamath:
    steps:
      - run: verum export --to metamath theorems/
      - run: metamath verify proof '*' < exports/metamath/verum.mm
```

### 9.2 Зависимость

Полный proof-term lowering (M-VVA Sub-2.5..2.7) ОБЯЗАТЕЛЕН. Без этого экспорт даёт только `axiom`/`Admitted`/`postulate`-stubs, которые external re-checker примет тривиально.

### 9.3 Acceptance

- [ ] CI green на 5 форматах.
- [ ] Любая регрессия (теорема не re-derived in any format) — block merge.
- [ ] `audit-reports/cross-format.json` показывает per-theorem-per-format pass/fail status.

---

## 10. Milestone M-DIAKRISIS (#10) — после полного MSFS

Не трогать пока MSFS не закрыт. Текущий baseline: 53 multi-step + 52 axiom-only + 32 axiom-placeholder = 137 entries (ожидается 142 теорем + 13 axioms).

### 10.1 Стэйджи (per `msfs-diakrisis-machine-verification.md`)

D.1-D.2: 13 axioms Axi-0..9 + 50 foundational (10.T-50.T)
D.3-D.4: AC/OC bi-adjunction + Five-axis internal + Three bypass paths (10 thm)
D.5-D.6: Maximality (Q1) + Q3/Q4/Q5 closures (12 thm)
D.7-D.8: Aктика 107.T-127.T + 138.T-141.T (28 thm) + Operational coherence
D.9: UHM articulation
Research: 136.T (T-2f***), 137.T (weak), 142.T (Eastern)

### 10.2 Acceptance

- [ ] 142 @theorem all multi-step.
- [ ] 13 Axi-* axioms remain @axiom но witness-parameterised.
- [ ] All 5 export formats re-check.

---

## 11. Milestone M-AUDIT (#11): peer review + reproducibility

### 11.1 External reviews

- [ ] Категорный эксперт (Riehl-уровень) оценивает framework-axiom coverage.
- [ ] Lean / Coq parallel formalization Theorem 5.1 от внешнего волонтёра — independent witness.
- [ ] Submit to ITP / FoSSaCS / LICS as paper "Machine-Verified MSFS in Verum".

### 11.2 Reproducibility

- [ ] `Dockerfile` с pinned Z3 4.12.x, CVC5 1.x, Lean 4.x.x, Coq 8.x, Agda 2.6.x.
- [ ] `make verify-all` reproducibly green в clean docker.
- [ ] Mutation tester: replace random axiom с `false ensures`, verify dependent theorems fail.

---

## 12. Realistic timeline summary

| Milestone | Optimistic | Realistic |
|---|---|---|
| M-PROOFS-A (#4) | 1 week | 2-4 weeks |
| M-PROOFS-B (#5) | 2 weeks | 1-2 months |
| M-VVA (#2) | 3 months | 6-9 months |
| M-INFRA (#3) MVP | 6 months | 12-18 months |
| M-PROOFS-C v1 (#6) | 2 weeks | 1 month (gated by M-PROOFS-B) |
| M-PROOFS-C v2-v3 | 2 months | 6-12 months (gated by M-INFRA) |
| M-PROOFS-D (#7) | 2 months | 4-6 months (gated by M-PROOFS-C v1) |
| M-PROOFS-E (#8) | 1 month | 3-4 months (gated by M-PROOFS-D) |
| M-EXPORT (#9) | 3 months | 6-9 months (gated by M-VVA Sub-2.5..2.7) |
| M-DIAKRISIS (#10) | 6 months | 12-24 months (gated by all above) |
| M-AUDIT (#11) | 3 months | 6-12 months (gated by M-DIAKRISIS) |

**Total optimistic:** 18 months.
**Total realistic:** 4-6 years (single-person), 2-3 years (3-5 person team).

Это сравнимо с Liquid Tensor Experiment (5 лет, 12 человек, 70K LOC) и Odd Order Theorem (6 человеко-лет, Coq+SSReflect).

---

## 13. Что делается прямо сейчас (текущая сессия 2026-04-28)

- [x] Все 10 milestones созданы как tasks #2-#11.
- [x] Этот roadmap document.
- [ ] Foundational `core/math/syn_mod.vr` skeleton (Syn(F) + Mod(F) + LambekScottAdjunction protocols) — start of M-INFRA (см. следующий commit).
- [ ] proof-honesty audit script enhancement: distinguish structural-vs-tautological axiom-only @theorems — start of M-PROOFS-A.

Future sessions pick up от любого pending task в TaskList.
