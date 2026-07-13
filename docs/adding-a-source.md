# Adding a Source

This document describes the verification protocol and implementation checklist for adding a new benchmark source to ipbr-rank.

Most registered sources are `Verified`. A small number of newly introduced,
diagnostic-only sources may remain `Experimental` while their methodology or
coverage matures; a source that is not ready is simply left out of the
registry. Verification status describes source/parser confidence, not whether a
metric affects ranking — see `crates/sources/src/registry.rs` and
`docs/sources.md`.
Refer to existing source modules (`sonar.rs`, `swerebench.rs`,
`artificial_analysis.rs`, `terminal_bench.rs`) for current patterns.

---

## Verification Protocol

Every source must pass two gates before it is added to the registry:

1. **Fixture-based contract test** — Capture a live response, write a test that parses it without panics and recognizes ≥N expected models.
2. **Live smoke test** — At least one successful fetch against the real endpoint, locally or through `ipbr-rank verify-sources` with any required secrets.

---

## Implementation Checklist

### 1. Define the Source

Create a new file `crates/sources/src/your_source.rs`:

```rust
use std::{collections::BTreeMap, time::Duration};

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus,
    cache_json_path, read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "your_source";
const CACHE_KEY: &str = "your_source";
const URL: &str = "https://example.com/api/your_endpoint";

#[derive(Debug, Default, Clone, Copy)]
pub struct YourSource;

#[async_trait::async_trait]
impl Source for YourSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let payload = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let dir = opts.cache_dir.ok_or_else(|| {
                SourceError::CacheMiss(format!("{} requires --cache", self.id()))
            })?;
            serde_json::from_slice::<Value>(&read_cached_bytes(&cache_json_path(
                dir,
                self.cache_key(),
            ))?)?
        } else {
            let payload = http.get_json(URL, &[]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };

        parse_response(&payload)
    }
}

fn parse_response(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("your_source payload missing models[]".into()))?;

    let mut rows = Vec::new();
    for item in models {
        let Some(model_name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(score) = item.get("score").and_then(Value::as_f64) else {
            continue;
        };

        let mut fields = BTreeMap::new();
        fields.insert("YourMetric".to_string(), Value::from(score));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: item
                .get("vendor")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            fields,
        });
    }

    Ok(rows)
}
```

### 2. Capture a Fixture

Fetch a live response and save it to `data/fixtures/your_source.json` (or `.html` for HTML sources):

```bash
# For JSON sources
curl -H "Authorization: Bearer $YOUR_API_KEY" \
  https://example.com/api/your_endpoint \
  > data/fixtures/your_source.json

# For HTML sources
curl https://example.com/leaderboard.html \
  > data/fixtures/your_source.html
```

**IMPORTANT**: Review the fixture to ensure it contains no secrets, PII, or ToS-violating data before committing.

### 3. Implement the Parser

The complete example above uses the current `Http` trait, cache helpers,
`SourceError`, and `RawRow` shape. Pick `cache_ttl()` from the upstream refresh
cadence: roughly one hour for hourly dashboards, 24 hours for daily indexes,
or two to seven days for weekly/monthly leaderboards.

The `Http` trait already retries `429`/`5xx` with exponential backoff and
respects `Retry-After`, so paginated fetches against rate-limited endpoints
(HuggingFace datasets-server, etc.) don't need per-source retry logic.

**For HTML sources**:
- Use the `scraper` crate for parsing (already a dependency).
- See `crates/sources/src/terminal_bench.rs` or `swerebench.rs` for examples.
- Override `cache_paths()` and use `use_cached_html` / `cache_html_path` so the cache lookup hits the right extension.

For JavaScript assets with embedded data, use the `.js` cache helpers and
extract the specifically named payload rather than scraping unrelated script
text. `eqbench.rs` is the reference implementation.

Deduplicate on stable model identity, lifecycle/revision, and effort tier; do
not collapse all effort variants to one row before core ingestion. The global
policy then selects max, xhigh, high, thinking/adaptive, medium, and default in
that order. Explicit `minimal` is a Low tier and is not scoreable by default.
`instant` is identity-bearing for products such as GPT-5.5 Instant, so treat it
as Low only when the source documents that it is an effort mode rather than a
separate product. If a source publishes agent/harness submissions instead of
comparable effort variants, document its best-row rule in `docs/sources.md` and
attach the winning submission label to `<Metric>__evidence_note`. Core ingestion
removes that reserved field from numeric/effort processing and emits it as the
direct metric's citation.

