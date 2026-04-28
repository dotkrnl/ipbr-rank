# Adding a Source

This document describes the verification protocol and implementation checklist for adding a new benchmark source to ipbr-rank.

> **Note:** Some snippets below reference patterns that have since been simplified — the `html-sources` feature flag was retired (HTML sources are always built), `include_experimental` was removed from the registry signature, and the `Experimental` status is no longer used in production (everything we ingest is `Verified`). Use this doc as a structural guide; refer to existing source modules in `crates/sources/src/` (e.g. `sonar.rs`, `swerebench.rs`, `artificial_analysis.rs`) for current patterns.

---

## Verification Protocol

Every source must pass two gates before being marked `verified = true`:

1. **Fixture-based contract test** — Capture a live response, write a test that parses it without panics and recognizes ≥N expected models.
2. **Live smoke test** — At least one successful fetch in CI against the real endpoint, with the date recorded in `docs/sources.md`.

Sources that fail verification stay in the registry as `experimental` and do not gate the build.

---

## Implementation Checklist

### 1. Define the Source Struct

Create a new file `crates/sources/src/your_source.rs`:

```rust
use crate::{Http, RawRow, SecretRef, SecretStore, Source, VerificationStatus, FetchOptions};
use anyhow::{Context, Result};

pub struct YourSource;

impl Source for YourSource {
    fn id(&self) -> &'static str {
        "your_source"
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Experimental  // Start as Experimental
    }

    fn required_secret(&self) -> Option<SecretRef> {
        None  // or Some(SecretRef::YourApiKey) if auth is needed
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        secrets: &SecretStore,
    ) -> Result<Vec<RawRow>> {
        // Implementation here
        todo!()
    }
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

In `your_source.rs`:

```rust
fn cache_ttl(&self) -> std::time::Duration {
    // How long the cache stays fresh when --cache is set without --offline.
    // Pick based on upstream refresh cadence: 1h for hourly dashboards,
    // 24h for daily indexes, 2-7d for weekly/monthly leaderboards.
    std::time::Duration::from_secs(24 * 3600)
}

async fn fetch(
    &self,
    http: &dyn Http,
    opts: FetchOptions<'_>,
    _secrets: &SecretStore,
) -> Result<Vec<RawRow>> {
    // `use_cached_json` returns true when --offline is set OR when --cache
    // contains a file younger than `cache_ttl()`. It's the single gate that
    // decides "cache vs network".
    let payload = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
        let dir = opts.cache_dir.context("offline mode requires --cache")?;
        serde_json::from_slice::<Value>(&read_cached_bytes(&cache_json_path(
            dir,
            self.cache_key(),
        ))?)?
    } else {
        let url = "https://example.com/api/your_endpoint";
        let response = http.get_json(url, &[]).await.context("fetch failed")?;
        if let Some(cache_dir) = opts.cache_dir {
            write_cache_json(cache_dir, self.cache_key(), &response)?;
        }
        response
    };
    parse_response(&payload)
}
```

The `Http` trait already retries `429`/`5xx` with exponential backoff and
respects `Retry-After`, so paginated fetches against rate-limited endpoints
(HuggingFace datasets-server, etc.) don't need per-source retry logic.

```rust

fn parse_response(body: &str) -> Result<Vec<RawRow>> {
    let data: YourResponseType = serde_json::from_str(body)?;
    
    let mut rows = Vec::new();
    for item in data.items {
        let mut fields = serde_json::Map::new();
        fields.insert("metric_name".to_string(), json!(item.score));
        // ... populate other fields
        
        rows.push(RawRow {
            source_id: "your_source".to_string(),
            model_name: item.name.clone(),
            vendor_hint: Some(item.vendor.clone()),
            fields: serde_json::Value::Object(fields),
        });
    }
    
    Ok(rows)
}
```

**For HTML sources**:
- Use the `scraper` crate for parsing (always available — the `html-sources` feature flag was retired)
- See `crates/sources/src/terminal_bench.rs` or `swerebench.rs` for examples

### 4. Write a Contract Test

In `crates/sources/tests/your_source_test.rs`:

```rust
use ipbr_sources::{YourSource, Source, FetchOptions, ReqwestHttp, SecretStore};

