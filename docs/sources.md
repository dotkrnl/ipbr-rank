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
- **Secret**: None required; if `HF_TOKEN` is set, the fetcher sends it as a HuggingFace bearer token to reduce datasets-server 429s.
- **Cache TTL**: 24 h
- **Metrics emitted**: `LMArenaText`, `LMArenaCreativeOrOpenEnded`, `CopilotArenaOrLMArenaCode`, `LMArenaSearch`, and `LMArenaDocument`. Search and document stay separate because their raw Elo scales are not comparable; `LM_ARENA_REVIEW_PROXY` combines them after normalization.
- **429 handling**: The HuggingFace datasets-server rate-limits aggressively on deep pagination. The fetcher sleeps 5 s between successful pages, writes `lmarena_overall.partial.json` after every page, resumes from that partial file on the next run, and only promotes a complete payload to `lmarena_overall.json`. If a stale full cache exists and live refresh still fails, scoring falls back to the stale full cache.
- **Fixture**: `data/fixtures/lmarena_overall.json`

## artificial_analysis

- **Status**: Verified
- **API**: Artificial Analysis `/api/v2/data/llms/models`, `x-api-key` header
- **Secret**: `AA_API_KEY` (via `--aa-api-key-file` or environment variable)
- **Cache TTL**: 24 h
- **Metrics emitted**: `ArtificialAnalysisIntelligence`, `ArtificialAnalysisCoding`, `ArtificialAnalysisReasoning` (gpqa+hle blend), `GPQA_HLE_Reasoning` (same blend, different group), `AIME25`, `Tau2Bench`, `TauBanking`, `SciCode`, `IFBench`, `TerminalBenchHard`, `AATerminalBench21`, `AALiveCodeBench`, `ArtificialAnalysisMath`, `MMLUPro`, `LongContextRecall` (lcr), and the operational metrics `OutputSpeed` / `TTFT` / `BlendedCost`.
- **Multi-row dedup**: AA ships several rows per logical model (e.g. "Claude Opus 4.7 (Adaptive Reasoning, Max Effort)" and "(Non-reasoning, High Effort)"). The fetcher sorts ascending by intelligence index so the highest-effort row appears last and wins the last-write merge; speed/ttft sentinel zeros are skipped.
- **DeepSeek merge**: The DeepSeek API routes both `deepseek-chat` and `deepseek-reasoner` to the same underlying model (thinking on vs. off), so both alias into `deepseek/deepseek-v4-flash` (`data/required_aliases.toml`).
- **Fixture**: `data/fixtures/artificial_analysis_llms.json`

## aistupidlevel

Removed from active scoring on 2026-05-21. We reproduced the benchmark
locally and found the tasks not representative enough of real model quality
and too noise-prone for the role scores. The source implementation and
fixture remain in the repo for audit/history, but `AiStupidLevelSource` is
not registered, the `AI_*` metrics are no longer in `data/coefficients.toml`,
the `A_*` perspective groups are gone, and the canary-health penalty is no
longer applied.

## openevals (removed)

Removed because of zero overlap with the flagship model set — none of the 14 required
canonical IDs appeared in its leaderboard — so it contributed no coverage while adding
fetch latency.

## bigcodebench, aider_polyglot, metr_horizons (removed)

These were removed during audit passes. `bigcodebench` (HuggingFace
dataset) stopped covering 2026-class models; `aider_polyglot` went stale
on the frontier; `metr_horizons` produced sparse measurements that warped
scores. BFCL was restored once the V4 CSV exposed current frontier rows
and agentic tool-use categories.

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
- **Metric**: `TerminalBench`
- **Fixture**: `data/fixtures/terminal_bench.html`

## terminal_bench_2_1

- **Status**: Verified
- **API**: Terminal-Bench 2.1 HTML leaderboard page
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `TerminalBench21` — newer, narrower Terminal-Bench track with current frontier agent/model combinations. The source canonicalizes duplicate agent rows down to one row per matched model. Scoring consumes it through `TerminalBench21Composite` together with AA's `AATerminalBench21`.
- **Fixture**: `data/fixtures/terminal_bench_2_1.html`

## livecodebench

- **Status**: Verified (retired from BUILD weighting — see GSO below)
- **API**: LiveCodeBench `performances_generation.json` (fetched from `livecodebench.github.io`)
- **Secret**: None
- **Cache TTL**: 2 d
- **Fixture**: `data/fixtures/livecodebench.json`
- **Note**: The upstream JSON has been frozen at mid-2025 frontier (latest entries are Claude-Opus-4 / Claude-Sonnet-4 / Gemini-2.5-Pro; no GPT-5/5.x, no Opus 4.5+, no Gemini 3, no Kimi K2.x, no DeepSeek V4) for ~12 months. The metric is still ingested for backwards-compat and historical reference but `groups = []` removes it from any role-score weighting. Successor: `gso`.

## gso

