# Output Schema Reference

This document describes the TOML output format produced by `ipbr-rank`. The schema is stable for v1; breaking changes require a `schema_version` bump.

---

## Schema Version

Current version: **1.1.0**

All output files include a `schema_version` field at the top level. Downstream consumers should check this field and error gracefully if the major version differs.

---

## Output Files

`ipbr-rank all` produces four outputs in the `--out` directory (default: `out/`):

1. **`scoreboard.toml`** — The primary deliverable. Contains all models with their scores, groups, metrics, and metadata.
2. **`missing.toml`** — Denormalized view of missing metrics per model (same content as `models.missing` in `scoreboard.toml`, easier to grep).
3. **`coefficients.toml`** — Echo of the effective coefficients used in the run (matches input when no overrides).
4. **`site/`** — Static HTML website (separate from TOML schema, not covered here).

---

## 1. `scoreboard.toml`

### Top-Level Fields

```toml
schema_version = "1.1.0"           # String, semver
generated_at = "2026-04-26T11:35:46Z"  # String, RFC3339 / ISO8601
generator = "ipbr-rank 0.1.0"      # String, "{binary} {version}"
methodology = "v1"                 # String, matches docs/methodology.md heading
```

- **`schema_version`**: Semver string. Major version bump = breaking change. Consumers should check this.
- **`generated_at`**: Timestamp in RFC3339 format (UTC). Overrideable via `--now` for deterministic tests.
- **`generator`**: Binary name + version from `Cargo.toml`.
- **`methodology`**: Identifies the scoring methodology version. Currently always `"v1"`.

### `[sources]` Table

Optional. Maps source ID → metadata.

```toml
[sources.openrouter]
status = "verified"               # String, "verified" | "experimental" | "skipped"
rows = 312                        # Integer, total rows fetched
matched = 298                     # Integer, rows successfully matched to canonical IDs
unmatched = 14                    # Integer, rows that failed alias matching
```

- **`status`**: Source status at runtime. `"skipped"` if required secret was missing.
- **`rows`**: Total number of raw rows fetched from this source.
- **`matched`**: Number of rows successfully matched to a canonical model ID via alias matcher.
- **`unmatched`**: Number of rows that failed matching (logged as warnings, discarded).

### `[[models]]` Array

Each model is an entry in the `models` array-of-tables.

```toml
[[models]]
canonical_id = "anthropic/claude-opus-4.7"
display_name = "Claude Opus 4.7"
vendor = "anthropic"
thinking_effort = "default"       # "default" | "low" | "medium" | "high"
aliases = ["opus 4.7", "claude-opus-4-7", "claude opus 4.7"]
sources = ["openrouter", "lmarena", "artificial_analysis"]

[models.scores]
i_raw = 78.4
p_raw = 81.1
b_raw = 79.6
r = 84.0
i_adj = 78.4
p_adj = 81.1
b_adj = 79.6

[models.groups]
CRE = 80.2
GEN = 79.1
PLAN = 75.3
BUILD = 82.0
LM_ARENA_REVIEW_PROXY = 84.5
OPS_long = 71.0
OPS_precision = 68.5
OPS_review = 69.2
A_I = 77.8
A_P = 76.5
A_B = 81.2
A_R = 83.0

[models.metrics]
LMArenaText = 82.5
SWEBenchVerified = 76.0
ArtificialAnalysisIntelligence = 78.0
AI_correctness = 75.0
# ... all normalized metrics populated for this model

[models.synthesized]
# Optional. Present only when at least one metric was estimated from
# a sibling model (see `synthesis_dominant` and the `*` marker on the
# rendered site). Omitted entirely when no metrics were synthesized.
SWEBenchVerified = { source = "swebench", from = "anthropic/claude-opus-4.6" }

[models.missing]
metrics = ["TerminalBench", "SonarFunctionalSkill"]
groups_shrunk = ["PLAN"]
synthesis_dominant = false
```

#### Model Fields

