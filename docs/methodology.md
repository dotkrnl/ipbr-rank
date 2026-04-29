# Methodology

This document describes the complete mathematical pipeline for computing the four building-role scores (Idea, Planning, Building, Reviewing) from public LLM benchmarks. The pipeline has been audited and rebalanced multiple times since the original v1 spec; this doc reflects the current behavior of the Rust implementation in `crates/core`.

> **How these numbers were chosen.** Every coefficient, group composition,
> and penalty curve below was settled by Claude, Gemini, GPT, and Kimi
> debating each other across iterative code-review rounds. The human
> referee only adjudicated when the models deadlocked. The four models
> hold the repo's copyright, and yes — they helped score themselves; the
> peer-review structure is the only safeguard against that.

---

## 1. Overview

The scoring pipeline has seven stages:

1. **Ingestion**: Fetch rows from each source, match model names to canonical IDs via alias matching, optionally synthesize missing rows from sibling models (`data/synthesis_aliases.toml`).
2. **Normalization**: Transform each raw metric to a 0–100 scale using one of three transforms — percentile, tail-penalty, or as-score passthrough.
3. **Uncertainty penalties**: Values that came in via sibling synthesis are pulled toward the 50 baseline by 15 %. Values from manual overrides are pulled toward 50 by 10 %.
4. **Composite metrics**: Computed as missing-safe weighted averages of normalized inputs (currently only `SWEComposite`).
5. **Group aggregation**: Combine related metrics into groups (CRE, GEN, PLAN, BUILD, LM_ARENA_REVIEW_PROXY, OPS_*, A_*), with shrink-to-50 for sparse data and a smooth transition to trusting present metrics across 60-80% group coverage.
6. **Canary health penalty**: AISL canary drift is consumed as `AI_canary_health`, outside all groups. Healthy or missing canary data adds no points; degraded canary data subtracts up to 6 points from each raw role score.
7. **Final scoring**: Role scores are weighted averages of groups; the reviewer-reservation penalty produces the `_adj` variants of I/P/B.

---

## 2. Metric Registry

All metrics used in the scoring system are defined in `data/coefficients.toml` under the `[metrics.*]` section. Each metric specifies:
- **higher_better**: Direction (true for metrics where higher is better, false for inverse metrics like cost / latency / issue-density)
- **log_scale**: Whether to apply log transform before normalization (used for cost, speed, latency, context window)
- **groups**: Which group(s) this metric contributes to (descriptive — actual contribution is driven by `[group_weights.X]`)
- **transform**: One of `as_score`, `percentile`, or `tail_penalty` (default `as_score`)

See Appendix A for the complete metric table.

---

## 3. Normalization

### 3.1 Percentile-Based Robust Normalization (`transform = "percentile"`)

For each metric, we collect raw values across active models, compute the 5th and 95th percentiles, and map:
- Values at or below p5 → 0
- Values at or above p95 → 100
- Values between p5 and p95 → linearly interpolated

**Formula**:
```
norm(x) = clip(100 × (x - p5) / (p95 - p5), 0, 100)
```

**Log-scale metrics** apply `ln(x)` before percentile computation.

**Inverse direction (`higher_better = false`)** flips the result so larger raw values map to smaller normalized scores.

This is the default transform for nearly every metric — passthrough was retired in the audit because raw benchmark percentages aren't on a comparable scale across leaderboards. Synthesized sibling values and manual overrides are excluded from the normalization baseline when at least two direct measurements exist, so uncertainty fills cannot move the population cut points for directly measured models. If fewer than two direct measurements exist for a metric, the baseline falls back to all present values.

### 3.2 Tail-Penalty (`transform = "tail_penalty"`)

Used for operational metrics (OutputSpeed, TTFT, BlendedCost, ContextWindow). Linear/percentile normalization scaled every speed difference equally — meaning a 30 % slower model looked 30 % worse, even though users perceive operational speed in tiers. The new curve squeezes the top 80 % of the population into a 70-100 band (mild differentiation) and stretches the bottom 20 % across 0-70 (sharp penalty for extremely slow models). Net effect: fast and "fast enough" models look similar; only models that are genuinely sluggish stand out.

