# Verum's Unified Type Theory — Reference Architecture

> **Это центральный архитектурный документ языка.** Он отвечает на
> вопрос «что на самом деле является теоретическим основанием
> Verum?» и показывает, что язык не — собрание фич, а **когерентная
> стратифицированная теория типов**.

## Тезис

Verum — это первая реализация **Stratified Refined Quantitative
Two-Level Type Theory** (SR-QTT-2LTT) с интеграцией SMT-backed
gradual verification и model-theoretic protocol discharge.

Все «фичи» языка — рефайнмент-типы, dependent types, HoTT/cubical,
CBGR, протокольные аксиомы, tactic DSL — суть **специализации**
этой теории, а не параллельные механизмы.

---

## 1. Исследование: ландшафт type theory

Сравнительная карта современных проof-oriented языков:

| Система | MLTT | Cubical/HoTT | QTT | Refinement | SMT | Classical | Practical |
|---------|------|--------------|-----|------------|-----|-----------|-----------|
| Coq     | ✓ CIC | partial (HoTT lib) | — | — | — | axiom-only | **+** |
| Lean 4  | ✓ CIC | mathlib | — | — | — | axiom | **++** |
| Agda    | ✓ | ✓ (cubical-mode) | partial | — | — | — | **+** |
| Idris 2 | ✓ | — | **✓** | — | — | — | **+** |
| F*      | ✓ | — | — | **✓** | **✓** | classical | **++** |
| Isabelle/HOL | simple+poly | — | — | — | partial | **✓** | **+++** |
| **Verum** | **✓** | **✓** | **✓** | **✓** | **✓** | **gradual** | **target: +++** |

Ни одна existing система не интегрирует все 5 столпов *когерентно*.
F* близко (refinement + SMT + deps), но без HoTT/QTT. Idris 2 имеет
QTT, но без SMT. Verum — первая попытка unified.

**Риск**: комбинирование может дать ad-hoc грабли. Этот документ
показывает, что при правильной стратификации — это строго
согласованная теория.

---

## 2. Анализ: существующие основания Verum

Что уже есть в кодовой базе:

### 2.1 Two-Level Type Theory (`core/types/two_level.vr`)

```verum
public type Layer is
    | Fibrant    // HoTT/cubical; UIP does NOT hold
    | Strict     // UIP holds, decidable equality
```

**Смысл**: универсумы разделены на два слоя. Fibrant-слой поддерживает
path-типы и univalence (`Path<T>(a, b)` — нетривиальные equalities).
Strict-слой имеет UIP (Uniqueness of Identity Proofs) — все доказательства
равенства равны. **Strict contagion**: `mix(Strict, Any) = Strict`.

### 2.2 Quantitative Type Theory (`core/types/qtt.vr`)

```verum
public type Quantity is Zero | One | Many | AtMost { n }
```

**Смысл**: каждая переменная — ресурс. `Zero` = erased (доказательства,
types); `One` = linear (must-use-exactly-once); `Many` = unrestricted;
`AtMost(n)` = bounded.

**Алгебра**: `add(One, One) = AtMost(2)`, `Many ⊕ _ = Many` —
линейность заразна снизу-вверх, ограниченность — к верху.

### 2.3 Refinement Types (`verum_types::refinement`)

```verum
type Nat is Int{self >= 0};
fn safe_div(a: Int, b: Int{self != 0}) -> Int { a / b }
```

**Смысл**: базовый тип + SMT-проверяемый предикат. Подтипирование:
`Int{p} <: Int{q}` ⟺ SMT ⊢ ∀x:Int. p(x) → q(x).

### 2.4 Cubical HoTT (`core/math/hott.vr`)

Интервал `I`, `Path<T>(a, b)`, `Equiv`, `IsContr`, `IsProp`, `IsSet`,
HITs (S¹, Susp, Trunc). 8 правил редукции в cubical.rs.

### 2.5 CBGR три-уровневых ссылок

```verum
&T           // Tier 0: CBGR-проверяемый (15ns)
&checked T   // Tier 1: compiler-proven safe (0ns)
&unsafe T    // Tier 2: manual proof obligation (0ns)
```

### 2.6 Протокольные axioms (`ProtocolItemKind::Axiom`)

```verum
type Group is protocol {
    axiom assoc(a, b, c) ensures (a · b) · c == a · (b · c);
};
```

### 2.7 SMT-интеграция

`ProofSearchEngine::auto_prove(goal)` — Z3/CVC5 через
`verum_smt` crate. Bridge к tactic DSL (T1-O).

---

## 3. Синтез: унифицированная модель

