# ipbr-rank — Public LLM Building-Role Scoreboard

A Rust workspace that fetches public LLM benchmarks from verified sources, normalizes them, computes four building-role scores (Idea, Planning, Building, Reviewing), and emits a canonical TOML scoreboard plus a beautifully rendered static website.

> **Fully vibe-coded.** No human wrote the scoring weights — Claude, Gemini,
> GPT, and Kimi argued over every coefficient, every group composition, and
> every penalty curve in this repo. Round after round of cross-model code
> review settled the numbers; the human just refereed and pressed merge.
> The repo's copyright reflects that: the four debating models are the
> credited authors. See `docs/methodology.md` for what they landed on.

## Quick Start

```bash
# One-shot live refresh (sources .env, builds release, runs fetch→score→render)
scripts/refresh.sh                 # writes out/
scripts/refresh.sh --open          # also opens out/site/index.html
scripts/refresh.sh --offline       # use cached responses only
scripts/refresh.sh --only artificial_analysis,lmarena
scripts/refresh.sh --publish       # also deploy out/site to Cloudflare Pages
```

`.env` is sourced for credentials:

| variable | needed for |
|---|---|
| `AA_API_KEY` | Artificial Analysis fetcher |
| `OPENROUTER_API_KEY` | OpenRouter pricing/context |
| `HF_TOKEN` | LMArena via HuggingFace |
| `CLOUDFLARE_ACCOUNT_ID` | only required for `--publish` |
| `CLOUDFLARE_PAGES_PROJECT` | optional, default `ipbr` |

Manual invocation works too:

```bash
cargo build --release -p ipbr-rank-cli
./target/release/ipbr-rank --cache cache --out out all
```

## Deployment

The rendered site lives at `out/site/` and is fully static (no external
network deps; the validator rejects `http://`, `https://`, and `data:` URLs).

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

## Four Building Roles

- **I** (Idea): Creativity, general intelligence, open-ended generation
- **P** (Planning): Structured reasoning, function calling, multi-step task decomposition
- **B** (Building): Implementation, benchmarks (SWE-bench, LiveCodeBench, etc.)
- **R** (Reviewing): Judging code quality, correctness, preference evaluation

Each role has a **raw** score (0–100, based on public benchmarks) and an **adjusted** score that applies a **reviewer-reservation penalty** to prevent vendors from gaming their own preference evaluations.

## Sources

All data comes from public, verifiable sources. See [`docs/sources.md`](docs/sources.md) for the full list.

**Verified sources** (always run):
- OpenRouter API — model discovery, pricing, context windows
- LM Arena — preference ratings across text, code, and hard prompts
- Artificial Analysis — intelligence/coding/reasoning indices, plus tau2-bench, scicode, ifbench, lcr, and gpqa+hle reasoning blend
- AI Stupid Level — 17 capability axes across hourly + deep + tooling suites, plus a dedicated canary health signal used only as a fast degradation penalty (tooling-suite errorHandling dropped due to upstream measurement quirk — see `docs/sources.md` AISL entry)
- SWE-bench JSON — Verified + Multilingual leaderboards (single fetch, both fed into the SWE composite)
- SWE-bench Pro (Scale) — harder, multi-file SWE-bench (1.8k tasks across 41 repos), also fed into the SWE composite
- SWE-rebench — continuously-refreshed agentic SWE leaderboard, rolling-window resolved rate
- LiveCodeBench — competitive-programming pass@1 (ingested for back-compat; *retired* from BUILD weighting after the upstream JSON froze at mid-2025 frontier — see `docs/sources.md`)
- GSO — "Generalized Software Optimization" track from the LiveCodeBench operators; replaces LiveCodeBench in BUILD using the contamination-resistant `score_hack_control` field
- Terminal-Bench 2.0 — agentic terminal task leaderboard
- Sonar Code Quality — functional pass rate plus issue, bug, and vulnerability density (the only public benchmark that measures generated-code quality directly)
- MCP-Atlas (Scale) — real Model Context Protocol tool-orchestration over 36 servers / 220 tools / 1k tasks
- ARC-AGI v2 — novel pattern-induction benchmark from ARC Prize (semi-private track)
- Manual overrides (`data/score_overrides.toml`) — hand-curated vendor-published metric values (SWE-bench Verified, Terminal-Bench, GDPval) for models the public leaderboards have not yet rated

## Math Summary

### Normalization
Each benchmark metric is **percentile-normalized** within the active model population (5th/95th boundaries, log-scaled for cost/speed/latency). Operational metrics (speed/cost/TTFT/context window) use a **tail-penalty** curve instead — top 80 % of the population maps into 70-100 (mild differentiation) and only the bottom 20 % drops sharply, because users perceive operational speed in tiers, not linearly.

### Synthesis Penalty
Values that came in via sibling synthesis (e.g. GLM-5.1 borrowing from Kimi K2.6 on AISL) are blended toward 50 by 15 % so they read as a softer signal than direct measurements: `final = score × 0.85 + 50 × 0.15`. Synthesized metrics still contribute, just slightly more conservatively.

Manual overrides from `data/score_overrides.toml` are also softened after
normalization, but less aggressively: `final = score × 0.90 + 50 × 0.10`.
They are public, cited values, yet still hand-curated rather than directly
ingested leaderboard rows.

