# Output schema reference

This document describes schema 2.1, the TOML emitted by `ipbr-rank`. Schema
2.1 retains the evidence-rich 2.0 shape and adds auditable methodology-v3
eligibility classes, qualification paths, and Balanced status.

## Versioning

Every `scoreboard.toml` begins with a semantic schema version. Consumers must
reject unsupported major versions and ignore unknown fields within a supported
major version.

```toml
schema_version = "2.1.0"
methodology = "v3"
configuration_policy = "best_available_max_effort"
```

`schema_version` describes the file shape. `methodology` describes scoring
semantics. Either may change independently.

## Generated artifacts

`ipbr-rank all` writes:

1. `scoreboard.toml` — canonical scores, raw observations, provenance, and
   evidence coverage.
2. `missing.toml` — compact missing-data view.
3. `coefficients.toml` — effective weights, metric families, reliabilities,
   and versioned normalization anchors.
4. `site/` — static presentation generated from the same scoreboard.

## `scoreboard.toml`

### Top-level fields

```toml
schema_version = "2.1.0"
generated_at = "2026-07-12T19:44:00Z"
generator = "ipbr-rank 0.1.0"
methodology = "v3"
configuration_policy = "best_available_max_effort"
```

| Field | Meaning |
|---|---|
| `schema_version` | Output shape version. |
| `generated_at` | RFC 3339 generation timestamp. |
| `generator` | CLI name and version. |
| `methodology` | Ranking methodology identifier. |
| `configuration_policy` | One canonical model record using best-available, strongest eligible effort/harness observations. |

The configuration policy means a model record is a capability envelope, not
necessarily one provider configuration that can be run end to end.

### Source summary

Each fetched source has a table:

```toml
[sources.eqbench_judgemark]
status = "verified"
n_rows_ingested = 33
n_rows_matched = 18
n_rows_unmatched = 15
```

`status` is normally `verified`; it can be `skipped` when a required secret is
unavailable. Row counts distinguish parsed upstream coverage from canonical
model matches.

### Model records

Models are repeated TOML array tables sorted deterministically by canonical ID:

```toml
[[models]]
canonical_id = "anthropic/claude-opus-4.7"
display_name = "Claude Opus 4.7"
vendor = "anthropic"
thinking_effort = "best_available"
aliases = ["claude-opus-4-7", "opus 4.7"]
sources = ["artificial_analysis", "eqbench_judgemark", "lmarena"]
```

| Field | Meaning |
|---|---|
| `canonical_id` | Stable `{vendor}/{model}` identity. |
| `display_name` | Human-readable name. |
| `vendor` | Normalized vendor slug. |
| `thinking_effort` | Record-level policy summary: `best_available`, `low`, `medium`, or `high`. `best_available` means individual metrics may use different strongest eligible settings; `configuration_policy` is authoritative. |
| `aliases` | Source spellings accepted by alias matching. |
| `sources` | Sources that matched the canonical record. |

### Role scores and status

```toml
[models.scores]
i_raw = 78.400000
p_raw = 81.100000
b_raw = 79.600000
r = 84.000000
i_status = "ranked"
p_status = "ranked"
b_status = "ranked"
r_status = "provisional"
balanced_status = "ranked"
```

`i_raw`, `p_raw`, `b_raw`, and `r` are the Idea, Planning, Building, and
Review proxy scores.

Each status is `ranked` or `provisional`. Methodology v3 can qualify a role
through the full current score path, a core-only current path that is not
diluted by sparse specialist benchmarks, or a current-plus-historical breadth
path. Full-current qualification also requires at least 35% direct core weight
across three core families, preventing a favorable specialist subset from
qualifying by itself. The numeric score remains available in every case.

Balanced capability is not stored as a fifth score. Consumers can reproduce
the presentation view with:

```text
(i_raw + p_raw + b_raw + r) / 4
```

`balanced_status` is `ranked` when at least three component roles are ranked
and the remaining role has at least 20% current direct coverage. Otherwise it
is `provisional`.

### Groups

```toml
[models.groups]
BUILD = 79.200000
CRE = 80.200000
GEN = 79.100000
OPS_precision = 72.300000
```

Group values are 0–100 diagnostics computed from their configured inputs.
Operational `OPS_*` groups can appear here, but pricing, speed, latency, TTFT,
and context have zero path to any capability rank in methodology v3.

Final roles are flattened to leaf metrics and family-capped. Therefore a role
score cannot always be reproduced by naively averaging the displayed groups.

### Normalized metrics

```toml
[models.metrics]
EQBenchCreativeWriting = 86.200000
SWEBenchVerified = 77.100000
SWEComposite = 79.400000
```

`models.metrics` contains normalized, evidence-adjusted 0–100 observations and
derived composites. Missing leaves and fully absent composites are omitted.

The normalized value already includes evidence reliability:

