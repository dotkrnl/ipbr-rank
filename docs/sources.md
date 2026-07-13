# Sources

Each source declares a `cache_ttl()` controlling how long its on-disk
cache is considered fresh. With `--cache DIR`, a fetch is skipped when the
cached file's age is under the TTL; `--offline` always reads from cache
regardless. The HTTP layer retries on `429`/`5xx` with exponential backoff
(honoring `Retry-After`), so HuggingFace datasets-server rate limits don't
fail the run.

Ingestion produces one canonical record per model under the
`best_available_max_effort` policy. Within a benchmark, max/xhigh, high,
thinking/adaptive, medium, then default observations are preferred. The
winning metric keeps its source and evidence class; direct observations take
precedence over cited reported overrides, which take precedence over sibling
synthesis.

Methodology v3 also separates score use from eligibility use. `core` metrics
are broad enough to establish current ranking coverage. `supplemental` metrics
still affect a role score when observed, but a missing row on their narrow
leaderboards does not count against eligibility. `historical_support` metrics
are retired from scoring and can only corroborate that an established model
has direct role-relevant evidence. Operational and other diagnostic metrics
affect neither scores nor eligibility.

## openrouter

- **Status**: Verified
- **API**: OpenRouter `/api/v1/models` JSON endpoint
- **Secret**: `OPENROUTER_API_KEY` (via `--openrouter-api-key-file` or environment variable)
- **Cache TTL**: 24 h
- **Metrics emitted**: pricing, provider routing, advertised context, and supported-parameter fields.
- **Ranking use**: Diagnostic only. Pricing and context metadata have zero path to a methodology-v3 capability rank.
- **Fixture**: `data/fixtures/openrouter_models.json`

## lmarena

- **Status**: Verified
- **API**: LMArena leaderboard via HuggingFace datasets-server `/rows` (paginated, configs `text`, `webdev`, `search`, `document`)
- **Secret**: None required; if `HF_TOKEN` is set, the fetcher sends it as a HuggingFace bearer token to reduce datasets-server 429s.
- **Cache TTL**: 24 h
- **Metrics emitted**: `LMArenaText`, `CopilotArenaOrLMArenaCode`, `LMArenaSearch`, and `LMArenaDocument`. Search and document stay separate because their raw Elo scales are not comparable; `LM_ARENA_REVIEW_PROXY` combines them after normalization. Text Elo is not copied into a fake creativity field.
- **429 handling**: The HuggingFace datasets-server rate-limits aggressively on deep pagination. The fetcher sleeps 5 s between successful pages, writes `lmarena_overall.partial.json` after every page, resumes from that partial file on the next run, and only promotes a complete payload to `lmarena_overall.json`. If a stale full cache exists and live refresh still fails, scoring falls back to the stale full cache.
- **Fixture**: `data/fixtures/lmarena_overall.json`

## eqbench_creative_writing

- **Status**: Verified
- **API**: EQ-Bench Creative Writing v3 JavaScript asset; the source extracts the `leaderboardDataCreativeWritingV3` embedded CSV.
- **Secret**: None
- **Cache TTL**: 24 h
- **Metric**: `EQBenchCreativeWriting` — raw Glicko-derived Elo from the current Creative Writing v3 leaderboard.
- **Deduplication**: Leading source markers are removed and the best row per canonical model is retained.
- **Fixture**: `data/fixtures/eqbench_creative_writing_v3.js`

## eqbench_judgemark

- **Status**: Verified
- **API**: EQ-Bench Judgemark v4 JavaScript asset; the source extracts the `leaderboardDataJudgemarkV4` embedded CSV.
- **Secret**: None
- **Cache TTL**: 24 h
- **Metric**: `EQBenchJudgemark` — judge-discrimination/separability score, scaled from 0–1 to 0–100.
- **Uncertainty**: `EQBenchJudgemarkCILow` and `EQBenchJudgemarkCIHigh` preserve the prompt-bootstrap 95% interval. They are unscored and never synthesized.
- **Ranking use**: Diagnostic. Judge discrimination is not direct code review, so methodology v3 removes it from the Review score and eligibility portfolio.
- **Fixture**: `data/fixtures/eqbench_judgemark_v4.js`

## artificial_analysis

