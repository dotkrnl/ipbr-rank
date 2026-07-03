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

The scoring pipeline has six stages:

1. **Ingestion**: Fetch rows from each source, match model names to canonical IDs via alias matching, optionally synthesize missing rows from sibling models (`data/synthesis_aliases.toml`).
2. **Normalization**: Transform each raw metric to a 0–100 scale using one of three transforms — percentile, tail-penalty, or as-score passthrough.
3. **Uncertainty penalties**: Conservative values that came in via sibling synthesis are pulled toward the 50 baseline by 15 %. Same-series forward synthesis carries no penalty. Manual overrides are cited reported values and receive no additional pull after normalization.
4. **Composite metrics**: Computed as missing-safe weighted averages of normalized inputs (`SWEComposite` and `SonarComposite`).
5. **Group aggregation**: Combine related metrics into groups (CRE, GEN, PLAN, BUILD, LM_ARENA_REVIEW_PROXY, OPS_*), with shrink-to-50 for sparse data and a smooth transition to trusting present metrics across 60-80% group coverage.
6. **Final scoring**: Role scores are weighted averages of groups. AISL was removed from active scoring after local reproduction showed the benchmark surface was not representative enough of real model quality and was too noise-prone.

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

Used for scored operational metrics (OutputSpeed, TTFT, ContextWindow). Linear/percentile normalization scaled every speed difference equally — meaning a 30 % slower model looked 30 % worse, even though users perceive operational speed in tiers. The new curve squeezes the top 80 % of the population into a 70-100 band (mild differentiation) and stretches the bottom 20 % across 0-70 (sharp penalty for extremely slow models). Net effect: fast and "fast enough" models look similar; only models that are genuinely sluggish stand out. `BlendedCost` is still emitted for inspection, but it is no longer used in role scoring.

### 3.3 As-Score Passthrough (`transform = "as_score"`)

Default for metrics that come in already calibrated to 0-100. Currently only used as a no-op fallback; the active scoring portfolio percentile-normalizes everything for cross-leaderboard comparability.

### 3.4 Synthesis Penalty

Synthesis is **field-level and last-priority**. The synthesis layer
emits a donor row whenever a `(target, from)` pair appears in
`data/synthesis_aliases.toml` and a real donor row exists for the
source. The ingest layer then drops any synthesized field that the
target already has a real value for (`ingest_synthesized_row` in
`crates/core/src/ingest.rs`). So a model with partial real coverage
keeps its real values, and synthesis fills only the genuinely missing
fields.

After normalization, conservative values that came in via the synthesis
layer (i.e., `r.synthesized.contains_key(metric)`) are blended toward 50:

```
final = normalized × 0.85 + 50 × 0.15
```

This reflects genuine uncertainty about whether a sibling's score transfers
cleanly. `data/synthesis_aliases.toml` can also mark a pair as
`category = "same_series_forward"` when the donor is the same vendor and
same product line, and the target is a newer version. Those version-advance
fills carry no synthesis pull after normalization. A pair can also use
`category = "stronger_successor"` when the target is documented as stronger
than the donor but is missing from an older public leaderboard; those fills
also carry no synthesis pull. Cross-vendor, cross-series, weaker, uncertain,
and older-target-from-newer-donor fills remain `category = "conservative"` and
keep the 15% penalty. Conservative provenance is sticky through chained
synthesis, so a conservative donor does not become zero-penalty just because a
later hop is same-series-forward or stronger-successor.

### 3.5 Manual Overrides

Manual overrides from `data/score_overrides.toml` are public, cited
measurements used to fill gaps before a source lands on the ingested
leaderboard. They are excluded from the direct-source normalization baseline
when at least two direct measurements exist, so reported outliers cannot move
the percentile cut points for directly measured models. After normalization,
they are not pulled toward 50:

```
final = normalized
```

If a public source later reports the same metric, the public row overwrites
the override during ingestion.

---

## 4. Group Aggregation with Missing-Data Shrinkage

Metrics are grouped by domain. Each group is a weighted average of its member metrics.

### 4.1 Group Definitions

