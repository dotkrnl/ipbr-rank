# ipbr

> **Models drift. Evidence accumulates. Ranks update.**
> Live LLM coding scoreboard — site at <https://ipbr.pages.dev>.

A Rust workspace that fetches registered public LLM benchmark sources, normalizes them, computes four building-role scores (Idea, Planning, Building, Reviewing), and emits a canonical TOML scoreboard plus a static website. (CLI binary and crate names stay `ipbr-rank`.)

Methodology v3 separates capability from deployment diagnostics, publishes
evidence quality and provenance, and marks genuinely under-supported roles
provisional without penalizing established models for gaps on narrow new
leaderboards.
See [`docs/methodology.md`](docs/methodology.md) for the scoring contract.

## Quick Start

```bash
# One-shot live refresh (sources .env, builds release, runs fetch→score→render)
scripts/refresh.sh                 # writes out/
scripts/refresh.sh --open          # also opens out/site/index.html
scripts/refresh.sh --offline       # use cached responses only
scripts/refresh.sh --only artificial_analysis,lmarena
scripts/refresh.sh --publish       # also deploy out/site to Cloudflare Pages
```

`.env` is sourced for credentials. OpenRouter's public model endpoint does not
require a key:

| variable | purpose |
|---|---|
| `AA_API_KEY` | Artificial Analysis fetcher |
| `HF_TOKEN` | optional HuggingFace bearer token that reduces LMArena rate limits |
| `CLOUDFLARE_ACCOUNT_ID` | only required for `--publish` |
| `CLOUDFLARE_PAGES_PROJECT` | optional, default `ipbr` |

Manual invocation works too:

```bash
cargo build --release -p ipbr-rank-cli
./target/release/ipbr-rank --cache cache --out out all
```

## Deployment

The rendered site lives at `out/site/` and is fully static, with no runtime
network dependencies. The validator rejects external resource URLs and `data:`
URLs; the explicit GitHub navigation link is allowlisted.

`scripts/refresh.sh --publish` deploys it to Cloudflare Pages via wrangler:

```bash
# one-time auth (interactive, opens a browser)
npx wrangler login

# fetch + render + deploy
scripts/refresh.sh --publish
```

The current production deployment is at https://ipbr.pages.dev. CI re-runs
`refresh.sh --publish` every 10 minutes on `main` (see
`.github/workflows/refresh.yml`), so the site stays current without manual
intervention.

### Rank change

The `▲2` / `▼1` chips on the site are **places gained or lost in that role**,
measured against the scoreboard as it stood at the **start of last week**. Weeks
open on Sunday, so every refresh from Sun 2026-07-12 through Sat 2026-07-18
compares against the snapshot frozen on Sun 2026-07-05 — one stable baseline per
week, rather than a number that twitches with each 10-minute refresh.

`scripts/weekly_baseline.sh` maintains that cache. It fetches the live published
scoreboard, freezes `cache/week_<sunday>.toml` on the first run of each week,
prints last week's snapshot for the pipeline's `--prev` flag, and prunes weeks
older than the baseline (by week, never by mtime — the baseline can be 15 days
old). With no snapshot from last week (a fresh cache, or the first run under
this scheme) it seeds the baseline from today, so changes accrue from now
instead of disappearing for a week.

Baseline ranks are computed within the baseline board's *own* population: a
model that has since arrived has no prior rank and shows no chip, and models
that were pushed down by new arrivals show that as real movement.

## Four Building Roles

- **I** (Idea): Creativity, general intelligence, open-ended generation
- **P** (Planning): Structured reasoning, function calling, multi-step task decomposition
- **B** (Building): Implementation, SWE-style work, terminal tasks, and code-quality benchmarks
- **R** (Reviewing proxy): Direct code-review evidence where available, plus review-adjacent search/document preference and Planning and Building capability

Each role has one 0–100 evidence-adjusted benchmark score.

## Sources

All data comes from public, verifiable sources. See [`docs/sources.md`](docs/sources.md) for the full list.

**Registered sources** (run by default; experimental and credentialed status is
reported in the output):