- **`canonical_id`**: String, unique identifier in `vendor/model[+thinking-effort]` format.
- **`display_name`**: String, human-readable name for rendering.
- **`vendor`**: String, vendor name (lowercase). Possible values: `"openai"`, `"anthropic"`, `"google"`, `"moonshot"`, `"zai"`, `"xai"`, `"alibaba"`, `"deepseek"`, `"mistral"`, `"meta"`, `"minimax"`, `"nvidia"`, `"baidu"`, `"tencent"`, `"inclusionai"`, `"xiaomi"`, or any other vendor as a string.
- **`thinking_effort`**: String, one of:
  - `"default"` — No explicit thinking effort level
  - `"low"` — Low reasoning budget
  - `"medium"` — Medium reasoning budget
  - `"high"` — High reasoning budget
- **`aliases`**: Array of strings, normalized aliases that match this model.
- **`sources`**: Array of strings, source IDs that contributed data for this model.

#### `[models.scores]` Table

All scores are floats in the range [0.0, 100.0].

- **`i_raw`**: Idea score (raw, before reviewer-reservation penalty)
- **`p_raw`**: Planning score (raw)
- **`b_raw`**: Building score (raw)
- **`r`**: Reviewing score (not adjusted — used to compute the penalty for others)
- **`i_adj`**: Idea score (adjusted for reviewer-reservation penalty)
- **`p_adj`**: Planning score (adjusted)
- **`b_adj`**: Building score (adjusted)

**Why R is not adjusted**: The reviewing score is the penalty *source*, not a penalty *target*. Applying the penalty to R would create circular dependency.

#### `[models.groups]` Table

Maps group key → group score (float, 0–100).

Possible group keys:
- **`CRE`** (Creativity)
- **`GEN`** (General Intelligence)
- **`PLAN`** (Planning)
- **`BUILD`** (Building)
- **`LM_ARENA_REVIEW_PROXY`** (LM Arena search/document review proxy)
- **`OPS_long`** (Ops for long generation)
- **`OPS_precision`** (Ops for precise tasks)
- **`OPS_review`** (Ops for reviewing)
- **`A_I`** (AI Stupid Level: Idea perspective)
- **`A_P`** (AI Stupid Level: Planning perspective)
- **`A_B`** (AI Stupid Level: Building perspective)
- **`A_R`** (AI Stupid Level: Reviewing perspective)

Groups where `present_weight / total_weight < 0.70` (the trust threshold) are marked as "shrunk" in `models.missing.groups_shrunk`.

#### `[models.metrics]` Table

Maps metric key → normalized score (float, 0–100).

Metric keys are defined in `data/coefficients.toml` under `[metrics.*]`. See `docs/methodology.md` Appendix A for the complete list.

Only metrics that are *present* for this model appear in this table. Missing metrics are listed in `models.missing.metrics`.

#### `[models.synthesized]` Table (added in 1.1.0)

Optional. Maps metric key → provenance record. **The entire table is omitted when no metrics were synthesized**, so consumers should treat its absence as equivalent to an empty map.

```toml
[models.synthesized]
SWEBenchVerified = { source = "swebench", from = "anthropic/claude-opus-4.6" }
```

- **`<MetricKey>`**: A metric key that also appears in `[models.metrics]` for this model. Its value was estimated from a sibling model rather than measured directly for this model.
- **`source`**: String. The source ID that produced the donor row. Always identical to the source attached to the donor model — synthesis is same-source-only.
- **`from`**: String. The canonical ID of the donor (sibling) model.

Sorted by metric key. The metric value itself is still emitted under `[models.metrics]`; this table records where it came from.

#### `[models.missing]` Table

- **`metrics`**: Array of strings, metric keys that are missing for this model.
- **`groups_shrunk`**: Array of strings, group keys where less than 70% of the weight was present (the trust threshold — see methodology §4.2 — below which the score is shrunk toward 50).
- **`synthesis_dominant`** *(added in 1.1.0)*: Boolean. `true` when more than `synthesis.per_model_cap` metrics for this model were filled in from sibling synthesis. Always present; defaults to `false`.