- **Status**: Verified
- **API**: Artificial Analysis `/api/v2/data/llms/models`, `x-api-key` header
- **Secret**: `AA_API_KEY` (via `--aa-api-key-file` or environment variable)
- **Cache TTL**: 10 min
- **Metrics emitted**: current `GPQA`, `HLE`, `Tau3Banking`, `SciCode`, `AATerminalBench21`, and `LongContextRecall`, plus aggregate, legacy, operational, and pricing diagnostics. The upstream `tau_banking` payload key is emitted under the current tau3 semantic name; `Tau2Bench`, `TauBanking`, `IFBench`, and `TerminalBenchHard` remain historical diagnostics.
- **Eligibility use**: Current general-reasoning, SciCode, tau3, and long-context leaves are core evidence in their respective roles. Stable retired task metrics may be marked `historical_support`; their values never enter methodology-v3 scores.
- **Duplicate correction**: current AA general/scientific leaves feed `AAGeneralComposite` once. Aggregate Intelligence/Coding indices do not stack with their components.
- **Multi-row dedup**: AA ships several rows per logical model. The parser preserves distinct effort rows and ingestion selects the strongest eligible effort; equal-effort duplicate rows keep the highest intelligence observation. Speed/TTFT sentinel zeros are skipped.
- **Product tiers**: reasoning labels such as `xhigh` and `max` are eligible configurations of a canonical model. Separately named products such as `GPT-5.5 Pro` are not silently folded into the base `GPT-5.5` record; they require their own catalog entry before ranking.
- **Model-only guard**: upstream rows whose labels explicitly disclose a fallback model are excluded from the pure model API source.
- **Ranking use**: Output speed, TTFT, price, and blended cost are diagnostics only.
- **DeepSeek merge**: The DeepSeek API routes both `deepseek-chat` and `deepseek-reasoner` to the same underlying model (thinking on vs. off), so both alias into `deepseek/deepseek-v4-flash` (`data/required_aliases.toml`).
- **Fixture**: `data/fixtures/artificial_analysis_llms.json`

The five Artificial Analysis evaluation-page sources below parse official
server-rendered model objects, need no secret, cache for 24 hours, and rename
fallback-assisted observations to unranked `*HybridFallback` diagnostics.

## aa_gdpval_v2

- **Status**: Verified
- **Metric**: `GDPvalAA2`, a core Plan signal; published confidence bounds are diagnostic.
- **Fixture**: `data/fixtures/aa_gdpval_v2.html`

## aa_critpt

- **Status**: Verified
- **Metric**: `CritPt`, a core input to `AAGeneralComposite`.
- **Fixture**: `data/fixtures/aa_critpt.html`

## aa_omniscience

- **Status**: Verified
- **Metrics**: `AAOmniscienceAccuracy` and `AAOmniscienceNonHallucination`, core inputs to `AAGeneralComposite`; the upstream combined index is diagnostic.
- **Fixture**: `data/fixtures/aa_omniscience.html`

## aa_enterprise_ops_gym

- **Status**: Verified
- **Metric**: `EnterpriseOpsGymAA`, a supplemental input to the correlated enterprise-workflow Plan composite.
- **Fixture**: `data/fixtures/aa_enterprise_ops_gym.html`

## aa_automation_bench

- **Status**: Verified
- **Metric**: `AutomationBenchAA`, a supplemental input to the correlated enterprise-workflow Plan composite.
- **Fixture**: `data/fixtures/aa_automation_bench.html`

## deep_swe_v1_1

- **Status**: Verified
- **API**: official DeepSWE v1.1 machine-readable leaderboard JSON
- **Secret**: None
- **Cache TTL**: 24 h
- **Metric**: `DeepSWE` pass@1 across 113 original long-horizon repository tasks.
- **Configuration**: fixed `mini-swe-agent` harness; max, xhigh, high, thinking/adaptive, medium/default, then low effort is preferred. Pass@4, confidence interval, attempts, task count, run count, and configuration provenance stay attached to the selected row.
- **Ranking use**: Supplemental Build evidence. It affects Build when present, but its narrow cohort cannot make an otherwise established model provisional.
- **Fixture**: `data/fixtures/deep_swe_v1_1.json`

## context_arena

- **Status**: Verified
- **API**: Context Arena `/api/needle-summary?needles=8`
- **Secret**: None
- **Cache TTL**: 24 h
- **Metrics**: `ContextArenaMRCR128k` (active inside `LongContextComposite`) and diagnostic `ContextArenaMRCR1M`.
- **Configuration**: one strongest published reasoning mode per model; AUC@128k is used for broad comparability rather than rewarding only models that expose a 1M window.

## agc_bench

- **Status**: Experimental
- **API**: official AGC-Bench Hugging Face leaderboard CSV
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `AGCBench` mean calibrated creativity z-score.
- **Ranking use**: Diagnostic while the newly released meta-benchmark and current-model coverage mature.