- OpenRouter API — model discovery, pricing, context windows
- LM Arena — text, code, search, and document preference ratings
- EQ-Bench Creative Writing v3 — direct creative-writing Elo
- EQ-Bench Judgemark v4 — diagnostic judge-discrimination score with a published 95% confidence interval
- Artificial Analysis model API — GPQA, HLE, SciCode, AA-LCR, current tau3-Banking, Terminal-Bench 2.1 fallback, and legacy/operational diagnostics. Vendor-automatic fallback is part of the served ranked product and counts as direct evidence with explicit provenance.
- Artificial Analysis evaluation pages — direct GDPval-AA v2, CritPt, AA-Omniscience, EnterpriseOps-Gym-AA, and AutomationBench-AA observations
- DeepSWE v1.1 — original long-horizon repository work under one fixed mini-swe-agent harness; supplemental Build evidence
- Context Arena — eight-needle GDM-MRCRv2 AUC through 128k, combined with AA-LCR as one long-context signal
- AGC-Bench — broad creativity meta-benchmark, currently diagnostic while the new suite matures
- Factory Code Review — direct precision/recall/F1 on human-curated PR bugs, diagnostic pending broader max-effort frontier coverage
- SWE-bench JSON — Multilingual feeds the current SWE composite; retired Verified rows provide historical Build/Review support only
- SWE-bench Pro (Scale) — harder, multi-file SWE-bench (1.8k tasks across 41 repos); supplemental in the SWE composite
- SWE Atlas (Scale) — codebase Q&A, test writing, and refactoring leaderboards, collapsed into a SWE Atlas composite
- SWE-rebench — continuously-refreshed agentic SWE leaderboard, rolling-window resolved rate
- LiveCodeBench — competitive-programming pass@1 (ingested for back-compat; *retired* from BUILD weighting after the upstream JSON froze at mid-2025 frontier — see `docs/sources.md`)
- GSO — "Generalized Software Optimization" track from the LiveCodeBench operators; retained as a diagnostic after a cross-machine reliability audit
- Terminal-Bench 2.0 — retired score input retained as direct historical Plan/Build/Review support
- Terminal-Bench 2.1 — newer narrow Terminal-Bench track, combined with AA's terminalbench_v2_1 field
- BFCL V4 — Berkeley function/tool-calling leaderboard; diagnostic pending a broader refreshed cohort
- Sonar Code Quality — diagnostic functional pass rate plus issue, bug, and vulnerability density; excluded from rank while published effort levels are not comparable across models
- MCP-Atlas (Scale) — supplemental Model Context Protocol tool-orchestration over 36 servers / 220 tools / 1k tasks
- HiL-Bench (Scale) — human-in-the-loop escalation accuracy; a small Plan-only signal
- ARC-AGI v2 — active novel pattern-induction benchmark; ARC-AGI-3 is ingested separately as a floor-compressed diagnostic
- Manual overrides (`data/score_overrides.toml`) — cited, hand-curated same-product benchmark observations for gaps that native public feeds have not yet filled. Vendor and system-card observations count as direct coverage; a native public row still replaces a duplicate override. Historical GDPval overrides remain diagnostic now that GDPval-AA v2 has a native source.

## Math Summary

### Ranked entity

Each canonical ranked product has one record. For each metric, the scorer keeps
the best eligible product-level observation, preferring max, then xhigh, high,
thinking/adaptive, medium, then default effort. This is a capability envelope,
not necessarily one runnable endpoint configuration.

### Normalization and evidence

Active scored leaves use fixed, versioned raw anchors with an asymptotic
logistic mapping: the low and high anchors map to approximately 5 and 95.
Scores therefore do not move merely because an unrelated model joins the
cohort.

Evidence is reliability-weighted toward the neutral prior of 50:

- actual same-product observation, whether fetched or manually curated: **1.00**

The ranked product includes vendor-automatic routing and fallback behavior;
separately named multi-agent or premium endpoints remain distinct. Capability
is averaged over available direct same-product observations; missing leaves do
not depress the point estimate. Their nominal weight remains visible as
confidence/coverage, and a fully unsupported role falls
back to 50. Final role paths are flattened to unique leaves, and no source
family may carry more than 30% of a role when enough independent families
exist.