### 3.1 Семантический core

Каждое значение в Verum классифицируется по **четырём ортогональным
координатам**:

```
(universe_layer, universe_level, quantity, refinement)
```

Где:
- **universe_layer** ∈ {Fibrant, Strict} — HoTT vs classical
- **universe_level** ∈ ℕ — Type(0) : Type(1) : Type(2) : ...
- **quantity** ∈ {Zero, One, Many, AtMost(n)} — resource usage
- **refinement** : `base -> Bool` (optional SMT predicate)

Тип `Int{>= 0}` в Strict-слое на уровне 0 с quantity Many — это
`(Strict, 0, Many, λx. x >= 0)`.

Тип `Path<S1>(base, base)` в Fibrant-слое на уровне 0 с quantity Many
и без refinement — это `(Fibrant, 0, Many, ⊤)`.

Тип `&mut Mutex<T>` в Strict-слое с quantity One — это линейная
ссылка, которую должна быть использована ровно один раз.

### 3.2 Правила коэрции (soundness gate)

Центральная гарантия **soundness**: значения могут течь между
этими классификациями только по строгим правилам:

#### Layer flow
```
Fibrant ↪ Strict    (ok — забыть path-структуру)
Strict  ⊬ Fibrant    (запрет — нельзя пересадить UIP в HoTT)
```

#### Level flow (cumulativity)
```
Type(i) ↪ Type(j) при i ≤ j
```

#### Quantity flow
```
Many ↪ One     (запрет — unrestricted нельзя использовать как linear)
One  ↪ Many    (разрешить — может, не обязано быть использовано больше)
Zero ↪ * at type level; * ↪ Zero только при erasure
```

#### Refinement flow
```
Int{p} <: Int{q}    ⟺    SMT ⊢ p → q
```

### 3.3 Протоколы как теории + модели

`type T is protocol { methods; axioms }` — это **сигнатура + теория** в
model-theoretic смысле. Семантика:

```
⟦type T is protocol { methods, axioms }⟧
  =  ⟨signature(methods), {M | ∀axiom ∈ axioms. M ⊨ axiom}⟩
```

То есть **тип протокола — это категория моделей**.

`implement T for Concrete { ... }` — это **выделение конкретной модели**.
Model-verification (T1-R phase 2) проверяет `Concrete ⊨ axiom` для
каждой аксиомы — чисто классическая model theory.

### 3.4 Tactic DSL как proof-in-a-monad

Tactics живут в монаде `TacticM`:
```
TacticM<A> ≈ ProofState -> Result<(A, ProofState), TacticError>
```

`let x = e; rest` ≈ monadic bind.
`match x { p => t }` ≈ monadic case analysis.
`try { t } else { f }` ≈ `MonadError.recover`.
`fail(msg)` ≈ `throwError msg`.

`ring`, `smt`, `auto` — это **atomic tactics** вызывающие `auto_prove`
из SMT backend. Это **uniform path**: tactic DSL + SMT + cubical
kernel — все говорят одним языком ProofGoal/VerificationResult.

### 3.5 Gradual verification spectrum

Каждая функция может задать свой уровень проверки:
```
@verify(none)       — no checks
@verify(runtime)    — runtime asserts
@verify(static)     — typechecker only
@verify(formal)     — SMT-discharged (default)
@verify(certified)  — externally-checkable proof artifact
```

Это не **отдельная система**, а **параметр**: то же самое
`refinement Int{p}` превращается в runtime assert при `@verify(runtime)`
и в SMT goal при `@verify(formal)`.

---

## 4. Эталонное решение: пять архитектурных правил

### Правило 1. **Stratification over unification**
Не пытайся унифицировать Fibrant и Strict в одну теорию. Разделяй
чётко: cubical computation только в Fibrant, SMT только в Strict,
bridge через layer-coerce.

**Следствие для реализации**: `verum_types::ty::Universe` имеет
поле `layer: Option<Layer>`. Default = Strict (обычный код). Fibrant
при явном `@layer(fibrant)` или при появлении `Path<T>(_)` в типе.

### Правило 2. **Refinement при объявлении, SMT при запросе**
Refinement-предикат живёт как expression AST (не решённый).
SMT вызывается только когда нужно подтипирование / контракт /
обязательство доказательства.

**Следствие**: `Type::Refined { base, predicate: Expr }` не содержит
Z3 AST. Компиляция идёт normally; SMT goal генерится лениво.

### Правило 3. **Model theory как first-class конструкция**
`implement` — это не «сахар над dispatch table». Это
экзистенциальная attestation: «существует модель теории на этом
конкретном типе, и доказательства discharged».

