# ipbr-rank — Public LLM Building-Role Scoreboard

A Rust workspace that fetches public LLM benchmarks from verified sources, normalizes them, computes four building-role scores (Idea, Planning, Building, Reviewing), and emits a canonical TOML scoreboard plus a beautifully rendered static website.

## Quick Start

```bash
# Install
cargo build --release

# Run with all verified sources (requires AA_API_KEY)
export AA_API_KEY=your_key_here
./target/release/ipbr-rank all

# Output: out/scoreboard.toml, out/missing.toml, out/coefficients.toml, out/site/index.html
```

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
- AI Stupid Level — 17 axes across hourly + deep + tooling suites (correctness, codeQuality, planCoherence, hallucinationResistance, taskCompletion, contextAwareness, etc.; tooling-suite errorHandling dropped due to upstream measurement quirk — see `docs/sources.md` AISL entry)
- SWE-bench JSON — Verified + Multilingual leaderboards (single fetch, both fed into the SWE composite)
- SWE-bench Pro (Scale) — harder, multi-file SWE-bench (1.8k tasks across 41 repos), also fed into the SWE composite
- SWE-rebench — continuously-refreshed agentic SWE leaderboard, rolling-window resolved rate
- LiveCodeBench — competitive-programming pass@1
- Terminal-Bench 2.0 — agentic terminal task leaderboard
- Sonar Code Quality — issue density, vulnerability density, functional pass rate (the only public benchmark that measures generated-code quality directly)
- MCP-Atlas (Scale) — real Model Context Protocol tool-orchestration over 36 servers / 220 tools / 1k tasks
- ARC-AGI v2 — novel pattern-induction benchmark from ARC Prize (semi-private track)
- Manual overrides (`data/score_overrides.toml`) — hand-curated vendor-published metric values (SWE-bench Verified, Terminal-Bench, GDPval) for models the public leaderboards have not yet rated

## Math Summary

### Normalization
Each benchmark metric is **percentile-normalized** within the active model population (5th/95th boundaries, log-scaled for cost/speed/latency). Operational metrics (speed/cost/TTFT/context window) use a **tail-penalty** curve instead — top 80 % of the population maps into 70-100 (mild differentiation) and only the bottom 20 % drops sharply, because users perceive operational speed in tiers, not linearly.

### Synthesis Penalty
Values that came in via sibling synthesis (e.g. GLM-5.1 borrowing from Kimi K2.6 on AISL) are blended toward 50 by 15 % so they read as a softer signal than direct measurements: `final = score × 0.85 + 50 × 0.15`. Synthesized metrics still contribute, just slightly more conservatively.

### Group Aggregation
Metrics are grouped into **CRE**, **GEN**, **PLAN**, **BUILD**, **JUDGE**, **OPS_long**, **OPS_precision**, **OPS_review**, and **A_I / A_P / A_B / A_R** (AI Stupid Level perspectives across the 17 AISL axes). Each group is a weighted average of its metrics. When a model is missing metrics, the aggregator uses the present-weight mean directly if **≥70 %** of the group's weight is present (so peripheral missing metrics don't penalize otherwise well-covered models); below that threshold, the score still shrinks toward 50 proportional to the missing weight.

### Final Scores
Each role score is a weighted average of groups. AISL's role-shaped
perspective (`A_*`) is the dominant signal at 0.50 in every formula.
Operational metrics (speed, cost, context window) carry 0.08 — paired
with the tail-penalty curve, this means "fast enough" models cluster
within a 1-2 point spread but genuinely slow models lose 4-6 points:
- **I_raw** = 0.30×CRE + 0.12×GEN + 0.50×A_I + 0.08×OPS_long
- **P_raw** = 0.25×PLAN + 0.17×GEN + 0.50×A_P + 0.08×OPS_precision
- **B_raw** = 0.40×BUILD + 0.02×PLAN + 0.50×A_B + 0.08×OPS_precision
- **R** = 0.20×JUDGE + 0.12×BUILD + 0.10×PLAN + 0.50×A_R + 0.08×OPS_review

### Reviewer-Reservation Penalty
For each vendor **v**, compute:
```
L_v = max(0, max(R_all) - max(R_outside_v))
```
Then apply penalties:
```
I_adj = I_raw - 0.08 × L_v
P_adj = P_raw - 0.18 × L_v
B_adj = B_raw - 0.32 × L_v
```

This prevents vendors from artificially inflating scores through their own preference evaluations.

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
| openrouter, lmarena, artificial_analysis, openevals | 24h | daily refresh |
| livecodebench | 2d | weekly contests |
| swebench, bfcl, terminal_bench, aider_polyglot | 7d | infrequent updates |

To force a refresh of one source, delete its cache file (or `touch -t` it to
the past) and rerun.

The HTTP layer also retries on `429 Too Many Requests` and `5xx` with
exponential backoff (500 ms → 60 s, up to 6 attempts), honoring `Retry-After`
when present — the HuggingFace datasets-server in particular rate-limits
aggressively while paginating LMArena/OpenEvals.

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
└── templates/                  # Tera templates for the static site
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

Not currently open-source. Internal use only.