Do not assume `preview` or `latest` is a harmless suffix. Preview builds and
moving aliases require an explicit entry in `data/required_aliases.toml`; a
route that changed over time must be resolved in the source parser using its
published date/version metadata, or left unmatched when that context is not
available.

Preserve published uncertainty as auxiliary numeric fields (`*Uncertainty`,
`*SEM`, or `*CILow`/`*CIHigh`). Auxiliary uncertainty is unscored; it belongs
to the measured observation.

### 4. Write a Contract Test

In `crates/sources/tests/your_source_test.rs`:

```rust
use ipbr_sources::{YourSource, Source, FetchOptions, ReqwestHttp, SecretStore};

#[tokio::test]
async fn test_your_source_fixture() {
    let source = YourSource;
    let http = ReqwestHttp::default();
    let secrets = SecretStore::default();

    let cache_dir = std::path::PathBuf::from("../../data/fixtures");
    let rows = source
        .fetch(
            &http,
            FetchOptions {
                cache_dir: Some(&cache_dir),
                offline: true,
            },
            &secrets,
        )
        .await
        .expect("fixture parse failed");

    // Contract assertions
    assert!(rows.len() >= 10, "expected at least 10 models, got {}", rows.len());

    let first = &rows[0];
    assert!(!first.model_name.is_empty(), "model_name must not be empty");
    assert!(!first.fields.is_empty(), "fields must be populated");

    // Check expected models are recognized
    let model_names: Vec<&str> = rows.iter().map(|r| r.model_name.as_str()).collect();
    assert!(model_names.contains(&"expected-model-name"),
            "fixture must include expected-model-name");
}
```

**Contract assertions should check**:
- Minimum number of rows parsed (≥N where N is reasonable for the source)
- No panics during parsing
- Expected fields are present and parseable
- At least one known model is recognized
- Parsed-row and canonical-match coverage are reported and justified for the source
- Duplicate effort/agent rows collapse deterministically to the documented winner
- Uncertainty bounds stay attached to the selected row, when available

### 5. Register the Source

In `crates/sources/src/lib.rs`:

```rust
mod your_source;
pub use your_source::YourSource;
```

In `crates/sources/src/registry.rs`:

```rust
use crate::{YourSource, ...};

pub fn registry() -> Vec<Box<dyn Source>> {
    vec![
        // ... existing sources
        Box::new(YourSource),
    ]
}
```

### 6. Document the Source

Add a section to `docs/sources.md`:

```markdown
## your_source

- **Status**: Verified
- **API**: Your Source Name endpoint description
- **Secret**: `YOUR_API_KEY` (via `--your-api-key-file` or environment variable) / None
- **Cache TTL**: 24 h
- **Fixture**: `data/fixtures/your_source.json`
- **Metrics emitted**: MetricName1, MetricName2

Description of what this source provides and any caveats.
```

### 7. Run Tests Locally

```bash
# Contract test against fixture
cargo test --package ipbr-rank-sources your_source

# Full integration (will use cached fixture)
cargo test --workspace

# Live smoke (requires network and any source-specific secrets)
cargo run -p ipbr-rank-cli -- --cache /tmp/ipbr-live-cache --only your_source verify-sources
```

---

## Adding New Metrics

If your source contributes new metrics not in `data/coefficients.toml`:

1. Add the metric definition to `[metrics.*]` in `data/coefficients.toml`:
   ```toml
   [metrics.YourNewMetric]
   family = "your_source_family"
   eligibility = "core"
   higher_better = true
   log_scale = false
   groups = ["BUILD"]  # or whichever groups this metric belongs to
   transform = "percentile"  # fallback for explicitly unanchored diagnostics
   anchor_low = 10.0
   anchor_high = 90.0
   ```

   Active scored leaves in methodology v3 require fixed raw-unit anchors.
   Derive them from a frozen direct-evidence reference snapshot and review
   them as part of the methodology/coefficient version. The logistic scorer
   maps low/high anchors to approximately 5/95. Do not derive anchors from the
   live cohort at scoring time.

   `family` identifies correlated observations for the role-level source-family
   cap. Reuse a family for metrics from the same benchmark suite or aggregate
   source; do not create a new family merely to evade the cap.

   Choose the eligibility class independently from score weight:

   - `core` for broad, role-representative current benchmarks;
   - `supplemental` for useful but narrow current benchmarks whose missing rows
     must not make established models provisional; or
   - `historical_support` for retired direct evidence with no current score
     path.

   Do not classify a source as core merely to make more models qualify. Record
   cohort coverage, construct relevance, refresh cadence, and overlap with
   existing families in `docs/sources.md`. Historical support must identify its
   role relevance explicitly and must remain unreachable from group weights.