### 3.3 Canary Health Penalty

`AI_canary_health` is an AISL drift-detection signal, not a normal
capability benchmark. It has `groups = []` in `data/coefficients.toml`
and is consumed directly after role aggregation:

```text
canary_penalty = clamp((60 - AI_canary_health) / 40, 0, 1) × 6
role_raw = max(0, role_raw - canary_penalty)
```

A canary deadband of `60` means healthy or mildly degraded canaries
(`>= 60`) attract no penalty at all; below the deadband the penalty ramps
linearly to the full 6-point cap once health falls to `20`. Missing
canary data is also a no-op. Synthesized canary values are ignored for
this penalty, because sibling health should not stand in for a model's
own drift signal.

### 3.4 As-Score Passthrough (`transform = "as_score"`)

Default for metrics that come in already calibrated to 0-100. Currently only used as a no-op fallback; the active scoring portfolio percentile-normalizes everything for cross-leaderboard comparability.

### 3.5 Synthesis Penalty

Synthesis is **field-level and last-priority**. The synthesis layer
emits a donor row whenever a `(target, from)` pair appears in
`data/synthesis_aliases.toml` and a real donor row exists for the
source. The ingest layer then drops any synthesized field that the
target already has a real value for (`ingest_synthesized_row` in
`crates/core/src/ingest.rs`). So a model with partial real coverage
keeps its real values, and synthesis fills only the genuinely missing
fields.

After normalization, values that came in via the synthesis layer (i.e.,
`r.synthesized.contains_key(metric)`) are blended toward 50:

```
final = normalized × 0.85 + 50 × 0.15
```

This reflects genuine uncertainty about whether a sibling's score
transfers cleanly. Synthesized values still count, just slightly more
conservatively than direct measurements.

### 3.6 Manual Override Penalty

Manual overrides from `data/score_overrides.toml` are public, cited
measurements used to fill gaps before a source lands on the ingested
leaderboard. Because they are still hand-curated cells, they are softer
than directly ingested leaderboard rows. After normalization, override-
reported values are blended toward 50 by 10 %:

```
final = normalized × 0.90 + 50 × 0.10
```

If a public source later reports the same metric, the public row overwrites
the override during ingestion and the override penalty is removed.

---

## 4. Group Aggregation with Missing-Data Shrinkage

Metrics are grouped by domain. Each group is a weighted average of its member metrics.

### 4.1 Group Definitions