---

## 2. `missing.toml`

A denormalized view of missing data, easier to query than iterating `scoreboard.toml`.

```toml
schema_version = "1.1.0"
generated_at = "2026-04-26T11:35:46Z"

[[models]]
canonical_id = "anthropic/claude-opus-4.7"
display_name = "Claude Opus 4.7"
vendor = "anthropic"
missing_metrics = ["TerminalBench", "SonarFunctionalSkill"]
missing_count = 2
groups_shrunk = ["PLAN"]
```

### Fields

- **`canonical_id`**, **`display_name`**, **`vendor`**: Same as in `scoreboard.toml`.
- **`missing_metrics`**: Array of strings, missing metric keys.
- **`missing_count`**: Integer, `missing_metrics.len()`.
- **`groups_shrunk`**: Array of strings, groups where `present_weight / total_weight < 0.70` (the trust threshold).

---

## 3. `coefficients.toml`

A verbatim echo of the *effective* coefficients used in the run. When `--coefficients` is not provided, this matches the embedded `data/coefficients.toml`. When coefficients are overridden, this reflects the overrides.

### Structure

Same as `data/coefficients.toml`:

```toml
[ai_stupid_perspective_weights.A_I]
AI_correctness = 0.16
AI_spec = 0.16
AI_code = 0.04
AI_efficiency = 0.08
AI_stability = 0.14
AI_refusal = 0.10
AI_recovery = 0.10
AI_complexity = 0.08
AI_edge_cases = 0.06
AI_plan_coherence = 0.08

[ai_stupid_perspective_weights.A_P]
# ... same structure for A_P, A_B, A_R (each pulls a tailored slice
# of the 17 AISL axes — see data/coefficients.toml for full weights)

[group_weights.CRE]
LMArenaCreativeOrOpenEnded = 0.65
LMArenaText = 0.35

[group_weights.GEN]
# ... same structure for other groups

[final_score_weights.I_raw]
CRE = 0.30
GEN = 0.12
A_I = 0.50
OPS_long = 0.08

[final_score_weights.P_raw]
# ... same structure for P_raw, B_raw, R

[reviewer_reservation]
I_adj = 0.08
P_adj = 0.18
B_adj = 0.32

[metrics.LMArenaText]
higher_better = true
log_scale = false
groups = ["CRE", "GEN"]
transform = "percentile"

[metrics.ArtificialAnalysisIntelligence]
higher_better = true
log_scale = false
groups = ["GEN"]

# ... same structure for all metrics
```

See `data/coefficients.toml` in the repository for the authoritative v1 values.

---

## Parsing Examples

### Python

```python
import tomllib  # Python 3.11+

with open("out/scoreboard.toml", "rb") as f:
    scoreboard = tomllib.load(f)

for model in scoreboard["models"]:
    print(f"{model['canonical_id']}: I={model['scores']['i_adj']:.1f}")
```

### Rust

```rust
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Scoreboard {
    schema_version: String,
    generated_at: String,
    generator: String,
    methodology: String,
    sources: Option<BTreeMap<String, SourceSummary>>,
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct SourceSummary {
    status: String,
    rows: usize,
    matched: usize,
    unmatched: usize,
}

#[derive(Deserialize)]
struct Model {
    canonical_id: String,
    display_name: String,
    vendor: String,
    thinking_effort: String,
    aliases: Vec<String>,
    sources: Vec<String>,
    scores: RoleScores,
    groups: BTreeMap<String, f64>,
    metrics: BTreeMap<String, f64>,
    missing: Missing,
}

#[derive(Deserialize)]
struct RoleScores {
    i_raw: f64,
    p_raw: f64,
    b_raw: f64,
    r: f64,
    i_adj: f64,
    p_adj: f64,
    b_adj: f64,
}

#[derive(Deserialize)]
struct Missing {
    metrics: Vec<String>,
    groups_shrunk: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string("out/scoreboard.toml")?;
    let scoreboard: Scoreboard = toml::from_str(&raw)?;
    
    for model in scoreboard.models {
        println!("{}: I={:.1}", model.canonical_id, model.scores.i_adj);
    }
    
    Ok(())
}
```

