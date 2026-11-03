# Model roster

The active catalog lives in [`required_aliases.toml`](../data/required_aliases.toml).
Admit a product when its available same-product observations qualify it for at
least one ranked role under the existing [evidence gates](methodology.md).
Keep older products that still meet that rule. Before retiring a product with
no ranked roles, compare successive published snapshots and check source health;
a missing local API key or a failed refresh is insufficient retirement evidence.

Releases, dated snapshots, preview products, and premium variants retain separate
identities. OpenRouter availability supplies deployment metadata; it does not
establish benchmark coverage. Ranking weights and thresholds do not change with
a catalog refresh.

## 2026-11-03 audit

The audit considered 21 additions and admitted 19. Five existing products were
retired, taking the active catalog from 81 to 95 models.

Retirement checks used the published scoreboards generated at
`2026-10-02T06:58:29.942026895Z` and `2026-11-03T19:37:03.341336626Z`.
The November snapshot came from the first successful
[refresh after the CI repair](https://github.com/dotkrnl/ipbr-rank/actions/runs/41704652029),
with live Artificial Analysis and Arena responses. AA Omniscience and GDP.pdf
used valid cached observations; the two official Terminal-Bench feeds were
unavailable. The same five products had no ranked role in both snapshots.

Candidate checks used freshly fetched source caches, including a completed
Arena download, and the unchanged scoring pipeline. The local audit had no AA
API key, so the [public AA model leaderboard](https://artificialanalysis.ai/leaderboards/models)
supplied its published observations for the existing API adapter's equivalent
benchmark fields: GPQA, HLE, long-context reasoning, SciCode, and the older
historical evaluations. New benchmark versions were not substituted for older
ones. These audit-only response conversions are not shipped as source caches or
manual score overrides. The production refresh uses its configured AA API key.

Each admitted model has at least the following qualifying role in the audit.
Coverage is the fraction of that role's current weighted evidence; core coverage
is normalized within the role's core portfolio. Families count benchmark
constructs, not hosting domains. I means Idea; P means Planning.

| Product | Role | Qualification | Direct | Families | Core direct | Core families |
|---|---|---|---:|---:|---:|---:|
| anthropic/claude-haiku-5.5 | P | breadth | 54.6% | 8 | 74.7% | 7 |
| anthropic/claude-sonnet-5.5 | P | breadth | 54.6% | 8 | 74.7% | 7 |
| deepseek/deepseek-v4.1-flash-1017 | P | breadth | 42.3% | 7 | 57.3% | 6 |
| google/gemini-3.7-flash | I | standard | 86.1% | 3 | 86.1% | 3 |
| google/gemini-4-argon | P | breadth | 36.9% | 6 | 45.9% | 4 |
| google/gemini-4-carbon | P | breadth | 38.0% | 6 | 51.0% | 5 |
| ifm/k2-horizon-375b-a23b | P | breadth | 45.6% | 7 | 61.9% | 6 |
| inclusionai/ling-3.1-flash-1019 | P | core_breadth | 31.8% | 6 | 42.3% | 5 |
| meta/muse-spark-open | P | breadth | 47.0% | 8 | 60.2% | 6 |
| meta/muse-wonder | P | breadth | 46.7% | 8 | 59.7% | 6 |
| meta/muse-wonder-full | P | core_breadth | 31.8% | 6 | 42.3% | 5 |
| minimax/minimax-m3.1-flash | P | core_breadth | 31.8% | 6 | 42.3% | 5 |
| moonshotai/kimi-k3.1-pro | P | core_breadth | 31.8% | 6 | 42.3% | 5 |
| nvidia/nemotron-3.5-super-vl | P | breadth | 54.6% | 8 | 74.7% | 7 |
| openai/gpt-6.1-sol | P | breadth | 57.2% | 9 | 74.7% | 7 |
| openai/gpt-6.1-terra | P | core_breadth | 31.8% | 6 | 42.3% | 5 |
| reflection/beam-1.0 | P | breadth | 54.6% | 8 | 74.7% | 7 |
| stepfun/step-5-preview | P | breadth | 47.9% | 7 | 59.7% | 6 |
| thinkingmachines/inkling-small | P | standard | 76.9% | 9 | 73.7% | 6 |

### Retirements and held candidates

The following products had four provisional roles in both published snapshots:

- `google/gemini-2.5-flash`
- `moonshotai/kimi-k2-0905`
- `openai/gpt-5.2-codex`
- `openai/gpt-5.3-codex`
- `xai/grok-code-fast-1`

Their aliases and eight manual observations were removed from the active inputs.
Git history retains those observations and their citations. Regression tests
ensure their upstream names do not get reassigned to another active product.

Mistral Large 4 and MiMo V2.6 Flash remain outside the roster because neither
qualified for a role in the available source pipeline. Their Planning direct
coverage was 28.7% across five families and 21.8% across three families,
respectively; their remaining core and historical evidence did not satisfy an
alternative qualification path. Revisit them when the registered feeds expand
coverage.

### Identity checks

- [OpenRouter's model catalog](https://openrouter.ai/api/v1/models) confirms the
  dated routes used in the aliases. GPT-6.1 Pro routes remain separate from the
  base Sol and Terra products.
- [Meta documents Muse Wonder Full as a separate product](https://dev.meta.ai/models/muse-wonder).
  Its observations stay separate from Muse Wonder. Muse Spark Open also stays
  separate from Muse Spark. Fuzzy matching protects both `full` and `open` labels.
- DeepSeek V4.1 Flash 1017 uses the October 17 snapshot; the September 10 base
  record remains separate. Ling 3.1 Flash 1019 does not absorb OpenRouter's
  October 2 Ling 3.1 Flash route.
- AA's [`step-5` slug](https://artificialanalysis.ai/models/step-5) identifies
  Step 5 Preview; the canonical ID preserves that lifecycle label. AA's
  [`reflection-beam` slug](https://artificialanalysis.ai/models/reflection-beam)
  identifies Beam 1.0 501B A23B.
- [IFM identifies K2 Horizon 375B A23B as its own product](https://ifm.ai/blog/k2/).
  It does not inherit observations from OpenRouter's anonymous Horizon routes.

For a future audit, run `triage` against refreshed caches to discover source
names and audit identities. Rebuild after editing the catalog so source-level
identity handling uses the proposed aliases, then run `score`. Inspect
`scores.*_status` and `evidence.roles.*` in `scoreboard.toml`, and verify the
admitted products again in the published refresh. The deterministic fixture
golden tests pipeline behavior; live coverage determines roster admission.