### Group Aggregation
Metrics are grouped into **CRE**, **GEN**, **PLAN**, **BUILD**, **LM_ARENA_REVIEW_PROXY**, **OPS_long**, **OPS_precision**, **OPS_review**, and **A_I / A_P / A_B / A_R** (AI Stupid Level perspectives across the 17 AISL capability axes). Each group is a weighted average of its metrics. When a model is missing metrics, the aggregator blends smoothly from shrink-to-50 to trusting the present-weight mean across **60-80 %** group coverage; at **≥80 %** coverage, peripheral missing metrics no longer penalize otherwise well-covered models. AISL canary health is kept outside the groups and can only subtract a small penalty from role scores.

### Final Scores
Each role score is a weighted average of groups. AISL's role-shaped
perspective (`A_*`) carries 0.24 in every formula (down from 0.30 after
the 2026-04 multi-agent review — AISL is one correlated source family,
not a population of independent leaderboards), leaving role-specific
public benchmark groups collectively dominant at 0.68.
Operational metrics (speed, cost, context window) carry 0.08 — paired
with the tail-penalty curve, this means "fast enough" models cluster
within a 1-2 point spread but genuinely slow models lose 4-6 points:
- **I_raw** = 0.46×CRE + 0.22×GEN + 0.24×A_I + 0.08×OPS_long
- **P_raw** = 0.41×PLAN + 0.27×GEN + 0.24×A_P + 0.08×OPS_precision
- **B_raw** = 0.62×BUILD + 0.06×PLAN + 0.24×A_B + 0.08×OPS_precision
- **R** = 0.13×LM_ARENA_REVIEW_PROXY + 0.27×BUILD + 0.28×PLAN + 0.24×A_R + 0.08×OPS_review

### Reviewer-Reservation Penalty
For each vendor **v**, compute the reservation gap:
```
L_v = max(0, max(R_all) - max(R_outside_v))
```
That gap is the *available* penalty budget. Each model **m** in vendor
**v** then pays a share proportional to its own contribution to the lead
(`share_m = clamp((R_m - R_outside_v) / L_v, 0, 1)`, so the actual top-R
model pays the full reservation and siblings tied with the outside max
pay nothing). The per-model penalty is:
```
penalty_m = L_v × share_m
I_adj = I_raw - 0.08 × penalty_m
P_adj = P_raw - 0.18 × penalty_m
B_adj = B_raw - 0.32 × penalty_m
```

This prevents vendors from artificially inflating scores through their
own preference evaluations without taxing every model that happens to
share a vendor with a strong reviewer. See `docs/methodology.md` §6 for
the derivation.

See [`docs/methodology.md`](docs/methodology.md) for the complete mathematical derivation and all coefficients.

## Sample Output (TOML)

```toml
schema_version = "1.0.0"
generated_at = "2026-04-26T11:35:46Z"
generator = "ipbr-rank 0.1.0"
methodology = "v1"

[[models]]
canonical_id = "anthropic/claude-opus-4.7"
display_name = "Claude Opus 4.7"
vendor = "anthropic"
thinking_effort = "default"
aliases = ["opus 4.7", "claude-opus-4-7"]
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
# ...

[models.metrics]
LMArenaText = 82.5
SWEBenchVerified = 76.0
# ...

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
  render           Read scoreboard.toml, write static site to out/site/
  all              fetch -> score -> render (default)
  verify-sources   Run contract tests against live endpoints
  list-models      Emit canonical IDs + vendor from required_aliases.toml
  triage           List unmatched leaderboard rows from the cache (--min-count N)

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
| aistupidlevel | 1h | hourly stupidity dashboard |
| openrouter, lmarena, artificial_analysis | 24h | daily refresh |
| livecodebench, gso | 2d | weekly-ish leaderboard refreshes |
| swebench, swebench_pro, swerebench, terminal_bench, mcp_atlas, arc_agi, sonar | 7d | infrequent updates |

To force a refresh of one source, delete its cache file (or `touch -t` it to
the past) and rerun.

The HTTP layer also retries on `429 Too Many Requests` and `5xx` with
exponential backoff (500 ms → 60 s, up to 6 attempts), honoring `Retry-After`
when present — the HuggingFace datasets-server in particular rate-limits
aggressively while paginating LMArena, so set `HF_TOKEN` when available.

### Offline Mode (for CI/tests)
```bash
# Deterministic golden test against fixtures
ipbr-rank all \
  --offline \
  --cache data/fixtures \
  --out tests/golden/out \
  --now 2026-01-01T00:00:00Z
```

## Overriding Coefficients

```bash
# Copy embedded coefficients
cp data/coefficients.toml my_coefficients.toml

# Edit weights, then:
ipbr-rank all --coefficients my_coefficients.toml

# The effective coefficients are echoed to out/coefficients.toml
```

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
    └── refresh.sh              # One-shot fetch → score → render (+ optional --publish)
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

# Live smoke (best-effort, network-dependent)
cargo test --workspace --features live

# Update golden files (review diff before committing)
UPDATE_GOLDEN=1 cargo test
```

## License

Released under the MIT License — see [`LICENSE`](LICENSE).
