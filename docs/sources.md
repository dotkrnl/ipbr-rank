# Sources

Each source declares a `cache_ttl()` controlling how long its on-disk
cache is considered fresh. With `--cache DIR`, a fetch is skipped when the
cached file's age is under the TTL; `--offline` always reads from cache
regardless. The HTTP layer retries on `429`/`5xx` with exponential backoff
(honoring `Retry-After`), so HuggingFace datasets-server rate limits don't
fail the run.

## openrouter

- **Status**: Verified
- **API**: OpenRouter `/api/v1/models` JSON endpoint
- **Secret**: `OPENROUTER_API_KEY` (via `--openrouter-api-key-file` or environment variable)
- **Cache TTL**: 24 h
- **Fixture**: `data/fixtures/openrouter_models.json`

## lmarena

- **Status**: Verified
- **API**: LMArena leaderboard via HuggingFace datasets-server `/rows` (paginated, configs `text`, `webdev`, `search`, `document`)
- **Secret**: None (HF token recommended to avoid 429s)
- **Cache TTL**: 24 h
- **Fixture**: `data/fixtures/lmarena_overall.json`

## artificial_analysis

- **Status**: Verified
- **API**: Artificial Analysis `/api/v2/data/llms/models`, `x-api-key` header
- **Secret**: `AA_API_KEY` (via `--aa-api-key-file` or environment variable)
- **Cache TTL**: 24 h
- **Metrics emitted**: `ArtificialAnalysisIntelligence`, `ArtificialAnalysisCoding`, `ArtificialAnalysisReasoning` (gpqa+hle blend), `GPQA_HLE_Reasoning` (same blend, different group), `LiveCodeBench` fallback, `Tau2Bench`, `SciCode`, `IFBench`, `LongContextRecall` (lcr), and the operational metrics `OutputSpeed` / `InverseTTFT` / `InverseCost`.
- **Multi-row dedup**: AA ships several rows per logical model (e.g. "Claude Opus 4.7 (Adaptive Reasoning, Max Effort)" and "(Non-reasoning, High Effort)"). The fetcher sorts ascending by intelligence index so the highest-effort row appears last and wins the last-write merge; speed/ttft sentinel zeros are skipped.
- **Fixture**: `data/fixtures/artificial_analysis_llms.json`

## aistupidlevel

