# Ranking methodology v3

This document defines the scoring contract for ipbr's four role proxies:
Idea, Planning, Building, and Reviewing. The authoritative numeric settings
live in `data/coefficients.toml`; this document explains how the scorer uses
them.

The scores are benchmark-based capability proxies, not guarantees about a
particular provider endpoint, agent, or deployment. In particular, Reviewing
is still largely a proxy: it now leads with a direct code-review signal
(Factory's mean F1 over 50 real PRs) where one exists, but that cohort is
narrow and supplemental, so most of the Review weight still comes from
implementation, planning, and a search/document preference proxy. EQ-Bench
Judgemark measures judge discrimination and remains diagnostic-only.

## 1. Ranked entity and configuration policy

The ranked entity is one canonical model record. ipbr does not publish a
separate rank for every effort, provider, agent, or harness combination.

For each source metric, ingestion keeps the strongest eligible observation
available for that canonical model. Reasoning effort is preferred in this
order:

```text
max → xhigh → high → thinking/adaptive → medium → default
```

Low, explicit `minimal`, and non-reasoning variants are excluded unless an
explicit effort-policy exception permits them. `instant` is not a global
effort suffix: it remains part of product identity unless a source-audited
endpoint uses it for a Low mode. Sources that publish several agents or harnesses
likewise retain their best model-level row. The resulting record is therefore
a best-available capability envelope, not necessarily one runnable
configuration. Schema 2.0 makes this explicit with:

```toml
configuration_policy = "best_available_max_effort"
```

The per-model `thinking_effort` field serializes this envelope as
`best_available`; it is not a promise that every metric used one common
setting.

Canonical identity includes lifecycle. `preview` denotes a particular preview
product/build and `latest` can denote a moving route, so neither word is
removed by generic alias normalization. A lifecycle spelling is accepted only
when it is an explicit, source-audited alias. When a retired endpoint starts
redirecting to a successor, historical benchmark observations remain on the
frozen build while post-redirect observations belong to the served successor
and must carry dated routing provenance.

## 2. Pipeline overview

1. Fetch each verified source and match source names to canonical model IDs.
2. Select one winning value for each `(model, metric)` using evidence and
   effort precedence.
3. Normalize each raw benchmark against fixed, versioned raw-unit anchors.
4. Pull observations toward the neutral prior according to evidence
   reliability.
5. Build non-overlapping composites and diagnostic groups.
6. Flatten each role to scored leaf metrics, cap correlated source families,
   and estimate capability from available same-product evidence.
7. Publish role scores together with provenance, evidence coverage, and
   ranked/provisional status.

## 3. Observation precedence and provenance

Evidence precedence is order-independent:

```text
native public observation > manual same-product observation
```

A native leaderboard observation always replaces a manual override for the
same ranked product and metric. This is a source-precedence rule, not an evidence-class
difference: a cited override is still an actual same-product observation and
counts as direct coverage, including when the model vendor published it. Every
scored observation is a direct same-product measurement; there are no
synthesized sibling fills.

Schema 2.1 records the winning source and evidence class for every scored raw
metric. Manual observations retain their citation and `source = "overrides"`.
See `docs/output-schema.md`.

## 4. Fixed-anchor normalization

### 4.1 Versioned raw anchors

Every active scored leaf has `anchor_low` and `anchor_high` in the versioned
coefficient set. Anchor set `2026-07-12.v2` freezes the direct-evidence p5/p95
raw values from the refreshed 2026-07-12 snapshot. Manually curated same-product
observations are direct evidence too. Adding or removing unrelated models from
the current cohort therefore does not move another model's normalized score.

Changing an anchor is a methodology change and must be reviewed together with
the coefficient diff. The effective anchors are echoed to
`out/coefficients.toml` on every run.

The implementation retains cohort transforms for explicitly unanchored
reference or diagnostic metrics and for custom coefficient files. They are
not the v3 contract for active scored leaves.

### 4.2 Logistic 5/95 mapping

Let `a` and `b` be the low and high anchors after any configured log transform,
and let:

```text
m = (a + b) / 2
k = 2 ln(19) / (b - a)
```

For a higher-is-better metric:

```text
N(x) = 100 / (1 + exp(-k(x - m)))
```

For a lower-is-better metric, the sign of the exponent is reversed. The low
and high anchors map to approximately 5 and 95 for higher-is-better metrics,
or 95 and 5 for lower-is-better metrics. Values outside the anchors continue
asymptotically toward 0 or 100 instead of clipping.

When `log_scale = true`, `x`, `a`, and `b` are transformed with `ln` first and
must be positive.

## 5. Evidence reliability and continuous priors

The neutral prior is 50. A normalized observation `N` with reliability `q`
becomes:

```text
S = 50 + q (N - 50)
```

Default v3 reliabilities are:

| Evidence class | Reliability |
|---|---:|
| Actual same-product observation, native or manually curated | 1.00 |

Every scored observation is a direct same-product measurement, so reliability
is uniform; the neutral prior only stands in for missing evidence.

### 5.1 Missing weight and confidence

The capability point estimate is conditional on available direct same-product
evidence. For observed weights `w_i`:

```text
capability = Σ observed w_i S_i / Σ observed w_i
```

Missing weights do not enter that numerator or denominator; they remain in the
separate nominal evidence summary. A fully unsupported aggregate uses 50 only
as a compatibility fallback and is provisional.

This prevents benchmark availability from being mistaken for low capability.
It can make sparse estimates more extreme, which is why ordinal qualification
is evaluated separately from score contribution. Broad core benchmarks form
the primary eligibility portfolio; narrow supplemental benchmarks may move the
score when present without making every missing row an eligibility penalty.
Retired but still valid direct benchmarks can establish that an older model
has real historical coverage, but their values have zero weight in the current
score. Section 9 defines the gates precisely. A fully absent composite remains
absent; a partially observed composite uses available evidence and carries
recursive coverage separately.

`groups_shrunk` remains a compatibility name for groups with incomplete
nominal coverage; it does not select a different scoring formula.

## 6. Independent source families

Before a final role score is computed, the role graph is flattened to unique
leaf observations. Leaves are grouped by their configured benchmark family
(`lmarena`, `eqbench`, `artificial_analysis`, `swe`, `scale`, and so on).

No family may carry more than 30% of a role's nominal leaf weight. Excess
mass is redistributed across the other families by water-filling. If a role
has too few families for a 30% cap to sum to one, the smallest feasible cap,
`1 / family_count`, is used.

This prevents several correlated metrics from one publisher or benchmark
suite from masquerading as independent confirmation.

## 7. Composites and duplicate-signal corrections

Composites first combine related observations so the same construct is not
counted repeatedly.

| Composite | Inputs |
|---|---|
| `SWEComposite` | SWE-rebench .45, SWE-Bench Pro .45, SWE-Bench Multilingual .10 |
| `SWEAtlasComposite` | Q&A .30, test writing .30, refactoring .40 |
| `SonarComposite` | diagnostic only: functional skill .60, total issue density .40 |
| `LiveCodingComposite` | SciCode .60, LM Arena code .40 |
| `AAGeneralComposite` | HLE .333, GPQA .167, CritPt .167, Omniscience accuracy .222, non-hallucination .111 |
| `LongContextComposite` | AA-LCR .60, Context Arena MRCRv2 AUC@128k .40 |
| `EnterpriseWorkflowComposite` | EnterpriseOps-Gym-AA .65, AutomationBench-AA .35 |
| `TerminalBench21Composite` | official Terminal-Bench 2.1, with AA as fallback |
| `BFCLComposite` | diagnostic only: BFCL overall 1.00 |

Important v3 corrections:

- LM Arena text Elo is no longer copied into a fictitious creativity metric.
  EQ-Bench Creative Writing v3 supplies direct creativity evidence.
- GPQA and HLE are independent leaves and are combined once; the identical
  `ArtificialAnalysisReasoning` / `GPQA_HLE_Reasoning` pair is no longer
  emitted or scored.
- BFCL overall is not combined with the categories from which it is derived.
- Sonar total issue density is not stacked with its nested bug and
  vulnerability components.
- Aggregate AA intelligence/coding indices are not stacked with their
  component evaluations in the same role path.
- AA-LCR appears only inside `LongContextComposite`, not again inside the AA
  general composite, so planning and building never count one observation
  twice.
- AA v4.1 retired tau2 and Terminal-Bench Hard in favor of tau3-Banking and
  Terminal-Bench 2.1, and removed saturated IFBench. The retired fields remain
  readable diagnostics but have no primary path.
- GDPval-AA v2 now uses a neutral native source; the older manually curated
  GDPval observations remain diagnostic and are not stacked with it.
- AIME 2025, MMLU-Pro, Terminal-Bench 2.0, and SWE-bench Verified have no
  current score path but may provide direct historical support. AA
  LiveCodeBench, BrowseComp, HLE-with-tools, Toolathlon, and OSWorld remain
  diagnostics with no score or eligibility path.
- DeepSWE v1.1 supplies direct long-horizon BUILD evidence under one fixed
  harness. Its weight replaces Terminal-Bench Hard and part of the correlated
  SWE composite rather than simply increasing the total SWE-family mass.
- Vendor-automatic fallback is part of the served ranked product. Upstream
  fallback-labelled observations therefore use the primary metric keys, count
  as direct evidence, and retain a routing citation. Separately named
  multi-agent or premium endpoints remain distinct products.
- Benchmark sources that select the best agent/harness submission retain the
  winning upstream submission label as a per-metric citation. This does not
  create a separate ranked model; it makes the capability-envelope choice
  auditable.
- BFCL, GSO, and Judgemark remain available as diagnostics but have no path to
  a role score. HiL-Bench contributes only a small Plan signal.
- DeepSWE and SWE Atlas remain scored Build evidence, but are supplemental for
  eligibility: their narrow cohorts cannot make an otherwise established
  model provisional.
- Sonar remains diagnostic because current rows mix explicit Medium and
  Thinking effort levels and its narrow code-quality profile was decisive in
  otherwise close Build rankings.

Legacy metric keys may remain declared with no scoring path so older consumers
can parse historical files.

## 8. Groups and role formulas

The active capability groups are:

| Group | Purpose |
|---|---|
| `CRE` | Direct creative writing, text preference, and novel pattern induction |
| `GEN` | General/reasoning breadth, factual calibration, and research reasoning |
| `PLAN` | Multi-step reasoning, enterprise workflows, tool use, escalation, and long-context planning |
| `BUILD` | Long-horizon software implementation, terminal work, tool orchestration, and live coding |
| `CODE_REVIEW_DIRECT` | Direct code-review evidence: Factory mean F1 over 50 real PRs |
| `REVIEW_DIRECT` | diagnostic-only EQ-Bench Judgemark judge discrimination |
| `LM_ARENA_REVIEW_PROXY` | Search/document preference proxy |

The group metric weights are authoritative in `data/coefficients.toml`. Final
role paths are:

```text
Idea     = 0.65 CRE + 0.35 GEN
Planning = 0.65 PLAN + 0.35 GEN
Building = 0.95 BUILD + 0.05 PLAN
Review   = 0.20 CODE_REVIEW_DIRECT + 0.20 LM_ARENA_REVIEW_PROXY + 0.30 BUILD + 0.30 PLAN
```

Where a model has no Factory code-review observation, the
`CODE_REVIEW_DIRECT` weight redistributes across the remaining observed
evidence, so unmeasured models are ranked on the other three inputs rather
than penalized for a missing benchmark.

These expressions define the nominal graph. The final calculation is made
from flattened leaves after the family cap, so the published result cannot be
reconstructed by simply averaging the displayed group numbers.

### 8.1 Balanced capability

The site and rank-change report expose a balanced capability view:

```text
Balanced = (Idea + Planning + Building + Review) / 4
```

It is an unweighted presentation view, not a separately trained score. It is
ranked when at least three component roles are ranked and the remaining role
has at least 20% current direct coverage. Otherwise it is provisional. The
four numeric inputs do not change when only the Balanced status changes.

## 9. Ranked versus provisional

A role remains computable when evidence is sparse. Each scored metric declares
one eligibility class:

- `core` — broad, role-representative current evidence used by the primary
  eligibility gate;
- `supplemental` — valuable specialist evidence that affects the score when it
  exists, but whose absence is not an eligibility penalty; or
- `historical_support` — retired but still valid direct evidence that affects
  coverage history only and has no current score path.

The role is `ranked` when any of these auditable paths holds:

- full-current path: at least 60% direct current weight across three families,
  or at least 35% across five, plus a representative base of at least 35%
  direct core weight across three core families;
- core-current path: after renormalizing the role's core leaves, at least 60%
  direct core weight across three families, at least 50% across four, or at
  least 35% across five; or
- established-history path: at least 25% current direct weight across two
  current families, direct historical support from at least two relevant
  families, and at least five current-plus-historical families in union.

Otherwise it is `provisional`. Every actual same-product observation counts as
direct, including a cited manual override from a vendor report or system card.
Any future rank-derived estimates are non-direct and cannot qualify a role by
themselves. Historical support must also be a same-product observation; its
score is never blended into the capability point estimate.
The representative-core floor prevents a model evaluated only on a favorable
specialist subset from using those observations to fill the entire confidence
claim; it does not change the model's numeric capability estimate.

The v3 historical portfolio is deliberately restricted to stable retired
metrics: AIME 2025 and MMLU-Pro for Idea; tau2, IFBench, and Terminal-Bench 2.0
for Plan; and SWE-bench Verified, Terminal-Bench 2.0, and LiveCodeBench for
Build/Review. Current experimental sources, sparse diagnostics, and
effort-mixed Sonar observations do not establish eligibility.

## 10. Operational diagnostics

Pricing, blended cost, output speed, time to first token, latency, advertised
context window, and related operational fields are diagnostics only. They have
zero path to Idea, Planning, Building, Review, or Balanced capability.

Diagnostic `OPS_*` group values may still be emitted for inspection and
filtering. They must not be interpreted as part of a capability score. This
separation prevents deployment economics or provider routing from changing a
model-capability rank.

## 11. Uncertainty

Where a source publishes measurement uncertainty, schema 2.1 preserves it in
native raw fields:

- `TerminalBenchUncertainty`
- `TerminalBench21Uncertainty`
- `SWERebenchSEM`
- `EQBenchJudgemarkCILow`
- `EQBenchJudgemarkCIHigh`
- `DeepSWECILow` and `DeepSWECIHigh`
- `GDPvalAA2CILow` and `GDPvalAA2CIHigh`
- `FactoryCodeReviewF1Stdev`

These fields are observation-specific and unscored. They support auditing and
future interval-aware ranking; v3 does not claim that all role scores already
have statistical confidence intervals.

## 12. Reproducibility and limitations

- `schema_version = "2.1.0"` identifies the evidence-rich output shape.
- `methodology = "v3"` identifies this scoring and eligibility method.
- The effective coefficients and anchors are emitted with every run.
- `--offline`, a fixed fixture cache, and `--now` produce deterministic output.
- Role labels remain proxies. Weight choices are explicit design judgments,
  not a substitute for held-out human validation on real ideation, planning,
  implementation, and review outcomes.
- Rank stability should be checked with the pre/post comparison report and
  leave-one-family or coefficient sensitivity analyses when methodology
  changes.

For field-level details, see `docs/output-schema.md`. For source lineage, see
`docs/sources.md`.