Lifecycle labels are part of model identity: `preview` builds and moving
`latest` routes are matched only through explicit, source-audited aliases.
Historical observations remain attached to the build that was actually tested;
observations made after a documented redirect belong to the served successor.

### Final scores

- **I_raw** = 0.65×CRE + 0.35×GEN
- **P_raw** = 0.65×PLAN + 0.35×GEN
- **B_raw** = 0.95×BUILD + 0.05×PLAN
- **R** = 0.20×CODE_REVIEW_DIRECT + 0.20×LM_ARENA_REVIEW_PROXY + 0.30×BUILD + 0.30×PLAN

When a model has no direct code-review observation, the
`CODE_REVIEW_DIRECT` weight redistributes across its remaining observed
evidence, so unmeasured models are not penalized.

The displayed **Balanced capability** rank is the unweighted mean of the four
role proxies. Methodology v3 separates score contribution from ranking
eligibility: broad core benchmarks establish current coverage, sparse
supplemental benchmarks may improve the score without making their absence a
penalty, and retired-but-valid direct observations may support the coverage
history without entering the current score. Total-portfolio qualification also
requires a representative base of at least 35% direct core weight across three
core families, so a favorable specialist subset cannot qualify by itself.
Balanced is ranked when at least three roles are ranked and the fourth has at
least 20% current direct coverage.

Pricing, speed, latency, TTFT, and context-window values are diagnostics only.
They have zero path to any role or Balanced capability rank.

See [`docs/methodology.md`](docs/methodology.md) for the complete mathematical derivation and all coefficients.

## Sample Output (TOML)

```toml
schema_version = "2.1.0"
generated_at = "2026-07-12T19:44:00Z"
generator = "ipbr-rank 0.1.0"
methodology = "v3"
configuration_policy = "best_available_max_effort"

[[models]]
canonical_id = "anthropic/claude-opus-4.7"
display_name = "Claude Opus 4.7"
vendor = "anthropic"
thinking_effort = "best_available"
aliases = ["opus 4.7", "claude-opus-4-7"]
sources = ["openrouter", "lmarena", "artificial_analysis"]

[models.scores]
i_raw = 78.4
p_raw = 81.1
b_raw = 79.6
r = 84.0
i_status = "ranked"
p_status = "ranked"
b_status = "ranked"
r_status = "provisional"
balanced_status = "ranked"

[models.groups]
CRE = 80.2
GEN = 79.1
# ...

[models.metrics]
LMArenaText = 82.5
SWEBenchVerified = 76.0
# ...

[models.raw_metrics]
EQBenchJudgemark = 83.96
EQBenchJudgemarkCILow = 80.47
EQBenchJudgemarkCIHigh = 89.49

[models.metric_evidence.EQBenchJudgemark]
class = "direct"
source = "eqbench_judgemark"

[models.evidence.roles.R]
direct = 0.70
missing = 0.30
effective = 0.70
family_count = 2
direct_families = ["lmarena", "scale"]
core_direct = 0.42
core_family_count = 2
core_direct_families = ["lmarena", "swe"]
historical_family_count = 1
historical_direct_families = ["terminal_bench"]
qualification_path = "unqualified"
provisional = true

[models.missing]
metrics = []
groups_shrunk = []
```

See [`docs/output-schema.md`](docs/output-schema.md) for the complete TOML schema reference.

## CLI Reference

```bash
ipbr-rank [OPTIONS] <COMMAND>

Commands:
  fetch            Download all enabled sources into --cache
  score            Read --cache, write scoreboard.toml + missing.toml + coefficients.toml
  render           Read scoreboard.toml + coefficients.toml, write static site to out/site/
  all              fetch -> score -> render (default)
  verify-sources   Run contract tests against live endpoints
  list-models      Emit canonical IDs + vendor from required_aliases.toml
  triage           List unmatched leaderboard rows from the cache (--min-count N)

Command options (render, all):
  --prev PATH                   Prior scoreboard.toml to measure rank change
                                against (see "Rank change"). Render-only; never
                                persisted. Missing/unreadable file -> no chips.

Options:
  --out DIR                     Output directory [default: out]
  --coefficients PATH           Override embedded coefficients.toml
  --aliases PATH                Override embedded required_aliases.toml
  --cache DIR                   Cache directory for fetched responses
  --offline                     Fail if any source is not in --cache
  --only SOURCE,SOURCE          Fetch only specific sources
  --aa-api-key-file PATH        File containing AA_API_KEY
  --openrouter-api-key-file PATH
  --hf-token-file PATH
  --now ISO8601                 Override generated_at timestamp (for tests)
```