```text
adjusted = 50 + reliability × (normalized - 50)
```

### Raw metrics and uncertainty

```toml
[models.raw_metrics]
EQBenchCreativeWriting = 2202.500000
EQBenchJudgemark = 83.961200
EQBenchJudgemarkCILow = 80.473100
EQBenchJudgemarkCIHigh = 89.492300
SWERebench = 61.500000
SWERebenchSEM = 0.400000
TerminalBench21 = 74.600000
TerminalBench21Uncertainty = 2.200000
```

`models.raw_metrics` retains native upstream units. Unlike normalized scores,
raw values are not constrained to 0–100; for example, Creative Writing uses
Elo units. Auxiliary uncertainty fields are unscored and never transferred by
sibling synthesis.

Currently preserved auxiliary keys include:

- `TerminalBenchUncertainty`
- `TerminalBench21Uncertainty`
- `SWERebenchSEM`
- `EQBenchJudgemarkCILow`
- `EQBenchJudgemarkCIHigh`

### Per-metric provenance

Every present raw metric, including manually curated observations and
uncertainty auxiliaries, has a provenance table.

Direct observation:

```toml
[models.metric_evidence.EQBenchJudgemark]
class = "direct"
source = "eqbench_judgemark"
```

A native benchmark may also attach a citation when the winning observation has
configuration provenance worth retaining, such as a best-agent submission:

```toml
[models.metric_evidence.TerminalBench21]
class = "direct"
source = "terminal_bench_2_1"
citation = "TerminalBench21 upstream winning submission: model label=...; agent=..."
```

Manually curated same-product observation:

```toml
[models.metric_evidence.GDPval]
class = "direct"
source = "overrides"
citation = "Artificial Analysis GDPval-AA leaderboard, accessed ..."
```

Synthesized observation:

```toml
[models.metric_evidence.SWERebench]
class = "synthesized"
source = "swerebench"
donor = "openai/gpt-5.4"
synthesis_category = "same_series_forward"
```

Actual same-product observations use `direct`, whether their winning source is a
native public feed or the cited manual `overrides` source. `synthesized` is
used for sibling substitutions. The `reported` value and coverage field remain
only for compatibility with older schema-2.1 snapshots; current output does
not emit that evidence class. A vendor-published measurement is not classified
as `reported` merely because it was curated manually. Vendor-automatic routing
and fallback remain part of the same ranked product and retain a citation on
the affected direct metrics. Native sources may likewise retain the winning
agent/harness label as a citation. Valid synthesis
categories are `conservative`, `same_series_forward`, and
`stronger_successor`.

Winning-observation precedence is:

```text
native public direct > manual override direct > synthesized
```

There are currently no rank-derived estimates. If introduced later, they must
remain explicitly non-direct and cannot qualify a role.

### Evidence coverage

Every group and role has an evidence summary:

```toml
[models.evidence.groups.BUILD]
direct = 0.740000
reported = 0.000000
synthesized = 0.080000
missing = 0.180000
effective = 0.740000
family_count = 5
direct_families = ["eqbench", "gso", "scale", "sonar", "swe"]

[models.evidence.roles.B_raw]
direct = 0.740000
reported = 0.000000
synthesized = 0.070000
missing = 0.190000
effective = 0.740000
family_count = 5
direct_families = ["artificial_analysis", "deepswe", "lmarena", "scale", "swe"]
core_direct = 0.700000
core_family_count = 4
core_direct_families = ["artificial_analysis", "lmarena", "swe", "terminal_bench"]
historical_family_count = 2
historical_direct_families = ["livecodebench", "swe"]
qualification_path = "core_standard"
provisional = false
```

The four nominal shares `direct + reported + synthesized + missing` sum to
approximately 1. `reported` is retained only for compatibility; current actual
same-product observations, including manual overrides, contribute to `direct`.
`effective` is confidence-weighted coverage rather than another nominal class.
`family_count` counts direct families only.

Group summaries describe the displayed group graph. Role summaries describe
the flattened and family-capped role calculation. Methodology-v3 role summaries
also expose:

- `core_direct`, `core_direct_families`, and `core_family_count` — coverage
  after retaining and renormalizing only current `core` leaves;
- `historical_direct_families` and `historical_family_count` — direct,
  role-relevant coverage-only families whose scores are not current inputs; and
- `qualification_path` — one of `standard`, `breadth`, `core_standard`,
  `core_corroborated`, `core_breadth`, `historical_breadth`, or `unqualified`.

`provisional` is true exactly when `qualification_path = "unqualified"`.

### Missing-data table

```toml
[models.missing]
metrics = ["EQBenchCreativeWriting", "SWERebench"]
groups_shrunk = ["CRE", "BUILD"]
synthesis_dominant = false
```

