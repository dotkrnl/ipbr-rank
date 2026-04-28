use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "livecodebench";
const CACHE_KEY: &str = "livecodebench";
/// Discovered from `https://livecodebench.github.io/leaderboard.html`: the SPA's
/// `DEFAULT_DATASET` is fetched as `performances_generation.json`.
const URL: &str = "https://livecodebench.github.io/performances_generation.json";

#[derive(Debug, Default, Clone, Copy)]
pub struct LiveCodeBenchSource;

#[async_trait::async_trait]
impl Source for LiveCodeBenchSource {
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
        Duration::from_secs(2 * 24 * 3600)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let payload = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
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
        parse_rows(&payload)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("LiveCodeBench payload missing models[]".into()))?;
    let performances = payload
        .get("performances")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("LiveCodeBench payload missing performances[]".into()))?;

    let mut totals: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for perf in performances {
        let Some(model) = perf.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(pass_at_1) = perf.get("pass@1").and_then(number_like) else {
            continue;
        };
        if !pass_at_1.is_finite() {
            continue;
        }
        let entry = totals.entry(model).or_default();
        entry.0 += pass_at_1;
        entry.1 += 1;
    }

    let mut rows = Vec::new();
    for model in models {
        let model_key = match model.get("model_repr").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => value.trim(),
            _ => continue,
        };
        let Some((total, count)) = totals.get(model_key) else {
            continue;
        };
        if *count == 0 {
            continue;
        }
        let pass_at_1 = total / *count as f64;

        let mut fields = BTreeMap::new();
        fields.insert(
            "LiveCodeBench".to_string(),
            serde_json::Value::from(pass_at_1),
        );
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_key.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }
    Ok(rows)
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_livecodebench_fixture() {
        let payload: Value =
            serde_json::from_str(include_str!("../../../data/fixtures/livecodebench.json"))
                .expect("fixture should parse as JSON");
        let rows = parse_rows(&payload).expect("fixture should parse");
        assert!(rows.len() >= 10, "expected >=10 rows, got {}", rows.len());
        assert!(rows.iter().all(|r| r.fields.contains_key("LiveCodeBench")));
        assert!(rows.iter().any(|r| r.model_name.contains("Claude-Opus-4")));
    }
}