### JavaScript / Node.js

```javascript
const toml = require('@iarna/toml');
const fs = require('fs');

const raw = fs.readFileSync('out/scoreboard.toml', 'utf8');
const scoreboard = toml.parse(raw);

for (const model of scoreboard.models) {
    console.log(`${model.canonical_id}: I=${model.scores.i_adj.toFixed(1)}`);
}
```

---

## Stability Guarantees

### Non-Breaking Changes (Patch/Minor)
- Adding optional top-level fields
- Adding new models to `[[models]]` array
- Adding new metrics/groups to existing models (downstream parsers ignore unknown keys)
- Changing float precision (values remain semantically equivalent)
- Reordering models/metrics/groups (order is not guaranteed)

### Breaking Changes (Major)
- Renaming or removing top-level fields
- Changing `models.scores` field names
- Changing the structure of `models.missing`
- Changing the format of `canonical_id`
- Removing required fields

When a breaking change is necessary, `schema_version` major version will be incremented (e.g., `"2.0.0"`).

---

## Field Constraints

### Floats
- All score/metric/group values are floats in the range [0.0, 100.0]
- Formatted with fixed precision (typically 1 decimal place)
- Missing values are represented by absence from the table, not `null` or `NaN`

### Strings
- `canonical_id` format: `{vendor}/{model}` or `{vendor}/{model}+thinking-{level}`
- `vendor` is lowercase, no spaces
- `thinking_effort` is one of: `"default"`, `"low"`, `"medium"`, `"high"`
- `generated_at` is RFC3339 (e.g., `"2026-04-26T11:35:46Z"`)

### Arrays
- All arrays are sorted deterministically (alphabetical for strings, undefined but stable for models)
- No duplicates within an array field

### Maps
- All maps (`groups`, `metrics`, `sources`) are sorted by key (alphabetical)
- Keys are case-sensitive

---

## Determinism

With `--offline --cache <fixtures> --now <timestamp>`:
- The output is byte-for-byte deterministic across runs
- Timestamps use the `--now` override
- All maps are sorted
- Floats use fixed formatting

This enables golden testing and reproducible builds.

---

## Consumption Best Practices

1. **Check `schema_version`** before parsing. Fail gracefully if major version differs.
2. **Use missing fields defensively**: If a field is documented as optional, handle its absence.
3. **Ignore unknown fields**: Future minor versions may add new fields.
4. **Do not rely on ordering**: The order of models, metrics, and groups is not guaranteed (though currently stable for determinism).
5. **Validate floats are in [0, 100]**: All scores/metrics/groups should be in this range. File a bug if you see values outside.

---

## Schema Changelog

### 1.1.0

- Added optional `[models.synthesized]` table mapping metric → `{source, from}` provenance for sibling-synthesized cells. Omitted entirely when no metrics were synthesized for a model.
- Added `synthesis_dominant: bool` inside `[models.missing]`. Defaults to `false`; set to `true` when more than `synthesis.per_model_cap` metrics were filled by synthesis.
- Pre-1.1.0 consumers that gate on the major version (`1.x.x`) and ignore unknown fields parse 1.1.0 output unchanged.

### 1.0.0 (Initial)
- First stable schema
- All four role scores (I_raw, P_raw, B_raw, R) plus adjusted (I_adj, P_adj, B_adj)
- 12 groups (CRE, GEN, PLAN, BUILD, LM_ARENA_REVIEW_PROXY, OPS_*, A_*)
- Metrics defined in `data/coefficients.toml`
- Missing-data tracking via `models.missing`

---

End of Output Schema Reference.