| Group Key | Member Metrics (with weights from `[group_weights.*]`) |
|-----------|-------------------------------------------------------|
| **CRE** (Creativity) | LMArenaCreativeOrOpenEnded (0.65), LMArenaText (0.35) |
| **GEN** (General Intelligence) | ArtificialAnalysisIntelligence (0.42), LMArenaText (0.25), GPQA_HLE_Reasoning (0.18), ARC_AGI_2 (0.15) |
| **PLAN** (Planning) | TerminalBench (0.34), ArtificialAnalysisReasoning (0.20), Tau2Bench (0.20), IFBench (0.12), LongContextRecall (0.08), MCPAtlas (0.06) |
| **BUILD** (Building) | SWEComposite (0.40), MCPAtlas (0.12), TerminalBench (0.09), LiveCodeBench (0.05), ArtificialAnalysisCoding (0.05), SciCode (0.05), GDPval (0.05), SonarFunctionalSkill (0.04), SonarIssueDensity (0.025), SonarBugDensity (0.02), SonarVulnerabilityDensity (0.015), LongContextRecall (0.05), CopilotArenaOrLMArenaCode (0.04) |
| **LM_ARENA_REVIEW_PROXY** (Reviewing proxy) | LMArenaSearchDocument (1.00) |
| **OPS_long** (Ops for long generation) | OutputSpeed (0.55), TTFT (0.20), BlendedCost (0.10), ContextWindow (0.15) |
| **OPS_precision** (Ops for precise tasks) | OutputSpeed (0.35), TTFT (0.35), BlendedCost (0.15), ContextWindow (0.15) |
| **OPS_review** (Ops for reviewing) | OutputSpeed (0.30), TTFT (0.40), BlendedCost (0.20), ContextWindow (0.10) |
| **A_I** (AIStupid Idea) | AI_correctness (0.18), AI_spec (0.18), AI_efficiency (0.08), AI_stability (0.16), AI_recovery (0.12), AI_complexity (0.10), AI_edge_cases (0.08), AI_plan_coherence (0.10) — `AI_refusal` and `AI_code` removed (safety/code-quality signals that don't measure idea quality) |
| **A_P** (AIStupid Planning) | AI_correctness, AI_spec, AI_efficiency, AI_stability, AI_recovery, AI_plan_coherence, AI_memory_retention, AI_context_awareness, AI_task_completion, AI_tool_selection, AI_parameter_accuracy |
| **A_B** (AIStupid Building) | AI_correctness, AI_spec, AI_code, AI_efficiency, AI_stability, AI_recovery, AI_complexity, AI_edge_cases, AI_hallucination_resistance, AI_memory_retention |
| **A_R** (AIStupid Reviewing) | AI_correctness, AI_spec, AI_code, AI_stability, AI_recovery, AI_hallucination_resistance, AI_edge_cases (`AI_error_handling` was dropped — see `docs/sources.md` AISL entry for the upstream measurement quirk that motivated it; the freed 0.08 weight folded into `AI_recovery`) |

`SWEComposite` is a derived metric defined in `[composite_metrics.SWEComposite]`,
computed as a missing-safe weighted average of `SWERebench` (0.30),
`SWEBenchVerified` (0.25), `SWEBenchPro` (0.35), and `SWEBenchMultilingual`
(0.10). All four inputs use percentile normalization so they're on a
comparable scale before the composite collapses them. See the source-level
scoreboard for the raw input values when diagnosing per-model performance.

### 4.2 Shrink-to-50 with Trust Threshold

When a model is missing some metrics in a group, the aggregator either
trusts the present-weighted mean or pulls it toward 50, depending on how
much weight is actually present:

```
present_metrics = { m : metric m is present for this model }
present_weight = sum(weight[m] for m in present_metrics)
total_weight = sum(weight[m] for m in all_group_metrics)

weighted_avg = sum(normalized[m] × weight[m] for m in present_metrics) / present_weight
w_present = present_weight / total_weight

shrink_value = weighted_avg × w_present + 50 × (1 - w_present)

if w_present <= 0.60:
    group_score = shrink_value
elif w_present >= 0.80:
    group_score = weighted_avg
else:
    group_score = smoothstep_blend(shrink_value, weighted_avg)
```

**Why the threshold.** Without it, models with mostly-complete coverage
got penalized for not appearing on every peripheral leaderboard — a
flagship missing one or two ~0.10-weight metrics would still drift
toward 50 even though every direct measurement said top-of-population.
The transition uses a smooth step across a 0.60–0.80 band instead of a
hard cliff at 0.70. This prevents a tiny change in coverage (e.g. a new
source adding one small metric) from causing a discontinuous ~15‑point
jump in the group score. Well-covered models (≥80 %) trust the present
mean directly; models below 60 % get the full proportional shrink;
between those points the score blends smoothly.

**Invariant**: If all metrics are missing, `present_weight = 0`, and
`group_score = 50`.

**Shrunk groups**: A group is marked "shrunk" in the output if
`present_weight / total_weight` is below the top of the configured
transition band: `trust_threshold + trust_transition_width / 2`. With the
default coefficients, that cutoff is `0.80`.

---

## 5. Final Role Scores

Each of the four roles (I_raw, P_raw, B_raw, R) is a weighted average of groups.

### 5.1 Role Score Definitions

From `[final_score_weights.*]` in `data/coefficients.toml`:

All four role formulas put **A_\* at 0.30** — AISL's role-shaped
perspectives remain a major behavioral signal, while the role-specific
public-leaderboard groups collectively carry more weight. OPS_* stays at
0.08.

**I_raw** (Idea):
```
I_raw = 0.43×CRE + 0.19×GEN + 0.30×A_I + 0.08×OPS_long
```

**P_raw** (Planning):
```
P_raw = 0.37×PLAN + 0.25×GEN + 0.30×A_P + 0.08×OPS_precision
```

PLAN's basket of TerminalBench / Tau2Bench / AAReasoning / MCPAtlas can
favor any of the top-3 vendors depending on which gets a strong value
in each — A_P captures planning behavior more directly than the
leaderboard mix.

**B_raw** (Building):
```
B_raw = 0.57×BUILD + 0.05×PLAN + 0.30×A_B + 0.08×OPS_precision
```

**R** (Reviewing):
```
R = 0.12×LM_ARENA_REVIEW_PROXY + 0.25×BUILD + 0.25×PLAN + 0.30×A_R + 0.08×OPS_review
```

A_R remains the largest single review-specific behavioral signal.
LM_ARENA_REVIEW_PROXY (LMArena search/document preference) sits at 0.12:
useful review-adjacent evidence, but intentionally not treated as a direct
code-review benchmark. BUILD 0.25 keeps reviewing tied to "you can read the
code." PLAN 0.25 captures review-as-planning.

**Operational metrics (OPS_long / OPS_precision / OPS_review)** carry
weight 0.08 in the role formulas, paired with the tail-penalty
normalization on each underlying metric (top 80 % of the population →
70..100, bottom 20 % → 0..70). The combination expresses two distinct
behaviors at the same time: among "fast enough" models the OPS group
score sits in a ~30-point band, so weight 0.08 produces only a 1-2
point spread in the role score (the small-penalty regime); on the
slowest tail the OPS group score collapses below 50 and the same 0.08
weight delivers a 4-6 point penalty (the "great penalty" regime).
Inspect the `OPS_*` groups directly for a pure speed/cost view of the
population.

**Verification**: For each role, the weights sum to 1.0 (within floating-point epsilon).

---

## 6. Reviewer-Reservation Penalty

The raw I/P/B scores do not account for vendor bias in preference evaluations (e.g., a vendor's preference model may favor its own LLMs). The reviewer-reservation penalty corrects for this.

### 6.1 Penalty Derivation

For each vendor **v**:
1. Compute `R_all = max(R across all models)` — the best reviewer score globally.
2. Compute `R_outside_v = max(R across models not from vendor v)` — the best reviewer score excluding vendor v's models.
3. Define the **reservation gap**: `L_v = max(0, R_all - R_outside_v)`.

If vendor v does not have the unique best reviewer, its per-model penalty share is zero. If vendor v has the unique best reviewer, `L_v` measures how much better that reviewer is than the best non-v reviewer.

### 6.2 Penalty Coefficients

From `[reviewer_reservation]` in `data/coefficients.toml`:
- I_adj coefficient: 0.08
- P_adj coefficient: 0.18
- B_adj coefficient: 0.32

### 6.3 Per-Model Penalty Share

The vendor-level gap `L_v` is the *available* reservation budget. It is
applied to each individual model in vendor **v** in proportion to that
model's own contribution to the lead:

```
share_m = clamp((R_m - R_outside_v) / L_v, 0, 1)   # 1.0 for the actual top-R model
penalty_m = L_v × share_m
```

So the actual top reviewer in vendor **v** pays the full reservation,
sibling models with R at or below `R_outside_v` pay nothing, and models
in between pay proportionally. This stops the reservation tax from
hitting every model that happens to share a vendor with a strong
reviewer, while preserving the original semantics for the model that
actually drives the lead.

### 6.4 Adjusted Scores

For each model **m** from vendor **v**:
```
I_adj = I_raw - 0.08 × penalty_m
P_adj = P_raw - 0.18 × penalty_m
B_adj = B_raw - 0.32 × penalty_m
```

The reviewing score **R** is not adjusted (it is used to compute the penalty).

**Rationale**: Building tasks are most sensitive to reviewer bias (highest coefficient 0.32), followed by planning (0.18), then idea (0.08).

---

## 7. Alias Matching

Model names vary across sources. The alias matcher normalizes names and fuzzy-matches against canonical IDs loaded from `data/required_aliases.toml`.

### 7.1 Normalization Steps

1. HTML-unescape, lowercase, strip whitespace.
2. Replace vendor-colon prefixes (`openai:`, `anthropic:`, etc.) with vendor-space.
3. Replace `_` and `/` with space.
4. Preserve dots between digits (e.g., `4.7`), remove all other non-alphanumeric characters.
5. Collapse whitespace.
6. Apply organization aliases: `moonshot ai` → `moonshot`, `z ai` → `zai`.

### 7.2 Compact Key

`compact_key(s)` = normalized name with all non-alphanumeric removed (no spaces, no dots). Used for fuzzy matching.

### 7.3 Matching Pipeline

1. **Exact lookup**: Try `normalize_name(input)`, `compact_key(input)`, and vendor-prefixed variants against the alias index.
2. **Fuzzy fallback**: For each candidate, compute substring match score. Add +20 vendor bonus if the vendor matches. Accept best match if score ≥ `max(12, len(input_ck) / 2)`.
3. **Unmatched rows**: Logged as warnings, discarded.

**Collision handling**: The alias index is built in canonical-ID iteration order. A later record cannot steal an alias already claimed.

---

## 8. Thinking Effort

Models with vendor-exposed reasoning levels (OpenAI `reasoning_effort`, Anthropic extended thinking budgets, Gemini "thinking") get separate canonical IDs with a `+thinking-{low|medium|high}` suffix when a source provides distinguishable per-effort scores.

By default, we **only split** when at least one source provides measurable differentiation (e.g., LMArena's `claude-opus-4.7-thinking` vs `claude-opus-4.7`).

---

## 9. Determinism

With `--offline --cache <fixtures> --now <timestamp>`:
- All source responses are read from fixtures (no network variance).
- All timestamps use the overridden value.
- All maps are sorted by key.
- Floats use fixed formatting.

This guarantees byte-for-byte deterministic output for testing.

---

## 10. Coefficient Overrides

The CLI accepts `--coefficients path/to/file.toml` to override the embedded coefficients. The *effective* coefficients (after overrides) are echoed to `out/coefficients.toml` so the scoreboard is self-describing.

---

## Appendix A: Complete Metric Table

| Metric Key | Direction | Log-scale | Transform | Primary Source(s) | Groups |
|------------|-----------|-----------|-----------|-------------------|--------|
| LMArenaText | higher | no | percentile | LMArena | CRE, GEN |
| LMArenaCreativeOrOpenEnded | higher | no | percentile | LMArena | CRE |
| CopilotArenaOrLMArenaCode | higher | no | percentile | LMArena | BUILD |
| LMArenaSearchDocument | higher | no | percentile | LMArena search/document | LM_ARENA_REVIEW_PROXY |
| ArtificialAnalysisIntelligence | higher | no | percentile | Artificial Analysis | GEN |
| ArtificialAnalysisCoding | higher | no | percentile | Artificial Analysis | BUILD |
| ArtificialAnalysisReasoning | higher | no | percentile | Artificial Analysis (gpqa+hle blend) | PLAN |
| LiveCodeBench | higher | no | percentile | LiveCodeBench JSON | BUILD |
| GPQA_HLE_Reasoning | higher | no | percentile | Artificial Analysis (gpqa+hle blend) | GEN |
| SWEBenchVerified | higher | no | percentile | SWE-bench JSON | (input to SWEComposite) |
| SWEBenchMultilingual | higher | no | percentile | SWE-bench JSON | (input to SWEComposite) |
| SWERebench | higher | no | percentile | SWE-rebench HTML | (input to SWEComposite) |
| SWEBenchPro | higher | no | percentile | Scale Labs (RSC HTML) | (input to SWEComposite) |
| MCPAtlas | higher | no | percentile | Scale Labs (RSC HTML) | PLAN, BUILD |
| ARC_AGI_2 | higher | no | percentile | ARC Prize (static JSON, v2 semi-private) | GEN |
| TerminalBench | higher | no | percentile | Terminal-Bench HTML | PLAN, BUILD |
| Tau2Bench | higher | no | percentile | Artificial Analysis (tau2 field) | PLAN |
| SciCode | higher | no | percentile | Artificial Analysis (scicode field) | BUILD |
| IFBench | higher | no | percentile | Artificial Analysis (ifbench field) | PLAN |
| GDPval | higher | no | percentile | overrides table (GDPval-AA Elo) | BUILD |
| LongContextRecall | higher | no | percentile | Artificial Analysis (lcr field) | BUILD, PLAN |
| SonarFunctionalSkill | higher | no | percentile | Sonar code-quality JSON | BUILD |
| SonarIssueDensity | **lower** | no | percentile | Sonar code-quality JSON | BUILD |
| SonarBugDensity | **lower** | no | percentile | Sonar code-quality JSON | BUILD |
| SonarVulnerabilityDensity | **lower** | no | percentile | Sonar code-quality JSON | BUILD |
| OutputSpeed | higher | **yes** | tail_penalty | Artificial Analysis | OPS_* |
| TTFT | **lower** | **yes** | tail_penalty | Artificial Analysis | OPS_* |
| BlendedCost | **lower** | **yes** | tail_penalty | Artificial Analysis / OpenRouter | OPS_* |
| ContextWindow | higher | **yes** | tail_penalty | OpenRouter | OPS_* |
| AI_correctness | higher | no | percentile | AIStupidLevel (hourly suite) | A_I, A_P, A_B, A_R |
| AI_spec | higher | no | percentile | AIStupidLevel (hourly suite, `format` axis) | A_I, A_P, A_B, A_R |
| AI_code | higher | no | percentile | AIStupidLevel (`codeQuality` axis) | A_B, A_R (removed from A_I — code quality ≠ idea quality) |
| AI_efficiency | higher | no | percentile | AIStupidLevel | A_I, A_P, A_B, A_R |
| AI_stability | higher | no | percentile | AIStupidLevel | A_I, A_P, A_B, A_R |
| AI_refusal | higher | no | percentile | AIStupidLevel (`safety` axis) | none — retained as an ingested metric, excluded from role perspectives |
| AI_recovery | higher | no | percentile | AIStupidLevel (`debugging` axis) | A_I, A_P, A_B, A_R |
| AI_complexity | higher | no | percentile | AIStupidLevel (hourly+deep) | A_I, A_B |
| AI_edge_cases | higher | no | percentile | AIStupidLevel (hourly+deep) | A_I, A_B, A_R |
| AI_plan_coherence | higher | no | percentile | AIStupidLevel (deep suite) | A_I, A_P |
| AI_memory_retention | higher | no | percentile | AIStupidLevel (deep suite) | A_P, A_B |
| AI_hallucination_resistance | higher | no | percentile | AIStupidLevel (deep suite, passthrough — upstream already returns `1 - rate`) | A_B, A_R |
| AI_context_awareness | higher | no | percentile | AIStupidLevel (tooling suite) | A_P |
| AI_canary_health | higher | no | as_score | AIStupidLevel canary/drift incidents | penalty-only |
| AI_task_completion | higher | no | percentile | AIStupidLevel (tooling suite) | A_P |
| AI_tool_selection | higher | no | percentile | AIStupidLevel (tooling suite) | A_P |
| AI_parameter_accuracy | higher | no | percentile | AIStupidLevel (tooling suite) | A_P |
`AI_error_handling` was previously emitted from AISL's tooling suite but is
now dropped from both the metric registry and `A_R` weights. Upstream
defines it as `recoveredFromErrors / failedCalls.length`, with the
`failedCalls = 0` branch returning 0 instead of 1 — so a model that never
fails gets the same score as one that fails everything and recovers
nothing. The freed weight in A_R folded into AI_recovery.

`AI_safety_compliance` was also dropped: it was fetched but never
referenced in any weight table, so it contributed nothing to scores.
Removing it keeps the coefficient surface honest.

---

## Appendix B: Coefficient Summary Table

All coefficients are verbatim from `data/coefficients.toml`. This table is for quick reference; the TOML file is authoritative.

### Final Score Weights
All four roles weight the AISL perspective at 0.30. Role-specific public
benchmark groups sum to 0.62, and OPS_* contributes 0.08 (paired with the
tail-penalty curve so only genuinely slow models lose meaningful score).

| Role | Group Contributions |
|------|---------------------|
| I_raw | CRE 0.43, GEN 0.19, A_I 0.30, OPS_long 0.08 |
| P_raw | PLAN 0.37, GEN 0.25, A_P 0.30, OPS_precision 0.08 |
| B_raw | BUILD 0.57, PLAN 0.05, A_B 0.30, OPS_precision 0.08 |
| R | LM_ARENA_REVIEW_PROXY 0.12, BUILD 0.25, PLAN 0.25, A_R 0.30, OPS_review 0.08 |

### Reviewer-Reservation Penalties
| Role | Coefficient |
|------|-------------|
| I_adj | 0.08 |
| P_adj | 0.18 |
| B_adj | 0.32 |

### Synthesis Penalty
| Constant | Value |
|----------|-------|
| `SYNTHESIS_PENALTY` (in `crates/core/src/score.rs`) | 0.15 |
| `OVERRIDE_REPORTED_PENALTY` (in `crates/core/src/score.rs`) | 0.10 |

When a metric value comes in via the synthesis layer, its normalized score
is blended toward 50: `final = score × 0.85 + 50 × 0.15`. Synthesized
values still contribute, just slightly more conservatively than direct
measurements.

AISL perspective groups (`A_I`, `A_P`, `A_B`, `A_R`) are aggregated as
plain missing-safe weighted averages of the AISL capability metrics — no
extra group-level synthesis discount. The metric-level synthesis pull
(`final = normalized × 0.85 + 50 × 0.15` in 3.5) already discounts each
synthesised AISL axis before it enters the perspective average; stacking
a second pull on top double-discounted AISL synthesis relative to every
other source family without any documented justification, so the group-
level pull was removed.

### AI Stupid Level Perspective Weights
See `[ai_stupid_perspective_weights.*]` in `data/coefficients.toml` for the
full breakdown of how the 17 AISL axes (the 9 hourly-suite axes plus
`plan_coherence`, `memory_retention`, `hallucination_resistance` from the
deep suite, and `context_awareness`, `task_completion`, `tool_selection`,
`parameter_accuracy` from the tooling suite) are
weighted into A_I / A_P / A_B / A_R.

`AI_canary_health` is intentionally excluded from these perspective weights.
It is a fast degradation signal, so it can only subtract the canary health
penalty described above; it cannot raise a model above what the full
capability suites support.

A_R was tuned in 2026-04 to lean into review-specific axes:
`hallucination_resistance` 0.17, `edge_cases` 0.13, `recovery` 0.18 (which
absorbed the dropped `error_handling` weight), `correctness` 0.18,
`spec` 0.14, `code` 0.10, `stability` 0.10. The shift away from
correctness/spec saturation reduced AISL's tendency to flatten reviewer
rankings across already-saturated frontier models.