| Group Key | Member Metrics (with weights from `[group_weights.*]`) |
|-----------|-------------------------------------------------------|
| **CRE** (Creativity) | LMArenaCreativeOrOpenEnded (0.50), LMArenaText (0.30), ARC_AGI_2 (0.20) |
| **GEN** (General Intelligence) | ArtificialAnalysisIntelligence (0.34), LMArenaText (0.15), GPQA_HLE_Reasoning (0.14), ARC_AGI_2 (0.10), ArtificialAnalysisMath (0.08), AIME25 (0.06), MMLUPro (0.06), BrowseComp (0.04), HLETools (0.03) |
| **PLAN** (Planning) | TerminalBench (0.120), TerminalBench21Composite (0.035), TerminalBenchHard (0.060), BFCLComposite (0.060), HiLBench (0.040), Toolathlon (0.035), OSWorldVerified (0.030), HLETools (0.025), BrowseComp (0.020), TauComposite (0.145), ArtificialAnalysisReasoning (0.160), IFBench (0.100), LongContextRecall (0.075), MCPAtlas (0.095) |
| **BUILD** (Building) | SWEComposite (0.290), SWEAtlasComposite (0.140), LiveCodingComposite (0.140), MCPAtlas (0.060), TerminalBench (0.045), TerminalBench21Composite (0.025), TerminalBenchHard (0.040), BFCLComposite (0.025), HiLBench (0.015), Toolathlon (0.015), OSWorldVerified (0.010), GSO (0.035), GDPval (0.045), SonarComposite (0.090), LongContextRecall (0.025) |
| **LM_ARENA_REVIEW_PROXY** (Reviewing proxy) | LMArenaSearch (0.50), LMArenaDocument (0.50) |
| **OPS_long** (Ops for long generation) | OutputSpeed (0.61), TTFT (0.22), ContextWindow (0.17) |
| **OPS_precision** (Ops for precise tasks) | OutputSpeed (0.375), TTFT (0.4375), ContextWindow (0.1875) |
| **OPS_review** (Ops for reviewing) | OutputSpeed (0.375), TTFT (0.3125), ContextWindow (0.3125) |

AISL's former `A_*` perspective groups were removed from active scoring on
2026-05-21. We reproduced the benchmark locally and found the tasks not
representative enough of real model quality and too noise-prone for this
scoreboard.

`SWEComposite` is a derived metric defined in `[composite_metrics.SWEComposite]`,
computed as a missing-safe weighted average of `SWERebench` (0.45),
`SWEBenchVerified` (0.10), `SWEBenchPro` (0.35), and `SWEBenchMultilingual`
(0.10). Verified was reduced because it saturates near the top of the frontier,
while Rebench — a rolling-window benchmark — was increased for better
contamination resistance and top-end differentiation. All four inputs use
percentile normalization so they're on a comparable scale before the composite
collapses them. See the source-level scoreboard for the raw input values when
diagnosing per-model performance.

`SWEAtlasComposite` similarly collapses Scale's SWE Atlas Q&A, test-writing,
and refactoring tracks into one BUILD signal with weights 0.30 / 0.30 / 0.40.

`SonarComposite` is the same pattern applied to the four Sonar code-quality
submetrics (functional pass rate plus issue / bug / vulnerability density).
Defined in `[composite_metrics.SonarComposite]` as a missing-safe weighted
average of `SonarFunctionalSkill` (0.40), `SonarIssueDensity` (0.25),
`SonarBugDensity` (0.20), and `SonarVulnerabilityDensity` (0.15) — weights
proportional to their previous standalone BUILD weights. Collapsing them
into one signal stops a single Sonar payload from registering as four
independent missing entries when a model isn't on Sonar's leaderboard, and
prevents the three highly-correlated density metrics from triple-counting
the same "buggy code" signal.

`LiveCodingComposite` collapses four live coding/reasoning signals into one
BUILD input so they don't pile up as four small independent weights.
Defined in `[composite_metrics.LiveCodingComposite]` as a missing-safe
weighted average of `ArtificialAnalysisCoding` (0.286), `SciCode` (0.286),
`AALiveCodeBench` (0.286), and `CopilotArenaOrLMArenaCode` (0.142) — weights
proportional to their previous standalone BUILD weights.

`TauComposite` combines AA's broader `Tau2Bench` field (0.75) with the newer
`TauBanking` field (0.25). `TerminalBench21Composite` combines the official
Terminal-Bench 2.1 source (0.55) with AA's `AATerminalBench21` field (0.45).
`BFCLComposite` keeps Berkeley's overall BFCL score as the anchor (0.35) and
adds the V4 category splits for non-live AST (0.10), live (0.15), multi-turn
(0.15), web search (0.08), memory (0.07), relevance detection (0.06), and
irrelevance detection (0.04).

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