### Cache & TTL

`--cache DIR` activates a persistent on-disk cache. Each source declares its
own freshness window (`Source::cache_ttl`); when the cache file's mtime is
within that window, the live fetch is skipped. `--offline` always reads from
cache regardless of mtime.

| source | TTL | rationale |
|---|---|---|
| artificial_analysis | 10m | high-churn live model/perf payload |
| openrouter, lmarena, EQ-Bench, AA evaluation pages, context_arena, deep_swe_v1_1 | 24h | daily refresh |
| livecodebench, gso | 2d | weekly-ish leaderboard refreshes |
| all other network sources | 7d | infrequent updates |
| overrides | n/a | embedded local data; no fetch cache |

To force a refresh of one source, delete its cache file (or `touch -t` it to
the past) and rerun.

The HTTP layer also retries on `429 Too Many Requests` and `5xx` with
exponential backoff (500 ms → 60 s, up to 6 attempts), honoring `Retry-After`
when present — the HuggingFace datasets-server in particular rate-limits
aggressively while paginating LMArena, so set `HF_TOKEN` when available.

### Offline Mode (for CI/tests)

```bash
# Deterministic fixture render
ipbr-rank all \
  --offline \
  --cache data/fixtures \
  --out /tmp/ipbr-fixture-out \
  --now 2026-01-01T00:00:00Z

# Compare the same fixture pipeline with the tracked golden
cargo test -p ipbr-rank-cli --test golden
```

## Overriding Coefficients

```bash
# Copy embedded coefficients
cp data/coefficients.toml my_coefficients.toml

# Edit weights, then:
ipbr-rank all --coefficients my_coefficients.toml

# The effective coefficients are echoed to out/coefficients.toml
```

A standalone `render` reads those echoed coefficients back, so the site is
always labelled with the set the scores were actually produced under. Passing
`--coefficients` to `render` overrides them.

## Adding a Source

See [`docs/adding-a-source.md`](docs/adding-a-source.md) for the verification protocol and implementation checklist.

## Architecture

```
ipbr-rank/
├── crates/
│   ├── core/          # Pure math: data model, normalization, scoring
│   ├── sources/       # Per-source fetchers behind trait Source
│   ├── render/        # TOML + static HTML emission
│   └── cli/           # Binary orchestration
├── data/
│   ├── coefficients.toml       # All weights and metric definitions
│   ├── required_aliases.toml   # Canonical ID → vendor + alias list
│   └── fixtures/               # Snapshotted responses for offline tests
├── docs/
│   ├── methodology.md          # Full math explanation
│   ├── sources.md              # One section per source
│   ├── adding-a-source.md      # Verification protocol
│   └── output-schema.md        # TOML schema reference
└── scripts/
    ├── refresh.sh              # One-shot fetch → score → render (+ optional --publish)
    ├── weekly_baseline.sh      # Weekly rank-change baseline (see "Rank change")
    └── check-docs.sh           # Fails if a source or schema value is undocumented
```

The static site (theme, scripts, HTML) is generated entirely from Rust in
`crates/render/src/site/` — there are no external template files.

## Pre-commit

A `.pre-commit-config.yaml` runs `cargo fmt --check`, `cargo clippy -D
warnings`, `scripts/check-docs.sh`, and basic repo-hygiene hooks.

```bash
pip install pre-commit  # one-time
pre-commit install      # installs the git hook
pre-commit run --all-files  # one-shot full sweep
```

## Testing

```bash
# All unit + contract + golden tests
cargo test --workspace

# Live source verification (best-effort, network-dependent)
cargo run -p ipbr-rank-cli -- --cache /tmp/ipbr-live-cache verify-sources

# Regenerate the deterministic scoreboard golden from the CLI's own output
UPDATE_GOLDEN=1 cargo test -p ipbr-rank-cli --test golden
```

## License

Released under the MIT License — see [`LICENSE`](LICENSE).
