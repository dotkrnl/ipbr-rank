# Ranking methodology v2

This document defines the scoring contract for ipbr's four role proxies:
Idea, Planning, Building, and Reviewing. The authoritative numeric settings
live in `data/coefficients.toml`; this document explains how the scorer uses
them.

The scores are benchmark-based capability proxies, not guarantees about a
particular provider endpoint, agent, or deployment. In particular, Reviewing
is still a review proxy: EQ-Bench Judgemark measures judge discrimination, but
the portfolio does not yet contain a broad, direct code-review benchmark.

## 1. Ranked entity and configuration policy

The ranked entity is one canonical model record. ipbr does not publish a
separate rank for every effort, provider, agent, or harness combination.

For each source metric, ingestion keeps the strongest eligible observation
available for that canonical model. Reasoning effort is preferred in this
order:

```text
max/xhigh → high → thinking/adaptive → medium → default
```

Low and non-reasoning variants are excluded unless an explicit effort-policy
exception permits them. Sources that publish several agents or harnesses
likewise retain their best model-level row. The resulting record is therefore
a best-available capability envelope, not necessarily one runnable
configuration. Schema 2.0 makes this explicit with:

```toml
configuration_policy = "best_available_max_effort"
```

The per-model `thinking_effort` field serializes this envelope as
`best_available`; it is not a promise that every metric used one common
setting.

## 2. Pipeline overview

1. Fetch each verified source and match source names to canonical model IDs.
2. Select one winning value for each `(model, metric)` using evidence and
   effort precedence.
3. Normalize each raw benchmark against fixed, versioned raw-unit anchors.
4. Pull observations toward the neutral prior according to evidence
   reliability.
5. Build non-overlapping composites and diagnostic groups.
6. Flatten each role to scored leaf metrics, cap correlated source families,
   and estimate capability from available same-model evidence.
7. Publish role scores together with provenance, evidence coverage, and
   ranked/provisional status.

## 3. Observation precedence and provenance

Evidence precedence is order-independent:

```text
direct public source > cited reported override > sibling synthesis
```

A direct leaderboard observation always replaces a manual override for the
same model and metric. An override can replace a synthesized fill, but cannot
replace a direct observation. Synthesis is fill-only.

Schema 2.0 records the winning source and evidence class for every scored raw
metric. Reported values retain their citation. Synthesized values retain the
donor and synthesis category. See `docs/output-schema.md`.

## 4. Fixed-anchor normalization

### 4.1 Versioned raw anchors

Every active ranked leaf has `anchor_low` and `anchor_high` in the versioned
coefficient set. Anchor set `2026-07-12.v1` freezes the direct-evidence p5/p95
raw values from the refreshed 2026-07-12 snapshot. Metrics available only as
cited reports use the same quantiles over reported evidence. Adding or
removing unrelated models from the current cohort therefore does not move
another model's normalized score.

Changing an anchor is a methodology change and must be reviewed together with
the coefficient diff. The effective anchors are echoed to
`out/coefficients.toml` on every run.

The implementation retains cohort transforms for explicitly unanchored
reference or diagnostic metrics and for custom coefficient files. They are
not the v2 contract for active ranked leaves.

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

Default v2 reliabilities are:

| Evidence class | Reliability |
|---|---:|
| Direct public observation | 1.00 |
| Cited reported override | 0.60 |
| Sibling synthesis, any category | 0.00 |

Sibling fills remain published for provenance and sensitivity analysis, but
their primary-score contribution is exactly the neutral prior. Donor choice
therefore cannot change a model's official point score.

### 5.1 Missing weight and confidence

The capability point estimate is conditional on available direct or cited
same-model evidence. For observed weights `w_i`:

```text
capability = Σ observed w_i S_i / Σ observed w_i
```

Missing and sibling-only weights do not enter that numerator or denominator;
they remain in the separate nominal evidence summary. A fully unsupported
aggregate uses 50 only as a compatibility fallback and is provisional.