**Следствие**: pipeline обязан вызывать `collect_impl_obligations`
+ `verify_impl_axioms` для каждого `ImplKind::Protocol`. Нет
обхода.

### Правило 4. **QTT как оптимизация, а не ограничение**
Zero-quantity значения автоматически erased на Tier-1 (AOT).
Linear значения компилируются в stack-allocation с destructor после
последнего use. Many — обычный heap.

**Следствие**: CBGR (three-tier refs) — это QTT в маске.
`&T` = Many ref с runtime generation; `&checked T` = One ref в
compiler-proven scope; `&unsafe T` = manual Zero obligation.

### Правило 5. **Tactic как monad, не как macro**
Tactics — first-class values в proof monad. Composition —
normal monadic. `ring` и `auto` — методы на том же ProofState.

**Следствие**: T1-W step 2 — это не «wire SMT into tactic engine»,
а **confluence**: показать, что два пути (direct `auto_prove` и
`apply_ring` → `try_ring` → `auto_prove`) дают один результат.

---

## 5. Дорожная карта через унифицированный core

С этой архитектурой все открытые T1-* задачи находят своё место:

| Task | Место в унифицированной теории |
|------|--------------------------------|
| T1-R phase 2 | Правило 3: model-theoretic discharge |
| T1-W step 2 | Правило 5: monadic tactic confluence |
| T1-F phase 2 | Правило 2: SMT при @verify(runtime) = runtime assert |
| T1-H (opcodes) | Правило 4: QTT erasure решает какие opcodes нужны |
| T1-T phase 3 | quotient type = fibrant-universe HIT; следует из Layer rules |
| T1-Y phase 3 | graph algorithms — Many quantity, Strict layer, no refinements |
| T1-X phase 2 | morphism coherence = T1-R под другим именем |
| T1-G step 2 | VBC sentinel — нарушение правила «stdlib-agnostic type system» |

### Приоритизация по impact × alignment

1. **T1-R phase 2** (model-theoretic discharge) — единственное,
   что делает слово «protocol» означающим теорию, а не интерфейс.
2. **T1-W step 2** (tactic confluence) — делает proof DSL
   реально выполняемым.
3. **T1-F phase 2** (refinement runtime) — закрывает gradual
   verification spectrum.
4. **T1-Y phase 3** (graph algorithms) — показывает unified design
   на конкретной предметной области.

После этих четырёх язык переходит из состояния «parser + AST +
infrastructure» в состояние «functioning unified proof system».

---

## 6. Соответствие 6 принципам Verum

| Принцип | Как унифицированная теория его реализует |
|---------|-------------------------------------------|
| **Semantic honesty** | Тип — это `(layer, level, qty, refinement)`, а не «размер байт». Имя `List<T>` описывает category of models. |
| **Verification spectrum** | Прямое следствие Правила 2 — `@verify(...)` переключает SMT между compile-time goal и runtime assert. |
| **No hidden state** | QTT даёт explicit resource tracking. Layer coercion предотвращает unsoundness. Axioms видимы. |
| **Zero-cost default** | Правило 4 + layer erasure: Zero-quantity, Type-level values автоматически стираются на Tier-1. |
| **No magic** | Каждый шаг — mechanical. SMT-бэкенд вызывается только через explicit tactic или verified contract. Cubical computation — 8 правил redукции, документированы. |
| **Radical expressiveness** | Все 5 столпов (MLTT + HoTT + QTT + Refinement + SMT + Classical-gradual) в одной теории. Никто больше так не делает. |

---

## 7. Заключение: что делает Verum уникальным

Verum — **первый язык, где proof-oriented и system-oriented
концепции живут в одной теории**. Не в двух (как F* с refinement + ML)
и не в трёх (как Lean + Mathlib + runtime), а в **одной
стратифицированной теории типов**.

Это значит:
- Нет «embedded proof system» — есть единый язык.
- Нет «unsafe world» — unsafe — это QTT quantity с явной proof
  obligation.
- Нет «runtime vs compile-time» — это `@verify(...)` attribute
  на том же коде.
- Нет «pure vs effectful» — это `using [...]` context clause.
- Нет «strict vs lazy» — это layer annotation.

**Реферанс-решение для каждого нерешённого вопроса в T1-campaign:**
посмотри на эти четыре координаты, найди куда фича попадает,
применяй правила 1-5. Ответ всегда получается конкретный, локальный,
soundness-preserving.

Это и есть «эталонное решение, согласованное с философией и
архитектурой языка».