- **Status**: Verified
- **API**: GSO ("Generalized Software Optimization") leaderboard JSON at `gso-bench.github.io/assets/leaderboard.json`. Same operators as LiveCodeBench's leaderboard SPA but actively accepting frontier submissions where LiveCodeBench has not since mid-2025.
- **Secret**: None
- **Cache TTL**: 2 d
- **Fixture**: `data/fixtures/gso.json`
- **Metric**: `GSO` — pass rate over 102 software-optimization tasks. We ingest the `score_hack_control` field (GSO's contamination-resistant variant, added explicitly to penalize deceptive optimizations) rather than raw `score`, mirroring our portfolio's bias toward contamination-resistant signals (cf. SWERebench preferred over SWE-Bench Verified). Filtered to `setting == "Opt@1"`. Among Opt@1 rows for the same model we keep the one with the lowest `reasoning_effort` so the variant policy (medium/thinking/adaptive only) is honored where possible — with the documented carve-out that GSO publishes only `-high` rows for some frontier models, which we accept rather than synthesize.

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

## sweatlas

- **Status**: Verified
- **APIs**: Scale Labs `labs.scale.com/leaderboard/sweatlas-qna`, `/sweatlas-tw`, and `/sweatlas-refactoring` (same RSC pattern as `swebench_pro` / `mcp_atlas`).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `SWEAtlasQnA`, `SWEAtlasTestWriting`, `SWEAtlasRefactoring`. These feed the derived `SWEAtlasComposite` BUILD input so the three correlated Scale SWE Atlas tracks do not stack as independent BUILD weights.
- **Fixtures**: `data/fixtures/sweatlas_qna.html`, `data/fixtures/sweatlas_test_writing.html`, `data/fixtures/sweatlas_refactoring.html`

## mcp_atlas

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/mcp_atlas` (same RSC pattern as `swebench_pro` — they share a parser).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `MCPAtlas` — pass rate over 1,000 tasks across 36 real Model Context Protocol servers / 220 tools. Each task asks the agent to identify the right servers, sequence 3-6 tool calls across multiple servers, and produce a correct end-state. Closest public proxy for "real Claude Code / Codex tool-use loops" we can ingest. Feeds both `PLAN` (multi-step tool sequencing) and `BUILD` (real coding agents *are* tool-orchestration loops).
- **Coverage**: 19 models, all 14 flagships matched directly (opus-4.7 max=79.1%, gemini-3.1-pro=78.2%, glm-5.1=75.6%, gpt-5.4=70.6%, …, haiku-4.5=40.2%).
- **Fragility note**: Same as `swebench_pro` — RSC field names.
- **Fixture**: `data/fixtures/mcp_atlas.html`

## hil_bench

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/hil` (same RSC pattern as `mcp_atlas`).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `HiLBench` — human-in-the-loop escalation accuracy. It measures whether an agent recognizes ambiguous or blocked tasks and asks targeted human questions instead of guessing, so it feeds PLAN with a smaller BUILD contribution.
- **Fixture**: `data/fixtures/hil_bench.html`

## bfcl

- **Status**: Verified
- **API**: Berkeley Function Calling Leaderboard V4 `data_overall.csv`
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `BFCL` plus category splits `BFCLNonLiveAST`, `BFCLLive`, `BFCLMultiTurn`, `BFCLWebSearch`, `BFCLMemory`, `BFCLRelevanceDetection`, and `BFCLIrrelevanceDetection`. Scoring consumes these through `BFCLComposite`, with the overall leaderboard score kept as the anchor.
- **Fixture**: `data/fixtures/bfcl.csv`

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
- **API**: `sonarsource.com/.../leaderboard/data/models.json` plus per-model metrics JSON files under `leaderboard/data/<org>/...`; no auth, no rate limit. Older snapshots used a single flat `leaderboard/data.json`, and the fetcher still reads that cached shape for back-compat.
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `SonarFunctionalSkill` (pass rate, higher better), `SonarIssueDensity` (issues per kLOC, lower better), `SonarBugDensity` (bugs per kLOC, lower better), and `SonarVulnerabilityDensity` (vulnerabilities per kLOC, lower better). Lower-is-better metrics are flipped via `higher_better = false`. Sonar is the only public benchmark in our portfolio that measures generated-code quality directly instead of just pass rate.
- **Coverage**: 70 Java rows in the 2026-05-21 live payload, including Opus 4.5/4.6/4.7 Thinking/High variants, GPT-5.2/5.3-Codex/5.4/5.5 variants, Gemini 3 Pro/Flash/3.1 Pro, GLM-5, Kimi K2.5, and MiniMax M2.5/M2.7.
- **Fixture**: `data/fixtures/sonar.json`

## overrides

- **Status**: Verified
- **API**: None — reads `data/score_overrides.toml` (embedded into the binary at build time).
- **Purpose**: Hand-curated metric values pulled from vendor system cards, launch posts, and other authoritative secondary sources. Fills coverage gaps for models that public leaderboards have not yet rated (typically newest frontier models — e.g. Claude Opus 4.7 SWE-bench Verified, GPT-5.5 Terminal-Bench 2.0, Kimi K2.7 Code launch-card metrics).
- **Discipline**: Every entry MUST cite its source in the `note` field; values without citations are explicitly disallowed by code review.
- **Precedence**: Overrides flow through the same ingest path as live sources. They are excluded from direct-source normalization baselines when enough direct data exists, but they no longer receive a post-normalization discount. If a public source later lands the same metric for the same model, the public value overwrites the override on the next run.

## Synthesis

`data/synthesis_aliases.toml` lists sibling-substitution pairs. For every
pair `(target, from)` and every source `S`, a synthesized RawRow is
emitted carrying the donor (`from`) row's fields, tagged
`synthesized_from = "<from>"`.

**Field-level fill, not row-level replace.** The ingest layer
(`ingest_synthesized_row` in `crates/core/src/ingest.rs`) skips any field
that the target already has a real value for. So a model with partial
real coverage from a source keeps its real values, and synthesis fills
only the genuinely missing fields. Synthesis is the last-priority signal:
real values always win.

The synthesis layer respects per-source caps (configured at 65 %) so a single
donor can't dominate a model's signal across an entire source. Individual
pairs can also be source-scoped; the Terminal-Bench 2.1 / HiL-Bench gap-fills
are restricted to those two narrow new sources.

After per-metric normalization, conservative fields that came in via synthesis
are pulled toward 50 by 15 % (the **synthesis penalty**, see methodology
§3.4) so they read as a softer signal than direct measurements. Same-vendor,
same-series version-advance fills are marked `category = "same_series_forward"`
in `data/synthesis_aliases.toml` and carry no synthesis penalty. Cross-vendor,
cross-series, and older-target-from-newer-donor fills remain
`category = "conservative"`.

Active pairs (target ← donor; see `data/synthesis_aliases.toml` for the
authoritative list with rationale comments):

- New-source scoped: `gpt-5.5-pro ← gpt-5.5`, `gpt-5.4-pro ← gpt-5.5`, `gpt-5.4-mini ← gpt-5.5`, `gpt-5.3-codex ← gpt-5.5`, `deepseek-v4-flash ← glm-5.1`, `deepseek-v3.2 ← deepseek-v4-flash`, `gemini-3.5-flash ← gemini-3.1-pro-preview`, `gemini-3.1-flash-lite ← gemini-3.5-flash`, `qwen3.6-plus ← glm-5.1`, `qwen3.5-397b-a17b ← qwen3.6-plus`, `qwen3.6-max-preview ← qwen3.5-397b-a17b` (Terminal-Bench 2.1 / HiL-Bench only)
- OpenAI: `gpt-5.3-codex ← gpt-5.4`, `gpt-5.2-codex ← gpt-5.3-codex`, `gpt-5.2 ← gpt-5.4`, `gpt-5.4 ← gpt-5.3-codex`, `gpt-5.4-pro ← gpt-5.4`, `gpt-5.4-mini ← gpt-5.4`, `gpt-5.5-pro ← gpt-5.4-pro`
- Anthropic: `claude-opus-4.7 ← claude-opus-4.6`, `claude-opus-4.5 ← claude-opus-4.6`, `claude-sonnet-4.6 ← claude-sonnet-4.5`, `claude-sonnet-4 ← claude-sonnet-4.5`
- Google: `gemini-3.1-pro-preview ← gemini-3-pro`, `gemini-3.5-flash ← gemini-3-flash`, `gemini-2.5-pro ← gemini-3-pro`, `gemini-2.5-flash ← gemini-3-flash`
- z.ai / Moonshot / Qwen: `z-ai/glm-5.1 ← moonshotai/kimi-k2.6`, `z-ai/glm-5.2 ← z-ai/glm-5.1`, `moonshotai/kimi-k2.7-code ← moonshotai/kimi-k2.6`, `moonshotai/kimi-k2.5 ← moonshotai/kimi-k2.6`, `z-ai/glm-4.6 ← z-ai/glm-4.7`, `z-ai/glm-5 ← z-ai/glm-5.1`, `qwen/qwen3.6-plus ← z-ai/glm-5`, `qwen/qwen3.7-max ← qwen/qwen3.6-plus`
- DeepSeek / Xiaomi / MiniMax: `deepseek/deepseek-v4-flash ← moonshotai/kimi-k2.6`, `deepseek/deepseek-v4-pro ← deepseek/deepseek-v4-flash`, `xiaomi/mimo-v2.5-pro ← moonshotai/kimi-k2.5`, `xiaomi/mimo-v2.5 ← xiaomi/mimo-v2.5-pro`, `minimax/minimax-m2.5 ← moonshotai/kimi-k2.5`, `minimax/minimax-m2.7 ← minimax/minimax-m2.5`
- xAI: `xai/grok-code-fast-1 ← xai/grok-4-latest`, `xai/grok-4.3 ← xai/grok-4-latest`
- Meta: `meta/muse-spark ← google/gemini-3.5-flash`