## factory_code_review

- **Status**: Experimental
- **API**: official Factory research results table
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: F1, precision, recall, and F1 standard deviation over 50 real PRs with human-curated bugs and repeated runs.
- **Ranking use**: Diagnostic until broader current max-effort coverage is available. Cost fields are intentionally not ingested.

## Removed sources

### aistupidlevel

Removed from active scoring on 2026-05-21. We reproduced the benchmark
locally and found the tasks not representative enough of real model quality
and too noise-prone for the role scores. The source implementation and
fixture remain in the repo for audit/history, but `AiStupidLevelSource` is
not registered, the `AI_*` metrics are no longer in `data/coefficients.toml`,
the `A_*` perspective groups are gone, and the canary-health penalty is no
longer applied.

### openevals

Removed because of zero overlap with the flagship model set — none of the 14 required
canonical IDs appeared in its leaderboard — so it contributed no coverage while adding
fetch latency.

### bigcodebench, aider_polyglot, and metr_horizons

These were removed during audit passes. `bigcodebench` (HuggingFace
dataset) stopped covering 2026-class models; `aider_polyglot` went stale
on the frontier; `metr_horizons` produced sparse measurements that warped
scores. BFCL remains ingested for diagnostics, but its sparse overlapping
cohort is not part of methodology-v3 scoring or eligibility.

## swebench

- **Status**: Verified
- **API**: SWE-bench leaderboards JSON (raw GitHub Pages source from `swe-bench/swe-bench.github.io`)
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `SWEBenchVerified` (Verified leaderboard), `SWEBenchMultilingual` (multilingual leaderboard, 9 languages incl. C/C++/Go/Java/JS/PHP/Ruby/Rust). Single fetch covers both — no extra HTTP cost.
- **Ranking use**: Multilingual remains a scored component of `SWEComposite`; Verified is retired from the current score but retained as direct historical Build/Review support.
- **Fixture**: `data/fixtures/swebench_leaderboards.json`

## terminal_bench

- **Status**: Verified
- **API**: Terminal-Bench 2.0 HTML leaderboard page
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `TerminalBench`; `TerminalBenchUncertainty` preserves the published ± value as an unscored auxiliary field.
- **Ranking use**: Retired from the current score in favor of Terminal-Bench 2.1, but retained as direct historical Plan/Build/Review support.
- **Fixture**: `data/fixtures/terminal_bench.html`

## terminal_bench_2_1

- **Status**: Verified
- **API**: Terminal-Bench 2.1 HTML leaderboard page
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `TerminalBench21` — newer, narrower Terminal-Bench track with current frontier agent/model combinations. `TerminalBench21Uncertainty` preserves the published ± value. The source canonicalizes duplicate agent rows to the best row per model. Scoring consumes the primary metric through `TerminalBench21Composite` together with AA's `AATerminalBench21`; uncertainty is unscored.
- **Fixture**: `data/fixtures/terminal_bench_2_1.html`

## livecodebench

- **Status**: Verified (retired from Build scoring)
- **API**: LiveCodeBench `performances_generation.json` (fetched from `livecodebench.github.io`)
- **Secret**: None
- **Cache TTL**: 2 d
- **Fixture**: `data/fixtures/livecodebench.json`
- **Note**: The upstream JSON has been frozen at mid-2025 frontier (latest entries are Claude-Opus-4 / Claude-Sonnet-4 / Gemini-2.5-Pro; no GPT-5/5.x, no Opus 4.5+, no Gemini 3, no Kimi K2.x, no DeepSeek V4) for ~12 months. The metric has no current score weight, but direct rows may provide historical Build/Review support. GSO was evaluated as a successor and is retained only as a diagnostic.

## gso

- **Status**: Verified
- **API**: GSO ("Generalized Software Optimization") leaderboard JSON at `gso-bench.github.io/assets/leaderboard.json`. Same operators as LiveCodeBench's leaderboard SPA but actively accepting frontier submissions where LiveCodeBench has not since mid-2025.
- **Secret**: None
- **Cache TTL**: 2 d
- **Fixture**: `data/fixtures/gso.json`
- **Metric**: `GSO` — pass rate over 102 software-optimization tasks. We ingest the contamination-resistant `score_hack_control` field and filter to `setting == "Opt@1"`. Among duplicate Opt@1 rows, max/pro, xhigh, high, thinking/adaptive, medium/default, then low effort is preferred, matching the best-available configuration policy.
- **Ranking use**: Diagnostic only. A July 2026 cross-machine audit found substantial reference-patch reproducibility problems, so GSO affects neither Build nor eligibility pending a corrected release.