if w_present <= 0.70:
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
The transition uses a smooth step across a 0.70–0.80 band instead of a
hard cliff. This prevents a tiny change in coverage (e.g. a new source
adding one small metric) from causing a discontinuous jump in the group
score, while requiring fuller evidence before the missing-weight shrink
is removed. Well-covered models (≥80 %) trust the present mean directly;
models below 70 % get the full proportional shrink; between those points
the score blends smoothly.

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

AISL's former 0.15 role slot is redistributed into the remaining
non-operational public benchmark groups for each role. OPS_* stays at
0.08 for Planning, Building, and Reviewing; Idea generation is less
operationally sensitive, so `I_raw` reduces the operational weight to
0.05 and increases GEN accordingly.

**I_raw** (Idea):
```
I_raw = 0.62×CRE + 0.33×GEN + 0.05×OPS_long
```

**P_raw** (Planning):
```
P_raw = 0.55×PLAN + 0.37×GEN + 0.08×OPS_precision
```

PLAN's basket of TerminalBench / TauComposite / AAReasoning / MCPAtlas can
favor any of the top-3 vendors depending on which gets a strong value
in each.

**B_raw** (Building):
```
B_raw = 0.84×BUILD + 0.08×PLAN + 0.08×OPS_precision
```

**R** (Reviewing):
```
R = 0.25×LM_ARENA_REVIEW_PROXY + 0.29×BUILD + 0.38×PLAN + 0.08×OPS_review
```

LM_ARENA_REVIEW_PROXY (LMArena search/document preference) sits at 0.25:
useful review-adjacent evidence, but intentionally not treated as a direct
code-review benchmark. Search and document are normalized as separate metrics
before the proxy combines them, because their raw LMArena Elo scales differ.
BUILD 0.29 keeps reviewing tied to "you can read the code." PLAN 0.38 captures
review-as-planning.

**Operational metrics (OPS_long / OPS_precision / OPS_review)** carry
weight 0.08 in the role formulas, paired with the tail-penalty
normalization on each underlying metric (top 80 % of the population →
70..100, bottom 20 % → 0..70). The combination expresses two distinct
behaviors at the same time: among "fast enough" models the OPS group
score sits in a ~30-point band, so weight 0.08 produces only a 1-2
point spread in the role score (the small-penalty regime); on the
slowest tail the OPS group score collapses below 50 and the same 0.08
weight delivers a 4-6 point penalty (the "great penalty" regime).
Inspect the `OPS_*` groups directly for a pure speed/context view of the
population. `BlendedCost` remains visible as a metric but no longer
contributes to these groups or the final role scores.

**Verification**: For each role, the weights sum to 1.0 (within floating-point epsilon).

The rendered `scoreboard.toml` also emits `i_adj`, `p_adj`, and `b_adj` fields under `[models.scores]`. These are TOML-only aliases of `i_raw`, `p_raw`, and `b_raw` retained so existing API consumers keep parsing; the previous reviewer-reservation adjustment was removed and these fields are now always equal to their raw counterparts.

---

## 6. Alias Matching

Model names vary across sources. The alias matcher normalizes names and fuzzy-matches against canonical IDs loaded from `data/required_aliases.toml`.

### 6.1 Normalization Steps

1. HTML-unescape, lowercase, strip whitespace.
2. Replace vendor-colon prefixes (`openai:`, `anthropic:`, etc.) with vendor-space.
3. Replace `_` and `/` with space.
4. Preserve dots between digits (e.g., `4.7`), remove all other non-alphanumeric characters.
5. Collapse whitespace.
6. Apply organization aliases: `moonshot ai` → `moonshot`, `z ai` → `zai`.

### 6.2 Compact Key

`compact_key(s)` = normalized name with all non-alphanumeric removed (no spaces, no dots). Used for fuzzy matching.

### 6.3 Matching Pipeline

1. **Exact lookup**: Try `normalize_name(input)`, `compact_key(input)`, and vendor-prefixed variants against the alias index.
2. **Fuzzy fallback**: For each candidate, compute substring match score. Add +20 vendor bonus if the vendor matches. Accept best match if score ≥ `max(12, len(input_ck) / 2)`.
3. **Unmatched rows**: Logged as warnings, discarded.

**Collision handling**: The alias index is built in canonical-ID iteration order. A later record cannot steal an alias already claimed.

---

## 7. Thinking Effort