| Field | Meaning |
|---|---|
| `metrics` | Active scored leaf metrics with no raw observation. |
| `groups_shrunk` | Compatibility name for groups with incomplete nominal coverage. Capability uses available same-product evidence; this field does not select a separate score formula. |
| `synthesis_dominant` | True when any role's weighted synthesized share exceeds the configured per-model cap. |

## `missing.toml`

This file is a denormalized subset for quick inspection:

```toml
generated_at = "2026-07-12T19:44:00Z"

[models."anthropic/claude-opus-4.7"]
display_name = "Claude Opus 4.7"
metrics = ["EQBenchCreativeWriting"]
groups_shrunk = ["CRE"]
```

Its version follows the associated `scoreboard.toml`; it does not carry a
separate `schema_version` field.

## `coefficients.toml`

This is the effective configuration used for the run. Important schema 2.1
fields include:

```toml
[normalization]
anchor_version = "2026-07-12.v2"
snapshot_date = "2026-07-12"
derivation = "direct_p05_p95_with_reported_only_fallback"
low_quantile = 0.05
high_quantile = 0.95
reported_fallback = true

[metrics.EQBenchCreativeWriting]
family = "eqbench"
eligibility = "core"
higher_better = true
log_scale = false
transform = "percentile"
anchor_low = 1367.42
anchor_high = 2185.10
groups = ["CRE"]

[evidence]
prior_score = 50.0
direct_reliability = 1.0
reported_reliability = 0.60
conservative_synthesis_reliability = 0.0
same_series_synthesis_reliability = 0.0
stronger_successor_synthesis_reliability = 0.0
provisional_min_direct = 0.60
provisional_min_families = 3
provisional_breadth_min_direct = 0.35
provisional_breadth_min_families = 5
representative_min_core_direct = 0.35
representative_min_core_families = 3
core_corroborated_min_direct = 0.50
core_corroborated_min_families = 4
historical_min_current_direct = 0.25
historical_min_current_families = 2
historical_min_families = 2
historical_min_total_families = 5
balanced_min_fourth_direct = 0.20
max_family_weight = 0.30
```

Consumers must still read the effective file rather than hard-code this
example. `reported_reliability` is a reserved compatibility setting for older
snapshots; it is not applied to actual manual overrides.
Anchor set `2026-07-12.v2` was derived from the frozen 2026-07-12 evidence
snapshot. Active scored leaves use fixed anchors; explicitly unanchored
diagnostic or custom metrics may use their configured fallback transform.

`metrics.*.eligibility` is `core`, `supplemental`, or `historical_support`.
Unannotated metrics default to `supplemental`, preventing a newly ingested
diagnostic from silently tightening eligibility. A historical-support metric
also declares `eligibility_roles`, for example `["B_raw", "R"]`, and must have
no current group/final-score path.

## Parsing example

Python 3.11+:

```python
import tomllib

with open("out/scoreboard.toml", "rb") as handle:
    board = tomllib.load(handle)

if not board["schema_version"].startswith("2."):
    raise RuntimeError("unsupported scoreboard schema")

for model in board["models"]:
    scores = model["scores"]
    balanced = sum(scores[key] for key in ("i_raw", "p_raw", "b_raw", "r")) / 4
    status = scores["balanced_status"]
    print(model["canonical_id"], balanced, status)
```

## Stability guarantees

Non-breaking changes within schema 2 may add models, sources, metrics,
evidence fields, or optional tables. Consumers should ignore unknown fields
and not depend on table ordering.

A major bump is required to remove or rename required top-level/model fields,
change the model identity format, or materially restructure score/evidence
tables.

With `--offline`, a fixed cache, and `--now`, output is deterministic: maps and
arrays are sorted and floats use fixed precision.

## Constraints

- Normalized metrics, composites, groups, and role scores are in `[0, 100]`.
- Raw metrics use native units and need only be finite.
- Missing values are represented by absence, not `null` or `NaN`.
- `generated_at` is RFC 3339.
- Arrays contain no duplicates.
- Map keys are case-sensitive.

## Changelog

### 2.1.0

- Changed the scoring identifier to methodology v3.
- Added per-metric eligibility classes and role declarations for historical
  support.
- Added core and historical family coverage plus an explicit qualification
  path to role evidence.
- Added `balanced_status`; Balanced now requires three ranked roles and 20%
  current direct evidence in the fourth instead of an all-four-role veto.

### 2.0.0

- Added explicit `best_available_max_effort` configuration policy.
- Added ranked/provisional status per role.
- Added raw metrics and auxiliary uncertainty fields.
- Added per-metric evidence class, winning source, donor/category, and override
  citation.
- Added recursive group/role evidence coverage and direct-family counts.
- Restored `synthesis_dominant` as a weighted role-path diagnostic.
- Changed methodology identifier to `v2`.

### 1.1.0

- Synthesis provenance was internal and missing-data reporting was limited to
  metrics and group shrink flags.

### 1.0.0

- Initial stable four-role scoreboard.