2. Add the metric to the appropriate group weights:
   ```toml
   [group_weights.BUILD]
   # ... existing metrics
   YourNewMetric = 0.05  # adjust other weights to sum to 1.0
   ```

3. Run the coefficient validation test:
   ```bash
   cargo test --package ipbr-rank-core
   ```

4. Confirm the new metric has an intentional score and eligibility path.
   Missing weight is tracked separately from capability, and every actual
   same-product observation counts as direct (including cited manual overrides).
   A metric with no final role path is a diagnostic unless it is explicitly
   declared as coverage-only historical support. Run eligibility tests with at
   least one established model and one
   genuinely under-covered model; a new narrow source should not flip either
   status merely because its row is absent.

---

## Secret Handling

If your source requires an API key:

### 1. Define the Secret Reference

In `crates/sources/src/lib.rs`:

```rust
pub enum SecretRef {
    // ... existing variants
    YourApiKey,
}
```

### 2. Update SecretStore

In `crates/sources/src/lib.rs`:

```rust
pub struct SecretStore {
    // ... existing fields
    your_api_key: Option<String>,
}

impl SecretStore {
    pub fn new(
        aa_api_key: Option<String>,
        openrouter_api_key: Option<String>,
        hf_token: Option<String>,
        your_api_key: Option<String>,  // Add parameter
    ) -> Self {
        Self {
            aa_api_key,
            openrouter_api_key,
            hf_token,
            your_api_key,
        }
    }

    pub fn get(&self, secret: SecretRef) -> Option<&str> {
        match secret {
            // ... existing cases
            SecretRef::YourApiKey => self.your_api_key.as_deref(),
        }
    }
}
```

### 3. Update CLI

In `crates/cli/src/main.rs`:

```rust
#[derive(Parser)]
struct Cli {
    // ... existing args
    #[arg(global = true, long)]
    your_api_key_file: Option<PathBuf>,
}

fn resolve_secrets(cli: &Cli) -> anyhow::Result<SecretStore> {
    // ... existing secret resolution
    let your_api_key = resolve_secret("YOUR_API_KEY", cli.your_api_key_file.as_deref())?;
    Ok(SecretStore::new(
        aa_api_key,
        openrouter_api_key,
        hf_token,
        your_api_key,
    ))
}

fn secret_env_name(secret: SecretRef) -> &'static str {
    match secret {
        // ... existing cases
        SecretRef::YourApiKey => "YOUR_API_KEY",
    }
}
```

---

## CI Integration

Once your source is verified, ensure it's covered by CI:

1. **Contract test** — runs on every PR against the fixture
2. **Live smoke** — runs in scheduled CI job, non-gating
3. **Doc-source consistency** — the `scripts/check-docs.sh` ensures every registered source has a section in `docs/sources.md`

---

## Troubleshooting

**Q: My source returns 0 rows but doesn't error**
A: Check the parser logic. The contract test should fail if `rows.len() < expected_minimum`.

**Q: The alias matcher isn't recognizing my models**
A: Check `data/required_aliases.toml` — if the models are new, you may need to add canonical IDs. The unmatched models are logged as warnings at runtime.

**Q: My HTML source's cache file isn't being used**
A: Override `cache_paths()` so it points at the `.html` extension, and gate the cache lookup with `use_cached_html` instead of `use_cached_json`.

---

## Minimal JSON Source

Sections 1–3 contain a complete minimal JSON source using the current trait,
cache, error, and row APIs. Use a nearby production module with the same
transport (`context_arena.rs` for JSON, `terminal_bench.rs` for HTML, or
`eqbench.rs` for JavaScript) when adding source-specific validation.

---

End of Adding a Source guide.