#[tokio::test]
async fn test_your_source_fixture() {
    let source = YourSource;
    let http = ReqwestHttp::default();
    let secrets = SecretStore::empty();
    
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
    assert!(!first.fields.is_null(), "fields must be populated");
    
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
- Row drop rate is acceptable (≤20%)

### 5. Register the Source

In `crates/sources/src/lib.rs`:

```rust
mod your_source;
pub use your_source::YourSource;
```

In `crates/sources/src/registry.rs`:

```rust
use crate::{YourSource, ...};

pub fn registry(include_experimental: bool) -> Vec<Box<dyn Source>> {
    let mut sources: Vec<Box<dyn Source>> = vec![
        // ... existing sources
        Box::new(YourSource),
    ];
    // ...
}
```

### 6. Document the Source

Add a section to `docs/sources.md`:

```markdown
## your_source

- **Status**: Experimental
- **API**: Your Source Name endpoint description
- **Secret**: `YOUR_API_KEY` (via `--your-api-key-file` or environment variable) / None
- **Fixture**: `data/fixtures/your_source.json`
- **Metrics contributed**: MetricName1, MetricName2
- **Last verified**: Not yet verified

Description of what this source provides and any caveats.
```

### 7. Run Tests Locally

```bash
# Contract test against fixture
cargo test --package ipbr-sources your_source

# Full integration (will use cached fixture)
cargo test --workspace

# Live smoke (requires network + secrets)
cargo test --package ipbr-sources your_source -- --ignored
```

### 8. Mark as Verified (After CI Success)

Once the live smoke test passes in CI:

1. Update `docs/sources.md` with the verification date and CI run link
2. Change `status()` in `your_source.rs`:
   ```rust
   fn status(&self) -> VerificationStatus {
       VerificationStatus::Verified
   }
   ```
3. Commit with message: `feat(sources): mark your_source as verified`

---

## Adding New Metrics

If your source contributes new metrics not in `data/coefficients.toml`:

1. Add the metric definition to `[metrics.*]` in `data/coefficients.toml`:
   ```toml
   [metrics.YourNewMetric]
   higher_better = true
   log_scale = false
   groups = ["BUILD"]  # or whichever groups this metric belongs to
   transform = "direct"  # or "percentile"
   ```

2. Add the metric to the appropriate group weights:
   ```toml
   [group_weights.BUILD]
   # ... existing metrics
   YourNewMetric = 0.05  # adjust other weights to sum to 1.0
   ```

3. Run the coefficient validation test:
   ```bash
   cargo test --package ipbr-core test_weights_sum_to_one
   ```

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

## HTML Sources (Feature-Gated)

For HTML-based sources (web scraping):

1. **Feature gate the module**:
   ```rust
   #[cfg(feature = "html-sources")]
   mod your_html_source;
   
   #[cfg(feature = "html-sources")]
   pub use your_html_source::YourHtmlSource;
   ```

2. **Register only when feature is enabled**:
   ```rust
   #[cfg(feature = "html-sources")]
   fn html_sources(sources: &mut Vec<Box<dyn Source>>, include_experimental: bool) {
       if include_experimental {
           sources.push(Box::new(crate::YourHtmlSource));
       }
   }
   ```

3. **Use `scraper` for parsing** (see `bfcl.rs` example)

4. **Document ToS compliance** in `docs/sources.md`

5. **Mark as Experimental** — HTML sources are fragile and should remain experimental unless the site has a stable structure and explicit permission for scraping.

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

**Q: My source is verified but still shows as experimental**  
A: Verify you changed `status()` to return `VerificationStatus::Verified` and that the change is committed.

**Q: HTML source isn't compiling**  
A: Ensure `#[cfg(feature = "html-sources")]` is on the module and that you're running `cargo build --features html-sources`.

---

## Example: Minimal JSON Source

```rust
use crate::{Http, RawRow, Source, VerificationStatus, FetchOptions, SecretStore};
use anyhow::{Context, Result};
use serde::Deserialize;

pub struct MinimalSource;

#[derive(Deserialize)]
struct ApiResponse {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    name: String,
    score: f64,
}

impl Source for MinimalSource {
    fn id(&self) -> &'static str {
        "minimal"
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Experimental
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>> {
        let body = if opts.offline {
            let path = opts.cache_dir.context("offline needs cache")?.join("minimal.json");
            std::fs::read_to_string(&path)?
        } else {
            let resp = http.get("https://example.com/api/models").await?;
            if let Some(cache) = opts.cache_dir {
                let _ = std::fs::write(cache.join("minimal.json"), &resp);
            }
            resp
        };

        let data: ApiResponse = serde_json::from_str(&body)?;
        Ok(data
            .models
            .into_iter()
            .map(|m| RawRow {
                source_id: "minimal".to_string(),
                model_name: m.name,
                vendor_hint: None,
                fields: serde_json::json!({ "SomeMetric": m.score }),
            })
            .collect())
    }
}
```

---

End of Adding a Source guide.