## swerebench

- **Status**: Verified
- **API**: `swe-rebench.com` HTML page (Next.js server-rendered React Server Component blob; we extract the embedded `"items":[…]` array, unescape it, and parse with serde_json).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `SWERebench` — resolved rate over the newest, widest range whose start is on or after the model's release. Rows with no uncontaminated post-release range are omitted. Prefers the `tools` (agentic) variant per model and falls back to `text`. `SWERebenchSEM` preserves the selected row's standard error as an unscored auxiliary field.
- **Fragility note**: Depends on the embedded RSC payload format. If the site switches to client-side hydration or renames `items`/`modelName`/`rangeStats`/`taskRangeTimestamp`, the parser will need updating.
- **Fixture**: `data/fixtures/swerebench.html`

## swebench_pro

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/swe_bench_pro_public` (Next.js page; data is embedded in the streamed React Server Component chunks as `\"model\":\"…\",\"score\":N`).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `SWEBenchPro` — feeds the `SWEComposite` derived metric alongside `SWERebench` and `SWEBenchMultilingual`; retired Verified rows do not enter the composite. Frontier models top out near 60-65% (vs Verified saturating near 90), so it differentiates better at the top of the leaderboard. 1,865 multi-file tasks across 41 actively-maintained Python/Go/TypeScript/JavaScript repos; average edit is 107 LOC across 4.1 files.
- **Ranking use**: Supplemental Build evidence.
- **Fragility note**: Depends on Scale's RSC embedding. If field names change (`model` → `name`, `score` → `passRate`), the parser will need updating.
- **Fixture**: `data/fixtures/swebench_pro.html`

The three verified SWE Atlas sources need no secret, cache Scale's RSC pages
for seven days, and feed one supplemental `SWEAtlasComposite` Build input so
their correlated tracks do not stack as independent weights.

## sweatlas_qna

- **Metric**: `SWEAtlasQnA`, codebase question answering.
- **Fixture**: `data/fixtures/sweatlas_qna.html`

## sweatlas_test_writing

- **Metric**: `SWEAtlasTestWriting`, production-grade test writing.
- **Fixture**: `data/fixtures/sweatlas_test_writing.html`

## sweatlas_refactoring

- **Metric**: `SWEAtlasRefactoring`, repository refactoring.
- **Fixture**: `data/fixtures/sweatlas_refactoring.html`