- **Status**: Verified
- **API**: `/api/dashboard/cached` (primary), `/dashboard/cached` as fallback. Both return the same payload shape; the legacy `/api/dashboard` endpoint now returns 404.
- **Schema**: `data.modelScores[]` for the model list, `data.historyMap[id][]` for per-model time series. AISL runs three suites — `hourly`, `deep`, `tooling` — that each contribute different axes. The fetcher walks `historyMap` in array order (newest-first per upstream `timestamp`) and takes the first numeric value per axis, merging across suites so models carry their full axis surface. The `contextWindow` axis is dropped (overlaps OpenRouter's ContextWindow).
- **`hallucinationRate` is NOT re-inverted**: upstream's `calculateHallucinationRate` already returns `Math.max(0, 1 - rate)` — the field name is misleading but the value is already a resistance score (higher = better). We pass it through as `AI_hallucination_resistance` directly. Earlier revisions did `1 - value` here and produced wildly wrong rankings; see commit history for the diagnosis.
- **`errorHandling` is dropped**: upstream defines it as `recoveredFromErrors / failedCalls.length`, with the `failedCalls = 0` branch returning `0` instead of `1`. A model that never fails gets the same score as one that fails everything and recovers nothing — an upstream measurement quirk that the metric definition can't recover from. The freed weight in `A_R` was reabsorbed into `AI_recovery`.
- **Secret**: None
- **Cache TTL**: 1 h
- **Axes emitted (17 total)**:
  - **Hourly suite (9)**: `AI_correctness`, `AI_spec` (`format`), `AI_code` (`codeQuality`), `AI_efficiency`, `AI_stability`, `AI_refusal` (`safety`), `AI_recovery` (`debugging`), `AI_complexity`, `AI_edge_cases`
  - **Deep suite (3)**: `AI_plan_coherence`, `AI_memory_retention`, `AI_hallucination_resistance`
  - **Tooling suite (5)**: `AI_context_awareness`, `AI_task_completion`, `AI_tool_selection`, `AI_parameter_accuracy`, `AI_safety_compliance`
- **Fixture**: `data/fixtures/aistupidlevel_dashboard.json`

## openevals (removed)

Removed because of zero overlap with the flagship model set — none of the 14 required
canonical IDs appeared in its leaderboard — so it contributed no coverage while adding
fetch latency.

## bigcodebench, bfcl, aider_polyglot, metr_horizons (removed)

All four were removed during audit passes. `bigcodebench` (HuggingFace
dataset) stopped covering 2026-class models; `bfcl` and `aider_polyglot`
upstreams went stale on the frontier; `metr_horizons` produced sparse
measurements that warped scores. Code-axis coverage is now provided by
`swebench` (Verified + Multilingual), `swerebench`, `livecodebench`,
`terminal_bench`, `sonar`, and `artificial_analysis` (which surfaces
`tau2-bench`, `scicode`, `ifbench`, and the gpqa+hle reasoning blend
from its existing payload). Long-context and tool-use signal comes from
the AISL deep + tooling suites instead.

## swebench

- **Status**: Verified
- **API**: SWE-bench leaderboards JSON (raw GitHub Pages source from `swe-bench/swe-bench.github.io`)
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `SWEBenchVerified` (Verified leaderboard), `SWEBenchMultilingual` (multilingual leaderboard, 9 languages incl. C/C++/Go/Java/JS/PHP/Ruby/Rust). Single fetch covers both — no extra HTTP cost.
- **Fixture**: `data/fixtures/swebench_leaderboards.json`

## terminal_bench

- **Status**: Verified
- **API**: Terminal-Bench 2.0 HTML leaderboard page
- **Secret**: None
- **Cache TTL**: 7 d
- **Fixture**: `data/fixtures/terminal_bench.html`

## livecodebench

- **Status**: Verified
- **API**: LiveCodeBench `performances_generation.json` (fetched from `livecodebench.github.io`)
- **Secret**: None
- **Cache TTL**: 2 d
- **Fixture**: `data/fixtures/livecodebench.json`

## swerebench

- **Status**: Verified
- **API**: `swe-rebench.com` HTML page (Next.js server-rendered React Server Component blob; we extract the embedded `"items":[…]` array, unescape it, and parse with serde_json).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `SWERebench` — resolved-rate over each model's full observation window. Prefers the `tools` (agentic) variant per model and falls back to `text`. Continuously-refreshed via a rolling window of post-release GitHub PRs, which removes contamination concerns vs. static SWE-bench.
- **Fragility note**: Depends on the embedded RSC payload format. If the site switches to client-side hydration or renames `items`/`modelName`/`rangeStats`/`taskRangeTimestamp`, the parser will need updating.
- **Fixture**: `data/fixtures/swerebench.html`

## swebench_pro

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/swe_bench_pro_public` (Next.js page; data is embedded in the streamed React Server Component chunks as `\"model\":\"…\",\"score\":N`).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `SWEBenchPro` — feeds the `SWEComposite` derived metric alongside `SWERebench`, `SWEBenchVerified`, and `SWEBenchMultilingual`. Frontier models top out near 60-65% (vs Verified saturating near 90), so it differentiates better at the top of the leaderboard. 1,865 multi-file tasks across 41 actively-maintained Python/Go/TypeScript/JavaScript repos; average edit is 107 LOC across 4.1 files.
- **Fragility note**: Depends on Scale's RSC embedding. If field names change (`model` → `name`, `score` → `passRate`), the parser will need updating.
- **Fixture**: `data/fixtures/swebench_pro.html`

## mcp_atlas

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/mcp_atlas` (same RSC pattern as `swebench_pro` — they share a parser).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `MCPAtlas` — pass rate over 1,000 tasks across 36 real Model Context Protocol servers / 220 tools. Each task asks the agent to identify the right servers, sequence 3-6 tool calls across multiple servers, and produce a correct end-state. Closest public proxy for "real Claude Code / Codex tool-use loops" we can ingest. Feeds both `PLAN` (multi-step tool sequencing) and `BUILD` (real coding agents *are* tool-orchestration loops).
- **Coverage**: 19 models, all 14 flagships matched directly (opus-4.7 max=79.1%, gemini-3.1-pro=78.2%, glm-5.1=75.6%, gpt-5.4=70.6%, …, haiku-4.5=40.2%).
- **Fragility note**: Same as `swebench_pro` — RSC field names.
- **Fixture**: `data/fixtures/mcp_atlas.html`

## arc_agi

- **Status**: Verified
- **API**: ARC Prize static JSON — `arcprize.org/media/data/models.json` + `evaluations.json`. Combined into one cached payload `{models, evaluations}`.
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `ARC_AGI_2` — score on the **v2_Semi_Private** track (contamination-controlled). Scores are 0-1 in the JSON; rescaled to 0-100 to align with the rest of the metric population. The other tracks (Public, Private) are skipped because Public is leaky and Private is closed to most of our flagships.
- **Why we ingest it**: ARC-AGI v2 is the only public benchmark that explicitly tests *novel pattern induction* — every task is unfamiliar at evaluation time. Orthogonal to GPQA/HLE which test learned knowledge. Frontier models sit around 75-85% while humans top out at 100%, so it discriminates well at the very top.
- **Fragility note**: Depends on the static JSON URLs the leaderboard's bundled JS fetches. If ARC Prize moves the data-pack path, the constants need updating.
- **Fixture**: `data/fixtures/arc_agi.json`

## sonar

- **Status**: Verified
- **API**: `sonarsource.com/.../leaderboard/data.json` — Vite SPA backed by a static JSON file; no auth, no rate limit.
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `SonarFunctionalSkill` (pass rate, higher better) and `SonarIssueDensity` (issues per kLOC, lower better, flipped via `higher_better = false`). Sonar is the only public benchmark in our portfolio that measures generated-code quality directly (issue density, vuln density, bug density, complexity) instead of just pass rate.
- **Coverage**: 58 models including all 2026 frontier — Opus 4.5/4.6/4.7 Thinking, GPT-5.1/5.2/5.3/5.4/5.5 variants, Gemini 3 Pro/Flash/3.1 Pro, GLM-5, Kimi K2 Thinking. 13 of 14 flagships matched directly via existing aliases.
- **Fixture**: `data/fixtures/sonar.json`

## overrides

- **Status**: Verified
- **API**: None — reads `data/score_overrides.toml` (embedded into the binary at build time).
- **Purpose**: Hand-curated metric values pulled from vendor system cards, launch posts, and other authoritative secondary sources. Fills coverage gaps for models that public leaderboards have not yet rated (typically newest frontier models — e.g. Claude Opus 4.7 SWE-bench Verified, GPT-5.5 Terminal-Bench 2.0).
- **Discipline**: Every entry MUST cite its source in the `note` field; values without citations are explicitly disallowed by code review.
- **Precedence**: Overrides flow through the same ingest path as live sources, so a vendor-reported override carries the same weight as a leaderboard hit. If a public source later lands the same metric for the same model, the public value will overwrite the override on the next run.

## Synthesis

`data/synthesis_aliases.toml` lists sibling-substitution pairs. For every
pair `(target, from)` and every source `S`, a synthesized RawRow is
emitted carrying the donor (`from`) row's fields, tagged
`synthesized_from = "<from>"`.

**Field-level fill, not row-level replace.** The ingest layer
(`ingest_synthesized_row` in `crates/core/src/ingest.rs`) skips any field
that the target already has a real value for. So a model with partial
real coverage from a source — e.g. AISL's hourly-suite axes for a
freshly-released model that hasn't been deep+tooling-evaluated yet —
keeps its real values, and synthesis fills only the genuinely missing
fields. Synthesis is the last-priority signal: real values always win.

The synthesis layer respects per-source caps (default 30 %) so a single
donor can't dominate a model's signal across an entire source.

After per-metric normalization, fields that came in via synthesis are
pulled toward 50 by 15 % (the **synthesis penalty**, see methodology
§3.4) so they read as a softer signal than direct measurements.

Active pairs:
- `openai/gpt-5.5 → openai/gpt-5.4` (forward sibling)
- `google/gemini-3.1-pro-preview → google/gemini-3-pro` (size tier)
- `google/gemini-3-flash → google/gemini-3-pro` (same generation)
- `openai/gpt-5.3-codex → openai/gpt-5.4` (sibling)
- `anthropic/claude-sonnet-4 → anthropic/claude-sonnet-4.5` (forward)
- `z-ai/glm-5.1 ↔ moonshotai/kimi-k2.6` (symmetric, same capability tier)