This prevents benchmark availability from being mistaken for low capability.
It can make sparse estimates more extreme, which is why official ordinal ranks
require at least 60% direct role weight and three independent direct families.
A fully absent composite remains absent; a partially observed composite uses
available evidence and carries recursive coverage separately.

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
| `AAReasoningComposite` | GPQA .50, HLE .50 |
| `TauComposite` | Tau2 .75, Tau Banking .25 |
| `TerminalBench21Composite` | official Terminal-Bench 2.1, with AA as fallback |
| `BFCLComposite` | BFCL overall 1.00 |

Important v2 corrections:

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
- AIME 2025, MMLU-Pro, AA LiveCodeBench, Terminal-Bench 2.0, and SWE-bench
  Verified remain available as diagnostics but have no primary rank path.
- BrowseComp, HLE-with-tools, Toolathlon, OSWorld, and GDPval remain diagnostic
  until a neutral direct feed replaces the current reported-only coverage.
- Current sparse signals such as BFCL, GSO, HiL-Bench, Judgemark, and SWE
  Atlas remain active; lack of a row lowers confidence rather than ability.
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
| `GEN` | General/reasoning breadth from independent families |
| `PLAN` | Multi-step reasoning, tool use, escalation, and long-context planning |
| `BUILD` | Software implementation, terminal work, tool orchestration, and code quality |
| `REVIEW_DIRECT` | EQ-Bench Judgemark judge discrimination |
| `LM_ARENA_REVIEW_PROXY` | Search/document preference proxy |

The group metric weights are authoritative in `data/coefficients.toml`. Final
role paths are:

```text
Idea     = 0.65 CRE + 0.35 GEN
Planning = 0.65 PLAN + 0.35 GEN
Building = 0.95 BUILD + 0.05 PLAN
Review   = 0.20 REVIEW_DIRECT + 0.20 LM_ARENA_REVIEW_PROXY
         + 0.30 BUILD + 0.30 PLAN
```

These expressions define the nominal graph. The final calculation is made
from flattened leaves after the family cap, so the published result cannot be
reconstructed by simply averaging the displayed group numbers.

### 8.1 Balanced capability

The site and rank-change report expose a balanced capability view:

```text
Balanced = (Idea + Planning + Building + Review) / 4
```

It is an unweighted presentation view, not a separately trained score. It is
provisional if any of the four role proxies is provisional.

## 9. Ranked versus provisional

A role remains computable when evidence is sparse, but it is marked
`provisional` unless both conditions hold:

- at least 60% of its family-capped nominal weight is direct evidence; and
- direct evidence spans at least three independent families.

Otherwise its status is `ranked`. Cited same-model reports help produce a
discounted estimate. Sibling fills are prior-only; neither reports nor fills
count as direct families or can qualify a role by themselves.

## 10. Operational diagnostics

Pricing, blended cost, output speed, time to first token, latency, advertised
context window, and related operational fields are diagnostics only. They have
zero path to Idea, Planning, Building, Review, or Balanced capability.

Diagnostic `OPS_*` group values may still be emitted for inspection and
filtering. They must not be interpreted as part of a capability score. This
separation prevents deployment economics or provider routing from changing a
model-capability rank.

## 11. Uncertainty

Where a source publishes measurement uncertainty, schema 2.0 preserves it in
native raw fields:

- `TerminalBenchUncertainty`
- `TerminalBench21Uncertainty`
- `SWERebenchSEM`
- `EQBenchJudgemarkCILow`
- `EQBenchJudgemarkCIHigh`

These fields are observation-specific, unscored, and never transferred by
sibling synthesis. They support auditing and future interval-aware ranking;
v2 does not claim that all role scores already have statistical confidence
intervals.

## 12. Reproducibility and limitations

- `schema_version = "2.0.0"` identifies the evidence-rich output shape.
- `methodology = "v2"` identifies this scoring method.
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