## mcp_atlas

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/mcp_atlas` (same RSC pattern as `swebench_pro` — they share a parser).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `MCPAtlas` — pass rate over 1,000 tasks across 36 real Model Context Protocol servers / 220 tools. Each task asks the agent to identify the right servers, sequence 3-6 tool calls across multiple servers, and produce a correct end-state. Closest public proxy for "real Claude Code / Codex tool-use loops" we can ingest. Feeds both `PLAN` (multi-step tool sequencing) and `BUILD` (real coding agents *are* tool-orchestration loops).
- **Ranking use**: Supplemental current evidence. It affects Plan and Build scores when present without burdening eligibility when absent.
- **Coverage**: 19 models, all 14 flagships matched directly (opus-4.7 max=79.1%, gemini-3.1-pro=78.2%, glm-5.1=75.6%, gpt-5.4=70.6%, …, haiku-4.5=40.2%).
- **Fragility note**: Same as `swebench_pro` — RSC field names.
- **Fixture**: `data/fixtures/mcp_atlas.html`

## hil_bench

- **Status**: Verified
- **API**: Scale Labs `labs.scale.com/leaderboard/hil` (same RSC pattern as `mcp_atlas`).
- **Secret**: None
- **Cache TTL**: 7 d
- **Metric**: `HiLBench` — human-in-the-loop escalation accuracy. It measures whether an agent recognizes ambiguous or blocked tasks and asks targeted human questions instead of guessing.
- **Ranking use**: A 5% supplemental Plan signal only; it has no Build path.
- **Fixture**: `data/fixtures/hil_bench.html`

## bfcl

- **Status**: Verified
- **API**: Berkeley Function Calling Leaderboard V4 `data_overall.csv`
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: `BFCL` plus category splits `BFCLNonLiveAST`, `BFCLLive`, `BFCLMultiTurn`, `BFCLWebSearch`, `BFCLMemory`, `BFCLRelevanceDetection`, and `BFCLIrrelevanceDetection`. The diagnostic `BFCLComposite` contains only the upstream overall value, so the headline is not stacked with components from which it is derived.
- **Ranking use**: Diagnostic. Its current cohort is too narrow and overlaps MCP/tau tool-use evidence, so it affects neither scores nor eligibility pending a broader refresh.
- **Fixture**: `data/fixtures/bfcl.csv`

## arc_agi

- **Status**: Verified
- **API**: ARC Prize static JSON — `arcprize.org/media/data/models.json` + `evaluations.json`. Combined into one cached payload `{models, evaluations}`.
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: active `ARC_AGI_2` from **v2_Semi_Private**, plus diagnostic `ARC_AGI_3` from **v3_Semi_Private**. Scores are rescaled from 0–1 to 0–100. ARC-AGI-3 remains unweighted while tracked-model coverage is sparse and current scores are tightly floor-compressed.
- **Why we ingest it**: ARC-AGI v2 is the only public benchmark that explicitly tests *novel pattern induction* — every task is unfamiliar at evaluation time. Orthogonal to GPQA/HLE which test learned knowledge. Frontier models sit around 75-85% while humans top out at 100%, so it discriminates well at the very top.
- **Fragility note**: Depends on the static JSON URLs the leaderboard's bundled JS fetches. If ARC Prize moves the data-pack path, the constants need updating.
- **Fixture**: `data/fixtures/arc_agi.json`

## sonar

- **Status**: Verified
- **API**: `sonarsource.com/.../leaderboard/data/models.json` plus per-model metrics JSON files under `leaderboard/data/<org>/...`; no auth, no rate limit. Older snapshots used a single flat `leaderboard/data.json`, and the fetcher still reads that cached shape for back-compat.
- **Secret**: None
- **Cache TTL**: 7 d
- **Metrics**: Diagnostic-only `SonarFunctionalSkill` (pass rate, higher better), `SonarIssueDensity` (issues per kLOC, lower better), `SonarBugDensity`, and `SonarVulnerabilityDensity`. `SonarComposite` combines functional skill and total issue density for inspection; it has no rank path while the published cohort mixes explicit effort levels. Bug and vulnerability density remain descriptive because they are nested within total issue density. A legitimate issue density of zero is retained.
- **Coverage**: 70 Java rows in the 2026-05-21 live payload, including Opus 4.5/4.6/4.7 Thinking/High variants, GPT-5.2/5.3-Codex/5.4/5.5 variants, Gemini 3 Pro/Flash/3.1 Pro, GLM-5, Kimi K2.5, and MiniMax M2.5/M2.7.
- **Fixture**: `data/fixtures/sonar.json`

## overrides

- **Status**: Verified
- **API**: None — reads `data/score_overrides.toml` (embedded into the binary at build time).
- **Purpose**: Hand-curated metric values pulled from vendor system cards, launch posts, and other authoritative secondary sources. Fills coverage gaps for models that public leaderboards have not yet rated (typically newest frontier models — e.g. Claude Opus 4.7 SWE-bench Verified, GPT-5.5 Terminal-Bench 2.0, Kimi K2.7 Code launch-card metrics).
- **Discipline**: Every entry must cite its source in the `note` field. Schema 2.0 publishes the winning note as `citation` in the metric-evidence table.
- **Precedence and reliability**: Reported overrides beat synthesis but never replace a direct source, independent of ingest order. Their normalized deviation from 50 is multiplied by the default reported reliability of 0.60.

## Synthesis

`data/synthesis_aliases.toml` is the authoritative list of sibling-substitution pairs and rationale. Pairs may apply across sources or be restricted to one narrow leaderboard.

Synthesis is field-level and fill-only. A synthesized row carries the donor ID and category, but ingestion skips any metric for which the target already has a direct or reported observation. Observation-specific metadata such as confidence intervals, standard errors, and evidence notes is never transferred.

Evidence precedence is order-independent:

```text
direct > reported override > synthesized
```

Synthesis categories control reliability, not a hidden claim that the value was directly measured:

| Category | Default reliability |
|---|---:|
| `conservative` | 0.00 (prior-only) |
| `same_series_forward` | 0.00 (prior-only) |
| `stronger_successor` | 0.00 (prior-only) |

For normalized value `N`, the scored observation is `50 + reliability × (N - 50)`. Synthesized evidence does not count toward direct-family coverage and cannot make a provisional role ranked. Historical support likewise requires a direct observation; a reported override or sibling fill cannot establish it.

The configured per-source cap limits emitted synthetic rows. After scoring, `synthesis_dominant` is computed from weighted role paths and becomes true when any role's synthesized share exceeds the configured per-model cap. Schema 2.1 publishes each synthesized metric's source, donor, category, and evidence coverage.