Models with vendor-exposed reasoning levels (OpenAI `reasoning_effort`, Anthropic extended thinking budgets, Gemini "thinking") get separate canonical IDs with a `+thinking-{low|medium|high}` suffix when a source provides distinguishable per-effort scores.

By default, we **only split** when at least one source provides measurable differentiation (e.g., LMArena's `claude-opus-4.7-thinking` vs `claude-opus-4.7`).

---

## 8. Determinism

With `--offline --cache <fixtures> --now <timestamp>`:
- All source responses are read from fixtures (no network variance).
- All timestamps use the overridden value.
- All maps are sorted by key.
- Floats use fixed formatting.

This guarantees byte-for-byte deterministic output for testing.

---

## 9. Coefficient Overrides

The CLI accepts `--coefficients path/to/file.toml` to override the embedded coefficients. The *effective* coefficients (after overrides) are echoed to `out/coefficients.toml` so the scoreboard is self-describing.

---

## Appendix A: Complete Metric Table

| Metric Key | Direction | Log-scale | Transform | Primary Source(s) | Groups |
|------------|-----------|-----------|-----------|-------------------|--------|
| LMArenaText | higher | no | percentile | LMArena | CRE, GEN |
| LMArenaCreativeOrOpenEnded | higher | no | percentile | LMArena | CRE |
| CopilotArenaOrLMArenaCode | higher | no | percentile | LMArena | (input to LiveCodingComposite) |
| LMArenaSearch | higher | no | percentile | LMArena search | LM_ARENA_REVIEW_PROXY |
| LMArenaDocument | higher | no | percentile | LMArena document | LM_ARENA_REVIEW_PROXY |
| ArtificialAnalysisIntelligence | higher | no | percentile | Artificial Analysis | GEN |
| ArtificialAnalysisCoding | higher | no | percentile | Artificial Analysis | (input to LiveCodingComposite) |
| ArtificialAnalysisReasoning | higher | no | percentile | Artificial Analysis (gpqa+hle blend) | PLAN |
| LiveCodeBench | higher | no | percentile | LiveCodeBench JSON | (retired — see GSO) |
| GSO | higher | no | percentile | gso-bench.github.io leaderboard JSON (`score_hack_control` field, `setting=Opt@1`) | BUILD |
| GPQA_HLE_Reasoning | higher | no | percentile | Artificial Analysis (gpqa+hle blend) | GEN |
| SWEBenchVerified | higher | no | percentile | SWE-bench JSON | (input to SWEComposite) |
| SWEBenchMultilingual | higher | no | percentile | SWE-bench JSON | (input to SWEComposite) |
| SWERebench | higher | no | percentile | SWE-rebench HTML | (input to SWEComposite) |
| SWEBenchPro | higher | no | percentile | Scale Labs (RSC HTML) | (input to SWEComposite) |
| SWEAtlasQnA | higher | no | percentile | Scale SWE Atlas Q&A | (input to SWEAtlasComposite) |
| SWEAtlasTestWriting | higher | no | percentile | Scale SWE Atlas Test Writing | (input to SWEAtlasComposite) |
| SWEAtlasRefactoring | higher | no | percentile | Scale SWE Atlas Refactoring | (input to SWEAtlasComposite) |
| MCPAtlas | higher | no | percentile | Scale Labs (RSC HTML) | PLAN, BUILD |
| KimiCodeBenchV2 | higher | no | percentile | overrides table (Moonshot Kimi K2.7 model card) | (reported only) |
| ProgramBench | higher | no | percentile | overrides table (Moonshot Kimi K2.7 model card) | (reported only) |
| MLSBenchLite | higher | no | percentile | overrides table (Moonshot Kimi K2.7 model card) | (reported only) |
| KimiClaw247Bench | higher | no | percentile | overrides table (Moonshot Kimi K2.7 model card) | (reported only) |
| MCPMarkVerified | higher | no | percentile | overrides table (Moonshot Kimi K2.7 model card) | (reported only) |
| ARC_AGI_2 | higher | no | percentile | ARC Prize (static JSON, v2 semi-private) | GEN, CRE |
| TerminalBench | higher | no | percentile | Terminal-Bench HTML | PLAN, BUILD |
| TerminalBench21 | higher | no | percentile | Terminal-Bench 2.1 HTML | (input to TerminalBench21Composite) |
| AATerminalBench21 | higher | no | percentile | Artificial Analysis (`terminalbench_v2_1` field) | (input to TerminalBench21Composite) |
| TerminalBenchHard | higher | no | percentile | Artificial Analysis (`terminalbench_hard` field) | PLAN, BUILD |
| BFCL | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV overall score | (input to BFCLComposite) |
| BFCLNonLiveAST | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLLive | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLMultiTurn | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLWebSearch | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLMemory | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLRelevanceDetection | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| BFCLIrrelevanceDetection | higher | no | percentile | Berkeley Function Calling Leaderboard V4 CSV | (input to BFCLComposite) |
| HiLBench | higher | no | percentile | Scale HiL-Bench HTML | PLAN, BUILD |
| Tau2Bench | higher | no | percentile | Artificial Analysis (tau2 field) | (input to TauComposite) |
| TauBanking | higher | no | percentile | Artificial Analysis (`tau_banking` field) | (input to TauComposite) |
| SciCode | higher | no | percentile | Artificial Analysis (scicode field) | (input to LiveCodingComposite) |
| AALiveCodeBench | higher | no | percentile | Artificial Analysis (livecodebench field) | (input to LiveCodingComposite) |
| IFBench | higher | no | percentile | Artificial Analysis (ifbench field) | PLAN |
| ArtificialAnalysisMath | higher | no | percentile | Artificial Analysis math index | GEN |
| AIME25 | higher | no | percentile | Artificial Analysis (`aime_25` field) | GEN |
| MMLUPro | higher | no | percentile | Artificial Analysis (mmlu_pro field) | GEN |
| GDPval | higher | no | percentile | overrides table (GDPval-AA Elo) | BUILD |
| LongContextRecall | higher | no | percentile | Artificial Analysis (lcr field) | BUILD, PLAN |
| SonarFunctionalSkill | higher | no | percentile | Sonar code-quality JSON | (input to SonarComposite) |
| SonarIssueDensity | **lower** | no | percentile | Sonar code-quality JSON | (input to SonarComposite) |
| SonarBugDensity | **lower** | no | percentile | Sonar code-quality JSON | (input to SonarComposite) |
| SonarVulnerabilityDensity | **lower** | no | percentile | Sonar code-quality JSON | (input to SonarComposite) |
| OutputSpeed | higher | **yes** | tail_penalty | Artificial Analysis | OPS_* |
| TTFT | **lower** | **yes** | tail_penalty | Artificial Analysis | OPS_* |
| BlendedCost | **lower** | **yes** | tail_penalty | Artificial Analysis / OpenRouter | emitted, not scored |
| ContextWindow | higher | **yes** | tail_penalty | OpenRouter | OPS_* |

---

## Appendix B: Coefficient Summary Table

All coefficients are verbatim from `data/coefficients.toml`. This table is for quick reference; the TOML file is authoritative.

### Final Score Weights
AISL's former 0.15 role slot is redistributed into non-operational public
benchmark groups. OPS_* contributes 0.08 (paired with the tail-penalty
curve so only genuinely slow models lose meaningful score).

| Role | Group Contributions |
|------|---------------------|
| I_raw | CRE 0.62, GEN 0.33, OPS_long 0.05 |
| P_raw | PLAN 0.55, GEN 0.37, OPS_precision 0.08 |
| B_raw | BUILD 0.84, PLAN 0.08, OPS_precision 0.08 |
| R | LM_ARENA_REVIEW_PROXY 0.25, BUILD 0.29, PLAN 0.38, OPS_review 0.08 |

### Synthesis Penalty
| Constant | Value |
|----------|-------|
| `[penalties].synthesis` | 0.15 |
| `[penalties].override_reported` | 0.0 |

When a conservative metric value comes in via the synthesis layer, its
normalized score is blended toward 50:
`final = score × 0.85 + 50 × 0.15`. Same-series forward values carry no
synthesis penalty. Synthesized values still contribute; their category
controls whether they are discounted.

### Synthesis Caps
| Constant | Value |
|----------|-------|
| `[synthesis].per_model_cap` | 0.50 |
| `[synthesis].per_source_cap` | 0.65 |

A model whose synthetic cells exceed 50 % of its total scored cells is
flagged `synthesis_dominant` in the output.

### Removed AISL Surface
AI Stupid Level (`aistupidlevel`) is retained only as historical source
code and fixture data. It is not registered, its `AI_*` metrics are not in
the coefficient registry, its `A_*` perspective groups are absent, and no
canary-health penalty is applied.
